use mdns_sd::{ServiceDaemon, ServiceInfo};
use rand::RngCore;
use std::collections::{HashMap, HashSet};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

use super::crypto::{PairRole, PakeStart};
use super::package::PlainItem;
use super::protocol::{
    configure_stream, hex_decode, hex_encode, recv_encrypted, recv_plain, send_encrypted,
    send_plain, wait_readable, IoGuard, ShareMessage, PROTOCOL_VERSION,
};
use super::ShareError;

const SERVICE_TYPE: &str = "_cryptenv._tcp.local.";

/// A receiver tries at most this many advertised services per attempt.
const MAX_SERVICES_TRIED: usize = 5;

/// Message shown to the user when the peer speaks another protocol version.
pub const VERSION_MISMATCH_HINT: &str =
    "The other device runs an incompatible CryptEnv version. Update the other device.";

// ─── Timeouts ─────────────────────────────────────────────────────────────────

/// Network bounds for a LAN session. Production values come from `Default`;
/// tests shorten them.
#[derive(Clone)]
pub struct LanConfig {
    /// Read/write timeout applied to every share socket.
    pub io_timeout: Duration,
    /// Maximum time for a whole handshake, measured from the TCP connection.
    pub handshake_timeout: Duration,
    /// Set when the owning session is cancelled; blocking reads then abort.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for LanConfig {
    fn default() -> Self {
        LanConfig {
            io_timeout: Duration::from_secs(30),
            handshake_timeout: Duration::from_secs(60),
            cancel: None,
        }
    }
}

impl LanConfig {
    pub fn with_cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    fn io_guard(&self) -> IoGuard {
        let guard = IoGuard::new(self.io_timeout);
        match &self.cancel {
            Some(c) => guard.with_cancel(c.clone()),
            None => guard,
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed))
    }
}

// ─── Discovery advertisement ──────────────────────────────────────────────────

/// Random per-session id advertised over mDNS. Not derived from the pairing code.
pub fn generate_session_id() -> String {
    let mut buf = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut buf);
    hex_encode(&buf)
}

/// mDNS TXT record: only the session id and the protocol version.
pub fn service_properties(session_id: &str) -> HashMap<String, String> {
    let mut properties = HashMap::new();
    properties.insert("sid".to_string(), session_id.to_string());
    properties.insert("v".to_string(), PROTOCOL_VERSION.to_string());
    properties
}

// ─── Listener ─────────────────────────────────────────────────────────────────

