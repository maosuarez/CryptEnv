//! `crypt-env-setup` — static helper the Windows GUI pushes into a WSL distro
//! to apply or remove the `setup wsl` shell configuration without requiring a
//! Linux `crypt-env` build. Prints the [`cryptenv_setup::ActionReport`] as a
//! single JSON line on stdout; errors go to stderr with exit code 1.
//!
//! Usage:
//!   crypt-env-setup [--api-url URL] [--cert-path PATH] [--appdata WINPATH]
//!                   [--windows-cli WINPATH]
//!   crypt-env-setup --remove
//!
//! `--windows-cli` is the Windows `crypt-env.exe` the managed `crypt-env`
//! launcher delegates to; without it (or when it is not reachable from the
//! distro) the launcher is reported as skipped.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cryptenv_setup::{
    apply, apply_launcher, cert_under, remove, resolve_values, windows_path_to_wsl, SetupError,
};

#[derive(Debug, Default, PartialEq, Eq)]
struct Args {
    remove: bool,
    api_url: Option<String>,
    cert_path: Option<PathBuf>,
    /// Windows `%APPDATA%` (e.g. `C:\Users\me\AppData\Roaming`).
    appdata: Option<String>,
    /// Windows path of `crypt-env.exe` (e.g. `C:\Program Files\CryptEnv\crypt-env.exe`).
    windows_cli: Option<String>,
}

fn parse_args(argv: impl IntoIterator<Item = String>) -> Result<Args, SetupError> {
    let mut args = Args::default();
    let mut it = argv.into_iter();
    while let Some(flag) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .ok_or_else(|| SetupError::Config(format!("{name} requires a value")))
        };
        match flag.as_str() {
            "--remove" => args.remove = true,
            "--api-url" => args.api_url = Some(value("--api-url")?),
            "--cert-path" => args.cert_path = Some(PathBuf::from(value("--cert-path")?)),
            "--appdata" => args.appdata = Some(value("--appdata")?),
            "--windows-cli" => args.windows_cli = Some(value("--windows-cli")?),
            other => return Err(SetupError::Config(format!("unknown argument: {other}"))),
        }
    }
    Ok(args)
}

/// Converts a Windows path (`%APPDATA%`, the CLI location) to its in-distro
/// path, preferring `wslpath -u` (honours a custom automount root) over the
/// `/mnt/<drive>` default mapping.
fn windows_to_linux(win: &str) -> Option<PathBuf> {
    let via_wslpath = std::process::Command::new("wslpath")
        .arg("-u")
        .arg(win)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    via_wslpath.or_else(|| windows_path_to_wsl(win))
}

fn run(args: Args) -> Result<cryptenv_setup::ActionReport, SetupError> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| SetupError::Config("HOME is not set".to_string()))?;

    if args.remove {
        return remove(&home);
    }

    let derived = args
        .appdata
        .as_deref()
        .and_then(windows_to_linux)
        .map(cert_under);
    let values = resolve_values(
        args.api_url.as_deref(),
        args.cert_path.as_deref(),
        None,
        None,
        derived.as_deref().map(Path::new),
    )?;
    let mut report = apply(&home, &values)?;
    let exe = args
        .windows_cli
        .as_deref()
        .and_then(windows_to_linux)
        .filter(|p| p.is_file());
    apply_launcher(&home, exe.as_deref(), &mut report)?;
    Ok(report)
}

fn main() -> ExitCode {
    let result = parse_args(std::env::args().skip(1)).and_then(run);
    match result.and_then(|r| {
        serde_json::to_string(&r)
            .map_err(|e| SetupError::Config(format!("cannot serialize report: {e}")))
    }) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("crypt-env-setup: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cryptenv_setup::LauncherStatus;

    fn argv(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_all_flags() {
        let a = parse_args(argv(&[
            "--api-url",
            "https://127.0.0.1:47821",
            "--appdata",
            r"C:\Users\me\AppData\Roaming",
            "--windows-cli",
            r"D:\Apps\Crypt Env\crypt-env.exe",
        ]))
        .unwrap();
        assert_eq!(a.windows_cli.as_deref(), Some(r"D:\Apps\Crypt Env\crypt-env.exe"));
        assert_eq!(a.api_url.as_deref(), Some("https://127.0.0.1:47821"));
        assert_eq!(a.appdata.as_deref(), Some(r"C:\Users\me\AppData\Roaming"));
        assert!(!a.remove);

        assert!(parse_args(argv(&["--remove"])).unwrap().remove);
    }

    #[test]
    fn rejects_unknown_and_missing_values() {
        assert!(parse_args(argv(&["--bogus"])).is_err());
        assert!(parse_args(argv(&["--cert-path"])).is_err());
        assert!(parse_args(argv(&["--windows-cli"])).is_err());
    }

    fn run_in(home: &Path, args: Args) -> cryptenv_setup::ActionReport {
        // `run` reads HOME; tests in this module that call it are serialised
        // through this lock.
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("HOME", home);
        run(args).unwrap()
    }

    #[test]
    fn run_without_windows_cli_skips_launcher() {
        let home = tempfile::tempdir().unwrap();
        let r = run_in(
            home.path(),
            Args { cert_path: Some("/c.pem".into()), ..Args::default() },
        );
        assert_eq!(r.launcher_status, LauncherStatus::Skipped);
        assert_eq!(r.launcher_note.as_deref(), Some(cryptenv_setup::LAUNCHER_NOTE_CLI_NOT_FOUND));
    }

    #[test]
    fn run_with_unreachable_windows_cli_skips_launcher() {
        let home = tempfile::tempdir().unwrap();
        let r = run_in(
            home.path(),
            Args {
                cert_path: Some("/c.pem".into()),
                windows_cli: Some(r"Q:\nowhere\crypt-env.exe".into()),
                ..Args::default()
            },
        );
        assert_eq!(r.launcher_status, LauncherStatus::Skipped);
        assert!(!cryptenv_setup::launcher_path(home.path()).exists());
    }

    #[test]
    fn run_then_remove_round_trips_launcher() {
        let home = tempfile::tempdir().unwrap();
        // A Linux absolute path is not a drive path, so exercise the helper
        // through the lib directly for the "found" case.
        let exe = home.path().join("crypt-env.exe");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        let mut r = run_in(home.path(), Args { cert_path: Some("/c.pem".into()), ..Args::default() });
        apply_launcher(home.path(), Some(&exe), &mut r).unwrap();
        assert_eq!(r.launcher_status, LauncherStatus::Written);

        let removed = run_in(home.path(), Args { remove: true, ..Args::default() });
        assert_eq!(removed.launcher_status, LauncherStatus::Deleted);
        assert!(removed.env_file_changed);
    }
}
