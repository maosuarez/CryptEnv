//! Manage scoped access tokens (`crate::tokens` on the server side) — the
//! credential a headless caller (CI, an autonomous agent, a script) uses to
//! read one environment's values without the interactive unlock flow.
//! Creating/listing/revoking tokens is itself a vault-owner operation, so it
//! goes through the normal session-authenticated endpoints, same as
//! `crypt-env project`.

use clap::{Args, Subcommand};
use serde::Deserialize;

use crate::client::{authenticated_delete, authenticated_get, authenticated_post, CliError, API_BASE};

// ─── CLI argument structs ─────────────────────────────────────────────────────

#[derive(Args)]
pub struct TokenArgs {
    #[command(subcommand)]
    pub cmd: TokenCmd,
}

#[derive(Subcommand)]
pub enum TokenCmd {
    /// Create a token scoped to one environment
    Create {
        /// Name for this token (shown in `token list`, not the secret itself)
        #[arg(long)]
        name: String,
        /// Environment ID
        #[arg(long, conflicts_with_all = ["project", "environment"])]
        id: Option<i64>,
        /// Project name (case-insensitive, used together with --environment)
        #[arg(long, requires = "environment")]
        project: Option<String>,
        /// Environment name within the project (case-insensitive)
        #[arg(long, requires = "project")]
        environment: Option<String>,
        /// Let this token decrypt on its own, without an unlocked vault —
        /// for headless/CI use. Default is session-bound: the token only
        /// works while the vault is unlocked on this machine.
        #[arg(long)]
        standalone: bool,
    },
    /// List all access tokens (never shows token values)
    List,
    /// Revoke an access token by ID
    Revoke {
        #[arg(long)]
        id: i64,
    },
}

// ─── Response types ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreatedToken {
    id: i64,
    token: String,
}

#[derive(Deserialize)]
struct TokenSummary {
    id: i64,
    name: String,
    #[serde(rename = "environmentId")]
    environment_id: i64,
    mode: String,
    created: String,
    #[serde(default)]
    revoked: Option<String>,
    #[serde(default, rename = "lastUsed")]
    last_used: Option<String>,
}

#[derive(Deserialize)]
struct ProjectLookup {
    name: String,
    environments: Vec<EnvLookup>,
}

#[derive(Deserialize)]
struct EnvLookup {
    id: i64,
    name: String,
}

// ─── Entry point ──────────────────────────────────────────────────────────────

pub fn run(args: TokenArgs) -> Result<(), CliError> {
    match args.cmd {
        TokenCmd::Create { name, id, project, environment, standalone } => {
            run_create(name, id, project, environment, standalone)
        }
        TokenCmd::List => run_list(),
        TokenCmd::Revoke { id } => run_revoke(id),
    }
}

// ─── Create ──────────────────────────────────────────────────────────────────

fn run_create(
    name: String,
    id: Option<i64>,
    project: Option<String>,
    environment: Option<String>,
    standalone: bool,
) -> Result<(), CliError> {
    let environment_id = match (id, project, environment) {
        (Some(i), _, _) => i,
        (None, Some(p), Some(e)) => resolve_environment_id(&p, &e)?,
        _ => {
            return Err(CliError::Api(
                "provide --id, or both --project and --environment to identify the environment".into(),
            ));
        }
    };

    let mode = if standalone { "standalone" } else { "session" };
    let body = serde_json::json!({
        "name": name,
        "environmentId": environment_id,
        "mode": mode,
    });

    let resp = authenticated_post(&format!("{API_BASE}/tokens"), &body)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(CliError::NotFound(format!("environment {environment_id}")));
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("create failed: {text}")));
    }

    let created: CreatedToken = resp.json().map_err(|e| CliError::Api(e.to_string()))?;
    println!("Token created (id {}):", created.id);
    println!();
    println!("  {}", created.token);
    println!();
    println!("This value is shown once and cannot be recovered — store it now.");
    if standalone {
        println!("Mode: standalone — this token can decrypt on its own, without an unlocked vault.");
    } else {
        println!("Mode: session — this token only works while the vault is unlocked.");
    }

    Ok(())
}

/// Find the environment ID matching a project name + environment name (both case-insensitive).
fn resolve_environment_id(project: &str, environment: &str) -> Result<i64, CliError> {
    let resp = authenticated_get(&format!("{API_BASE}/projects"))?;
    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        return Err(CliError::Api(format!("list projects failed: HTTP {}", resp.status())));
    }
    let projects: Vec<ProjectLookup> = resp.json().map_err(|e| CliError::Api(e.to_string()))?;

    let project_lower = project.to_lowercase();
    let env_lower = environment.to_lowercase();

    projects
        .into_iter()
        .find(|p| p.name.to_lowercase() == project_lower)
        .and_then(|p| p.environments.into_iter().find(|e| e.name.to_lowercase() == env_lower))
        .map(|e| e.id)
        .ok_or_else(|| CliError::NotFound(format!("{project} / {environment}")))
}

// ─── List ─────────────────────────────────────────────────────────────────────

fn run_list() -> Result<(), CliError> {
    let resp = authenticated_get(&format!("{API_BASE}/tokens"))?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("list failed: {text}")));
    }

    let tokens: Vec<TokenSummary> = resp.json().map_err(|e| CliError::Api(e.to_string()))?;

    if tokens.is_empty() {
        println!("No access tokens found.");
        return Ok(());
    }

    println!(
        "{:<5} {:<24} {:<14} {:<11} {:<9} {:<22} {}",
        "ID", "Name", "Environment", "Mode", "Status", "Created", "Last used"
    );
    println!("{}", "-".repeat(105));
    for t in tokens {
        let status = if t.revoked.is_some() { "revoked" } else { "active" };
        println!(
            "{:<5} {:<24} {:<14} {:<11} {:<9} {:<22} {}",
            t.id,
            t.name,
            t.environment_id,
            t.mode,
            status,
            t.created,
            t.last_used.as_deref().unwrap_or("never"),
        );
    }

    Ok(())
}

// ─── Revoke ───────────────────────────────────────────────────────────────────

fn run_revoke(id: i64) -> Result<(), CliError> {
    let resp = authenticated_delete(&format!("{API_BASE}/tokens/{id}"))?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if resp.status() == reqwest::StatusCode::NO_CONTENT || resp.status().is_success() {
        println!("Token {id} revoked.");
        return Ok(());
    }

    let text = resp.text().unwrap_or_default();
    Err(CliError::Api(format!("revoke failed: {text}")))
}
