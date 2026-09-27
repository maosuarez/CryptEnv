//! WSL Integration (Settings panel) — detect WSL readiness for the "vault on
//! Windows, CLI in WSL" workflow and configure / remove the `crypt-env` client
//! inside a chosen distro.
//!
//! The shell edits themselves are never implemented here: the bundled static
//! `crypt-env-setup` helper (built from the shared `cryptenv_setup` crate, the
//! same code behind `crypt-env setup wsl`) is copied into the distro's `/tmp`,
//! run, and deleted. This module only orchestrates `wsl.exe` — always through
//! argument vectors, never by interpolating the distro name (or any other
//! value) into a shell string. See openspec change
//! `windows-installer-cli-and-wsl-panel`, design D2/D3/D5.
//!
//! Everything below the Tauri commands is platform-neutral and driven through
//! the [`WslRunner`] trait so it is unit-tested on any host with a mock.

use cryptenv_setup::ActionReport;
use serde::Serialize;

// ─── Types ──────────────────────────────────────────────────────────────────

/// Typed error surfaced to the frontend as `{ kind, message? }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum WslError {
    /// Not running on Windows — the WSL panel is inert.
    Unsupported,
    /// `wsl.exe` is not installed / not on `PATH`.
    NotAvailable,
    /// The requested distro is not in the current `wsl --list`.
    UnknownDistro(String),
    /// `wsl.exe` or the helper failed or produced unexpected output.
    Tooling(String),
}

impl std::fmt::Display for WslError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WslError::Unsupported => write!(f, "WSL integration is only available on Windows"),
            WslError::NotAvailable => write!(f, "WSL is not installed"),
            WslError::UnknownDistro(d) => write!(f, "unknown WSL distribution: {d}"),
            WslError::Tooling(m) => write!(f, "WSL tooling error: {m}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslDistro {
    pub name: String,
    /// Default Linux user (`whoami`), `None` when the probe failed.
    pub default_user: Option<String>,
    /// The cryptenv `env.sh` + rc marker block are present.
    pub configured: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslStatus {
    pub available: bool,
    pub distros: Vec<WslDistro>,
    /// `[wsl2] networkingMode=mirrored` is set in `%USERPROFILE%\.wslconfig`.
    pub mirrored: bool,
}

/// Captured result of one `wsl.exe` invocation.
#[derive(Debug, Clone, Default)]
pub struct RunOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Runs `wsl.exe` with the given argument vector.
pub trait WslRunner {
    fn wsl(&self, args: &[String]) -> std::io::Result<RunOutput>;
}

/// Fixed probe script — takes no interpolated input.
const CONFIGURED_PROBE: &str = r#"test -f "$HOME/.config/cryptenv/env.sh" && grep -q "cryptenv initialize" "$HOME/.bashrc" 2>/dev/null"#;

/// Fixed runner script. `$1` is the helper's in-distro source path; the rest
/// are forwarded to the helper. Copies it to a private temp file, runs it, and
/// always deletes it.
const RUN_HELPER: &str = r#"t=$(mktemp /tmp/crypt-env-setup.XXXXXX) || exit 1
cp "$1" "$t" && chmod 700 "$t" || { rm -f "$t"; exit 1; }
shift
"$t" "$@"
rc=$?
rm -f "$t"
exit $rc"#;

// ─── Output decoding ────────────────────────────────────────────────────────

/// Strictly decodes `wsl.exe`'s own output: UTF-16LE (with or without BOM, the
/// default) or UTF-8 (`WSL_UTF8=1`). Anything else — odd-length UTF-16,
/// unpaired surrogates, invalid UTF-8, control characters — is a
/// [`WslError::Tooling`] rather than a best-effort guess.
pub fn decode_wsl_output(bytes: &[u8]) -> Result<String, WslError> {
    let garbled = || WslError::Tooling("unrecognised output from wsl.exe".to_string());

    let text = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8(rest.to_vec()).map_err(|_| garbled())?
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        decode_utf16le_strict(rest).ok_or_else(garbled)?
    } else if bytes.len() >= 2 && bytes.contains(&0) {
        decode_utf16le_strict(bytes).ok_or_else(garbled)?
    } else {
        String::from_utf8(bytes.to_vec()).map_err(|_| garbled())?
    };

    let text = text.replace('\0', "");
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t'))
    {
        return Err(garbled());
    }
    Ok(text)
}

