use clap::Args;
use crate::client::{CliError, read_token};
use crate::commands::scope;

#[derive(Args)]
pub struct DoctorArgs {}

pub fn run(_args: DoctorArgs) -> Result<(), CliError> {
    println!("crypt-env doctor — system diagnostics\n");

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| CliError::Api(e.to_string()))?;

    match client.get(format!("{base}/health", base = crate::client::api_base())).send() {
        Err(e) if e.is_connect() => {
            println!("  [!!] App running        not running — open crypt-env and try again");
            println!();
            return Ok(());
        }
        Err(e) => {
            println!("  [!!] App running        error: {e}");
            println!();
            return Ok(());
        }
        Ok(resp) => {
            let json: serde_json::Value =
                resp.json().map_err(|e| CliError::Api(e.to_string()))?;

            let version = json.get("version").and_then(|v| v.as_str()).unwrap_or("?");
            let vault_locked = json
                .get("vault_locked")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let mcp_configured = json
                .get("mcp_token_configured")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            println!("  [OK] App running        https://127.0.0.1:47821  (v{version})");

            if vault_locked {
                println!("  [!!] Vault status       locked");
            } else {
                // /health no longer reports item_count (it leaked vault size
                // to unauthenticated callers) — vault size is no longer
                // available from doctor.
                println!("  [OK] Vault status       unlocked");
            }

            if mcp_configured {
                println!("  [OK] MCP token          configured in app");
            } else {
                println!("  [--] MCP token          not configured  (Settings → INTEGRATIONS)");
            }
        }
    }

    // CLI session token
    if read_token().is_some() {
        println!("  [OK] CLI session        token cached");
    } else {
        println!("  [--] CLI session        no cached token  (will prompt on next use)");
    }

    // MCP token file on disk
    let mcp_path = std::env::var("APPDATA")
        .map(|d| {
            std::path::PathBuf::from(d)
                .join("com.maosuarez.cryptenv")
                .join("mcp_token")
        })
        .or_else(|_| {
            std::env::var("HOME").map(|d| {
                std::path::PathBuf::from(d)
                    .join(".local")
                    .join("share")
                    .join("com.maosuarez.cryptenv")
                    .join("mcp_token")
            })
        });

    if let Ok(path) = mcp_path {
        if path.exists() {
            println!("  [OK] MCP token file     {}", path.display());
        } else {
            println!("  [--] MCP token file     not found  ({})", path.display());
        }
    }

    // crypt-env.json — validate it's well-formed if present anywhere in the
    // search path (cwd upward), same discovery used by every scoped command.
    match std::env::current_dir() {
        Ok(cwd) => match find_project_config_path(&cwd) {
            Some(path) => match scope::parse_project_config(&path) {
                Ok(config) => {
                    let env_display = config.environment.as_deref().unwrap_or("(project default)");
                    println!(
                        "  [OK] crypt-env.json     {}  (project: {}, environment: {})",
                        path.display(),
                        config.project,
                        env_display
                    );
                }
                Err(e) => {
                    println!("  [!!] crypt-env.json     {}: {}", path.display(), e);
                }
            },
            None => {
                println!("  [--] crypt-env.json     none found  (using cwd folder name as project)");
            }
        },
        Err(e) => {
            println!("  [!!] crypt-env.json     cannot read current directory: {}", e);
        }
    }

    println!();
    Ok(())
}

/// Locates `crypt-env.json` without parsing it — used to report its path
/// separately from parse/validation errors.
fn find_project_config_path(start: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut dir = Some(start.to_path_buf());
    while let Some(d) = dir {
        let candidate = d.join(scope::CONFIG_FILE_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    None
}