/// Bind a TCP listener on a random port and register an mDNS service so that
/// receivers on the LAN can discover this session. Nothing derived from the
/// pairing code is advertised; the code is only used inside the PAKE handshake.
///
/// Returns `(TcpListener, port, ServiceDaemon)`.
/// The caller must keep the `ServiceDaemon` alive until the session ends —
/// dropping it unregisters the mDNS service.
pub fn start_listener(
    session_id: &str,
) -> Result<(TcpListener, u16, ServiceDaemon), ShareError> {
    // Bind to a random port on all interfaces so peers on the LAN can connect.
    let listener = TcpListener::bind("0.0.0.0:0")
        .map_err(|e| ShareError::Io(format!("bind TCP listener: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| ShareError::Io(e.to_string()))?
        .port();

    let mdns = ServiceDaemon::new()
        .map_err(|e| ShareError::Discovery(format!("mdns daemon: {e}")))?;

    let hostname = format!("cryptenv-{session_id}.local.");

    // Use enable_addr_auto() so the mDNS daemon fills in the real local
    // interface addresses at registration time.  Passing `()` alone leaves the
    // address set empty, which means the A record is never advertised and the
    // receiver's get_addresses() returns nothing — the root cause of LAN
    // connection failures on Windows.
    let service = ServiceInfo::new(
        SERVICE_TYPE,
        &format!("cryptenv-share-{session_id}"),
        &hostname,
        (),
        port,
        Some(service_properties(session_id)),
    )
    .map_err(|e| ShareError::Discovery(format!("build service info: {e}")))?
    .enable_addr_auto();

    mdns.register(service)
        .map_err(|e| ShareError::Discovery(format!("mdns register: {e}")))?;

    Ok((listener, port, mdns))
}

// ─── Handshake (both roles) ───────────────────────────────────────────────────

/// SPAKE2 pairing handshake over `stream`, keyed by `pairing_code`.
///
/// 1. Exchange protocol versions (mismatch -> `VersionMismatch`).
/// 2. Exchange SPAKE2 messages.
/// 3. Exchange key-confirmation MACs; a wrong pairing code fails here with
///    `PairingFailed` before anything sensitive has been sent.
///
/// Returns the session key and the `XXXX-XXXX-XXXX-XXXX` fingerprint. Every read
/// is bounded by `cfg.io_timeout`, and the whole exchange by `cfg.handshake_timeout`.
pub fn pair_handshake(
    stream: &mut TcpStream,
    pairing_code: &str,
    role: PairRole,
    cfg: &LanConfig,
) -> Result<(Zeroizing<[u8; 32]>, String), ShareError> {
    configure_stream(stream, cfg.io_timeout)?;
    let mut guard = cfg.io_guard();
    guard.deadline = Some(Instant::now() + cfg.handshake_timeout);

    send_plain(stream, &ShareMessage::Hello { version: PROTOCOL_VERSION })?;
    match recv_plain(stream, &guard)? {
        ShareMessage::Hello { version } if version == PROTOCOL_VERSION => {}
        ShareMessage::Hello { .. } => {
            let _ = send_plain(
                stream,
                &ShareMessage::Error {
                    code: "VERSION_MISMATCH".into(),
                    message: VERSION_MISMATCH_HINT.into(),
                },
            );
            return Err(ShareError::VersionMismatch);
        }
        ShareMessage::Error { code, .. } if code == "VERSION_MISMATCH" => {
            return Err(ShareError::VersionMismatch)
        }
        _ => return Err(ShareError::Protocol("expected Hello message".into())),
    }

    let pake = PakeStart::new(pairing_code, role);
    send_plain(stream, &ShareMessage::Pake { msg_hex: hex_encode(pake.message()) })?;
    let peer_msg = match recv_plain(stream, &guard)? {
        ShareMessage::Pake { msg_hex } => hex_decode(&msg_hex)?,
        _ => return Err(ShareError::Protocol("expected Pake message".into())),
    };
    let keys = pake.finish(&peer_msg)?;

    send_plain(stream, &ShareMessage::KeyConfirm { mac_hex: hex_encode(&keys.our_confirmation()?) })?;
    let peer_mac = match recv_plain(stream, &guard)? {
        ShareMessage::KeyConfirm { mac_hex } => hex_decode(&mac_hex)?,
        _ => return Err(ShareError::Protocol("expected KeyConfirm message".into())),
    };
    keys.verify_peer_confirmation(&peer_mac)?;

    let fingerprint = keys.fingerprint();
    Ok((keys.session_key.clone(), fingerprint))
}

// ─── Connector ────────────────────────────────────────────────────────────────

/// Browse mDNS for `_cryptenv._tcp.local.` services and pair with one of them.
///
/// The advertisement carries no pairing-code material, so the receiver tries each
/// compatible service in turn (at most `MAX_SERVICES_TRIED`); a service whose host
/// does not know the code fails confirmation and is skipped.
///
/// Returns the paired stream, session key and fingerprint, or `ShareError::Timeout`
/// if nothing is discovered within `timeout_secs`.
pub fn connect_and_pair(
    pairing_code: &str,
    timeout_secs: u64,
    cfg: &LanConfig,
) -> Result<(TcpStream, Zeroizing<[u8; 32]>, String), ShareError> {
    let mdns = ServiceDaemon::new()
        .map_err(|e| ShareError::Discovery(format!("mdns daemon: {e}")))?;

    let receiver = mdns
        .browse(SERVICE_TYPE)
        .map_err(|e| ShareError::Discovery(format!("mdns browse: {e}")))?;

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut tried: HashSet<String> = HashSet::new();
    let mut last_error: Option<ShareError> = None;
    let mut saw_incompatible = false;

    loop {
        if cfg.cancelled() {
            return Err(ShareError::Cancelled);
        }
        if tried.len() >= MAX_SERVICES_TRIED {
            return Err(last_error.unwrap_or(ShareError::PairingFailed));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(match last_error {
                Some(e) => e,
                None if saw_incompatible => ShareError::VersionMismatch,
                None => ShareError::Timeout,
            });
        }

        let event = match receiver.recv_timeout(remaining.min(Duration::from_millis(500))) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let info = match event {
            mdns_sd::ServiceEvent::ServiceResolved(info) => info,
            _ => continue,
        };

        if info.get_property_val_str("v") != Some(PROTOCOL_VERSION.to_string().as_str()) {
            saw_incompatible = true;
            continue;
        }
        if !tried.insert(info.get_fullname().to_string()) {
            continue;
        }

        // Prefer IPv4 to avoid Windows link-local IPv6 routing issues.
        // Fall back to any available address only if no IPv4 is present.
        let addrs = info.get_addresses();
        let addr = match addrs
            .iter()
            .find(|a| a.is_ipv4())
            .or_else(|| addrs.iter().next())
            .cloned()
        {
            Some(a) => a,
            None => continue,
        };
        let socket_addr = std::net::SocketAddr::new(addr, info.get_port());

        let attempt = TcpStream::connect_timeout(&socket_addr, Duration::from_secs(15))
            .map_err(|e| ShareError::Io(format!("TCP connect to {socket_addr}: {e}")))
            .and_then(|mut stream| {
                pair_handshake(&mut stream, pairing_code, PairRole::Receiver, cfg)
                    .map(|(key, fp)| (stream, key, fp))
            });

        match attempt {
            Ok(paired) => {
                drop(mdns); // unregisters the service when dropped
                return Ok(paired);
            }
            Err(ShareError::Cancelled) => return Err(ShareError::Cancelled),
            Err(e) => last_error = Some(e),
        }
    }
}

// ─── Send session ─────────────────────────────────────────────────────────────

/// Send items over an established, confirmed session.
/// `items_to_send` are already-decrypted PlainItem values.
pub fn sender_send_items(
    mut stream: TcpStream,
    shared_key: &Zeroizing<[u8; 32]>,
    items: Vec<PlainItem>,
    cfg: &LanConfig,
) -> Result<usize, ShareError> {
    send_encrypted(
        &mut stream,
        &**shared_key,
        &ShareMessage::Items { items },
    )?;

    // Wait for Ack
    let ack = recv_encrypted(&mut stream, &**shared_key, &cfg.io_guard())?;
    match ack {
        ShareMessage::Ack { received } => Ok(received),
        ShareMessage::Error { message, .. } => Err(ShareError::Remote(message)),
        _ => Err(ShareError::Protocol("expected Ack message".into())),
    }
}

/// Send an Error message and close the connection.
pub fn sender_reject(
    mut stream: TcpStream,
    shared_key: &Zeroizing<[u8; 32]>,
    reason: &str,
) -> Result<(), ShareError> {
    send_encrypted(
        &mut stream,
        &**shared_key,
        &ShareMessage::Error {
            code: "REJECTED".into(),
            message: reason.to_string(),
        },
    )?;
    Ok(())
}

// ─── Receive session ──────────────────────────────────────────────────────────

/// Receive items from an established, confirmed session.
/// The sender may still be waiting for its own user to confirm the fingerprint,
/// so we wait up to `sender_wait` for the first byte; once data flows the normal
/// `io_timeout` applies. Returns the list of received PlainItems.
pub fn receiver_receive_items(
    mut stream: TcpStream,
    shared_key: &Zeroizing<[u8; 32]>,
    cfg: &LanConfig,
    sender_wait: Duration,
) -> Result<Vec<PlainItem>, ShareError> {
    let mut wait_guard = IoGuard::new(sender_wait);
    wait_guard.cancel = cfg.cancel.clone();
    wait_readable(&stream, &wait_guard)?;

    let msg = recv_encrypted(&mut stream, &**shared_key, &cfg.io_guard())?;
    match msg {
        ShareMessage::Items { items } => {
            let count = items.len();
            // Send acknowledgement
            send_encrypted(
                &mut stream,
                &**shared_key,
                &ShareMessage::Ack { received: count },
            )?;
            Ok(items)
        }
        ShareMessage::Error { message, .. } => Err(ShareError::Remote(message)),
        _ => Err(ShareError::Protocol("expected Items message".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn fast_cfg() -> LanConfig {
        LanConfig {
            io_timeout: Duration::from_millis(400),
            handshake_timeout: Duration::from_secs(5),
            cancel: None,
        }
    }

    /// Connected localhost socket pair.
    fn socket_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        (client, server)
    }

    fn run_pair(
        sender_code: &'static str,
        receiver_code: &'static str,
    ) -> (
        Result<(Zeroizing<[u8; 32]>, String), ShareError>,
        Result<(Zeroizing<[u8; 32]>, String), ShareError>,
    ) {
        let (mut a, mut b) = socket_pair();
        let t = std::thread::spawn(move || {
            pair_handshake(&mut a, sender_code, PairRole::Sender, &fast_cfg())
        });
        let r = pair_handshake(&mut b, receiver_code, PairRole::Receiver, &fast_cfg());
        (t.join().unwrap(), r)
    }

    #[test]
    fn handshake_succeeds_with_same_code() {
        let (s, r) = run_pair("111111", "111111");
        let (sk, sfp) = s.unwrap();
        let (rk, rfp) = r.unwrap();
        assert_eq!(*sk, *rk);
        assert_eq!(sfp, rfp);
    }

    #[test]
    fn handshake_fails_with_wrong_code_on_both_sides() {
        let (s, r) = run_pair("111111", "222222");
        assert!(matches!(s, Err(ShareError::PairingFailed)));
        assert!(matches!(r, Err(ShareError::PairingFailed)));
    }

    #[test]
    fn version_mismatch_is_reported_and_no_pairing_happens() {
        let (mut a, mut old_peer) = socket_pair();
        let t = std::thread::spawn(move || {
            pair_handshake(&mut a, "111111", PairRole::Sender, &fast_cfg())
        });
        // Old (v1.0.6) peer: sends its own Hello, then reads whatever comes back.
        send_plain(&mut old_peer, &ShareMessage::Hello { version: 1 }).unwrap();
        let reply = recv_plain(&mut old_peer, &IoGuard::new(Duration::from_secs(2)));
        // The new side first sent its Hello; then the mismatch Error.
        assert!(matches!(reply, Ok(ShareMessage::Hello { version: PROTOCOL_VERSION })));
        assert!(matches!(t.join().unwrap(), Err(ShareError::VersionMismatch)));
    }

    #[test]
    fn silent_peer_times_out() {
        let (mut a, _silent) = socket_pair();
        let start = Instant::now();
        let r = pair_handshake(&mut a, "111111", PairRole::Sender, &fast_cfg());
        assert!(matches!(r, Err(ShareError::Timeout)));
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn handshake_deadline_bounds_a_slow_drip_peer() {
        let (mut a, mut slow) = socket_pair();
        let cfg = LanConfig {
            io_timeout: Duration::from_secs(5),
            handshake_timeout: Duration::from_millis(600),
            cancel: None,
        };
        let drip = std::thread::spawn(move || {
            // Valid-looking length prefix, then one byte at a time forever.
            use std::io::Write;
            let _ = slow.write_all(&100u32.to_le_bytes());
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(100));
                if slow.write_all(b"x").is_err() {
                    break;
                }
            }
        });
        let start = Instant::now();
        let r = pair_handshake(&mut a, "111111", PairRole::Sender, &cfg);
        assert!(matches!(r, Err(ShareError::Timeout)));
        assert!(start.elapsed() < Duration::from_secs(2));
        let _ = drip.join();
    }

    #[test]
    fn cancel_flag_aborts_a_blocked_handshake() {
        let (mut a, _silent) = socket_pair();
        let cancel = Arc::new(AtomicBool::new(false));
        let cfg = LanConfig {
            io_timeout: Duration::from_secs(30),
            handshake_timeout: Duration::from_secs(60),
            cancel: Some(cancel.clone()),
        };
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            flag.store(true, Ordering::Relaxed);
        });
        let r = pair_handshake(&mut a, "111111", PairRole::Sender, &cfg);
        assert!(matches!(r, Err(ShareError::Cancelled)));
    }

    #[test]
    fn txt_record_contains_only_session_id_and_version() {
        let sid = generate_session_id();
        let props = service_properties(&sid);
        let mut keys: Vec<&str> = props.keys().map(|k| k.as_str()).collect();
        keys.sort();
        assert_eq!(keys, vec!["sid", "v"]);
        assert_eq!(props["v"], "2");
        assert_eq!(props["sid"].len(), 16);
        // Session ids are random: not a function of any code.
        assert_ne!(generate_session_id(), generate_session_id());
    }
}
