use clap::{Args, Subcommand};

use crate::client::{authenticated_post, CliError};
use crate::commands::scope;

// ─── CLI argument structs ─────────────────────────────────────────────────────

#[derive(Args)]
pub struct RelayArgs {
    #[command(subcommand)]
    pub cmd: RelayCmd,
}

#[derive(Subcommand)]
pub enum RelayCmd {
    /// Send vault items via internet relay — prints a code and passphrase
    Send {
        /// Comma-separated item IDs to share (e.g. 1,2,3)
        #[arg(long)]
        items: String,
    },
    /// Receive items shared via internet relay
    Receive {
        /// Relay code (e.g. X7K2-M9P4)
        #[arg(long)]
        code: String,
        /// Passphrase provided by the sender
        #[arg(long)]
        passphrase: String,
        /// Project to own the received items (defaults to crypt-env.json or the cwd folder name)
        #[arg(long)]
        project: Option<String>,
        /// Environment to link the received items into (defaults to crypt-env.json or the project's default environment)
        #[arg(long = "env")]
        env: Option<String>,
    },
}

// ─── Entry point ──────────────────────────────────────────────────────────────

pub fn run(args: RelayArgs) -> Result<(), CliError> {
    match args.cmd {
        RelayCmd::Send { items } => run_send(items),
        RelayCmd::Receive { code, passphrase, project, env } => run_receive(code, passphrase, project, env),
    }
}

// ─── Send ─────────────────────────────────────────────────────────────────────

fn run_send(items_str: String) -> Result<(), CliError> {
    let item_ids: Vec<i64> = items_str
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<i64>().map_err(|e| CliError::Api(format!("invalid item ID '{s}': {e}"))))
        .collect::<Result<Vec<_>, _>>()?;

    if item_ids.is_empty() {
        return Err(CliError::Api("at least one item ID is required".into()));
    }

    let body = serde_json::json!({ "item_ids": item_ids });
    let resp = authenticated_post(&format!("{base}/relay/send", base = crate::client::api_base()), &body)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("relay send failed: {text}")));
    }

    let data: serde_json::Value = resp.json().map_err(|e| CliError::Api(e.to_string()))?;
    let code = data["code"].as_str().unwrap_or("").to_string();
    let passphrase = data["passphrase"].as_str().unwrap_or("").to_string();

    println!();
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║  RELAY SHARE CODE AND PASSPHRASE — show to recipient now ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║  Code:       {:<46} ║", code);
    println!("║  Passphrase: {:<46} ║", passphrase);
    println!("╚══════════════════════════════════════════════════════════╝");
    println!();
    println!("The passphrase is shown once. Provide both to the recipient.");

    Ok(())
}

// ─── Receive ──────────────────────────────────────────────────────────────────

fn run_receive(code: String, passphrase: String, project: Option<String>, env: Option<String>) -> Result<(), CliError> {
    let resolved_scope = scope::resolve(project.as_deref(), env.as_deref(), false)?;

    let body = serde_json::json!({ "code": code, "passphrase": passphrase });
    let url = resolved_scope.append_query(&format!("{base}/relay/receive", base = crate::client::api_base()));
    let resp = authenticated_post(&url, &body)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("relay receive failed: {text}")));
    }

    let data: serde_json::Value = resp.json().map_err(|e| CliError::Api(e.to_string()))?;
    let names: Vec<String> = data["names"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let skipped_keys: Vec<String> = data["skipped_keys"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    println!("Imported {} item(s) into {} / {}:", names.len(), resolved_scope.project, resolved_scope.environment);
    for name in &names {
        println!("  + {name}");
    }
    for key in &skipped_keys {
        eprintln!("warning: '{}' already linked in {} / {} — imported but not relinked", key, resolved_scope.project, resolved_scope.environment);
    }

    Ok(())
}
