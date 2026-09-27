use clap::{Args, Subcommand};
use std::path::PathBuf;

use crate::client::{authenticated_get, authenticated_post, CliError};
use crate::commands::scope;

// ─── CLI argument structs ─────────────────────────────────────────────────────

#[derive(Args)]
pub struct ShareArgs {
    #[command(subcommand)]
    pub cmd: ShareCmd,
}

#[derive(Subcommand)]
pub enum ShareCmd {
    /// Share selected vault items over the local network (sender)
    Send {
        /// Item IDs to share
        #[arg(required = true, num_args = 1..)]
        items: Vec<i64>,
        /// Project the shared items belong to (defaults to crypt-env.json or the cwd folder name)
        #[arg(long)]
        project: Option<String>,
        /// Environment the shared items must already be linked into (defaults to crypt-env.json or the project's default environment)
        #[arg(long = "env")]
        env: Option<String>,
    },
    /// Receive shared items over the local network (receiver)
    Receive {
        /// Project to own the received items (defaults to crypt-env.json or the cwd folder name)
        #[arg(long)]
        project: Option<String>,
        /// Environment to link the received items into (defaults to crypt-env.json or the project's default environment)
        #[arg(long = "env")]
        env: Option<String>,
    },
    /// Export selected items as an encrypted .vault file
    Export {
        /// Item IDs to export
        #[arg(required = true, num_args = 1..)]
        items: Vec<i64>,
        /// Output file path
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Import items from an encrypted .vault file
    Import {
        /// Path to the .vault package file
        #[arg(short, long)]
        file: PathBuf,
        /// Project to own the imported items (defaults to crypt-env.json or the cwd folder name)
        #[arg(long)]
        project: Option<String>,
        /// Environment to link the imported items into (defaults to crypt-env.json or the project's default environment)
        #[arg(long = "env")]
        env: Option<String>,
    },
}

pub fn run(args: ShareArgs) -> Result<(), CliError> {
    match args.cmd {
        ShareCmd::Send { items, project, env } => run_send(items, project, env),
        ShareCmd::Receive { project, env } => run_receive(project, env),
        ShareCmd::Export { items, output } => run_export(items, output),
        ShareCmd::Import { file, project, env } => run_import(file, project, env),
    }
}

// ─── Send ─────────────────────────────────────────────────────────────────────

fn run_send(items: Vec<i64>, project: Option<String>, env: Option<String>) -> Result<(), CliError> {
    let resolved_scope = scope::resolve(project.as_deref(), env.as_deref(), false)?;

    // 1. Start listen session — items must already be linked into this
    // project/environment (the API 422s otherwise).
    let body = serde_json::json!({ "items": items });
    let url = resolved_scope.append_query(&format!("{base}/share/listen", base = crate::client::api_base()));
    let resp = authenticated_post(&url, &body)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("listen failed: {text}")));
    }

    let listen_data: serde_json::Value =
        resp.json().map_err(|e| CliError::Api(e.to_string()))?;
    let pairing_code = listen_data["pairing_code"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    println!();
    println!("Share this code with your teammate: {} (expires in 5 min)", pairing_code);
    println!("Waiting for peer to connect...");

    // 2. Poll until AwaitingFingerprint
    let fingerprint = poll_until_fingerprint()?;

    println!();
    println!("Fingerprint: {} — confirm your teammate sees the same", fingerprint);
    println!();

    // 3. Prompt user for confirmation
    let confirmed = prompt_confirm("Confirm fingerprint? [y/N]: ")?;

    // 4. POST /share/confirm
    let confirm_body = serde_json::json!({ "confirmed": confirmed });
    let confirm_resp =
        authenticated_post(&format!("{base}/share/confirm", base = crate::client::api_base()), &confirm_body)?;
    if !confirm_resp.status().is_success() {
        let text = confirm_resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("confirm failed: {text}")));
    }

    if !confirmed {
        println!("Cancelled.");
        return Ok(());
    }

    println!("Fingerprint confirmed. Sending items...");

    // 5. Poll until Done or Failed
    poll_until_terminal("send")?;

    println!("Items shared successfully.");
    Ok(())
}

