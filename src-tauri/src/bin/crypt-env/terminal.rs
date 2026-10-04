//! Terminal identity for per-terminal CLI sessions (cli-tui-parity design
//! D4). A session token authenticated in one terminal is stored in a file
//! keyed by that terminal, so another terminal must enter the password
//! itself — the same model as sudo's `tty_tickets`.
//!
//! Identity: `CRYPTENV_TERMINAL_ID` when set (the WSL managed launcher uses
//! it, since interop gives every Windows process a fresh console); else
//! Unix: session id + controlling tty (+ the session leader's start time on
//! Linux, so a recycled session id doesn't inherit a session); Windows: the
//! console host process id + creation time (unique per Windows Terminal tab,
//! not reusable), or a per-process id when there is no console.
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
    use windows::Win32::System::{Console::GetConsoleWindow, Threading::GetCurrentProcessId};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    // The console host (conhost / OpenConsole / terminal) owning the console
    // window, with its creation time: a recycled window handle or pid is a
    // different process start, hence a different terminal.
    // SAFETY: GetConsoleWindow takes no arguments and only returns a handle;
    // GetWindowThreadProcessId writes the owner pid into the local `pid`.
    let host = unsafe {
        let hwnd = GetConsoleWindow();
        let mut pid = 0u32;
        if hwnd.0.is_null() || GetWindowThreadProcessId(hwnd, Some(&mut pid)) == 0 {
            None
        } else {
            Some(pid)
        }
    };
    if let Some(pid) = host {
        if let Some(created) = process_creation_time(pid) {
            return format!("win:{pid}:{created}");
        }
    }
    // No console (detached) or the host could not be inspected (e.g. an
    // elevated host): this process alone. Only costs extra prompts.
    // SAFETY: GetCurrentProcessId takes no arguments.
    let me = unsafe { GetCurrentProcessId() };
    format!("win:proc:{me}:{}", process_creation_time(me).unwrap_or_default())
}

/// Creation time (FILETIME ticks) of `pid`, `None` when it cannot be opened.
#[cfg(windows)]
fn process_creation_time(pid: u32) -> Option<u64> {
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    // SAFETY: the handle returned by OpenProcess is used only for
    // GetProcessTimes and closed before returning; the FILETIME out-params
    // are valid locals.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let (mut created, mut exited, mut kernel, mut user) =
            (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let ok = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user).is_ok();
        let _ = CloseHandle(handle);
        ok.then(|| (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }
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

/// `name` is exactly `<stem>.<16 lowercase hex chars>` — the shape
/// [`per_terminal_path`] produces. Nothing else next to the token file
/// (`<stem>.bak`, `<stem>.json`, ...) is ours to delete.
fn is_terminal_token_name(name: &str, stem: &str) -> bool {
    name.strip_prefix(stem)
        .and_then(|rest| rest.strip_prefix('.'))
        .map(|hex| hex.len() == 16 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .unwrap_or(false)
}

/// Deletes sibling per-terminal token files of `base` unused for a day.
/// Only regular files named `<stem>.<16 hex>` are considered; symlinks are
/// never followed or removed.
pub fn prune_stale(base: &Path, now: SystemTime) {
    let (Some(dir), Some(stem)) = (base.parent(), base.file_name().and_then(|n| n.to_str())) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !is_terminal_token_name(name, stem) {
            continue;
        }
        // `DirEntry::file_type` does not follow symlinks.
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
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
        let others = [
            dir.path().join("unrelated"),
            dir.path().join(".cli_token.bak"),
            dir.path().join(".cli_token.json"),
            dir.path().join(".cli_token.ABCDEF0123456789"), // uppercase hex
            dir.path().join(".cli_token.0123456789abcde"),  // 15 chars
            dir.path().join(".cli_token.0123456789abcdef0"), // 17 chars
        ];
        for p in [&old, &fresh].into_iter().chain(others.iter()) {
            std::fs::write(p, "t").unwrap();
        }
        // A symlink with a token-shaped name, pointing at a file that must survive.
        #[cfg(unix)]
        let linked_target = dir.path().join("precious");
        #[cfg(unix)]
        let link = dir.path().join(".cli_token.00000000deadbeef");
        #[cfg(unix)]
        {
            std::fs::write(&linked_target, "keep").unwrap();
            std::os::unix::fs::symlink(&linked_target, &link).unwrap();
        }
        let now = SystemTime::now();
        let age = |p: &Path| {
            let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
            f.set_modified(now - Duration::from_secs(2 * 24 * 3600)).unwrap();
        };
        age(&old);
        for p in &others {
            age(p);
        }
        #[cfg(unix)]
        age(&linked_target);
        prune_stale(&base, now);
        assert!(!old.exists());
        assert!(fresh.exists());
        for p in &others {
            assert!(p.exists(), "{} must survive", p.display());
        }
        #[cfg(unix)]
        {
            assert!(std::fs::symlink_metadata(&link).is_ok(), "token-shaped symlink must survive");
            assert!(linked_target.exists());
        }
    }

    #[test]
    fn token_name_shape() {
        assert!(is_terminal_token_name(".cli_token.0123456789abcdef", ".cli_token"));
        assert!(!is_terminal_token_name(".cli_token.0123456789abcdeF", ".cli_token"));
        assert!(!is_terminal_token_name(".cli_token0123456789abcdef", ".cli_token"));
        assert!(!is_terminal_token_name(".cli_token.", ".cli_token"));
    }
}
