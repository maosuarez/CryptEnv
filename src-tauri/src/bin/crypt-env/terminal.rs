//! Terminal identity for per-terminal CLI sessions (cli-tui-parity design
//! D4). A session token authenticated in one terminal is stored in a file
//! keyed by that terminal, so another terminal must enter the password
//! itself — the same model as sudo's `tty_tickets`.
//!
//! Identity: `CRYPTENV_TERMINAL_ID` when set (the WSL managed launcher uses
//! it, since interop gives every Windows process a fresh console); else
//! Unix: session id + controlling tty (+ the session leader's start time on
//! Linux, so a recycled session id doesn't inherit a session); Windows: the
//! console window handle (unique per Windows Terminal tab).
//!
//! The binding is enforced client-side only; see the design note.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Token files not used for this long are deleted opportunistically.
const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

pub fn terminal_id() -> String {
    match std::env::var("CRYPTENV_TERMINAL_ID") {
        Ok(v) if !v.trim().is_empty() => format!("env:{}", v.trim()),
        _ => native_id(),
    }
}

#[cfg(unix)]
fn native_id() -> String {
    // SAFETY: getsid(0) only reads the calling process's session id.
    let sid = unsafe { libc::getsid(0) };
    let tty = [0, 2, 1].iter().find_map(|&fd| tty_name(fd)).unwrap_or_else(|| "notty".to_string());
    let start = leader_start(sid).unwrap_or_default();
    format!("unix:{sid}:{tty}:{start}")
}

#[cfg(unix)]
fn tty_name(fd: libc::c_int) -> Option<String> {
    // SAFETY: isatty/ttyname only inspect `fd`; the returned pointer is
    // copied immediately, before any other call could overwrite it.
    unsafe {
        if libc::isatty(fd) != 1 {
            return None;
        }
        let p = libc::ttyname(fd);
        if p.is_null() {
            return None;
        }
        Some(std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned())
    }
}

/// Start time (clock ticks since boot, field 22) of the session leader.
#[cfg(unix)]
fn leader_start(sid: libc::pid_t) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{sid}/stat")).ok()?;
    // Fields after the parenthesized command name, which may contain spaces.
    let rest = stat.rsplit_once(')')?.1;
    rest.split_whitespace().nth(19).map(str::to_string)
}

#[cfg(windows)]
fn native_id() -> String {
    // SAFETY: GetConsoleWindow takes no arguments and only returns a handle.
    let hwnd = unsafe { windows::Win32::System::Console::GetConsoleWindow() };
    format!("win:{:x}", hwnd.0 as usize)
}

#[cfg(not(any(unix, windows)))]
fn native_id() -> String {
    "unknown".to_string()
}

/// `<base>.<16 hex chars of SHA-256(id)>`, next to `base`.
pub fn per_terminal_path(base: &Path, id: &str) -> PathBuf {
    let digest = Sha256::digest(id.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    let mut name = base.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(format!(".{hex}"));
    base.with_file_name(name)
}

/// Deletes sibling per-terminal token files of `base` unused for a day.
pub fn prune_stale(base: &Path, now: SystemTime) {
    let (Some(dir), Some(stem)) = (base.parent(), base.file_name().and_then(|n| n.to_str())) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let prefix = format!("{stem}.");
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(&prefix) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .map(|age| age > STALE_AFTER)
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_terminal_paths_differ_by_terminal_and_stay_next_to_base() {
        let base = Path::new("/data/com.maosuarez.cryptenv/.cli_token");
        let a = per_terminal_path(base, "unix:1:/dev/pts/1:9");
        let b = per_terminal_path(base, "unix:2:/dev/pts/2:9");
        assert_ne!(a, b);
        assert_eq!(a, per_terminal_path(base, "unix:1:/dev/pts/1:9"), "stable");
        assert_eq!(a.parent(), base.parent());
        assert!(a.file_name().unwrap().to_str().unwrap().starts_with(".cli_token."));
    }

    #[test]
    fn env_override_wins() {
        std::env::set_var("CRYPTENV_TERMINAL_ID", "wsl:42:/dev/pts/3");
        assert_eq!(terminal_id(), "env:wsl:42:/dev/pts/3");
        std::env::remove_var("CRYPTENV_TERMINAL_ID");
        assert!(!terminal_id().starts_with("env:"));
    }

    #[test]
    fn prune_removes_only_old_sibling_token_files() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join(".cli_token");
        let old = per_terminal_path(&base, "old");
        let fresh = per_terminal_path(&base, "fresh");
        let other = dir.path().join("unrelated");
        for p in [&old, &fresh, &other] {
            std::fs::write(p, "t").unwrap();
        }
        let now = SystemTime::now();
        let f = std::fs::OpenOptions::new().write(true).open(&old).unwrap();
        f.set_modified(now - Duration::from_secs(2 * 24 * 3600)).unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(&other).unwrap();
        f.set_modified(now - Duration::from_secs(2 * 24 * 3600)).unwrap();
        prune_stale(&base, now);
        assert!(!old.exists());
        assert!(fresh.exists());
        assert!(other.exists());
    }
}
