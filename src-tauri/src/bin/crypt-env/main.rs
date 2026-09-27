//! Standalone CLI binary for the encrypted secrets vault.
//! Connects to the local REST API at https://127.0.0.1:47821.

mod client;
mod commands;
mod prompts;
mod shell;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "crypt-env", version, about = "Encrypted secrets vault CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Add a secret from KEY=value, $VARNAME, or a .env file
    Add(commands::add::AddArgs),
    /// Check app health, vault status, token files, and version
    Doctor(commands::doctor::DoctorArgs),
    /// Fill a .env or .env.example with secrets from the vault
    Fill(commands::fill::FillArgs),
    /// Print a shell assignment for eval (stdout) — safe for pipe
    Inject(commands::inject::InjectArgs),
    /// List saved commands in a table
    List(commands::list::ListArgs),
    /// Execute a saved command by name
    Exec(commands::exec::ExecArgs),
    /// Save a command string to the vault (interactive)
    Memory(commands::memory::MemoryArgs),
    /// Search items by name (no values exposed)
    Search(commands::search::SearchArgs),
    /// Print export/env assignment for a secret
    Set(commands::set::SetArgs),
    /// Manage saved commands (list, info, run)
    Cmd(commands::cmd::CmdArgs),
    /// Share secrets with a teammate (LAN bridge or encrypted package)
    Share(commands::share::ShareArgs),
    /// Manage categories (list, create, edit, delete)
    Category(commands::category::CategoryArgs),
    /// Interactive TUI — browse, search, and reveal secrets in the terminal
    Tui(commands::tui::TuiArgs),
    /// Manage projects and their environments (list, inject, delete)
    Project(commands::project::ProjectArgs),
    /// Configure the CLI for a split setup (e.g. `setup wsl`)
    Setup(commands::setup::SetupArgs),
    /// Share secrets via internet relay (send, receive)
    Relay(commands::relay::RelayArgs),
    /// Sync new variables from .env.example into .env without overwriting existing values
    Sync(commands::sync::SyncArgs),
}

fn main() {
    let cli = Cli::parse();
    let result = client::init_api_base().and_then(|()| match cli.cmd {
        Cmd::Add(args) => commands::add::run(args),
        Cmd::Doctor(args) => commands::doctor::run(args),
        Cmd::Fill(args) => commands::fill::run(args),
        Cmd::Inject(args) => commands::inject::run(args),
        Cmd::List(args) => commands::list::run(args),
        Cmd::Exec(args) => commands::exec::run(args),
        Cmd::Memory(args) => commands::memory::run(args),
        Cmd::Search(args) => commands::search::run(args),
        Cmd::Set(args) => commands::set::run(args),
        Cmd::Cmd(args) => commands::cmd::run(args),
        Cmd::Share(args) => commands::share::run(args),
        Cmd::Category(args) => commands::category::run(args),
        Cmd::Tui(args) => commands::tui::run(args),
        Cmd::Project(args) => commands::project::run(args),
        Cmd::Relay(args) => commands::relay::run(args),
        Cmd::Sync(args) => commands::sync::run(args),
        Cmd::Setup(args) => commands::setup::run(args),
    });
    if let Err(e) = result {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}