fn decode_utf16le_strict(bytes: &[u8]) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&units).ok()
}

fn lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

// ─── .wslconfig ─────────────────────────────────────────────────────────────

/// `true` when `networkingMode = mirrored` appears under `[wsl2]`. Section and
/// key names are case-insensitive; `#` / `;` comment lines are ignored.
pub fn parse_mirrored(content: &str) -> bool {
    let mut in_wsl2 = false;
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_wsl2 = line[1..line.len() - 1].trim().eq_ignore_ascii_case("wsl2");
            continue;
        }
        if !in_wsl2 {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let v = v.split(['#', ';']).next().unwrap_or("").trim().trim_matches('"');
            if k.trim().eq_ignore_ascii_case("networkingMode") && v.eq_ignore_ascii_case("mirrored")
            {
                return true;
            }
        }
    }
    false
}

// ─── Core operations (platform-neutral) ─────────────────────────────────────

fn tooling_from_io(e: std::io::Error) -> WslError {
    WslError::Tooling(e.to_string())
}

/// Lists distros. `wsl.exe` missing → [`WslError::NotAvailable`]; a failing
/// `--list` (WSL present, zero distros) → empty list.
pub fn list_distros(r: &dyn WslRunner) -> Result<Vec<String>, WslError> {
    let out = match r.wsl(&["--list".into(), "--quiet".into()]) {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(WslError::NotAvailable),
        Err(e) => return Err(tooling_from_io(e)),
    };
    if !out.success {
        return Ok(Vec::new());
    }
    Ok(lines(&decode_wsl_output(&out.stdout)?))
}

/// Read-only detection. Runs `whoami` and a fixed `test`/`grep` probe per
/// distro; writes nothing anywhere.
pub fn detect(r: &dyn WslRunner, wslconfig: Option<&str>) -> Result<WslStatus, WslError> {
    let mirrored = wslconfig.map(parse_mirrored).unwrap_or(false);
    let names = match list_distros(r) {
        Ok(n) => n,
        Err(WslError::NotAvailable) => {
            return Ok(WslStatus {
                available: false,
                distros: Vec::new(),
                mirrored,
            })
        }
        Err(e) => return Err(e),
    };

    let distros = names
        .into_iter()
        .map(|name| {
            let default_user = r
                .wsl(&["-d".into(), name.clone(), "-e".into(), "whoami".into()])
                .ok()
                .filter(|o| o.success)
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let configured = r
                .wsl(&[
                    "-d".into(),
                    name.clone(),
                    "-e".into(),
                    "sh".into(),
                    "-c".into(),
                    CONFIGURED_PROBE.into(),
                ])
                .map(|o| o.success)
                .unwrap_or(false);
            WslDistro {
                name,
                default_user,
                configured,
            }
        })
        .collect::<Vec<_>>();

    Ok(WslStatus {
        available: !distros.is_empty(),
        distros,
        mirrored,
    })
}

/// Rejects any distro not in a *fresh* `wsl --list` (D5: closes the window
/// between UI render and action).
fn ensure_known(r: &dyn WslRunner, distro: &str) -> Result<(), WslError> {
    if list_distros(r)?.iter().any(|d| d == distro) {
        Ok(())
    } else {
        Err(WslError::UnknownDistro(distro.to_string()))
    }
}

/// Where the helper lives when viewed from inside `distro`.
fn helper_linux_path(r: &dyn WslRunner, distro: &str, helper_win: &str) -> Result<String, WslError> {
    let via_wslpath = r
        .wsl(&[
            "-d".into(),
            distro.into(),
            "-e".into(),
            "wslpath".into(),
            "-u".into(),
            helper_win.into(),
        ])
        .ok()
        .filter(|o| o.success)
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    via_wslpath
        .or_else(|| {
            cryptenv_setup::windows_path_to_wsl(helper_win)
                .map(|p| p.to_string_lossy().into_owned())
        })
        .ok_or_else(|| {
            WslError::Tooling("cannot map the bundled helper path into the distro".to_string())
        })
}