// ─── Receive ──────────────────────────────────────────────────────────────────

fn run_receive(project: Option<String>, env: Option<String>) -> Result<(), CliError> {
    let resolved_scope = scope::resolve(project.as_deref(), env.as_deref(), false)?;

    // 1. Prompt for pairing code
    let pairing_code = {
        use std::io::{self, Write};
        print!("Pairing code: ");
        io::stdout().flush().ok();
        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(CliError::Io)?;
        input.trim().to_string()
    };

    if pairing_code.is_empty() {
        return Err(CliError::Api("pairing code is required".into()));
    }

    // 2. POST /share/connect — received items get owned by this project and
    // linked into this environment.
    let body = serde_json::json!({ "pairing_code": pairing_code });
    let url = resolved_scope.append_query(&format!("{base}/share/connect", base = crate::client::api_base()));
    let resp = authenticated_post(&url, &body)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("connect failed: {text}")));
    }

    let connect_data: serde_json::Value =
        resp.json().map_err(|e| CliError::Api(e.to_string()))?;
    let fingerprint = connect_data["fingerprint"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    println!();
    println!("Fingerprint: {} — confirm your teammate sees the same", fingerprint);
    println!();

    // 3. Prompt for confirmation
    let confirmed = prompt_confirm("Confirm fingerprint? [y/N]: ")?;

    // 4. POST /share/confirm
    let confirm_body = serde_json::json!({ "confirmed": confirmed });
    let confirm_resp =
        authenticated_post(&format!("{base}/share/confirm", base = crate::client::api_base()), &confirm_body)?;
    if !confirm_resp.status().is_success() {
        let text = confirm_resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("confirm failed: {text}")));
    }

    if !confirmed {
        println!("Cancelled.");
        return Ok(());
    }

    println!("Receiving items...");

    // 5. Poll until Done or Failed
    let (names, skipped_keys) = poll_until_terminal_with_names()?;

    println!("Imported {} item(s):", names.len());
    for name in &names {
        println!("  - {}", name);
    }
    for key in &skipped_keys {
        eprintln!("warning: '{}' already linked in {} / {} — imported but not relinked", key, resolved_scope.project, resolved_scope.environment);
    }
    Ok(())
}

// ─── Export ───────────────────────────────────────────────────────────────────

fn run_export(items: Vec<i64>, output: PathBuf) -> Result<(), CliError> {
    let output_str = output.to_string_lossy().to_string();
    let body = serde_json::json!({
        "items": items,
        "output_path": output_str,
    });

    let resp = authenticated_post(&format!("{base}/share/export", base = crate::client::api_base()), &body)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("export failed: {text}")));
    }

    let data: serde_json::Value = resp.json().map_err(|e| CliError::Api(e.to_string()))?;
    let passphrase = data["passphrase"].as_str().unwrap_or("").to_string();
    let path = data["path"].as_str().unwrap_or(&output_str);

    println!();
    println!("╔════════════════════════════════════════════════════╗");
    println!("║  PASSPHRASE (save this now, will not be shown again) ║");
    println!("╠════════════════════════════════════════════════════╣");
    println!("║  {}  ║", passphrase);
    println!("╚════════════════════════════════════════════════════╝");
    println!();
    println!("Package written to: {}", path);
    Ok(())
}

// ─── Import ───────────────────────────────────────────────────────────────────

