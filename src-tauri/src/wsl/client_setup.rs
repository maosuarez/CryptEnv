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
use std::time::Duration;

/// Budget for each probe / listing `wsl.exe` call.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
/// Budget for the configure / remove helper run.
pub const ACTION_TIMEOUT: Duration = Duration::from_secs(120);

/// Only one WSL operation runs at a time; a second is rejected, not queued.
static WSL_OP: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Claims the WSL operation slot, or fails with [`WslError::Busy`].
pub fn begin_operation() -> Result<tokio::sync::MutexGuard<'static, ()>, WslError> {
    claim(&WSL_OP)
}

fn claim(slot: &'static tokio::sync::Mutex<()>) -> Result<tokio::sync::MutexGuard<'static, ()>, WslError> {
    slot.try_lock().map_err(|_| WslError::Busy)
}

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
    /// A `wsl.exe` call exceeded its budget; it was killed and reaped.
    Timeout(String),
    /// Another WSL operation is already running.
    Busy,
}

impl std::fmt::Display for WslError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WslError::Unsupported => write!(f, "WSL integration is only available on Windows"),
            WslError::NotAvailable => write!(f, "WSL is not installed"),
            WslError::UnknownDistro(d) => write!(f, "unknown WSL distribution: {d}"),
            WslError::Tooling(m) => write!(f, "WSL tooling error: {m}"),
            WslError::Timeout(m) => write!(f, "WSL timed out: {m}"),
            WslError::Busy => write!(f, "operation in progress"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DistroState {
    Running,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WslDistro {
    pub name: String,
    /// `running` distros are probed; `stopped` ones are never started by
    /// detection (the user opts in per distro).
    pub state: DistroState,
    /// Default Linux user (`whoami`), `None` when the probe failed.
    pub default_user: Option<String>,
    /// The cryptenv `env.sh` + rc marker block are present.
    pub configured: bool,
    /// The managed `crypt-env` launcher (delegating to the Windows CLI) is
    /// installed.
    pub launcher: bool,
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

/// Runs `wsl.exe` with the given argument vector. An implementation MUST kill
/// and reap the process when `timeout` elapses and return
/// [`std::io::ErrorKind::TimedOut`].
#[allow(async_fn_in_trait)]
pub trait WslRunner {
    async fn run(&self, args: &[String], timeout: Duration) -> std::io::Result<RunOutput>;
}

/// Spawns `cmd` with captured output under `timeout`. The child is killed on
/// drop; on timeout it is killed and awaited before returning `TimedOut`, so no
/// orphan survives an abandoned operation.
pub async fn run_process(mut cmd: tokio::process::Command, timeout: Duration) -> std::io::Result<RunOutput> {
    use tokio::io::AsyncReadExt as _;

    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn()?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();

    async fn drain(pipe: &mut Option<impl tokio::io::AsyncRead + Unpin>) -> std::io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        if let Some(p) = pipe {
            p.read_to_end(&mut buf).await?;
        }
        Ok(buf)
    }

    let work = async {
        let (status, out, err) = tokio::join!(child.wait(), drain(&mut stdout), drain(&mut stderr));
        Ok::<_, std::io::Error>(RunOutput { success: status?.success(), stdout: out?, stderr: err? })
    };
    match tokio::time::timeout(timeout, work).await {
        Ok(result) => result,
        Err(_) => {
            // `kill` sends the signal and waits, reaping the child.
            let _ = child.kill().await;
            Err(std::io::Error::from(std::io::ErrorKind::TimedOut))
        }
    }
}

/// Fixed probe script — takes no interpolated input.
const CONFIGURED_PROBE: &str = r#"test -f "$HOME/.config/cryptenv/env.sh" && grep -q "cryptenv initialize" "$HOME/.bashrc" 2>/dev/null"#;

/// Fixed probe script for the managed launcher — takes no interpolated input.
const LAUNCHER_PROBE: &str = r#"grep -q ">>> cryptenv managed launcher >>>" "$HOME/.local/share/cryptenv/bin/crypt-env" 2>/dev/null"#;

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
    if e.kind() == std::io::ErrorKind::TimedOut {
        return WslError::Timeout("WSL did not respond in time".to_string());
    }
    WslError::Tooling(e.to_string())
}

/// Docker Desktop's internal distributions are never listed or probed.
fn is_internal(name: &str) -> bool {
    name.starts_with("docker-desktop")
}

/// Lists distros without touching their state. `wsl.exe` missing →
/// [`WslError::NotAvailable`]; a failing `--list` (WSL present, zero distros)
/// → empty list.
pub async fn list_distros<R: WslRunner>(r: &R) -> Result<Vec<String>, WslError> {
    let out = match r.run(&["--list".into(), "--quiet".into()], PROBE_TIMEOUT).await {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(WslError::NotAvailable),
        Err(e) => return Err(tooling_from_io(e)),
    };
    if !out.success {
        return Ok(Vec::new());
    }
    Ok(lines(&decode_wsl_output(&out.stdout)?)
        .into_iter()
        .filter(|n| !is_internal(n))
        .collect())
}

/// Words `wsl -l -v` prints for a running distro, lowercase, per locale.
/// Anything else is treated as stopped (safe: the user can still click Detect).
const RUNNING_WORDS: &[&str] = &[
    "running",
    "en ejecución",
    "em execução",
    "wird ausgeführt",
    "en cours d'exécution",
    "in esecuzione",
];

/// Parses `wsl -l -v` output by the column offsets of its header row, so
/// localized, multi-word state names ("En ejecución") stay intact.
pub fn parse_distro_states(text: &str) -> Result<Vec<(String, DistroState)>, WslError> {
    let garbled = || WslError::Tooling("unrecognised output from wsl.exe".to_string());
    let mut rows = text.lines().filter(|l| !l.trim().is_empty());
    let header: Vec<char> = rows.next().ok_or_else(garbled)?.chars().collect();

    // Start offset of each whitespace-separated header word.
    let mut starts = Vec::new();
    for (i, c) in header.iter().enumerate() {
        if !c.is_whitespace() && (i == 0 || header[i - 1].is_whitespace()) {
            starts.push(i);
        }
    }
    let [name_at, state_at, version_at, ..] = starts[..] else {
        return Err(garbled());
    };

    let mut out = Vec::new();
    for row in rows {
        let row: Vec<char> = row.trim_end().chars().collect();
        if row.len() <= state_at {
            continue;
        }
        let cell = |from: usize, to: usize| -> String {
            row[from.min(row.len())..to.min(row.len())].iter().collect::<String>().trim().to_string()
        };
        let name = cell(name_at, state_at);
        if name.is_empty() {
            continue;
        }
        let state = cell(state_at, version_at).to_lowercase();
        let state = if RUNNING_WORDS.contains(&state.as_str()) {
            DistroState::Running
        } else {
            DistroState::Stopped
        };
        out.push((name, state));
    }
    Ok(out)
}

/// Distribution states from `wsl -l -v`. Does not start anything.
async fn list_states<R: WslRunner>(r: &R) -> Result<Vec<(String, DistroState)>, WslError> {
    let out = match r.run(&["-l".into(), "-v".into()], PROBE_TIMEOUT).await {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(WslError::NotAvailable),
        Err(e) => return Err(tooling_from_io(e)),
    };
    if !out.success {
        return Ok(Vec::new());
    }
    Ok(parse_distro_states(&decode_wsl_output(&out.stdout)?)?
        .into_iter()
        .filter(|(n, _)| !is_internal(n))
        .collect())
}

/// Runs `whoami` and the fixed `test`/`grep` probes inside `name`. This
/// starts the distro if it is stopped, so callers gate it.
async fn probe_distro<R: WslRunner>(r: &R, name: String) -> WslDistro {
    let sh_probe = |script: &'static str| {
        vec!["-d".to_string(), name.clone(), "-e".into(), "sh".into(), "-c".into(), script.into()]
    };
    let default_user = r
        .run(&["-d".into(), name.clone(), "-e".into(), "whoami".into()], PROBE_TIMEOUT)
        .await
        .ok()
        .filter(|o| o.success)
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let configured = r
        .run(&sh_probe(CONFIGURED_PROBE), PROBE_TIMEOUT)
        .await
        .map(|o| o.success)
        .unwrap_or(false);
    let launcher = r
        .run(&sh_probe(LAUNCHER_PROBE), PROBE_TIMEOUT)
        .await
        .map(|o| o.success)
        .unwrap_or(false);
    WslDistro { name, state: DistroState::Running, default_user, configured, launcher }
}

