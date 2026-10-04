use serde::{Deserialize, Serialize};
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::crypto::{decrypt_message, encrypt_message};
use super::package::PlainItem;
use super::ShareError;

/// LAN wire protocol version. Bumped to 2 with the SPAKE2 pairing; v1.0.6
/// peers (version 1) are rejected with an "update the other device" error.
pub const PROTOCOL_VERSION: u8 = 2;

/// All messages exchanged over the TCP share channel.
///
/// Handshake messages (Hello, Pake, KeyConfirm) are sent as plaintext JSON with a 4-byte
/// LE length prefix. Post-handshake messages (Confirm, Items, Ack, Error) are AES-256-GCM
/// encrypted with the session key, with the same 4-byte LE prefix framing the encrypted payload.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum ShareMessage {
    Hello {
        version: u8,
    },
    /// SPAKE2 message, hex encoded.
    Pake {
        msg_hex: String,
    },
    /// Key-confirmation MAC, hex encoded.
    KeyConfirm {
        mac_hex: String,
    },
    Confirm {
        accepted: bool,
    },
    Items {
        items: Vec<PlainItem>,
    },
    Ack {
        received: usize,
    },
    Error {
        code: String,
        message: String,
    },
}

const MAX_FRAME: usize = 16 * 1024 * 1024;
/// Handshake frames are tiny; cap them so an unauthenticated peer cannot make us allocate much.
const MAX_PLAIN_FRAME: usize = 64 * 1024;
/// Granularity at which blocked reads re-check the deadline and cancel flag.
const POLL_SLICE: Duration = Duration::from_millis(200);

// ─── Bounded waiting ──────────────────────────────────────────────────────────

/// Bounds for blocking socket reads: no progress for `idle` aborts, so does passing
/// `deadline`, and so does the `cancel` flag (set by `cancel_session`).
#[derive(Clone)]
pub struct IoGuard {
    pub idle: Duration,
    pub deadline: Option<Instant>,
    pub cancel: Option<Arc<AtomicBool>>,
}

impl IoGuard {
    pub fn new(idle: Duration) -> Self {
        IoGuard { idle, deadline: None, cancel: None }
    }

    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = Some(deadline);
        self
    }

    pub fn with_cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// Err when the read must stop, given when the last progress happened.
    fn check(&self, last_progress: Instant) -> Result<(), ShareError> {
        if self.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(ShareError::Cancelled);
        }
        if self.deadline.is_some_and(|d| Instant::now() >= d) || last_progress.elapsed() >= self.idle {
            return Err(ShareError::Timeout);
        }
        Ok(())
    }
}

/// Set the write timeout and a short read timeout slice used by the guarded reads.
pub fn configure_stream(stream: &TcpStream, idle: Duration) -> Result<(), ShareError> {
    stream
        .set_write_timeout(Some(idle))
        .map_err(|e| ShareError::Io(e.to_string()))?;
    stream
        .set_read_timeout(Some(POLL_SLICE))
        .map_err(|e| ShareError::Io(e.to_string()))
}

fn is_would_block(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted)
}

fn read_full(stream: &mut TcpStream, buf: &mut [u8], guard: &IoGuard) -> Result<(), ShareError> {
    let mut filled = 0;
    let mut last_progress = Instant::now();
    while filled < buf.len() {
        guard.check(last_progress)?;
        match stream.read(&mut buf[filled..]) {
            Ok(0) => return Err(ShareError::Io("connection closed by peer".into())),
            Ok(n) => {
                filled += n;
                last_progress = Instant::now();
            }
            Err(e) if is_would_block(&e) => continue,
            Err(e) => return Err(ShareError::Io(e.to_string())),
        }
    }
    Ok(())
}

/// Block until the peer has sent at least one byte (without consuming it), subject to `guard`.
/// Used to wait for the sender to start transmitting after the user confirmed the fingerprint.
pub fn wait_readable(stream: &TcpStream, guard: &IoGuard) -> Result<(), ShareError> {
    let mut probe = [0u8; 1];
    let start = Instant::now();
    loop {
        guard.check(start)?;
        match stream.peek(&mut probe) {
            Ok(0) => return Err(ShareError::Io("connection closed by peer".into())),
            Ok(_) => return Ok(()),
            Err(e) if is_would_block(&e) => continue,
            Err(e) => return Err(ShareError::Io(e.to_string())),
        }
    }
}

// ─── Frame helpers ────────────────────────────────────────────────────────────

/// Write a 4-byte LE length prefix followed by `payload` bytes.
fn write_frame(stream: &mut TcpStream, payload: &[u8]) -> Result<(), ShareError> {
    if payload.len() > MAX_FRAME {
        return Err(ShareError::Protocol("outgoing message too large".into()));
    }
    let len = payload.len() as u32;
    stream
        .write_all(&len.to_le_bytes())
        .map_err(|e| ShareError::Io(e.to_string()))?;
    stream
        .write_all(payload)
        .map_err(|e| ShareError::Io(e.to_string()))?;
    Ok(())
}

/// Read a 4-byte LE length prefix, then read exactly that many bytes.
/// Enforces `max_len` to prevent memory exhaustion on malformed input.
fn read_frame(stream: &mut TcpStream, guard: &IoGuard, max_len: usize) -> Result<Vec<u8>, ShareError> {
    let mut len_buf = [0u8; 4];
    read_full(stream, &mut len_buf, guard)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > max_len {
        return Err(ShareError::Protocol(format!(
            "incoming message length {len} exceeds {max_len} byte limit"
        )));
    }
    let mut buf = vec![0u8; len];
    read_full(stream, &mut buf, guard)?;
    Ok(buf)
}

// ─── Public wire functions ────────────────────────────────────────────────────

/// Send a message as plaintext JSON (handshake only).
pub fn send_plain(stream: &mut TcpStream, msg: &ShareMessage) -> Result<(), ShareError> {
    let json =
        serde_json::to_vec(msg).map_err(|e| ShareError::Protocol(e.to_string()))?;
    write_frame(stream, &json)
}

/// Receive a plaintext JSON message (handshake only).
pub fn recv_plain(stream: &mut TcpStream, guard: &IoGuard) -> Result<ShareMessage, ShareError> {
    let frame = read_frame(stream, guard, MAX_PLAIN_FRAME)?;
    serde_json::from_slice(&frame).map_err(|e| ShareError::Protocol(e.to_string()))
}

/// Send a message encrypted with the session key.
pub fn send_encrypted(
    stream: &mut TcpStream,
    key: &[u8; 32],
    msg: &ShareMessage,
) -> Result<(), ShareError> {
    let json =
        serde_json::to_vec(msg).map_err(|e| ShareError::Protocol(e.to_string()))?;
    let ct = encrypt_message(key, &json);
    write_frame(stream, &ct)
}

/// Receive and decrypt a message with the session key.
pub fn recv_encrypted(
    stream: &mut TcpStream,
    key: &[u8; 32],
    guard: &IoGuard,
) -> Result<ShareMessage, ShareError> {
    let frame = read_frame(stream, guard, MAX_FRAME)?;
    let plaintext = decrypt_message(key, &frame)?;
    serde_json::from_slice(&plaintext).map_err(|e| ShareError::Protocol(e.to_string()))
}

// ─── Hex helpers ──────────────────────────────────────────────────────────────

pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn hex_decode(s: &str) -> Result<Vec<u8>, ShareError> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return Err(ShareError::Protocol("invalid hex".into()));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|_| ShareError::Protocol("invalid hex".into()))
        })
        .collect()
}
