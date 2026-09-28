//! `crypt-env doctor` — end-to-end diagnostics: app/health, vault lock,
//! TLS trust anchor (path + expiry), CLI session token (+ permissions), MCP
//! token, project configuration, and WSL integration. Never prints secrets.

use clap::Args;
use std::path::{Path, PathBuf};

use crate::client::{self, CliError};
use crate::commands::scope::{self, manifest};

#[derive(Args)]
pub struct DoctorArgs {}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

pub struct Check {
    pub level: Level,
    pub name: &'static str,
    pub detail: String,
}

fn check(level: Level, name: &'static str, detail: impl Into<String>) -> Check {
    Check { level, name, detail: detail.into() }
}

pub fn run(_args: DoctorArgs) -> Result<(), CliError> {
    println!("crypt-env doctor — system diagnostics\n");
    let checks = execute();
    for c in &checks {
        let tag = match c.level {
            Level::Ok => "[OK]",
            Level::Warn => "[--]",
            Level::Fail => "[!!]",
        };
        println!("  {tag} {:<18} {}", c.name, c.detail);
    }
    println!();
    Ok(())
}

/// Runs every check (no prompts, no secret access), shared with the TUI.
pub fn execute() -> Vec<Check> {
    let mut out = Vec::new();
    health(&mut out);
    out.push(tls_check());
    out.push(token_check());
    out.push(mcp_token_file());
    out.push(project_check());
    out.push(wsl_check());
    out
}

fn health(out: &mut Vec<Check>) {
    let base = client::api_base();
    let resp = client::http_client().and_then(|c| {
        c.get(format!("{base}/health"))
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .map_err(|e| if e.is_connect() { CliError::ConnectionRefused } else { CliError::Api(e.to_string()) })
    });
    match resp {
        Err(CliError::ConnectionRefused) => {
            out.push(check(Level::Fail, "App running", "not running — open crypt-env and try again"));
        }
        Err(e) => out.push(check(Level::Fail, "App running", format!("{base}: {e}"))),
        Ok(r) => {
            let json: serde_json::Value = r.json().unwrap_or_default();
            let version = json.get("version").and_then(|v| v.as_str()).unwrap_or("?");
            out.push(check(Level::Ok, "App running", format!("{base}  (v{version})")));
            let locked = json.get("vault_locked").and_then(|v| v.as_bool()).unwrap_or(true);
            out.push(if locked {
                check(Level::Fail, "Vault", "locked — unlock it in the app")
            } else {
                check(Level::Ok, "Vault", "unlocked")
            });
            let mcp = json.get("mcp_token_configured").and_then(|v| v.as_bool()).unwrap_or(false);
            out.push(if mcp {
                check(Level::Ok, "MCP token", "configured in app")
            } else {
                check(Level::Warn, "MCP token", "not configured  (Settings → Integrations)")
            });
        }
    }
}

fn tls_check() -> Check {
    let Some(path) = client::cert_path_in_use() else {
        return check(Level::Fail, "TLS certificate", "no path could be determined (set CRYPTENV_CERT_PATH)");
    };
    let pem = match std::fs::read_to_string(&path) {
        Ok(p) => p,
        Err(e) => return check(Level::Fail, "TLS certificate", format!("{}: {e}", path.display())),
    };
    match crypt_env_lib::tls::parse_not_after_from_pem(&pem) {
        None => check(Level::Fail, "TLS certificate", format!("{}: not a valid PEM certificate", path.display())),
        Some(not_after) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let days = (not_after as i64 - now as i64) / 86_400;
            if days < 0 {
                check(Level::Fail, "TLS certificate", format!("{} expired — restart the app to regenerate", path.display()))
            } else if days < 30 {
                check(Level::Warn, "TLS certificate", format!("{} expires in {days} day(s)", path.display()))
            } else {
                check(Level::Ok, "TLS certificate", format!("{} (valid, {days} days left)", path.display()))
            }
        }
    }
}