fn run_helper(
    r: &dyn WslRunner,
    distro: &str,
    helper_win: &str,
    helper_args: &[String],
) -> Result<ActionReport, WslError> {
    ensure_known(r, distro)?;
    let helper = helper_linux_path(r, distro, helper_win)?;

    let mut args: Vec<String> = vec![
        "-d".into(),
        distro.into(),
        "-e".into(),
        "sh".into(),
        "-c".into(),
        RUN_HELPER.into(),
        "sh".into(),
        helper,
    ];
    args.extend_from_slice(helper_args);

    let out = r.wsl(&args).map_err(tooling_from_io)?;
    if !out.success {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(WslError::Tooling(if msg.is_empty() {
            "the setup helper failed inside the distribution".to_string()
        } else {
            msg
        }));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json = stdout.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
    serde_json::from_str(json)
        .map_err(|_| WslError::Tooling("the setup helper returned an unreadable report".to_string()))
}

/// Applies the `setup wsl` contract inside `distro` via the bundled helper.
/// `appdata_win` is the Windows `%APPDATA%`; the helper derives the live
/// `/mnt/c/.../tls/cert.pem` path from it (the cert is referenced, never copied).
pub fn configure_client(
    r: &dyn WslRunner,
    distro: &str,
    helper_win: &str,
    appdata_win: &str,
) -> Result<ActionReport, WslError> {
    run_helper(r, distro, helper_win, &["--appdata".into(), appdata_win.into()])
}

/// Reverses [`configure_client`] inside `distro`.
pub fn remove_client(r: &dyn WslRunner, distro: &str, helper_win: &str) -> Result<ActionReport, WslError> {
    run_helper(r, distro, helper_win, &["--remove".into()])
}

// ─── Host dispatch (Windows vs. everything else) ────────────────────────────

/// Relative location of the helper inside the app's resource dir; must match
/// `bundle.resources` in `tauri.windows-bundle.conf.json`.
pub const HELPER_RESOURCE: &str = "wsl/crypt-env-setup";

#[cfg(target_os = "windows")]
mod host {
    use super::*;
    use std::os::windows::process::CommandExt as _;
    use std::path::PathBuf;
    use std::time::Duration;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const BUDGET: Duration = Duration::from_secs(120);

    pub struct SystemRunner;

    impl WslRunner for SystemRunner {
        fn wsl(&self, args: &[String]) -> std::io::Result<RunOutput> {
            let out = std::process::Command::new("wsl.exe")
                .args(args)
                .env("WSL_UTF8", "1")
                .creation_flags(CREATE_NO_WINDOW)
                .output()?;
            Ok(RunOutput {
                success: out.status.success(),
                stdout: out.stdout,
                stderr: out.stderr,
            })
        }
    }

    async fn blocking<T: Send + 'static>(
        f: impl FnOnce() -> Result<T, WslError> + Send + 'static,
    ) -> Result<T, WslError> {
        match tokio::time::timeout(BUDGET, tokio::task::spawn_blocking(f)).await {
            Err(_) => Err(WslError::Tooling("WSL did not respond in time".to_string())),
            Ok(Err(e)) => Err(WslError::Tooling(e.to_string())),
            Ok(Ok(r)) => r,
        }
    }

    fn read_wslconfig() -> Option<String> {
        let profile = std::env::var_os("USERPROFILE")?;
        std::fs::read_to_string(PathBuf::from(profile).join(".wslconfig")).ok()
    }

    fn helper_path(resource_dir: Option<PathBuf>) -> Result<String, WslError> {
        let path = resource_dir
            .map(|d| d.join(HELPER_RESOURCE))
            .ok_or_else(|| WslError::Tooling("cannot resolve the app resource directory".to_string()))?;
        match std::fs::metadata(&path) {
            Ok(m) if m.is_file() && m.len() > 0 => Ok(path.to_string_lossy().into_owned()),
            _ => Err(WslError::Tooling(
                "the WSL setup helper is not bundled with this build".to_string(),
            )),
        }
    }

    pub async fn detect() -> Result<WslStatus, WslError> {
        blocking(|| super::detect(&SystemRunner, read_wslconfig().as_deref())).await
    }

    pub async fn configure(
        resource_dir: Option<PathBuf>,
        appdata: Option<PathBuf>,
        distro: String,
    ) -> Result<ActionReport, WslError> {
        let helper = helper_path(resource_dir)?;
        let appdata = appdata
            .map(|p| p.to_string_lossy().into_owned())
            .ok_or_else(|| WslError::Tooling("cannot resolve %APPDATA%".to_string()))?;
        blocking(move || super::configure_client(&SystemRunner, &distro, &helper, &appdata)).await
    }

    pub async fn remove(resource_dir: Option<PathBuf>, distro: String) -> Result<ActionReport, WslError> {
        let helper = helper_path(resource_dir)?;
        blocking(move || super::remove_client(&SystemRunner, &distro, &helper)).await
    }
}