/// Read-only detection. Only *running* distros are probed; stopped ones are
/// reported as such without being started. Writes nothing anywhere.
pub async fn detect<R: WslRunner>(r: &R, wslconfig: Option<&str>) -> Result<WslStatus, WslError> {
    let mirrored = wslconfig.map(parse_mirrored).unwrap_or(false);
    let states = match list_states(r).await {
        Ok(n) => n,
        Err(WslError::NotAvailable) => {
            return Ok(WslStatus { available: false, distros: Vec::new(), mirrored })
        }
        Err(e) => return Err(e),
    };

    let mut distros = Vec::with_capacity(states.len());
    for (name, state) in states {
        distros.push(match state {
            DistroState::Running => probe_distro(r, name).await,
            DistroState::Stopped => WslDistro {
                name,
                state,
                default_user: None,
                configured: false,
                launcher: false,
            },
        });
    }

    Ok(WslStatus { available: !distros.is_empty(), distros, mirrored })
}

/// Explicit, user-requested detection of one distro. Unlike [`detect`] this
/// starts the distro if it is stopped.
pub async fn detect_distro<R: WslRunner>(r: &R, distro: &str) -> Result<WslDistro, WslError> {
    ensure_known(r, distro).await?;
    Ok(probe_distro(r, distro.to_string()).await)
}

