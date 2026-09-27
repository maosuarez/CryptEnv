//! `crypt-env-setup` — static helper the Windows GUI pushes into a WSL distro
//! to apply or remove the `setup wsl` shell configuration without requiring a
//! Linux `crypt-env` build. Prints the [`cryptenv_setup::ActionReport`] as a
//! single JSON line on stdout; errors go to stderr with exit code 1.
//!
//! Usage:
//!   crypt-env-setup [--api-url URL] [--cert-path PATH] [--appdata WINPATH]
//!   crypt-env-setup --remove

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cryptenv_setup::{apply, cert_under, remove, resolve_values, windows_path_to_wsl, SetupError};

#[derive(Debug, Default, PartialEq, Eq)]
struct Args {
    remove: bool,
    api_url: Option<String>,
    cert_path: Option<PathBuf>,
    /// Windows `%APPDATA%` (e.g. `C:\Users\me\AppData\Roaming`).
    appdata: Option<String>,
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
            other => return Err(SetupError::Config(format!("unknown argument: {other}"))),
        }
    }
    Ok(args)
}

/// Converts the Windows `%APPDATA%` to its in-distro path, preferring
/// `wslpath -u` (honours a custom automount root) over the `/mnt/<drive>`
/// default mapping.
fn appdata_to_linux(appdata: &str) -> Option<PathBuf> {
    let via_wslpath = std::process::Command::new("wslpath")
        .arg("-u")
        .arg(appdata)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    via_wslpath.or_else(|| windows_path_to_wsl(appdata))
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
        .and_then(appdata_to_linux)
        .map(cert_under);
    let values = resolve_values(
        args.api_url.as_deref(),
        args.cert_path.as_deref(),
        None,
        None,
        derived.as_deref().map(Path::new),
    )?;
    apply(&home, &values)
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
        ]))
        .unwrap();
        assert_eq!(a.api_url.as_deref(), Some("https://127.0.0.1:47821"));
        assert_eq!(a.appdata.as_deref(), Some(r"C:\Users\me\AppData\Roaming"));
        assert!(!a.remove);

        assert!(parse_args(argv(&["--remove"])).unwrap().remove);
    }

    #[test]
    fn rejects_unknown_and_missing_values() {
        assert!(parse_args(argv(&["--bogus"])).is_err());
        assert!(parse_args(argv(&["--cert-path"])).is_err());
    }
}