fn run_import(file: PathBuf, project: Option<String>, env: Option<String>) -> Result<(), CliError> {
    let resolved_scope = scope::resolve(project.as_deref(), env.as_deref(), false)?;

    let passphrase =
        rpassword::prompt_password("Package passphrase: ").map_err(CliError::Io)?;

    let body = serde_json::json!({
        "path": file.to_string_lossy(),
        "passphrase": passphrase,
    });

    let url = resolved_scope.append_query(&format!("{base}/share/import", base = crate::client::api_base()));
    let resp = authenticated_post(&url, &body)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("import failed: {text}")));
    }

    let data: serde_json::Value = resp.json().map_err(|e| CliError::Api(e.to_string()))?;
    let count = data["imported"].as_u64().unwrap_or(0);
    let names: Vec<String> = data["item_names"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let skipped_keys: Vec<String> = data["skipped_keys"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    println!("Imported {} item(s) into {} / {}:", count, resolved_scope.project, resolved_scope.environment);
    for name in &names {
        println!("  - {}", name);
    }
    for key in &skipped_keys {
        eprintln!("warning: '{}' already linked in {} / {} — imported but not relinked", key, resolved_scope.project, resolved_scope.environment);
    }
    Ok(())
}

// ─── Polling helpers ──────────────────────────────────────────────────────────

/// Poll GET /share/status until state is "awaiting_fingerprint", then return fingerprint.
fn poll_until_fingerprint() -> Result<String, CliError> {
    let max_polls = 300; // 5 minutes at 1s intervals
    for _ in 0..max_polls {
        std::thread::sleep(std::time::Duration::from_secs(1));

        let resp = authenticated_get(&format!("{base}/share/status", base = crate::client::api_base()))?;
        if !resp.status().is_success() {
            continue;
        }

        let data: serde_json::Value = match resp.json() {
            Ok(v) => v,
            Err(_) => continue,
        };

        let state = data["state"].as_str().unwrap_or("");
        match state {
            "awaiting_fingerprint" => {
                let fp = data["fingerprint"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();
                return Ok(fp);
            }
            "failed" | "cancelled" => {
                return Err(CliError::Api(format!("session ended: {state}")));
            }
            _ => continue,
        }
    }
    Err(CliError::Api("timed out waiting for peer to connect".into()))
}

/// Poll GET /share/status until terminal state (done/failed/cancelled).
fn poll_until_terminal(context: &str) -> Result<(), CliError> {
    let max_polls = 600;
    for _ in 0..max_polls {
        std::thread::sleep(std::time::Duration::from_secs(1));

        let resp = authenticated_get(&format!("{base}/share/status", base = crate::client::api_base()))?;
        if !resp.status().is_success() {
            continue;
        }

        let data: serde_json::Value = match resp.json() {
            Ok(v) => v,
            Err(_) => continue,
        };

        let state = data["state"].as_str().unwrap_or("");
        match state {
            "done" => return Ok(()),
            s if s.starts_with("failed") => {
                return Err(CliError::Api(format!("{context} failed: {s}")));
            }
            "cancelled" => {
                return Err(CliError::Api(format!("{context} was cancelled")));
            }
            _ => continue,
        }
    }
    Err(CliError::Api(format!("{context} timed out")))
}

/// Poll until done and return (received item names, skipped-due-to-existing-key names).
fn poll_until_terminal_with_names() -> Result<(Vec<String>, Vec<String>), CliError> {
    let max_polls = 600;
    for _ in 0..max_polls {
        std::thread::sleep(std::time::Duration::from_secs(1));

        let resp = authenticated_get(&format!("{base}/share/status", base = crate::client::api_base()))?;
        if !resp.status().is_success() {
            continue;
        }

        let data: serde_json::Value = match resp.json() {
            Ok(v) => v,
            Err(_) => continue,
        };

        let state = data["state"].as_str().unwrap_or("");
        match state {
            "done" => {
                let names: Vec<String> = data["received_names"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let skipped_keys: Vec<String> = data["skipped_keys"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                return Ok((names, skipped_keys));
            }
            s if s.starts_with("failed") => {
                return Err(CliError::Api(format!("receive failed: {s}")));
            }
            "cancelled" => {
                return Err(CliError::Api("receive was cancelled".into()));
            }
            _ => continue,
        }
    }
    Err(CliError::Api("receive timed out".into()))
}

// ─── Prompt helper ────────────────────────────────────────────────────────────

fn prompt_confirm(prompt: &str) -> Result<bool, CliError> {
    use std::io::{self, Write};
    print!("{}", prompt);
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(CliError::Io)?;
    Ok(matches!(input.trim().to_lowercase().as_str(), "y" | "yes"))
}