/// Rejects any distro not in a *fresh* `wsl --list` (D5: closes the window
/// between UI render and action).
async fn ensure_known<R: WslRunner>(r: &R, distro: &str) -> Result<(), WslError> {
    if list_distros(r).await?.iter().any(|d| d == distro) {
        Ok(())
    } else {
        Err(WslError::UnknownDistro(distro.to_string()))
    }
}

/// Where the helper lives when viewed from inside `distro`.
async fn helper_linux_path<R: WslRunner>(r: &R, distro: &str, helper_win: &str) -> Result<String, WslError> {
    let via_wslpath = r
        .run(
            &[
                "-d".into(),
                distro.into(),
                "-e".into(),
                "wslpath".into(),
                "-u".into(),
                helper_win.into(),
            ],
            PROBE_TIMEOUT,
        )
        .await
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

async fn run_helper<R: WslRunner>(
    r: &R,
    distro: &str,
    helper_win: &str,
    helper_args: &[String],
) -> Result<ActionReport, WslError> {
    ensure_known(r, distro).await?;
    let helper = helper_linux_path(r, distro, helper_win).await?;

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

    let out = r.run(&args, ACTION_TIMEOUT).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::TimedOut {
            WslError::Timeout(
                "the operation timed out and was stopped; its result is unknown - refresh to check".to_string(),
            )
        } else {
            tooling_from_io(e)
        }
    })?;
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
/// `cli_win` is this installation's `crypt-env.exe`, the target of the managed
/// `crypt-env` launcher; `None` makes the helper report the launcher skipped.
pub async fn configure_client<R: WslRunner>(
    r: &R,
    distro: &str,
    helper_win: &str,
    appdata_win: &str,
    cli_win: Option<&str>,
) -> Result<ActionReport, WslError> {
    let mut args: Vec<String> = vec!["--appdata".into(), appdata_win.into()];
    if let Some(cli) = cli_win {
        args.push("--windows-cli".into());
        args.push(cli.into());
    }
    run_helper(r, distro, helper_win, &args).await
}

/// Reverses [`configure_client`] inside `distro`.
pub async fn remove_client<R: WslRunner>(
    r: &R,
    distro: &str,
    helper_win: &str,
) -> Result<ActionReport, WslError> {
    run_helper(r, distro, helper_win, &["--remove".into()]).await
}

// ─── Host dispatch (Windows vs. everything else) ────────────────────────────

/// Relative location of the helper inside the app's resource dir; must match
/// `bundle.resources` in `tauri.windows-bundle.conf.json`.
pub const HELPER_RESOURCE: &str = "wsl/crypt-env-setup";

/// File name of the Windows CLI installed next to the GUI executable.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const CLI_EXE: &str = "crypt-env.exe";

#[cfg(target_os = "windows")]
mod host {
    use super::*;
    use std::path::PathBuf;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub struct SystemRunner;

    impl WslRunner for SystemRunner {
        async fn run(&self, args: &[String], timeout: Duration) -> std::io::Result<RunOutput> {
            let mut cmd = tokio::process::Command::new("wsl.exe");
            cmd.args(args).env("WSL_UTF8", "1").creation_flags(CREATE_NO_WINDOW);
            run_process(cmd, timeout).await
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
        super::detect(&SystemRunner, read_wslconfig().as_deref()).await
    }

