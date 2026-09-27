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

fn run_wsl(args: WslArgs) -> Result<(), CliError> {
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
