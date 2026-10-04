//! Backend-side execution of stored commands for MCP callers.
//!
//! Secrets reach the child process environment only; they never go back to the
//! MCP caller, are never set in the MCP process, and never touch a shared temp
//! directory. See `openspec/changes/mcp-command-execution-hardening`.
//!
//! - [`params`]: strict `{{param}}` substitution (character allowlist).
//! - [`redact`]: secret/base64/hex redaction of captured output.
//! - [`tree`]: process-group (Unix) / Job Object (Windows) tree kill.
//! - [`wsl`]: running inside the caller's WSL distribution from Windows.
//! - [`tempfiles`]: private, expiring files for `generate_env`.
//!
//! This module does not know about `api` or `vault`: the API layer resolves
//! values and hands them in as a plain `(KEY, value)` list.

pub mod params;
pub mod redact;
pub mod tempfiles;
pub mod tree;
pub mod wsl;

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

use self::redact::Redactor;
use self::wsl::WslTarget;

/// Wall-clock limit for one command.
pub const EXEC_TIMEOUT: Duration = Duration::from_secs(120);
/// Bytes captured per stream; further output is read and discarded.
pub const CAPTURE_LIMIT: usize = 64 * 1024;
/// Characters returned per stream.
pub const RETURN_CHARS: usize = 2000;
const TRUNCATION_MARKER: &str = "... (truncated)";
/// How long to wait for the output pipes to close after the child is gone.
const READER_GRACE: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum ExecError {
    InvalidParam { name: String, reason: String },
    InvalidKey(String),
    InvalidContext(String),
    UnsupportedValue(String),
    Spawn(String),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::InvalidParam { name, reason } => {
                write!(f, "invalid parameter '{name}': {reason}")
            }
            ExecError::InvalidKey(k) => write!(f, "invalid or blocked variable name: '{k}'"),
            ExecError::InvalidContext(what) => write!(f, "invalid client context: {what}"),
            ExecError::UnsupportedValue(k) => write!(f, "value of '{k}' cannot be passed to the command"),
            ExecError::Spawn(kind) => write!(f, "failed to start command: {kind}"),
        }
    }
}

impl std::error::Error for ExecError {}

/// Variable names that may be injected into a child: `^[A-Z][A-Z0-9_]*$`,
/// excluding variables that change how programs load code.
pub fn is_safe_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() => {}
        _ => return false,
    }
    if !chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
        return false;
    }
    const BLOCKED: &[&str] = &[
        "PATH",
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
        "PYTHONPATH",
        "NODE_OPTIONS",
        "RUBYOPT",
    ];
    !BLOCKED.contains(&key) && !key.starts_with("LD_")
}

const UNIX_BASELINE: &[&str] = &["PATH", "HOME", "USER", "LOGNAME", "LANG", "LC_ALL", "TMPDIR", "TZ"];
const WINDOWS_BASELINE: &[&str] = &[
    "PATH", "SystemRoot", "SYSTEMROOT", "windir", "ComSpec", "PATHEXT", "TEMP", "TMP", "USERPROFILE",
    "APPDATA", "LOCALAPPDATA", "USERNAME", "HOMEDRIVE", "HOMEPATH", "ProgramData", "ProgramFiles",
    "ProgramFiles(x86)", "LANG",
];

/// The fixed set of variables a child inherits from the backend, read through
/// `get`. Everything else in the backend environment is dropped.
pub fn baseline_env(get: &dyn Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    let names = if cfg!(windows) { WINDOWS_BASELINE } else { UNIX_BASELINE };
    names
        .iter()
        .filter_map(|n| get(n).map(|v| (n.to_string(), v)))
        .collect()
}

/// Truncates on a character boundary to at most `max` characters, marker
/// included. Never panics, whatever the input.
pub fn truncate_chars(s: &str, max: usize) -> (String, bool) {
    if s.chars().count() <= max {
        return (s.to_string(), false);
    }
    let keep = max.saturating_sub(TRUNCATION_MARKER.chars().count());
    let end = s.char_indices().nth(keep).map(|(i, _)| i).unwrap_or(s.len());
    (format!("{}{TRUNCATION_MARKER}", &s[..end]), true)
}

/// Everything needed to start one command.
pub struct ExecSpec {
    /// Resolved command text. Never returned to the caller.
    pub command: String,
    pub cwd: Option<PathBuf>,
    /// Injected secrets, `(KEY, value)`.
    pub env: Vec<(String, Zeroizing<String>)>,
    /// Run through `wsl.exe` (only honoured when the backend is Windows).
    pub wsl: Option<WslTarget>,
    pub timeout: Duration,
}

/// Redacted, bounded result of one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

struct Capture {
    bytes: Vec<u8>,
    truncated: bool,
}

fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
) -> (Arc<Mutex<Capture>>, std::thread::JoinHandle<()>) {
    let capture = Arc::new(Mutex::new(Capture { bytes: Vec::new(), truncated: false }));
    let shared = capture.clone();
    let handle = std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if let Ok(mut cap) = shared.lock() {
                        let room = CAPTURE_LIMIT.saturating_sub(cap.bytes.len());
                        let take = room.min(n);
                        cap.bytes.extend_from_slice(&buf[..take]);
                        if take < n {
                            cap.truncated = true;
                        }
                    }
                    // Beyond the cap the data is read and dropped so the child
                    // never blocks on a full pipe.
                }
            }
        }
    });
    (capture, handle)
}

fn wait_for_readers(handles: Vec<std::thread::JoinHandle<()>>) {
    let deadline = Instant::now() + READER_GRACE;
    for h in handles {
        while !h.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if h.is_finished() {
            let _ = h.join();
        }
    }
}

fn build_command(spec: &ExecSpec) -> Result<(Command, Option<Zeroizing<String>>), ExecError> {
    let host_env = baseline_env(&|n| std::env::var(n).ok());

    if let (Some(target), true) = (&spec.wsl, cfg!(windows)) {
        let mut cmd = Command::new("wsl.exe");
        cmd.args(wsl::build_argv(target)?);
        // `wsl.exe` itself gets the baseline only: no WSLENV, no secrets.
        cmd.env_clear().envs(host_env);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let script = wsl::build_script(target, &spec.command, &spec.env)?;
        return Ok((cmd, Some(script)));
    }

    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", &spec.command]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", &spec.command]);
        c
    };
    cmd.env_clear().envs(host_env);
    for (key, value) in &spec.env {
        cmd.env(key, value.as_str());
    }
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    Ok((cmd, None))
}

fn finalize_stream(cap: &Arc<Mutex<Capture>>, redactor: &Redactor) -> (String, bool) {
    let (mut bytes, truncated) = match cap.lock() {
        Ok(c) => (c.bytes.clone(), c.truncated),
        Err(_) => (Vec::new(), false),
    };
    if truncated {
        // A secret cut by the capture cap would leave an unredacted prefix.
        let keep = bytes.len().saturating_sub(redactor.truncation_guard());
        bytes.truncate(keep);
    }
    let redacted = redactor.redact(&bytes);
    let text = String::from_utf8_lossy(&redacted).into_owned();
    let (text, cut) = truncate_chars(&text, RETURN_CHARS);
    (text, truncated || cut)
}

