//! The single implementation of the `setup wsl` shell-configuration contract.
//!
//! Model: `conda init` / `rustup`. A fully-owned `~/.config/cryptenv/env.sh`
//! holds the `export` lines and is rewritten every run; a single
//! marker-delimited block sourcing that file is ensured in `~/.bashrc` (and
//! `~/.zshrc` when it exists). All rc-file edits are whole-file read →
//! block-boundary slice → whole-file write, with a one-time backup before the
//! first modification. User content outside the marker block is never touched.
//!
//! Consumed by `crypt-env setup wsl` and by the `crypt-env-setup` helper the
//! Windows GUI runs inside a WSL distro — there is no second copy of this logic.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_API_BASE: &str = "https://127.0.0.1:47821";

pub const START_MARKER: &str = "# >>> cryptenv initialize >>>";
pub const END_MARKER: &str = "# <<< cryptenv initialize <<<";
const BACKUP_SUFFIX: &str = ".cryptenv.bak";

/// Identifies a `crypt-env` launcher written by this crate. A file at the
/// launcher path without this line is foreign and is never modified.
pub const LAUNCHER_MARKER: &str = "# >>> cryptenv managed launcher >>>";

// ─── Errors ──────────────────────────────────────────────────────────────────

/// Messages name only file paths and flag/variable names — never secret values.
#[derive(Debug)]
pub enum SetupError {
    Io(std::io::Error),
    Config(String),
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetupError::Io(e) => write!(f, "I/O error: {e}"),
            SetupError::Config(msg) => write!(f, "Configuration error: {msg}"),
        }
    }
}

impl std::error::Error for SetupError {}

impl From<std::io::Error> for SetupError {
    fn from(e: std::io::Error) -> Self {
        SetupError::Io(e)
    }
}

// ─── Report ──────────────────────────────────────────────────────────────────

/// What an `apply` / `remove` run actually did. Paths and booleans only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionReport {
    /// The managed env file (`~/.config/cryptenv/env.sh`).
    pub env_file: String,
    /// `apply`: the env file was (re)written. `remove`: it was deleted.
    pub env_file_changed: bool,
    /// rc files whose content was modified by this run.
    pub rc_files: Vec<String>,
    /// One-time backups created by this run (`<rc>.cryptenv.bak`).
    pub backups: Vec<String>,
    /// `apply`: a marker block was inserted into at least one rc file.
    pub marker_added: bool,
    /// `remove`: a marker block was deleted from at least one rc file.
    pub marker_removed: bool,
    /// In-distro path of the managed `crypt-env` launcher, when one was
    /// written, left unchanged, deleted, or skipped at that path.
    #[serde(default)]
    pub launcher: Option<String>,
    /// What happened to the launcher. `absent` when it was not involved.
    #[serde(default)]
    pub launcher_status: LauncherStatus,
    /// Why the launcher was skipped (`launcher_status == skipped`).
    #[serde(default)]
    pub launcher_note: Option<String>,
}

/// Outcome for the managed `crypt-env` launcher in an [`ActionReport`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LauncherStatus {
    /// Not requested / nothing there (also the value older reports decode to).
    #[default]
    Absent,
    Written,
    Unchanged,
    Deleted,
    Skipped,
}

pub const LAUNCHER_NOTE_CLI_NOT_FOUND: &str = "windows CLI not found";
pub const LAUNCHER_NOTE_FOREIGN: &str = "foreign file present";

// ─── Value resolution ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Values {
    pub api_url: String,
    pub cert_path: String,
}

/// Resolves the values `env.sh` will record. Precedence per field:
/// flag → environment → documented WSL default. The cert path has no safe
/// default when the Windows profile cannot be located, so it errors instead of
/// guessing.
pub fn resolve_values(
    flag_url: Option<&str>,
    flag_cert: Option<&Path>,
    env_url: Option<&str>,
    env_cert: Option<&str>,
    derived_cert: Option<&Path>,
) -> Result<Values, SetupError> {
    let non_empty = |s: &str| !s.trim().is_empty();

    let api_url = flag_url
        .filter(|s| non_empty(s))
        .or_else(|| env_url.filter(|s| non_empty(s)))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());

    let cert_path = flag_cert
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|s| non_empty(s))
        .or_else(|| {
            env_cert
                .filter(|s| non_empty(s))
                .map(|s| s.trim().to_string())
        })
        .or_else(|| derived_cert.map(|p| p.to_string_lossy().into_owned()))
        .ok_or_else(|| {
            SetupError::Config(
                "cannot determine the Windows TLS certificate path; re-run with \
                 --cert-path pointing at the /mnt/c path to cert.pem"
                    .to_string(),
            )
        })?;

    Ok(Values {
        api_url,
        cert_path: cert_path.trim().to_string(),
    })
}