    pub async fn detect_distro(distro: String) -> Result<WslDistro, WslError> {
        super::detect_distro(&SystemRunner, &distro).await
    }

    /// This installation's `crypt-env.exe` — the NSIS installer places it next
    /// to the GUI executable. `None` when absent (e.g. a dev build).
    fn cli_path() -> Option<String> {
        let exe = std::env::current_exe().ok()?;
        let cli = exe.parent()?.join(super::CLI_EXE);
        match std::fs::metadata(&cli) {
            Ok(m) if m.is_file() && m.len() > 0 => Some(cli.to_string_lossy().into_owned()),
            _ => None,
        }
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
        let cli = cli_path();
        super::configure_client(&SystemRunner, &distro, &helper, &appdata, cli.as_deref()).await
    }

    pub async fn remove(resource_dir: Option<PathBuf>, distro: String) -> Result<ActionReport, WslError> {
        let helper = helper_path(resource_dir)?;
        super::remove_client(&SystemRunner, &distro, &helper).await
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

    pub async fn detect_distro(_distro: String) -> Result<WslDistro, WslError> {
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
    let _op = begin_operation()?;
    host::detect().await
}

/// Probes one distro on explicit user request. **Starts the distro if it is
/// stopped** — the GUI labels the action accordingly.
#[tauri::command]
pub async fn wsl_detect_distro(distro: String) -> Result<WslDistro, WslError> {
    let _op = begin_operation()?;
    host::detect_distro(distro).await
}

/// Configures the `crypt-env` client inside `distro` (non-destructive; see the
/// `cli` capability's `setup wsl` contract) and reports what changed.
#[tauri::command]
pub async fn wsl_configure_client(
    app: tauri::AppHandle,
    distro: String,
) -> Result<ActionReport, WslError> {
    use tauri::Manager as _;
    let _op = begin_operation()?;
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
    let _op = begin_operation()?;
    let resource_dir = app.path().resource_dir().ok();
    host::remove(resource_dir, distro).await
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

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

    type Responder = Box<dyn Fn(&[String]) -> std::io::Result<RunOutput> + Send + Sync>;

    /// Records every argument vector and answers from a scripted queue, or
    /// from a matcher closure.
    struct MockRunner {
        calls: Mutex<Vec<Vec<String>>>,
        respond: Responder,
    }

    impl MockRunner {
        fn new(respond: impl Fn(&[String]) -> std::io::Result<RunOutput> + Send + Sync + 'static) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                respond: Box::new(respond),
            }
        }
    }

    impl WslRunner for MockRunner {
        async fn run(&self, args: &[String], _timeout: Duration) -> std::io::Result<RunOutput> {
            self.calls.lock().unwrap().push(args.to_vec());
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

    const REPORT: &str = r#"{"env_file":"/home/me/.config/cryptenv/env.sh","env_file_changed":true,"rc_files":["/home/me/.bashrc"],"backups":["/home/me/.bashrc.cryptenv.bak"],"marker_added":true,"marker_removed":false,"launcher":"/home/me/.local/share/cryptenv/bin/crypt-env","launcher_status":"written","launcher_note":null}"#;

    const EN_LIST_V: &str = "  NAME              STATE           VERSION\r\n* Ubuntu            Running         2\r\n  Debian            Stopped         2\r\n  docker-desktop    Running         2\r\n";
    const ES_LIST_V: &str = "  NOMBRE            ESTADO          VERSIÓN\r\n* Ubuntu            En ejecución    2\r\n  Debian            Detenido        2\r\n  docker-desktop    En ejecución    2\r\n";

    /// A two-distro WSL where `Ubuntu` is configured and `Debian` is not;
    /// only `Ubuntu` has the launcher.
    fn two_distros() -> MockRunner {
        MockRunner::new(|a| match a.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
            ["--list", "--quiet"] => ok(utf16le("Ubuntu\r\nDebian\r\ndocker-desktop\r\n", false)),
            ["-l", "-v"] => ok(utf16le(EN_LIST_V, false)),
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

    #[tokio::test]
    async fn detect_surfaces_garbled_list_as_tooling() {
        let r = MockRunner::new(|_| ok(vec![0xC3, 0x28]));
        assert!(matches!(detect(&r, None).await, Err(WslError::Tooling(_))));
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
    fn parses_english_and_spanish_state_listings() {
        for text in [EN_LIST_V, ES_LIST_V] {
            assert_eq!(
                parse_distro_states(text).unwrap(),
                vec![
                    ("Ubuntu".to_string(), DistroState::Running),
                    ("Debian".to_string(), DistroState::Stopped),
                    ("docker-desktop".to_string(), DistroState::Running),
                ]
            );
        }
    }

    #[test]
    fn utf16le_listing_with_bom_decodes_then_parses() {
        let bytes = utf16le(ES_LIST_V, true);
        let parsed = parse_distro_states(&decode_wsl_output(&bytes).unwrap()).unwrap();
        assert_eq!(parsed[0], ("Ubuntu".to_string(), DistroState::Running));
    }

    #[test]
    fn unknown_state_word_is_stopped_and_bad_header_is_an_error() {
        let text = "  NAME  STATE  VERSION\n  Foo   Zzz    2\n";
        assert_eq!(parse_distro_states(text).unwrap(), vec![("Foo".to_string(), DistroState::Stopped)]);
        assert!(matches!(parse_distro_states("garbage\n"), Err(WslError::Tooling(_))));
        assert!(matches!(parse_distro_states(""), Err(WslError::Tooling(_))));
    }

    #[tokio::test]
    async fn detect_probes_only_running_distros_and_skips_docker() {
        let r = two_distros();
        let s = detect(&r, Some("[wsl2]\nnetworkingMode=mirrored\n")).await.unwrap();
        assert!(s.available);
        assert!(s.mirrored);
        assert_eq!(
            s.distros,
            vec![
                WslDistro { name: "Ubuntu".into(), state: DistroState::Running, default_user: Some("ubuntuuser".into()), configured: true, launcher: true },
                WslDistro { name: "Debian".into(), state: DistroState::Stopped, default_user: None, configured: false, launcher: false },
            ]
        );
        // Read-only, and the stopped distro is never addressed.
        for call in r.calls.lock().unwrap().iter() {
            assert!(!call.iter().any(|a| a == "wslpath" || a == RUN_HELPER));
            assert!(!call.iter().any(|a| a == "--windows-cli" || a == "--remove"));
            assert!(!call.iter().any(|a| a == "Debian" || a.starts_with("docker-desktop")), "{call:?}");
        }
    }

    #[tokio::test]
    async fn detect_distro_starts_only_the_requested_distro() {
        let r = two_distros();
        let d = detect_distro(&r, "Debian").await.unwrap();
        assert_eq!(d.name, "Debian");
        assert_eq!(d.state, DistroState::Running);
        assert_eq!(d.default_user.as_deref(), Some("debianuser"));
        assert_eq!(
            detect_distro(&r, "docker-desktop").await.unwrap_err(),
            WslError::UnknownDistro("docker-desktop".into())
        );
    }

    #[tokio::test]
    async fn detect_reports_unavailable_when_wsl_missing() {
        let r = MockRunner::new(|_| Err(std::io::Error::from(std::io::ErrorKind::NotFound)));
        let s = detect(&r, None).await.unwrap();
        assert!(!s.available);
        assert!(s.distros.is_empty());
        assert!(!s.mirrored);
    }

    #[tokio::test]
    async fn detect_with_zero_distros_is_unavailable() {
        let r = MockRunner::new(|_| fail());
        let s = detect(&r, None).await.unwrap();
        assert!(!s.available);
    }

    #[tokio::test]
    async fn timed_out_listing_is_a_timeout_error() {
        let r = MockRunner::new(|_| Err(std::io::Error::from(std::io::ErrorKind::TimedOut)));
        assert!(matches!(detect(&r, None).await, Err(WslError::Timeout(_))));
    }

    // ─── process reaping / serialization ────────────────────────────────────

    #[test]
    fn detect_future_is_send() {
        // The Tauri commands need Send futures; this fails to compile otherwise.
        fn assert_send<T: Send>(_: T) {}
        let r = MockRunner::new(|_| fail());
        assert_send(detect(&r, None));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_process_kills_and_reaps_on_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(format!("echo $$ > '{}'; exec sleep 30", pidfile.display()));

        let started = std::time::Instant::now();
        let err = run_process(cmd, Duration::from_millis(500)).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(10));

        let pid: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
        // The child was killed and awaited: signalling it now finds no process.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "child {pid} still alive");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_process_captures_output() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg("printf hi; printf err >&2; exit 3");
        let out = run_process(cmd, Duration::from_secs(5)).await.unwrap();
        assert!(!out.success);
        assert_eq!(out.stdout, b"hi");
        assert_eq!(out.stderr, b"err");
    }

    #[tokio::test]
    async fn second_claim_is_rejected_while_one_is_held() {
        static SLOT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let first = claim(&SLOT).unwrap();
        assert_eq!(claim(&SLOT).unwrap_err(), WslError::Busy);
        drop(first);
        assert!(claim(&SLOT).is_ok());
    }

    // ─── configure / remove ─────────────────────────────────────────────────

    #[tokio::test]
    async fn configure_rejects_unknown_distro_without_running_anything_else() {
        let r = two_distros();
        let err = configure_client(
            &r,
            "Arch",
            r"C:\App\wsl\crypt-env-setup",
            r"C:\Users\me\AppData\Roaming",
            Some(r"C:\App\crypt-env.exe"),
        )
        .await
        .unwrap_err();
        assert_eq!(err, WslError::UnknownDistro("Arch".into()));
        let calls = r.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], vec!["--list", "--quiet"]);
    }

    #[tokio::test]
    async fn configure_passes_distro_and_values_as_discrete_args() {
        let r = two_distros();
        let report = configure_client(
            &r,
            "Ubuntu",
            r"C:\App\wsl\crypt-env-setup",
            r"C:\Users\me\AppData\Roaming",
            Some(r"D:\My Apps\Crypt'Env\crypt-env.exe"),
        )
        .await
        .unwrap();
        assert_eq!(report.launcher_status, cryptenv_setup::LauncherStatus::Written);
        assert_eq!(report.backups, vec!["/home/me/.bashrc.cryptenv.bak"]);
        assert!(report.marker_added);

        let calls = r.calls.lock().unwrap();
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
                "--windows-cli".into(),
                r"D:\My Apps\Crypt'Env\crypt-env.exe".into(),
            ]
        );
        // The fixed scripts never contain caller-supplied values.
        for script in [RUN_HELPER, CONFIGURED_PROBE, LAUNCHER_PROBE] {
            assert!(!script.contains("Ubuntu") && !script.contains("My Apps"));
        }
    }

    #[tokio::test]
    async fn configure_without_cli_omits_windows_cli_arg() {
        let r = two_distros();
        configure_client(&r, "Ubuntu", r"C:\h", r"C:\Users\me\AppData\Roaming", None).await.unwrap();
        let calls = r.calls.lock().unwrap();
        assert!(!calls.last().unwrap().iter().any(|a| a == "--windows-cli"));
    }

    #[tokio::test]
    async fn remove_runs_helper_with_remove_flag() {
        let r = two_distros();
        remove_client(&r, "Debian", r"C:\App\wsl\crypt-env-setup").await.unwrap();
        let calls = r.calls.lock().unwrap();
        let run = calls.last().unwrap();
        assert_eq!(run[1], "Debian");
        assert_eq!(run.last().map(String::as_str), Some("--remove"));
    }

    #[tokio::test]
    async fn helper_failure_and_bad_report_are_tooling_errors() {
        let queue = Mutex::new(VecDeque::from([
            ok(utf16le("Ubuntu\r\n", false)),
            ok("/mnt/c/h\n"),
            Ok(RunOutput { success: false, stdout: Vec::new(), stderr: b"boom".to_vec() }),
        ]));
        let r = MockRunner::new(move |_| queue.lock().unwrap().pop_front().unwrap_or_else(fail));
        assert_eq!(
            remove_client(&r, "Ubuntu", r"C:\h").await.unwrap_err(),
            WslError::Tooling("boom".into())
        );

        let queue = Mutex::new(VecDeque::from([
            ok(utf16le("Ubuntu\r\n", false)),
            ok("/mnt/c/h\n"),
            ok("not json"),
        ]));
        let r = MockRunner::new(move |_| queue.lock().unwrap().pop_front().unwrap_or_else(fail));
        assert!(matches!(remove_client(&r, "Ubuntu", r"C:\h").await, Err(WslError::Tooling(_))));
    }

    #[tokio::test]
    async fn helper_path_falls_back_to_drvfs_mapping() {
        let queue = Mutex::new(VecDeque::from([
            ok(utf16le("Ubuntu\r\n", false)),
            fail(), // wslpath unavailable
            ok(REPORT),
        ]));
        let r = MockRunner::new(move |_| queue.lock().unwrap().pop_front().unwrap_or_else(fail));
        remove_client(&r, "Ubuntu", r"C:\App\wsl\crypt-env-setup").await.unwrap();
        let calls = r.calls.lock().unwrap();
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
