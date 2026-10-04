//! Standalone CLI binary for the encrypted secrets vault.
//! Connects to the local REST API at https://127.0.0.1:47821.

mod client;
mod commands;
mod paths;
mod prompts;
mod shell;
#[cfg(test)]
mod testing;
mod terminal;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "crypt-env", version, about = "Encrypted secrets vault CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Register this directory as a vault project and write .crypt-env.yaml
    Init(commands::init::InitArgs),
    /// Sync .crypt-env.yaml with the vault (last modified wins)
    Config(commands::config::ConfigArgs),
    /// Add secrets from KEY=value, $VARNAME, or a .env file (halts on collisions)
    Add(commands::add::AddArgs),
    /// Write each environment's .env target(s) plus .env.example (password required)
    Fill(commands::fill::FillArgs),
    /// Provision keys from .env.example into the vault (password required)
    Sync(commands::sync::SyncArgs),
    /// Print shell assignments for eval, e.g. eval "$(crypt-env inject KEY)" (password required)
    Inject(commands::inject::InjectArgs),
    /// List/search variable names — never values (password required)
    Search(commands::search::SearchArgs),
    /// Diagnose app, vault, TLS, tokens, project config and WSL
    Doctor(commands::doctor::DoctorArgs),
    /// Configure the CLI for a split setup (`setup wsl [DISTRO]`)
    Setup(commands::setup::SetupArgs),
    /// Interactive terminal UI for projects, environments and variables
    Tui(commands::tui::TuiArgs),
}

fn main() {
    let cli = Cli::parse();
    let result = client::init_api_base().and_then(|()| match cli.cmd {
        Cmd::Init(args) => commands::init::run(args),
        Cmd::Config(args) => commands::config::run(args),
        Cmd::Add(args) => commands::add::run(args),
        Cmd::Fill(args) => commands::fill::run(args),
        Cmd::Sync(args) => commands::sync::run(args),
        Cmd::Inject(args) => commands::inject::run(args),
        Cmd::Search(args) => commands::search::run(args),
        Cmd::Doctor(args) => commands::doctor::run(args),
        Cmd::Setup(args) => commands::setup::run(args),
        Cmd::Tui(args) => commands::tui::run(args),
    });
    if let Err(e) = result {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_commands_are_unrecognized() {
        for name in ["memory", "list", "exec", "cmd", "project", "share", "relay", "category", "set"] {
            let err = Cli::try_parse_from(["crypt-env", name]).err().expect(name);
            assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand, "{name}");
        }
    }
}