/// Runs `spec` to completion, timeout or cancellation. Blocking: call from
/// `spawn_blocking`. `cancel` is polled; setting it kills the whole tree.
pub fn run(spec: &ExecSpec, cancel: &AtomicBool, redactor: &Redactor) -> Result<RunOutput, ExecError> {
    let (mut cmd, stdin_script) = build_command(spec)?;
    tree::configure(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| ExecError::Spawn(format!("{:?}", e.kind())))?;
    let process_tree = tree::ProcessTree::attach(&child);

    if let (Some(script), Some(mut stdin)) = (stdin_script, child.stdin.take()) {
        // The pipe is closed on drop; a child that exits early just makes the
        // write fail, which is fine.
        let _ = stdin.write_all(script.as_bytes());
    }

    let (out_cap, out_handle) = match child.stdout.take() {
        Some(s) => spawn_reader(s),
        None => return Err(ExecError::Spawn("stdout unavailable".to_string())),
    };
    let (err_cap, err_handle) = match child.stderr.take() {
        Some(s) => spawn_reader(s),
        None => return Err(ExecError::Spawn("stderr unavailable".to_string())),
    };

    let start = Instant::now();
    let mut timed_out = false;
    let mut cancelled = false;
    let mut exit_code: Option<i32> = None;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                exit_code = status.code();
                break;
            }
            Ok(None) => {}
            Err(_) => break,
        }
        if cancel.load(Ordering::SeqCst) {
            cancelled = true;
        } else if start.elapsed() >= spec.timeout {
            timed_out = true;
        }
        if cancelled || timed_out {
            process_tree.kill();
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    wait_for_readers(vec![out_handle, err_handle]);

    let (stdout, stdout_cut) = finalize_stream(&out_cap, redactor);
    let (stderr, stderr_cut) = finalize_stream(&err_cap, redactor);
    Ok(RunOutput {
        exit_code,
        timed_out,
        cancelled,
        stdout,
        stderr,
        stdout_truncated: stdout_cut,
        stderr_truncated: stderr_cut,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn spec(command: &str) -> ExecSpec {
        ExecSpec {
            command: command.to_string(),
            cwd: None,
            env: Vec::new(),
            wsl: None,
            timeout: Duration::from_secs(10),
        }
    }

    fn run_plain(command: &str) -> RunOutput {
        run(&spec(command), &AtomicBool::new(false), &Redactor::new(&[])).unwrap()
    }

    #[test]
    fn unrelated_backend_variable_is_absent_and_injected_one_present() {
        // The var name is unique to this test; setting it affects only this process.
        std::env::set_var("CRYPTENV_TEST_UNRELATED_SECRET", "leak-me");
        let mut s = spec("env");
        s.env = vec![("DB_PASSWORD".to_string(), Zeroizing::new("hunter2!".to_string()))];
        let out = run(&s, &AtomicBool::new(false), &Redactor::new(&s.env)).unwrap();
        assert_eq!(out.exit_code, Some(0));
        assert!(!out.stdout.contains("CRYPTENV_TEST_UNRELATED_SECRET"), "{}", out.stdout);
        assert!(!out.stdout.contains("leak-me"));
        assert!(out.stdout.contains("DB_PASSWORD=[REDACTED:DB_PASSWORD]"), "{}", out.stdout);
    }

    #[test]
    fn injected_secret_printed_is_redacted_in_all_encodings() {
        let mut s = spec("printenv DB_PASSWORD; printenv DB_PASSWORD | base64; printenv DB_PASSWORD | od -An -tx1 | tr -d ' \\n'");
        s.env = vec![("DB_PASSWORD".to_string(), Zeroizing::new("hunter2!".to_string()))];
        let out = run(&s, &AtomicBool::new(false), &Redactor::new(&s.env)).unwrap();
        assert!(out.stdout.contains("[REDACTED:DB_PASSWORD]"));
        assert!(!out.stdout.contains("hunter2!"));
        assert!(!out.stdout.contains("aHVudGVyMiEK"), "base64 leaked: {}", out.stdout);
    }

    #[test]
    fn multibyte_output_at_the_truncation_boundary_does_not_panic() {
        // 3000 two-byte characters: byte 2000 falls inside a character.
        let out = run_plain("i=0; while [ $i -lt 3000 ]; do printf 'é'; i=$((i+1)); done");
        assert_eq!(out.exit_code, Some(0));
        assert!(out.stdout.chars().count() <= RETURN_CHARS);
        assert!(out.stdout.ends_with(TRUNCATION_MARKER));
        assert!(out.stdout_truncated);
    }

    #[test]
    fn huge_output_stays_capped_and_the_child_is_not_blocked() {
        let out = run_plain("head -c 1048576 /dev/zero | tr '\\0' 'a'");
        assert_eq!(out.exit_code, Some(0));
        assert!(out.stdout.chars().count() <= RETURN_CHARS);
        assert!(out.stdout_truncated);
    }

    #[test]
    fn stdin_is_null() {
        let out = run_plain("cat; echo done");
        assert_eq!(out.exit_code, Some(0));
        assert_eq!(out.stdout.trim(), "done");
    }

    #[test]
    fn timeout_kills_the_whole_process_tree() {
        let marker = format!("sleep {}", 300 + std::process::id() % 100);
        let mut s = spec(&format!("{marker} & {marker}"));
        s.timeout = Duration::from_millis(300);
        let out = run(&s, &AtomicBool::new(false), &Redactor::new(&[])).unwrap();
        assert!(out.timed_out);
        assert_eq!(out.exit_code, None);
        std::thread::sleep(Duration::from_millis(200));
        let alive = Command::new("pgrep").args(["-f", &marker]).output().unwrap();
        assert!(
            alive.stdout.is_empty(),
            "descendant still running: {}",
            String::from_utf8_lossy(&alive.stdout)
        );
    }

    #[test]
    fn cancellation_kills_the_run() {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::SeqCst);
        });
        let out = run(&spec("sleep 60"), &cancel, &Redactor::new(&[])).unwrap();
        t.join().unwrap();
        assert!(out.cancelled);
        assert!(!out.timed_out);
    }

    #[test]
    fn nonzero_exit_is_reported_distinctly_from_timeout() {
        let out = run_plain("exit 3");
        assert_eq!(out.exit_code, Some(3));
        assert!(!out.timed_out);
    }

    #[test]
    fn env_key_allowlist() {
        assert!(is_safe_env_key("DB_PASSWORD"));
        for bad in ["db_password", "PATH", "LD_PRELOAD", "LD_X", "1A", "", "A-B", "NODE_OPTIONS"] {
            assert!(!is_safe_env_key(bad), "{bad}");
        }
    }

    #[test]
    fn truncate_chars_is_boundary_safe_for_any_input() {
        let (t, cut) = truncate_chars(&"é".repeat(5000), 2000);
        assert!(cut);
        assert_eq!(t.chars().count(), 2000);
        let (t, cut) = truncate_chars("short", 2000);
        assert!(!cut);
        assert_eq!(t, "short");
        let (t, _) = truncate_chars("日本語日本語日本語日本語日本語", 18);
        assert!(t.chars().count() <= 18);
    }

    #[test]
    fn baseline_only_contains_listed_variables() {
        let env = baseline_env(&|n| Some(format!("v-{n}")));
        assert!(env.iter().any(|(k, _)| k == "PATH"));
        assert!(env.iter().all(|(k, _)| UNIX_BASELINE.contains(&k.as_str())));
        assert!(baseline_env(&|_| None).is_empty());
    }
}