/// `<roaming>/com.maosuarez.cryptenv/tls/cert.pem`.
pub fn cert_under(roaming: PathBuf) -> PathBuf {
    roaming
        .join("com.maosuarez.cryptenv")
        .join("tls")
        .join("cert.pem")
}

/// Translates a Windows path (`C:\Users\me` or `C:/Users/me`) to its WSL DrvFs
/// mount (`/mnt/c/Users/me`). Returns `None` when `win` is not a drive-letter
/// absolute path.
pub fn windows_path_to_wsl(win: &str) -> Option<PathBuf> {
    let win = win.trim();
    let bytes = win.as_bytes();
    if bytes.len() < 3 || bytes[1] != b':' || !(bytes[2] == b'\\' || bytes[2] == b'/') {
        return None;
    }
    let drive = (bytes[0] as char).to_ascii_lowercase();
    if !drive.is_ascii_alphabetic() {
        return None;
    }
    let mut out = PathBuf::from(format!("/mnt/{drive}"));
    for segment in win[3..].split(['\\', '/']).filter(|s| !s.is_empty()) {
        out.push(segment);
    }
    Some(out)
}

// ─── Apply ───────────────────────────────────────────────────────────────────

/// Writes `env.sh` and ensures the marker block in `~/.bashrc` (created if
/// missing) and `~/.zshrc` (only if it already exists).
pub fn apply(home: &Path, values: &Values) -> Result<ActionReport, SetupError> {
    let env_sh = env_file_path(home);
    if let Some(dir) = env_sh.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write_env_sh(&env_sh, values)?;

    let mut report = ActionReport {
        env_file: env_sh.to_string_lossy().into_owned(),
        env_file_changed: true,
        rc_files: Vec::new(),
        backups: Vec::new(),
        marker_added: false,
        marker_removed: false,
        launcher: None,
        launcher_status: LauncherStatus::Absent,
        launcher_note: None,
    };

    let bashrc = home.join(".bashrc");
    let zshrc = home.join(".zshrc");
    let mut targets = vec![bashrc];
    if zshrc.exists() {
        targets.push(zshrc);
    }
    for rc in targets {
        let outcome = ensure_block(&rc)?;
        if outcome.modified {
            report.marker_added = true;
            report.rc_files.push(rc.to_string_lossy().into_owned());
        }
        if let Some(b) = outcome.backup {
            report.backups.push(b.to_string_lossy().into_owned());
        }
    }
    Ok(report)
}

/// Writes the fully-managed `env.sh` (mode 0644), overwriting any prior copy.
pub fn write_env_sh(path: &Path, values: &Values) -> Result<(), SetupError> {
    let body = format!(
        "# Managed by `crypt-env setup wsl`. Do not edit — re-run the command to regenerate.\n\
         export CRYPTENV_API_URL={}\n\
         export CRYPTENV_CERT_PATH={}\n\
         # Fallback `crypt-env` launcher dir, appended so a native client earlier on PATH wins.\n\
         case \":$PATH:\" in *\":$HOME/{LAUNCHER_DIR}:\"*) ;; *) PATH=\"$PATH:$HOME/{LAUNCHER_DIR}\" ;; esac\n\
         export PATH\n",
        sh_single_quote(&values.api_url),
        sh_single_quote(&values.cert_path),
    );
    std::fs::write(path, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644));
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct EnsureOutcome {
    /// The rc file was modified (block appended or file created).
    pub modified: bool,
    /// A backup was created by this call.
    pub backup: Option<PathBuf>,
}