#[cfg(not(target_os = "windows"))]
mod host {
    //! Inert everywhere but Windows: no process is spawned, nothing is read or
    //! written.
    use super::*;
    use std::path::PathBuf;

    pub async fn detect() -> Result<WslStatus, WslError> {
        Err(WslError::Unsupported)
    }

    pub async fn configure(
        _resource_dir: Option<PathBuf>,
        _appdata: Option<PathBuf>,
        _distro: String,
    ) -> Result<ActionReport, WslError> {
        Err(WslError::Unsupported)
    }

    pub async fn remove(_resource_dir: Option<PathBuf>, _distro: String) -> Result<ActionReport, WslError> {
        Err(WslError::Unsupported)
    }
}

// ─── Tauri commands ─────────────────────────────────────────────────────────

/// Reports WSL availability, distros (default user + configured flag) and
/// whether mirrored networking is set. Read-only.
#[tauri::command]
pub async fn wsl_detect() -> Result<WslStatus, WslError> {
    host::detect().await
}

/// Configures the `crypt-env` client inside `distro` (non-destructive; see the
/// `cli` capability's `setup wsl` contract) and reports what changed.
#[tauri::command]
pub async fn wsl_configure_client(
    app: tauri::AppHandle,
    distro: String,
) -> Result<ActionReport, WslError> {
    use tauri::Manager as _;
    let resource_dir = app.path().resource_dir().ok();
    // `%APPDATA%` is the parent of `app_data_dir` (`%APPDATA%\<identifier>`).
    let appdata = app
        .path()
        .app_data_dir()
        .ok()
        .and_then(|d| d.parent().map(|p| p.to_path_buf()));
    host::configure(resource_dir, appdata, distro).await
}