fn token_check() -> Check {
    let Some(path) = client::token_file_path() else {
        return check(Level::Warn, "CLI session", "no token path (set CRYPTENV_TOKEN_PATH)");
    };
    if !path.is_file() {
        return check(Level::Warn, "CLI session", "no session in this terminal (the next gated command prompts)");
    }
    if let Some(mode) = unix_mode(&path) {
        if mode & 0o077 != 0 {
            return check(
                Level::Warn,
                "CLI session",
                format!("{} is readable by others (mode {mode:o}) — run chmod 600", path.display()),
            );
        }
    }
    check(Level::Ok, "CLI session", format!("this terminal's session file: {}", path.display()))
}

#[cfg(unix)]
fn unix_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).ok().map(|m| m.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn unix_mode(_path: &Path) -> Option<u32> {
    None
}

fn mcp_token_file() -> Check {
    let path = std::env::var("APPDATA")
        .map(|d| PathBuf::from(d).join("com.maosuarez.cryptenv").join("mcp_token"))
        .or_else(|_| {
            std::env::var("HOME").map(|d| PathBuf::from(d).join(".local/share/com.maosuarez.cryptenv/mcp_token"))
        });
    match path {
        Ok(p) if p.exists() => check(Level::Ok, "MCP token file", p.display().to_string()),
        Ok(p) => check(Level::Warn, "MCP token file", format!("not found ({})", p.display())),
        Err(_) => check(Level::Warn, "MCP token file", "cannot resolve APPDATA/HOME"),
    }
}

fn project_check() -> Check {
    let cwd = match std::env::current_dir() {
        Ok(c) => c,
        Err(e) => return check(Level::Fail, "Project config", format!("cannot read current directory: {e}")),
    };
    let Some(path) = scope::find_config(&cwd) else {
        return check(Level::Warn, "Project config", format!("no {} found (run `crypt-env init`)", manifest::FILE_NAME));
    };
    match scope::load_config(&path) {
        Err(e) => check(Level::Fail, "Project config", e.to_string()),
        Ok(ws) if ws.manifest.is_none() => check(
            Level::Warn,
            "Project config",
            format!("{} is legacy — run `crypt-env init` to migrate", path.display()),
        ),
        Ok(ws) => {
            let m = ws.manifest.as_ref().map(|m| m.project.environments.len()).unwrap_or(0);
            check(Level::Ok, "Project config", format!("{} (project '{}', {m} environment(s))", path.display(), ws.project))
        }
    }
}

fn wsl_check() -> Check {
    if let Ok(distro) = std::env::var("WSL_DISTRO_NAME") {
        let configured = std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".config/cryptenv/env.sh").is_file())
            .unwrap_or(false);
        let api = std::env::var("CRYPTENV_API_URL").unwrap_or_else(|_| client::DEFAULT_API_BASE.to_string());
        return if configured {
            check(Level::Ok, "WSL", format!("inside '{distro}', shell configured (API {api})"))
        } else {
            check(Level::Warn, "WSL", format!("inside '{distro}', not configured — run `crypt-env setup wsl`"))
        };
    }
    if cfg!(target_os = "windows") {
        let out = std::process::Command::new("wsl.exe").args(["--list", "--quiet"]).env("WSL_UTF8", "1").output();
        return match out {
            Ok(o) if o.status.success() => {
                let distros = crypt_env_lib::wsl::parse_distro_list(&o.stdout);
                if distros.is_empty() {
                    check(Level::Warn, "WSL", "installed, no distributions")
                } else {
                    check(Level::Ok, "WSL", format!("distributions: {} (configure with `crypt-env setup wsl <distro>`)", distros.join(", ")))
                }
            }
            _ => check(Level::Warn, "WSL", "not installed"),
        };
    }
    check(Level::Ok, "WSL", "not applicable on this platform")
}