/// Ensures the marker block is present in `rc_path`.
///
/// - Marker already present → the file is left byte-for-byte unchanged and no
///   backup is created.
/// - Marker absent, file exists → a one-time `<rc>.cryptenv.bak` backup is made
///   (only if it does not already exist), then the block is appended after a
///   blank line, preserving every prior line and its order.
/// - File does not exist → it is created containing only the block.
pub fn ensure_block(rc_path: &Path) -> Result<EnsureOutcome, SetupError> {
    let existing = match std::fs::read_to_string(rc_path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(SetupError::Io(e)),
    };

    let mut outcome = EnsureOutcome::default();
    if let Some(content) = &existing {
        if content.contains(START_MARKER) {
            return Ok(outcome);
        }
        // First modification of an existing rc file — back it up once.
        let backup = backup_path(rc_path);
        if !backup.exists() {
            std::fs::copy(rc_path, &backup)?;
            outcome.backup = Some(backup);
        }
    }

    let mut next = existing.unwrap_or_default();
    if !next.is_empty() {
        if !next.ends_with('\n') {
            next.push('\n');
        }
        next.push('\n'); // blank line before our block
    }
    next.push_str(&block_text());

    write_following_symlink(rc_path, &next)?;
    outcome.modified = true;
    Ok(outcome)
}

// ─── Remove ──────────────────────────────────────────────────────────────────

/// Deletes the marker block from `~/.bashrc` / `~/.zshrc` and removes
/// `env.sh`. A run with nothing to remove succeeds and changes nothing.
pub fn remove(home: &Path) -> Result<ActionReport, SetupError> {
    let env_sh = env_file_path(home);
    let mut report = ActionReport {
        env_file: env_sh.to_string_lossy().into_owned(),
        env_file_changed: false,
        rc_files: Vec::new(),
        backups: Vec::new(),
        marker_added: false,
        marker_removed: false,
        launcher: None,
        launcher_status: LauncherStatus::Absent,
        launcher_note: None,
    };

    for rc in [home.join(".bashrc"), home.join(".zshrc")] {
        if remove_block(&rc)? {
            report.marker_removed = true;
            report.rc_files.push(rc.to_string_lossy().into_owned());
        }
    }

    match std::fs::remove_file(&env_sh) {
        Ok(()) => report.env_file_changed = true,
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(SetupError::Io(e)),
    }
    remove_launcher(home, &mut report)?;
    Ok(report)
}

/// Removes the inclusive `[start marker … end marker]` line range from
/// `rc_path`. Returns `true` when the file was modified. A missing file or a
/// file without the markers is a no-op.
pub fn remove_block(rc_path: &Path) -> Result<bool, SetupError> {
    let content = match std::fs::read_to_string(rc_path) {
        Ok(s) => s,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(SetupError::Io(e)),
    };

    let Some(stripped) = strip_block(&content) else {
        return Ok(false);
    };
    if stripped == content {
        return Ok(false);
    }
    write_following_symlink(rc_path, &stripped)?;
    Ok(true)
}

// ─── Launcher ────────────────────────────────────────────────────────────────

/// CryptEnv-owned directory (relative to `$HOME`) holding the launcher.
pub const LAUNCHER_DIR: &str = ".local/share/cryptenv/bin";

pub fn launcher_path(home: &Path) -> PathBuf {
    home.join(LAUNCHER_DIR).join("crypt-env")
}

/// The launcher script: a fixed template whose only variable is the
/// single-quoted in-distro path of the Windows `crypt-env.exe`.
pub fn launcher_script(exe: &Path) -> String {
    format!(
        "#!/bin/sh\n\
         {LAUNCHER_MARKER} regenerated by CryptEnv Configure; removed by Remove.\n\
         exe={}\n\
         if [ ! -x \"$exe\" ]; then\n\
         \x20 echo \"crypt-env: Windows CLI not found at $exe - reinstall CryptEnv or re-run Settings > WSL Integration > Configure\" >&2\n\
         \x20 exit 127\n\
         fi\n\
         exec env -u CRYPTENV_API_URL -u CRYPTENV_CERT_PATH -u CRYPTENV_TOKEN_PATH \"$exe\" \"$@\"\n",
        sh_single_quote(&exe.to_string_lossy()),
    )
}

/// `None` when nothing exists at `path`; otherwise whether it carries
/// [`LAUNCHER_MARKER`]. Non-UTF-8 content is simply "not managed".
fn launcher_state(path: &Path) -> Result<Option<bool>, SetupError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(
            bytes
                .windows(LAUNCHER_MARKER.len())
                .any(|w| w == LAUNCHER_MARKER.as_bytes()),
        )),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(SetupError::Io(e)),
    }
}

