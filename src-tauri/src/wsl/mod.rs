//! WSL bridge (Phase 1) — discovering installed distros and a seed directory
//! for the native file picker so a Windows user can attach a project's `.env`
//! living inside a WSL distro without knowing the `\\wsl.localhost\...` UNC
//! syntax or their distro's registered name.
//!
//! Deliberately decoupled: this module knows nothing about `db`, `vault`,
//! `api`, or `SharedState`, and nothing else calls into it except the
//! frontend (via the Tauri commands below and in `client_setup`) — `project::mod` does not
//! call into it either. See docs/plans/issue-3-wsl-bridge-env-paths.md §3.1.

pub mod client_setup;

pub use client_setup::{wsl_configure_client, wsl_detect, wsl_remove_client};

// ─── Pure parsing (no I/O) ─────────────────────────────────────────────────

/// Parses raw stdout bytes from `wsl.exe --list --quiet` into a list of
/// distro names. Handles, in order: a UTF-8 BOM, a UTF-16LE BOM, BOM-less
/// UTF-16LE (some `wsl.exe` builds omit the BOM), and UTF-8 with no BOM (the
/// `WSL_UTF8=1` path). No filtering of names — see plan §4, decision D7.
pub fn parse_distro_list(bytes: &[u8]) -> Vec<String> {
    decode_distro_bytes(bytes)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

fn decode_distro_bytes(bytes: &[u8]) -> String {
    // 1. UTF-8 BOM.
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    // 2. UTF-16LE BOM.
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16le(rest);
    }
    // 3. No BOM, but it looks like BOM-less UTF-16LE.
    if bytes.len() >= 2 && bytes.len() % 2 == 0 && looks_like_utf16le(bytes) {
        return decode_utf16le(bytes);
    }
    // 4. Fall back to UTF-8 lossy.
    String::from_utf8_lossy(bytes).into_owned()
}

/// Heuristic: in real distro-name text encoded as UTF-16LE, every second
/// byte is `0x00` (the ASCII high byte), which essentially never happens for
/// genuine UTF-8 text of the same content.
fn looks_like_utf16le(bytes: &[u8]) -> bool {
    let sample_len = bytes.len().min(32);
    bytes[..sample_len].iter().skip(1).step_by(2).all(|&b| b == 0)
}

fn decode_utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// `\\wsl.localhost\<distro>\` — the current, preferred UNC form (Win10
/// 21H2+ / Win11). Pure, no I/O.
pub fn unc_root(distro: &str) -> String {
    format!("\\\\wsl.localhost\\{distro}\\")
}

/// `\\wsl$\<distro>\` — the legacy UNC form, still resolves as an alias on
/// modern builds and is the only form that resolves pre-21H2. Pure, no I/O.
pub fn unc_root_legacy(distro: &str) -> String {
    format!("\\\\wsl$\\{distro}\\")
}

fn validate_distro_name(distro: &str) -> Result<(), String> {
    if distro.is_empty()
        || distro.contains('\\')
        || distro.contains('/')
        || distro.contains("..")
        || distro.contains('\0')
    {
        return Err("invalid distro name".to_string());
    }
    Ok(())
}

// ─── Tauri commands ─────────────────────────────────────────────────────────

/// Lists installed WSL distro names. Honest about absence: returns `Ok(vec![])`
/// — never `Err` — when `wsl.exe` is not on `PATH`, when WSL is installed
/// with zero distros, and on every non-Windows build target. Returns `Err`
/// only for a timeout or a genuinely unparseable spawn failure. See plan
/// §3.2 and §4 decision D8.
#[tauri::command]
pub async fn wsl_list_distros() -> Result<Vec<String>, String> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::list_distros().await
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(Vec::new())
    }
}

