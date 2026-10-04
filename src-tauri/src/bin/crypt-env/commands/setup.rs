//! `crypt-env setup wsl` — non-destructively persist the `CRYPTENV_*` client
//! configuration into the invoking user's shell startup.
//!
//! Model: `conda init` / `rustup`. A fully-owned `~/.config/cryptenv/env.sh`
//! holds the `export` lines and is rewritten every run; a single
//! marker-delimited block sourcing that file is ensured in `~/.bashrc` (and
//! `~/.zshrc` when it exists). All rc-file edits are whole-file read →
//! block-boundary slice → whole-file write, with a one-time backup before the
//! first modification. User content outside the marker block is never touched.
//!
//! The file logic lives in the shared `cryptenv_setup` crate so the Windows
//! GUI's WSL helper applies exactly the same contract.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use cryptenv_setup::{cert_under, windows_path_to_wsl, ActionReport, SetupError};

use crate::client::CliError;

#[derive(Args)]
pub struct SetupArgs {
    #[command(subcommand)]
    cmd: SetupCmd,
}

#[derive(Subcommand)]
enum SetupCmd {
    /// Persist CRYPTENV_* config into your shell startup (WSL ↔ Windows split)
    Wsl(WslArgs),
}

#[derive(Args)]
pub struct WslArgs {
    /// Distribution to configure when run on the Windows host (inside WSL the
    /// current distribution is used)
    distro: Option<String>,
    /// Remove the cryptenv shell block and env.sh instead of installing them
    #[arg(long)]
    remove: bool,
    /// Absolute path to the Windows TLS cert PEM as seen from WSL
    /// (e.g. /mnt/c/Users/<you>/AppData/Roaming/com.maosuarez.cryptenv/tls/cert.pem)
    #[arg(long)]
    cert_path: Option<PathBuf>,
    /// REST base URL to record (defaults to https://127.0.0.1:47821)
    #[arg(long)]
    api_url: Option<String>,
}

impl From<SetupError> for CliError {
    fn from(e: SetupError) -> Self {
        match e {
            SetupError::Io(e) => CliError::Io(e),
            SetupError::Config(msg) => CliError::Config(msg),
        }
    }
}

pub fn run(args: SetupArgs) -> Result<(), CliError> {
    match args.cmd {
        SetupCmd::Wsl(w) => run_wsl(w),
    }
}

/// What `setup wsl` does on the Windows host.
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    NoDistros,
    /// Ambiguous: list the distros and ask for one.
    Choose(Vec<String>),
    Configure(String),
    Unknown(String, Vec<String>),
}

fn plan_windows(distros: Vec<String>, arg: Option<&str>) -> Plan {
    match arg {
        _ if distros.is_empty() => Plan::NoDistros,
        None => Plan::Choose(distros),
        Some(d) => match distros.iter().find(|x| x.eq_ignore_ascii_case(d)) {
            Some(found) => Plan::Configure(found.clone()),
            None => Plan::Unknown(d.to_string(), distros),
        },
    }
}

fn run_wsl(args: WslArgs) -> Result<(), CliError> {
    if cfg!(target_os = "windows") {
        return run_wsl_from_windows(args);
    }
    if let (Some(want), Ok(current)) = (args.distro.as_deref(), std::env::var("WSL_DISTRO_NAME")) {
        if !want.eq_ignore_ascii_case(&current) {
            return Err(CliError::Config(format!(
                "this shell runs inside '{current}'; to configure '{want}' run `crypt-env setup wsl {want}` from Windows or inside that distro"
            )));
        }
    }
    if let Ok(current) = std::env::var("WSL_DISTRO_NAME") {
        println!("crypt-env: detected WSL distribution '{current}'");
    }
    run_local(args)
}

fn run_wsl_from_windows(args: WslArgs) -> Result<(), CliError> {
    use crypt_env_lib::wsl::client_setup as cs;
    let runner = WslExe;
    // The shared WSL core is async (bounded, reaped `wsl.exe` children); the CLI
    // is synchronous, so drive it on a throwaway current-thread runtime.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::Config(format!("cannot start async runtime: {e}")))?;
    let distros = match rt.block_on(cs::list_distros(&runner)) {
        Ok(d) => d,
        Err(cs::WslError::NotAvailable) => Vec::new(),
        Err(e) => return Err(CliError::Config(e.to_string())),
    };
    match plan_windows(distros, args.distro.as_deref()) {
        Plan::NoDistros => {
            println!("No WSL distributions found on this system.");
            Ok(())
        }
        Plan::Choose(list) => {
            println!("WSL distributions found:");
            for d in &list {
                println!("  {d}");
            }
            println!("Run 'crypt-env setup wsl <distro>' to configure a specific one.");
            Ok(())
        }
        Plan::Unknown(d, list) => Err(CliError::Config(format!(
            "unknown WSL distribution '{d}' (installed: {})",
            list.join(", ")
        ))),
        Plan::Configure(distro) => {
            let exe = std::env::current_exe()?;
            let dir = exe.parent().map(PathBuf::from).unwrap_or_default();
            let helper = dir.join(cs::HELPER_RESOURCE);
            if !helper.is_file() {
                return Err(CliError::Config(format!(
                    "the WSL setup helper was not found at {} — reinstall crypt-env or use Settings → WSL in the app",
                    helper.display()
                )));
            }
            let helper = helper.to_string_lossy().into_owned();
            let report = if args.remove {
                rt.block_on(cs::remove_client(&runner, &distro, &helper))
            } else {
                let appdata = std::env::var("APPDATA")
                    .map_err(|_| CliError::Config("APPDATA is not set".to_string()))?;
                rt.block_on(cs::configure_client(&runner, &distro, &helper, &appdata, exe.to_str()))
            }
            .map_err(|e| CliError::Config(e.to_string()))?;
            println!("crypt-env: configured distribution '{distro}'");
            if args.remove {
                print_remove(&report);
            } else {
                print_apply(&report);
            }
            Ok(())
        }
    }
}