/// Installs (or refreshes) the managed launcher targeting `exe` — the Windows
/// CLI as seen from inside the distro — and records the outcome in `report`.
/// `exe == None` records a skip. A foreign file at the launcher path is never
/// touched. The write is atomic (temp file + rename), mode 0755.
pub fn apply_launcher(
    home: &Path,
    exe: Option<&Path>,
    report: &mut ActionReport,
) -> Result<(), SetupError> {
    let path = launcher_path(home);
    report.launcher = Some(path.to_string_lossy().into_owned());

    let Some(exe) = exe else {
        report.launcher_status = LauncherStatus::Skipped;
        report.launcher_note = Some(LAUNCHER_NOTE_CLI_NOT_FOUND.to_string());
        return Ok(());
    };

    let body = launcher_script(exe);
    match launcher_state(&path)? {
        Some(false) => {
            report.launcher_status = LauncherStatus::Skipped;
            report.launcher_note = Some(LAUNCHER_NOTE_FOREIGN.to_string());
            return Ok(());
        }
        Some(true) if std::fs::read(&path)? == body.as_bytes() => {
            report.launcher_status = LauncherStatus::Unchanged;
            return Ok(());
        }
        _ => {}
    }

    let dir = home.join(LAUNCHER_DIR);
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join(".crypt-env.tmp");
    std::fs::write(&tmp, &body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(SetupError::Io(e));
    }
    report.launcher_status = LauncherStatus::Written;
    Ok(())
}

/// Deletes the managed launcher (never a foreign file) and the owned
/// directories it leaves empty; records the outcome in `report`.
pub fn remove_launcher(home: &Path, report: &mut ActionReport) -> Result<(), SetupError> {
    let path = launcher_path(home);
    match launcher_state(&path)? {
        None => {}
        Some(false) => {
            report.launcher = Some(path.to_string_lossy().into_owned());
            report.launcher_status = LauncherStatus::Skipped;
            report.launcher_note = Some(LAUNCHER_NOTE_FOREIGN.to_string());
        }
        Some(true) => {
            std::fs::remove_file(&path)?;
            report.launcher = Some(path.to_string_lossy().into_owned());
            report.launcher_status = LauncherStatus::Deleted;
        }
    }
    // Best effort: `remove_dir` only succeeds on an empty directory, so
    // anything else living there is left alone.
    let bin = home.join(LAUNCHER_DIR);
    if std::fs::remove_dir(&bin).is_ok() {
        if let Some(owned) = bin.parent() {
            let _ = std::fs::remove_dir(owned);
        }
    }
    Ok(())
}

// ─── Shared helpers ──────────────────────────────────────────────────────────

pub fn env_file_path(home: &Path) -> PathBuf {
    home.join(".config").join("cryptenv").join("env.sh")
}

fn block_text() -> String {
    format!(
        "{START_MARKER}\n\
         [ -f \"$HOME/.config/cryptenv/env.sh\" ] && . \"$HOME/.config/cryptenv/env.sh\"\n\
         {END_MARKER}\n"
    )
}

pub fn backup_path(rc_path: &Path) -> PathBuf {
    let name = rc_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    rc_path.with_file_name(format!("{name}{BACKUP_SUFFIX}"))
}

/// Drops the inclusive marker range. Returns `None` when no start marker line is
/// present, `Some(new_content)` otherwise (line endings preserved).
fn strip_block(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let start = lines
        .iter()
        .position(|l| l.trim_end_matches(['\r', '\n']).trim() == START_MARKER)?;
    let end = lines[start..]
        .iter()
        .position(|l| l.trim_end_matches(['\r', '\n']).trim() == END_MARKER)
        .map(|offset| start + offset)
        .unwrap_or(lines.len() - 1);

    let mut out = String::with_capacity(content.len());
    for (idx, line) in lines.iter().enumerate() {
        if idx < start || idx > end {
            out.push_str(line);
        }
    }
    Some(out)
}

/// Writes `content` to `path`, following a symlink to its target as a shell
/// would (`std::fs::write` opens with `O_TRUNC` through the link). On failure
/// the error is returned before anything destructive happens elsewhere, so the
/// original file and any backup are left intact.
fn write_following_symlink(path: &Path, content: &str) -> Result<(), SetupError> {
    std::fs::write(path, content).map_err(SetupError::Io)
}