/// Resolves the directory the "select .env file" dialog should be seeded at
/// for `distro`: `\\wsl.localhost\<distro>\home\<user>\` when exactly one
/// user directory exists, `\\wsl.localhost\<distro>\home\` otherwise (or the
/// legacy `\\wsl$\` form on older Windows builds). `Err` when the distro
/// cannot be reached at all (stopped distro, unsupported UNC form, WSL not
/// running) — the caller falls back to the manual path input. See plan §3.3.
///
/// **Side effect:** touching `\\wsl.localhost\<distro>\...` starts the distro
/// if it is stopped. This is inherent to the UNC bridge, which is why it
/// only happens behind an explicit user click (see the button's tooltip).
#[tauri::command]
pub async fn wsl_distro_home(distro: String) -> Result<String, String> {
    validate_distro_name(&distro)?;

    #[cfg(target_os = "windows")]
    {
        windows_impl::distro_home(distro).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = distro;
        Err("WSL browsing is only available on Windows".to_string())
    }
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::{parse_distro_list, unc_root, unc_root_legacy};
    use std::os::windows::process::CommandExt as _;
    use std::time::Duration;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const BUDGET: Duration = Duration::from_secs(10);

    pub(super) async fn list_distros() -> Result<Vec<String>, String> {
        let mut cmd = tokio::process::Command::new("wsl.exe");
        cmd.args(["--list", "--quiet"])
            .env("WSL_UTF8", "1")
            .creation_flags(CREATE_NO_WINDOW)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        match tokio::time::timeout(BUDGET, cmd.output()).await {
            Err(_) => Err("WSL did not respond within 10s".to_string()),
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Ok(Err(e)) => Err(e.to_string()),
            Ok(Ok(output)) if !output.status.success() => Ok(Vec::new()),
            Ok(Ok(output)) => Ok(parse_distro_list(&output.stdout)),
        }
    }

    pub(super) async fn distro_home(distro: String) -> Result<String, String> {
        let primary = format!("{}home\\", unc_root(&distro));
        let legacy = format!("{}home\\", unc_root_legacy(&distro));

        let base = if probe_dir(primary.clone()).await {
            primary
        } else if probe_dir(legacy.clone()).await {
            legacy
        } else {
            return Err(format!(
                "cannot reach \\\\wsl.localhost\\{distro} — is the distro running?"
            ));
        };

        Ok(descend_single_child(base).await)
    }

    /// UNC existence probe. `std::path::Path::is_dir` against a cold distro
    /// blocks on the SMB client, so it runs in `spawn_blocking` under the
    /// same 10 s budget rather than on the async runtime (plan §4, D5).
    async fn probe_dir(path: String) -> bool {
        let probe = tokio::task::spawn_blocking(move || std::path::Path::new(&path).is_dir());
        matches!(tokio::time::timeout(BUDGET, probe).await, Ok(Ok(true)))
    }

    /// If `base` has exactly one entry and it is a directory, returns that
    /// child path. Otherwise returns `base` unchanged. A `read_dir` failure
    /// is never fatal here — it just means we return `base` (plan §3.3.5).
    async fn descend_single_child(base: String) -> String {
        let base_for_probe = base.clone();
        let child = tokio::task::spawn_blocking(move || {
            let mut entries = std::fs::read_dir(&base_for_probe).ok()?;
            let first = entries.next()?.ok()?;
            if entries.next().is_some() {
                return None; // more than one entry — not unambiguous
            }
            if !first.path().is_dir() {
                return None;
            }
            first.path().to_str().map(str::to_string)
        });

        match tokio::time::timeout(BUDGET, child).await {
            Ok(Ok(Some(mut path))) => {
                if !path.ends_with('\\') {
                    path.push('\\');
                }
                path
            }
            _ => base,
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a synthetic UTF-16LE fixture inline. Raw UTF-16 byte literals
    /// in source are unreadable, so tests construct them from a `&str`.
    fn utf16le(s: &str, bom: bool) -> Vec<u8> {
        let mut out = Vec::new();
        if bom {
            out.extend_from_slice(&[0xFF, 0xFE]);
        }
        for unit in s.encode_utf16() {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out
    }

    // T1 — UTF-16LE with BOM.
    #[test]
    fn t1_utf16le_with_bom() {
        let bytes = utf16le("Ubuntu\r\ndocker-desktop\r\n", true);
        assert_eq!(parse_distro_list(&bytes), vec!["Ubuntu", "docker-desktop"]);
    }

    // T2 — UTF-16LE without BOM (same content).
    #[test]
    fn t2_utf16le_without_bom() {
        let bytes = utf16le("Ubuntu\r\ndocker-desktop\r\n", false);
        assert_eq!(parse_distro_list(&bytes), vec!["Ubuntu", "docker-desktop"]);
    }

    // T3 — UTF-8, no BOM (the WSL_UTF8=1 path).
    #[test]
    fn t3_utf8_no_bom() {
        let bytes = b"Ubuntu\r\ndocker-desktop\r\n".to_vec();
        assert_eq!(parse_distro_list(&bytes), vec!["Ubuntu", "docker-desktop"]);
    }

    // T4 — UTF-8 with BOM.
    #[test]
    fn t4_utf8_with_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"Ubuntu\r\ndocker-desktop\r\n");
        assert_eq!(parse_distro_list(&bytes), vec!["Ubuntu", "docker-desktop"]);
    }

    // T5 — empty byte slice.
    #[test]
    fn t5_empty() {
        assert_eq!(parse_distro_list(&[]), Vec::<String>::new());
    }

    // T6 — only CRLFs and spaces.
    #[test]
    fn t6_only_crlf_and_spaces() {
        let bytes = b"  \r\n\r\n   \r\n".to_vec();
        assert_eq!(parse_distro_list(&bytes), Vec::<String>::new());
    }

    // T7 — trailing blank lines, mixed \n / \r\n.
    #[test]
    fn t7_trailing_blank_lines_mixed_newlines() {
        let bytes = b"Ubuntu\ndocker-desktop\r\n\n\r\n".to_vec();
        assert_eq!(parse_distro_list(&bytes), vec!["Ubuntu", "docker-desktop"]);
    }

    // T8 — a name containing a space, preserved verbatim.
    #[test]
    fn t8_name_with_space() {
        let bytes = b"openSUSE Leap 15.5\r\n".to_vec();
        assert_eq!(parse_distro_list(&bytes), vec!["openSUSE Leap 15.5"]);
    }

    // T9 — the committed real capture fixture.
    //
    // NOTE (deviation from plan §7.1): the plan calls for "one real capture
    // from the maintainer's machine" via `wsl.exe --list --quiet`. This dev
    // environment has no Windows/WSL host to capture from, so this fixture
    // is a synthetic stand-in built to match the *shape* of a genuine
    // capture (raw UTF-16LE-with-BOM bytes, as `wsl.exe` emits without
    // WSL_UTF8 set) rather than a real one. Flagged for the maintainer to
    // swap in an authentic capture from real hardware — see §7.2 gate.
    #[test]
    fn t9_real_capture_fixture() {
        let bytes = include_bytes!("../../tests/fixtures/wsl-list-quiet.bin");
        assert_eq!(parse_distro_list(bytes), vec!["Ubuntu"]);
    }

    // T10 — unc_root / unc_root_legacy.
    #[test]
    fn t10_unc_root_forms() {
        assert_eq!(unc_root("Ubuntu"), r"\\wsl.localhost\Ubuntu\");
        assert_eq!(unc_root_legacy("Ubuntu"), r"\\wsl$\Ubuntu\");
    }

    // Not in §7.1's table, but guards `wsl_distro_home` against the
    // argument going straight into a path — worth covering.
    #[test]
    fn rejects_traversal_and_separators_in_distro_name() {
        assert!(validate_distro_name("Ubuntu").is_ok());
        assert!(validate_distro_name("").is_err());
        assert!(validate_distro_name("../etc").is_err());
        assert!(validate_distro_name("a\\b").is_err());
        assert!(validate_distro_name("a/b").is_err());
        assert!(validate_distro_name("a\0b").is_err());
    }
}