/// Removes the cryptenv client configuration from `distro`; a no-op when
/// nothing is installed.
#[tauri::command]
pub async fn wsl_remove_client(
    app: tauri::AppHandle,
    distro: String,
) -> Result<ActionReport, WslError> {
    use tauri::Manager as _;
    let resource_dir = app.path().resource_dir().ok();
    host::remove(resource_dir, distro).await
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

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

    type Responder = Box<dyn Fn(&[String]) -> std::io::Result<RunOutput>>;

    /// Records every argument vector and answers from a scripted queue, or
    /// from a matcher closure.
    struct MockRunner {
        calls: RefCell<Vec<Vec<String>>>,
        respond: Responder,
    }

    impl MockRunner {
        fn new(respond: impl Fn(&[String]) -> std::io::Result<RunOutput> + 'static) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                respond: Box::new(respond),
            }
        }
    }

    impl WslRunner for MockRunner {
        fn wsl(&self, args: &[String]) -> std::io::Result<RunOutput> {
            self.calls.borrow_mut().push(args.to_vec());
            (self.respond)(args)
        }
    }

    fn ok(stdout: impl Into<Vec<u8>>) -> std::io::Result<RunOutput> {
        Ok(RunOutput {
            success: true,
            stdout: stdout.into(),
            stderr: Vec::new(),
        })
    }

    fn fail() -> std::io::Result<RunOutput> {
        Ok(RunOutput::default())
    }

    const REPORT: &str = r#"{"env_file":"/home/me/.config/cryptenv/env.sh","env_file_changed":true,"rc_files":["/home/me/.bashrc"],"backups":["/home/me/.bashrc.cryptenv.bak"],"marker_added":true,"marker_removed":false}"#;

    /// A two-distro WSL where `Ubuntu` is configured and `Debian` is not.
    fn two_distros() -> MockRunner {
        MockRunner::new(|a| match a.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
            ["--list", "--quiet"] => ok(utf16le("Ubuntu\r\nDebian\r\n", false)),
            ["-d", d, "-e", "whoami"] => ok(format!("{}user\n", d.to_lowercase())),
            ["-d", "Ubuntu", "-e", "sh", "-c", _] => ok(""),
            ["-d", _, "-e", "sh", "-c", _] => fail(),
            ["-d", _, "-e", "wslpath", "-u", _] => ok("/mnt/c/App/wsl/crypt-env-setup\n"),
            ["-d", _, "-e", "sh", "-c", _, "sh", ..] => ok(REPORT),
            _ => fail(),
        })
    }

    // ─── decoding ───────────────────────────────────────────────────────────

    #[test]
    fn decodes_utf16le_with_and_without_bom_and_utf8() {
        for bytes in [
            utf16le("Ubuntu\r\nDebian\r\n", true),
            utf16le("Ubuntu\r\nDebian\r\n", false),
            b"Ubuntu\r\nDebian\r\n".to_vec(),
        ] {
            assert_eq!(lines(&decode_wsl_output(&bytes).unwrap()), vec!["Ubuntu", "Debian"]);
        }
    }

    #[test]
    fn garbled_bytes_are_a_tooling_error() {
        // Odd-length "UTF-16".
        assert!(matches!(decode_wsl_output(&[b'U', 0, b'b']), Err(WslError::Tooling(_))));
        // Invalid UTF-8.
        assert!(matches!(decode_wsl_output(&[0xC3, 0x28]), Err(WslError::Tooling(_))));
        // Unpaired surrogate in UTF-16LE.
        assert!(matches!(decode_wsl_output(&[0x00, 0xD8, b'a', 0]), Err(WslError::Tooling(_))));
        // Control bytes.
        assert!(matches!(decode_wsl_output(&[0x07, 0x1B, 0x02]), Err(WslError::Tooling(_))));
    }

    #[test]
    fn detect_surfaces_garbled_list_as_tooling() {
        let r = MockRunner::new(|_| ok(vec![0xC3, 0x28]));
        assert!(matches!(detect(&r, None), Err(WslError::Tooling(_))));
    }

    // ─── .wslconfig ─────────────────────────────────────────────────────────

    #[test]
    fn mirrored_detected_only_under_wsl2() {
        assert!(parse_mirrored("[wsl2]\nmemory=8GB\nnetworkingMode=mirrored\n"));
        assert!(parse_mirrored("[WSL2]\r\n  networkingmode = \"Mirrored\"  # comment\r\n"));
        assert!(!parse_mirrored("[wsl2]\nnetworkingMode=NAT\n"));
        assert!(!parse_mirrored("[experimental]\nnetworkingMode=mirrored\n"));
        assert!(!parse_mirrored("# [wsl2]\n# networkingMode=mirrored\n"));
        assert!(!parse_mirrored(""));
    }

    // ─── detect ─────────────────────────────────────────────────────────────

    #[test]
    fn detect_enumerates_distros_with_user_and_configured_flag() {
        let r = two_distros();
        let s = detect(&r, Some("[wsl2]\nnetworkingMode=mirrored\n")).unwrap();
        assert!(s.available);
        assert!(s.mirrored);
        assert_eq!(
            s.distros,
            vec![
                WslDistro { name: "Ubuntu".into(), default_user: Some("ubuntuuser".into()), configured: true },
                WslDistro { name: "Debian".into(), default_user: Some("debianuser".into()), configured: false },
            ]
        );
        // Read-only: no helper run, no wslpath, only list / whoami / probe.
        for call in r.calls.borrow().iter() {
            assert!(!call.iter().any(|a| a == "wslpath" || a == RUN_HELPER));
        }
    }

    #[test]
    fn detect_reports_unavailable_when_wsl_missing() {
        let r = MockRunner::new(|_| Err(std::io::Error::from(std::io::ErrorKind::NotFound)));
        let s = detect(&r, None).unwrap();
        assert!(!s.available);
        assert!(s.distros.is_empty());
        assert!(!s.mirrored);
    }

    #[test]
    fn detect_with_zero_distros_is_unavailable() {
        let r = MockRunner::new(|_| fail());
        let s = detect(&r, None).unwrap();
        assert!(!s.available);
    }

    // ─── configure / remove ─────────────────────────────────────────────────

    #[test]
    fn configure_rejects_unknown_distro_without_running_anything_else() {
        let r = two_distros();
        let err = configure_client(&r, "Arch", r"C:\App\wsl\crypt-env-setup", r"C:\Users\me\AppData\Roaming")
            .unwrap_err();
        assert_eq!(err, WslError::UnknownDistro("Arch".into()));
        let calls = r.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], vec!["--list", "--quiet"]);
    }

    #[test]
    fn configure_passes_distro_and_values_as_discrete_args() {
        let r = two_distros();
        let report = configure_client(
            &r,
            "Ubuntu",
            r"C:\App\wsl\crypt-env-setup",
            r"C:\Users\me\AppData\Roaming",
        )
        .unwrap();
        assert_eq!(report.backups, vec!["/home/me/.bashrc.cryptenv.bak"]);
        assert!(report.marker_added);

        let calls = r.calls.borrow();
        let run = calls.last().unwrap();
        assert_eq!(
            run,
            &vec![
                "-d".to_string(),
                "Ubuntu".into(),
                "-e".into(),
                "sh".into(),
                "-c".into(),
                RUN_HELPER.into(),
                "sh".into(),
                "/mnt/c/App/wsl/crypt-env-setup".into(),
                "--appdata".into(),
                r"C:\Users\me\AppData\Roaming".into(),
            ]
        );
        // The fixed scripts never contain caller-supplied values.
        assert!(!RUN_HELPER.contains("Ubuntu") && !CONFIGURED_PROBE.contains("Ubuntu"));
    }

    #[test]
    fn remove_runs_helper_with_remove_flag() {
        let r = two_distros();
        remove_client(&r, "Debian", r"C:\App\wsl\crypt-env-setup").unwrap();
        let calls = r.calls.borrow();
        let run = calls.last().unwrap();
        assert_eq!(run[1], "Debian");
        assert_eq!(run.last().map(String::as_str), Some("--remove"));
    }

    #[test]
    fn helper_failure_and_bad_report_are_tooling_errors() {
        let queue = RefCell::new(VecDeque::from([
            ok(utf16le("Ubuntu\r\n", false)),
            ok("/mnt/c/h\n"),
            Ok(RunOutput { success: false, stdout: Vec::new(), stderr: b"boom".to_vec() }),
        ]));
        let r = MockRunner::new(move |_| queue.borrow_mut().pop_front().unwrap_or_else(fail));
        assert_eq!(
            remove_client(&r, "Ubuntu", r"C:\h").unwrap_err(),
            WslError::Tooling("boom".into())
        );

        let queue = RefCell::new(VecDeque::from([
            ok(utf16le("Ubuntu\r\n", false)),
            ok("/mnt/c/h\n"),
            ok("not json"),
        ]));
        let r = MockRunner::new(move |_| queue.borrow_mut().pop_front().unwrap_or_else(fail));
        assert!(matches!(remove_client(&r, "Ubuntu", r"C:\h"), Err(WslError::Tooling(_))));
    }

    #[test]
    fn helper_path_falls_back_to_drvfs_mapping() {
        let queue = RefCell::new(VecDeque::from([
            ok(utf16le("Ubuntu\r\n", false)),
            fail(), // wslpath unavailable
            ok(REPORT),
        ]));
        let r = MockRunner::new(move |_| queue.borrow_mut().pop_front().unwrap_or_else(fail));
        remove_client(&r, "Ubuntu", r"C:\App\wsl\crypt-env-setup").unwrap();
        let calls = r.calls.borrow();
        assert_eq!(calls[2][7], "/mnt/c/App/wsl/crypt-env-setup");
    }

    // ─── non-Windows guard ──────────────────────────────────────────────────

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn every_command_is_unsupported_off_windows_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let before: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();

        assert_eq!(wsl_detect().await, Err(WslError::Unsupported));
        assert_eq!(
            host::configure(Some(dir.path().to_path_buf()), Some(dir.path().to_path_buf()), "Ubuntu".into()).await,
            Err(WslError::Unsupported)
        );
        assert_eq!(
            host::remove(Some(dir.path().to_path_buf()), "Ubuntu".into()).await,
            Err(WslError::Unsupported)
        );

        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), before.len());
    }

    #[test]
    fn error_serializes_as_tagged_kind() {
        assert_eq!(
            serde_json::to_value(WslError::Unsupported).unwrap(),
            serde_json::json!({ "kind": "unsupported" })
        );
        assert_eq!(
            serde_json::to_value(WslError::UnknownDistro("X".into())).unwrap(),
            serde_json::json!({ "kind": "unknownDistro", "message": "X" })
        );
    }
}