/// Runs `wsl.exe` with an argument vector (never a shell string).
struct WslExe;

impl crypt_env_lib::wsl::client_setup::WslRunner for WslExe {
    async fn run(
        &self,
        args: &[String],
        timeout: std::time::Duration,
    ) -> std::io::Result<crypt_env_lib::wsl::client_setup::RunOutput> {
        let mut cmd = tokio::process::Command::new("wsl.exe");
        cmd.args(args).env("WSL_UTF8", "1");
        crypt_env_lib::wsl::client_setup::run_process(cmd, timeout).await
    }
}

fn run_local(args: WslArgs) -> Result<(), CliError> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| {
            CliError::Config("HOME is not set; cannot locate the shell startup files".to_string())
        })?;

    if args.remove {
        let report = cryptenv_setup::remove(&home)?;
        print_remove(&report);
        return Ok(());
    }

    let values = cryptenv_setup::resolve_values(
        args.api_url.as_deref(),
        args.cert_path.as_deref(),
        std::env::var("CRYPTENV_API_URL").ok().as_deref(),
        std::env::var("CRYPTENV_CERT_PATH").ok().as_deref(),
        derive_windows_cert_path().as_deref(),
    )?;
    let report = cryptenv_setup::apply(&home, &values)?;
    print_apply(&report);
    Ok(())
}

/// Best-effort derivation of the Windows cert path from Windows environment
/// variables forwarded into WSL (`APPDATA`, else `USERPROFILE`). Returns `None`
/// on a default WSL setup where neither is forwarded — the caller then requires
/// `--cert-path`.
fn derive_windows_cert_path() -> Option<PathBuf> {
    if let Ok(appdata) = std::env::var("APPDATA") {
        if let Some(base) = windows_path_to_wsl(&appdata) {
            return Some(cert_under(base));
        }
    }
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        if let Some(base) = windows_path_to_wsl(&userprofile) {
            return Some(cert_under(base.join("AppData").join("Roaming")));
        }
    }
    None
}

fn print_apply(report: &ActionReport) {
    println!("crypt-env: wrote {}", report.env_file);
    if report.rc_files.is_empty() {
        println!("crypt-env: shell block already present; rc files unchanged");
    } else {
        for rc in &report.rc_files {
            println!("crypt-env: added the shell block to {rc}");
        }
    }
    for b in &report.backups {
        println!("crypt-env: backup written to {b}");
    }
    println!("Open a new shell, or run `. ~/.config/cryptenv/env.sh`, to apply.");
}

fn print_remove(report: &ActionReport) {
    use cryptenv_setup::LauncherStatus;
    if let (LauncherStatus::Deleted, Some(l)) = (report.launcher_status, &report.launcher) {
        println!("crypt-env: deleted the managed launcher {l}");
    }
    if report.rc_files.is_empty()
        && !report.env_file_changed
        && report.launcher_status != LauncherStatus::Deleted
    {
        println!("crypt-env: nothing to remove");
        return;
    }
    for rc in &report.rc_files {
        println!("crypt-env: removed the shell block from {rc}");
    }
    if report.env_file_changed {
        println!("crypt-env: deleted {}", report.env_file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_windows_cases() {
        let two = vec!["Ubuntu".to_string(), "Debian".to_string()];
        assert_eq!(plan_windows(vec![], None), Plan::NoDistros);
        assert_eq!(plan_windows(vec![], Some("Ubuntu")), Plan::NoDistros);
        assert_eq!(plan_windows(two.clone(), None), Plan::Choose(two.clone()));
        assert_eq!(plan_windows(two.clone(), Some("ubuntu")), Plan::Configure("Ubuntu".into()));
        assert_eq!(plan_windows(two.clone(), Some("Arch")), Plan::Unknown("Arch".into(), two));
    }
}