/// POSIX single-quote a value for use after `export NAME=`.
fn sh_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(url: &str, cert: &str) -> Values {
        Values {
            api_url: url.into(),
            cert_path: cert.into(),
        }
    }

    // ─── windows_path_to_wsl ─────────────────────────────────────────────────

    #[test]
    fn windows_path_to_wsl_translates_backslash_drive_path() {
        let p = windows_path_to_wsl(r"C:\Users\me\AppData\Roaming").unwrap();
        assert_eq!(p, PathBuf::from("/mnt/c/Users/me/AppData/Roaming"));
    }

    #[test]
    fn windows_path_to_wsl_accepts_forward_slashes_and_lowercases_drive() {
        let p = windows_path_to_wsl("D:/dev/app").unwrap();
        assert_eq!(p, PathBuf::from("/mnt/d/dev/app"));
    }

    #[test]
    fn windows_path_to_wsl_rejects_non_drive_paths() {
        assert!(windows_path_to_wsl("/home/me").is_none());
        assert!(windows_path_to_wsl("relative\\path").is_none());
        assert!(windows_path_to_wsl("").is_none());
    }

    // ─── resolve_values ──────────────────────────────────────────────────────

    #[test]
    fn resolve_prefers_flags_then_env_then_derived() {
        let derived = PathBuf::from("/mnt/c/derived/cert.pem");
        let v = resolve_values(
            Some("https://flag.example"),
            Some(Path::new("/flag/cert.pem")),
            Some("https://env.example"),
            Some("/env/cert.pem"),
            Some(&derived),
        )
        .unwrap();
        assert_eq!(v.api_url, "https://flag.example");
        assert_eq!(v.cert_path, "/flag/cert.pem");
    }

    #[test]
    fn resolve_falls_back_to_env_then_default_url() {
        let v = resolve_values(None, None, Some("https://env.example"), Some("/env/cert.pem"), None)
            .unwrap();
        assert_eq!(v.api_url, "https://env.example");
        assert_eq!(v.cert_path, "/env/cert.pem");

        let v2 = resolve_values(None, None, None, Some("/env/cert.pem"), None).unwrap();
        assert_eq!(v2.api_url, DEFAULT_API_BASE);
    }

    #[test]
    fn resolve_uses_derived_cert_when_no_flag_or_env() {
        let derived = PathBuf::from("/mnt/c/derived/cert.pem");
        let v = resolve_values(None, None, None, None, Some(&derived)).unwrap();
        assert_eq!(v.cert_path, "/mnt/c/derived/cert.pem");
        assert_eq!(v.api_url, DEFAULT_API_BASE);
    }

    #[test]
    fn resolve_errors_when_cert_cannot_be_determined() {
        let err = resolve_values(None, None, None, None, None).unwrap_err();
        assert!(matches!(err, SetupError::Config(_)));
    }

    // ─── env.sh ─────────────────────────────────────────────────────────────

    #[test]
    fn write_env_sh_is_deterministic_and_rewritable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("env.sh");

        write_env_sh(&path, &values("https://127.0.0.1:47821", "/mnt/c/x/cert.pem")).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        assert!(first.contains("export CRYPTENV_API_URL='https://127.0.0.1:47821'"));
        assert!(first.contains("export CRYPTENV_CERT_PATH='/mnt/c/x/cert.pem'"));

        // Re-run with the same values → identical bytes.
        write_env_sh(&path, &values("https://127.0.0.1:47821", "/mnt/c/x/cert.pem")).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first);

        // Re-run with new values → fully replaced.
        write_env_sh(&path, &values("https://10.0.0.5:47821", "/mnt/d/y/cert.pem")).unwrap();
        let third = std::fs::read_to_string(&path).unwrap();
        assert!(third.contains("10.0.0.5"));
        assert!(!third.contains("127.0.0.1"));
    }

    // ─── apply / remove reports ──────────────────────────────────────────────

    #[test]
    fn apply_reports_actions_and_reapply_touches_only_env_file() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join(".bashrc"), "# mine\n").unwrap();
        let v = values(DEFAULT_API_BASE, "/mnt/c/x/cert.pem");

        let first = apply(home.path(), &v).unwrap();
        assert!(first.env_file_changed);
        assert!(first.marker_added);
        assert_eq!(first.rc_files.len(), 1);
        assert_eq!(first.backups.len(), 1);
        assert!(first.backups[0].ends_with(".bashrc.cryptenv.bak"));

        let second = apply(home.path(), &v).unwrap();
        assert!(second.env_file_changed);
        assert!(!second.marker_added);
        assert!(second.rc_files.is_empty());
        assert!(second.backups.is_empty());
    }

    #[test]
    fn apply_handles_zshrc_only_when_present() {
        let home = tempfile::tempdir().unwrap();
        let v = values(DEFAULT_API_BASE, "/mnt/c/x/cert.pem");
        apply(home.path(), &v).unwrap();
        assert!(!home.path().join(".zshrc").exists());

        std::fs::write(home.path().join(".zshrc"), "# zsh\n").unwrap();
        let r = apply(home.path(), &v).unwrap();
        assert_eq!(r.rc_files.len(), 1);
        assert!(r.rc_files[0].ends_with(".zshrc"));
    }

    #[test]
    fn remove_reverses_apply_and_is_noop_second_time() {
        let home = tempfile::tempdir().unwrap();
        let prior = "line one\nline two\n";
        std::fs::write(home.path().join(".bashrc"), prior).unwrap();
        apply(home.path(), &values(DEFAULT_API_BASE, "/c.pem")).unwrap();

        let r = remove(home.path()).unwrap();
        assert!(r.marker_removed);
        assert!(r.env_file_changed);
        assert!(!env_file_path(home.path()).exists());
        let after = std::fs::read_to_string(home.path().join(".bashrc")).unwrap();
        assert!(after.starts_with(prior));
        assert!(!after.contains(START_MARKER));

        let again = remove(home.path()).unwrap();
        assert!(!again.marker_removed);
        assert!(!again.env_file_changed);
        assert!(again.rc_files.is_empty());
    }

    #[test]
    fn report_json_shape_is_stable() {
        let r = ActionReport {
            env_file: "/h/.config/cryptenv/env.sh".into(),
            env_file_changed: true,
            rc_files: vec!["/h/.bashrc".into()],
            backups: vec!["/h/.bashrc.cryptenv.bak".into()],
            marker_added: true,
            marker_removed: false,
            launcher: Some("/h/.local/share/cryptenv/bin/crypt-env".into()),
            launcher_status: LauncherStatus::Written,
            launcher_note: None,
        };
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "backups",
                "env_file",
                "env_file_changed",
                "launcher",
                "launcher_note",
                "launcher_status",
                "marker_added",
                "marker_removed",
                "rc_files"
            ]
        );
        let back: ActionReport = serde_json::from_value(v).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn report_from_older_helper_still_decodes() {
        let json = r#"{"env_file":"/h/.config/cryptenv/env.sh","env_file_changed":true,"rc_files":[],"backups":[],"marker_added":false,"marker_removed":false}"#;
        let r: ActionReport = serde_json::from_str(json).unwrap();
        assert_eq!(r.launcher, None);
        assert_eq!(r.launcher_status, LauncherStatus::Absent);
        assert_eq!(r.launcher_note, None);
    }

    // ─── env.sh PATH line ────────────────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn env_sh_appends_launcher_dir_to_path_exactly_once() {
        let dir = tempfile::tempdir().unwrap();
        let env_sh = dir.path().join("env.sh");
        write_env_sh(&env_sh, &values(DEFAULT_API_BASE, "/c.pem")).unwrap();
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(r#". "$1"; . "$1"; printf %s "$PATH""#)
            .arg("sh")
            .arg(&env_sh)
            .env("HOME", "/h")
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        assert!(out.status.success());
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "/usr/bin:/bin:/h/.local/share/cryptenv/bin"
        );
    }

    // ─── launcher ────────────────────────────────────────────────────────────

    fn empty_report() -> ActionReport {
        remove(tempfile::tempdir().unwrap().path()).unwrap()
    }

    #[test]
    fn launcher_is_written_then_unchanged_then_rewritten() {
        let home = tempfile::tempdir().unwrap();
        let path = launcher_path(home.path());

        let mut r = empty_report();
        apply_launcher(home.path(), Some(Path::new("/mnt/c/A/crypt-env.exe")), &mut r).unwrap();
        assert_eq!(r.launcher_status, LauncherStatus::Written);
        assert_eq!(r.launcher.as_deref(), Some(path.to_string_lossy().as_ref()));
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.starts_with("#!/bin/sh\n"));
        assert!(body.contains(LAUNCHER_MARKER));
        assert!(body.contains("exe='/mnt/c/A/crypt-env.exe'"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }

        let mut r = empty_report();
        apply_launcher(home.path(), Some(Path::new("/mnt/c/A/crypt-env.exe")), &mut r).unwrap();
        assert_eq!(r.launcher_status, LauncherStatus::Unchanged);

        let mut r = empty_report();
        apply_launcher(home.path(), Some(Path::new("/mnt/d/B/crypt-env.exe")), &mut r).unwrap();
        assert_eq!(r.launcher_status, LauncherStatus::Written);
        assert!(std::fs::read_to_string(&path).unwrap().contains("/mnt/d/B/"));
        // No temp file left behind.
        assert_eq!(std::fs::read_dir(home.path().join(LAUNCHER_DIR)).unwrap().count(), 1);
    }

    #[test]
    fn launcher_skipped_without_windows_cli() {
        let home = tempfile::tempdir().unwrap();
        let mut r = empty_report();
        apply_launcher(home.path(), None, &mut r).unwrap();
        assert_eq!(r.launcher_status, LauncherStatus::Skipped);
        assert_eq!(r.launcher_note.as_deref(), Some(LAUNCHER_NOTE_CLI_NOT_FOUND));
        assert!(!launcher_path(home.path()).exists());
    }

    #[test]
    fn foreign_launcher_file_is_never_modified_or_removed() {
        let home = tempfile::tempdir().unwrap();
        let path = launcher_path(home.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let foreign: &[u8] = b"\x7fELF\x00\xffnative binary";
        std::fs::write(&path, foreign).unwrap();

        let mut r = empty_report();
        apply_launcher(home.path(), Some(Path::new("/mnt/c/A/crypt-env.exe")), &mut r).unwrap();
        assert_eq!(r.launcher_status, LauncherStatus::Skipped);
        assert_eq!(r.launcher_note.as_deref(), Some(LAUNCHER_NOTE_FOREIGN));
        assert_eq!(std::fs::read(&path).unwrap(), foreign);

        let r = remove(home.path()).unwrap();
        assert_eq!(r.launcher_status, LauncherStatus::Skipped);
        assert_eq!(std::fs::read(&path).unwrap(), foreign);
    }

    #[test]
    fn remove_deletes_launcher_and_empty_owned_dirs_then_noop() {
        let home = tempfile::tempdir().unwrap();
        apply(home.path(), &values(DEFAULT_API_BASE, "/c.pem")).unwrap();
        let mut r = empty_report();
        apply_launcher(home.path(), Some(Path::new("/mnt/c/A/crypt-env.exe")), &mut r).unwrap();

        let r = remove(home.path()).unwrap();
        assert_eq!(r.launcher_status, LauncherStatus::Deleted);
        assert!(!launcher_path(home.path()).exists());
        assert!(!home.path().join(".local/share/cryptenv").exists());
        assert!(home.path().join(".local/share").exists());

        let again = remove(home.path()).unwrap();
        assert_eq!(again.launcher_status, LauncherStatus::Absent);
        assert_eq!(again.launcher, None);
    }

    /// Runs the generated launcher against a stub "Windows CLI" script.
    #[cfg(unix)]
    #[test]
    fn launcher_forwards_args_exit_code_and_strips_cryptenv_env() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        // Hostile characters in the target path must survive quoting.
        let exe_dir = home.path().join("it's $(x) dir");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let exe = exe_dir.join("crypt-env.exe");
        std::fs::write(
            &exe,
            "#!/bin/sh\nfor a in \"$@\"; do printf '[%s]' \"$a\"; done\n\
             printf '|%s|%s|%s|%s' \"${CRYPTENV_API_URL-unset}\" \"${CRYPTENV_CERT_PATH-unset}\" \"${CRYPTENV_TOKEN_PATH-unset}\" \"$KEEP\"\n\
             exit 3\n",
        )
        .unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut r = empty_report();
        apply_launcher(home.path(), Some(&exe), &mut r).unwrap();
        let out = std::process::Command::new(launcher_path(home.path()))
            .args(["search", "my key", "$HOME"])
            .env("CRYPTENV_API_URL", "https://x")
            .env("CRYPTENV_CERT_PATH", "/mnt/c/cert.pem")
            .env("CRYPTENV_TOKEN_PATH", "/t")
            .env("KEEP", "kept")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "[search][my key][$HOME]|unset|unset|unset|kept"
        );

        // Target gone → 127 with an actionable message, no fallback.
        std::fs::remove_file(&exe).unwrap();
        let out = std::process::Command::new(launcher_path(home.path())).output().unwrap();
        assert_eq!(out.status.code(), Some(127));
        let err = String::from_utf8(out.stderr).unwrap();
        assert!(err.contains("Windows CLI not found at"));
        assert!(err.contains("Configure"));
    }

    // ─── ensure_block ──────────────────────────────────────────────────────

    #[test]
    fn ensure_block_appends_and_preserves_prior_content() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join(".bashrc");
        let prior = "export PATH=$PATH:/opt/bin\nalias ll='ls -la'\n";
        std::fs::write(&rc, prior).unwrap();

        assert!(ensure_block(&rc).unwrap().modified);

        let after = std::fs::read_to_string(&rc).unwrap();
        assert!(after.starts_with(prior));
        assert!(after.contains(START_MARKER));
        assert!(after.contains(END_MARKER));
        assert!(after.contains(". \"$HOME/.config/cryptenv/env.sh\""));

        // Backup is a faithful copy of the pre-edit file.
        let backup = backup_path(&rc);
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), prior);
    }

    #[test]
    fn ensure_block_is_idempotent_and_creates_no_second_backup() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join(".bashrc");
        std::fs::write(&rc, "# user rc\n").unwrap();

        let first = ensure_block(&rc).unwrap();
        assert!(first.modified);
        assert!(first.backup.is_some());
        let after_first = std::fs::read_to_string(&rc).unwrap();
        let backup = backup_path(&rc);
        let backup_meta = std::fs::metadata(&backup).unwrap().modified().unwrap();

        // Second run: file untouched, backup untouched.
        assert_eq!(ensure_block(&rc).unwrap(), EnsureOutcome::default());
        assert_eq!(std::fs::read_to_string(&rc).unwrap(), after_first);
        assert_eq!(
            std::fs::metadata(&backup).unwrap().modified().unwrap(),
            backup_meta
        );
    }

    #[test]
    fn ensure_block_creates_missing_rc_file() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join(".bashrc");

        let outcome = ensure_block(&rc).unwrap();
        assert!(outcome.modified);
        assert!(outcome.backup.is_none());
        let after = std::fs::read_to_string(&rc).unwrap();
        assert!(after.starts_with(START_MARKER));
        // Nothing to back up when the file did not exist.
        assert!(!backup_path(&rc).exists());
    }

    // ─── remove_block ─────────────────────────────────────────────────────

    #[test]
    fn remove_block_is_surgical() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join(".bashrc");
        let prior = "line one\nline two\n";
        std::fs::write(&rc, prior).unwrap();
        ensure_block(&rc).unwrap();

        assert!(remove_block(&rc).unwrap());
        let after = std::fs::read_to_string(&rc).unwrap();
        assert!(!after.contains(START_MARKER));
        assert!(!after.contains(END_MARKER));
        assert!(after.contains("line one"));
        assert!(after.contains("line two"));
    }

    #[test]
    fn remove_block_noop_when_absent_or_missing() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join(".bashrc");

        // Missing file.
        assert!(!remove_block(&rc).unwrap());

        // File without markers.
        std::fs::write(&rc, "just user content\n").unwrap();
        assert!(!remove_block(&rc).unwrap());
        assert_eq!(
            std::fs::read_to_string(&rc).unwrap(),
            "just user content\n"
        );
    }

    #[test]
    fn strip_block_preserves_surrounding_lines_exactly() {
        let content = "a\nb\n\n# >>> cryptenv initialize >>>\nsource x\n# <<< cryptenv initialize <<<\nc\n";
        let stripped = strip_block(content).unwrap();
        assert_eq!(stripped, "a\nb\n\nc\n");
    }

    // ─── symlink / read-only targets ─────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn ensure_block_writes_through_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real_bashrc");
        std::fs::write(&target, "real content\n").unwrap();
        let link = dir.path().join(".bashrc");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(ensure_block(&link).unwrap().modified);

        // The link is still a link; the target received the block.
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        let target_content = std::fs::read_to_string(&target).unwrap();
        assert!(target_content.starts_with("real content\n"));
        assert!(target_content.contains(START_MARKER));
    }

    #[cfg(unix)]
    #[test]
    fn ensure_block_errors_on_read_only_target_without_corrupting_it() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join(".bashrc");
        let original = "important user rc\n";
        std::fs::write(&rc, original).unwrap();
        std::fs::set_permissions(&rc, std::fs::Permissions::from_mode(0o400)).unwrap();

        let result = ensure_block(&rc);

        // Restore write permission so tempdir cleanup and assertions work.
        std::fs::set_permissions(&rc, std::fs::Permissions::from_mode(0o600)).unwrap();

        assert!(matches!(result, Err(SetupError::Io(_))));
        assert_eq!(std::fs::read_to_string(&rc).unwrap(), original);
        // The backup, if it was taken, is a faithful copy — never a partial write.
        let backup = backup_path(&rc);
        if backup.exists() {
            assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);
        }
    }
}
