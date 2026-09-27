// crypt-env-mcp.rs — Standalone MCP server over stdio.
// Does not import from the project lib — uses only serde, serde_json, reqwest::blocking.
// All user-facing strings are in English to match the CLI.
#![recursion_limit = "512"]

use std::io::BufRead;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use rand::RngCore;
use serde::{Deserialize, Serialize};

/// Archivos .env temporales pendientes de limpieza del ciclo anterior.
static TEMP_FILES: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Built-in REST base URL, used when `CRYPTENV_API_URL` is unset or empty.
const DEFAULT_API_BASE: &str = "https://127.0.0.1:47821";
const MCP_VERSION: &str = "2024-11-05";

/// Process-wide resolved REST base URL. Set once from `main` so every request
/// targets the same endpoint.
static API_BASE_CACHE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Mirrors `crypt-env`'s endpoint resolution (there is no shared crate between
/// the two binaries — keep in sync): `CRYPTENV_API_URL` overrides the default
/// when set and non-empty; the value must be an absolute `http`/`https` URL; a
/// non-loopback host prints one stderr warning.
fn resolve_api_base() -> Result<String, String> {
    resolve_api_base_from(std::env::var("CRYPTENV_API_URL").ok().as_deref())
}

/// Pure resolver over the raw `CRYPTENV_API_URL` value; prints the non-loopback
/// warning as a side effect so the caller stays a one-liner.
fn resolve_api_base_from(raw: Option<&str>) -> Result<String, String> {
    match raw {
        Some(v) if !v.trim().is_empty() => {
            let trimmed = v.trim();
            let host = http_url_host(trimmed).ok_or_else(|| {
                "CRYPTENV_API_URL must be an absolute http:// or https:// URL".to_string()
            })?;
            if !host_is_loopback(&host) {
                eprintln!(
                    "[crypt-env-mcp] warning: CRYPTENV_API_URL host '{host}' is not a loopback \
                     address; the vault is expected to be reachable only over localhost"
                );
            }
            Ok(trimmed.trim_end_matches('/').to_string())
        }
        _ => Ok(DEFAULT_API_BASE.to_string()),
    }
}

/// Returns the resolved REST base URL. Falls back to [`DEFAULT_API_BASE`] when
/// `main` did not prime the cache (e.g. in unit tests).
fn api_base() -> &'static str {
    API_BASE_CACHE
        .get_or_init(|| resolve_api_base().unwrap_or_else(|_| DEFAULT_API_BASE.to_string()))
        .as_str()
}

/// Lowercased host of `s` when it is an absolute `http`/`https` URL with a
/// non-empty host; otherwise `None`.
fn http_url_host(s: &str) -> Option<String> {
    let rest = s
        .strip_prefix("http://")
        .or_else(|| s.strip_prefix("https://"))?;
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() {
        return None;
    }
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    if host_port.is_empty() {
        return None;
    }
    let host = if let Some(after_bracket) = host_port.strip_prefix('[') {
        let end = after_bracket.find(']')?;
        &after_bracket[..end]
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };
    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

/// True when `host` is a loopback address: `localhost`, `::1`, or `127.0.0.0/8`.
fn host_is_loopback(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(v4) = host.parse::<std::net::Ipv4Addr>() {
        return v4.is_loopback();
    }
    if let Ok(v6) = host.parse::<std::net::Ipv6Addr>() {
        return v6.is_loopback();
    }
    false
}

// ─── Token ────────────────────────────────────────────────────────────────────

/// Returns the platform-specific path to the MCP token file.
///
/// - Windows: %APPDATA%\com.maosuarez.cryptenv\mcp_token
/// - macOS:   ~/Library/Application Support/com.maosuarez.cryptenv/mcp_token
/// - Linux:   ~/.local/share/com.maosuarez.cryptenv/mcp_token
fn mcp_token_path() -> Result<std::path::PathBuf, String> {
    // Windows: %APPDATA% is always set when the GUI app runs.
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA")
            .map(|d| {
                std::path::PathBuf::from(d)
                    .join("com.maosuarez.cryptenv")
                    .join("mcp_token")
            })
            .map_err(|_| "APPDATA environment variable not set".to_string())
    }

    // macOS: ~/Library/Application Support/
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .map(|d| {
                std::path::PathBuf::from(d)
                    .join("Library")
                    .join("Application Support")
                    .join("com.maosuarez.cryptenv")
                    .join("mcp_token")
            })
            .map_err(|_| "HOME environment variable not set".to_string())
    }

    // Linux: ~/.local/share/
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::env::var("HOME")
            .map(|d| {
                std::path::PathBuf::from(d)
                    .join(".local")
                    .join("share")
                    .join("com.maosuarez.cryptenv")
                    .join("mcp_token")
            })
            .map_err(|_| "HOME environment variable not set".to_string())
    }
}

fn read_mcp_token() -> Result<String, String> {
    let path = mcp_token_path()?;
    std::fs::read_to_string(&path)
        .map(|s| s.trim().to_string())
        .map_err(|_| {
            "MCP token not found. Generate one in crypt-env Settings → INTEGRATIONS.".to_string()
        })
}

// ─── TLS-aware HTTP client ────────────────────────────────────────────────────

/// Returns the path to the self-signed cert written by the Tauri app.
///
/// A non-empty `CRYPTENV_CERT_PATH` short-circuits the platform probing (mirrors
/// `crypt-env`; see `design.md` D3), so a WSL client can reference the Windows
/// cert on `/mnt/c` without copying it.
fn tls_cert_path() -> Option<std::path::PathBuf> {
    if let Ok(explicit) = std::env::var("CRYPTENV_CERT_PATH") {
        if !explicit.trim().is_empty() {
            return Some(std::path::PathBuf::from(explicit.trim()));
        }
    }

    #[cfg(target_os = "windows")]
    if let Ok(appdata) = std::env::var("APPDATA") {
        return Some(
            std::path::PathBuf::from(appdata)
                .join("com.maosuarez.cryptenv")
                .join("tls")
                .join("cert.pem"),
        );
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
            return Some(
                std::path::PathBuf::from(xdg)
                    .join("com.maosuarez.cryptenv")
                    .join("tls")
                    .join("cert.pem"),
            );
        }
        if let Ok(home) = std::env::var("HOME") {
            return Some(
                std::path::PathBuf::from(home)
                    .join(".local")
                    .join("share")
                    .join("com.maosuarez.cryptenv")
                    .join("tls")
                    .join("cert.pem"),
            );
        }
    }
    None
}

/// Build a reqwest client that trusts the vault's self-signed cert.
/// Falls back to a plain client on failure (which will produce a clear TLS error).
fn mcp_http_client() -> reqwest::blocking::Client {
    (|| -> Result<reqwest::blocking::Client, Box<dyn std::error::Error>> {
        let cert_path = tls_cert_path().ok_or("cannot determine cert path")?;
        let pem_bytes = std::fs::read(&cert_path)?;
        let cert = reqwest::Certificate::from_pem(&pem_bytes)?;
        Ok(reqwest::blocking::ClientBuilder::new()
            .add_root_certificate(cert)
            .build()?)
    })()
    .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

/// Build a reqwest client with a custom timeout and the vault's self-signed cert.
fn mcp_http_client_timeout(timeout: std::time::Duration) -> reqwest::blocking::Client {
    (|| -> Result<reqwest::blocking::Client, Box<dyn std::error::Error>> {
        let cert_path = tls_cert_path().ok_or("cannot determine cert path")?;
        let pem_bytes = std::fs::read(&cert_path)?;
        let cert = reqwest::Certificate::from_pem(&pem_bytes)?;
        Ok(reqwest::blocking::ClientBuilder::new()
            .add_root_certificate(cert)
            .timeout(timeout)
            .build()?)
    })()
    .unwrap_or_else(|_| {
        reqwest::blocking::ClientBuilder::new()
            .timeout(timeout)
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new())
    })
}

// ─── JSON-RPC types ───────────────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct RpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<serde_json::Value>,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

// ─── Tool call result helpers ─────────────────────────────────────────────────

fn tool_ok(text: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "content": [{ "type": "text", "text": text.into() }],
        "isError": false
    })
}

fn tool_err(text: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "content": [{ "type": "text", "text": text.into() }],
        "isError": true
    })
}

// ─── Tool definitions ─────────────────────────────────────────────────────────

fn tool_definitions() -> serde_json::Value {
    serde_json::json!([
        {
            "name": "crypt_env_list_items",
            "description": "List available secrets by name (no values), scoped to a single project+environment. Use this first to discover which API keys and credentials are linked into that environment — so you know what tools and services you can configure before writing a .env.example. Requires scope: 'environment_id', or both 'project' and 'environment'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "description": "Filter by type: secret, credential, link, command, note" },
                    "category": { "type": "string", "description": "Filter by category name" },
                    "include_global": { "type": "string", "enum": ["true", "false", "only"], "description": "true (default) also lists reusable global secrets not yet linked into this environment — these appear with `linked: false` and will NOT be written by generate/inject/fill until linked. false restricts to items actually linked into this environment (what fill/inject will materialize). only returns just the global secrets, ignoring linkage." },
                    "environment_id": { "type": "integer", "description": "Environment ID (scope). Provide this, or both 'project' and 'environment'." },
                    "project": { "type": "string", "description": "Project name (case-insensitive). Used with 'environment' when 'environment_id' is not given." },
                    "environment": { "type": "string", "description": "Environment name within the project (case-insensitive), e.g. production, local, test. Used with 'project'." }
                }
            }
        },
        {
            "name": "crypt_env_get_item",
            "description": "Returns item metadata without the secret value.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Item ID" }
                },
                "required": ["id"]
            }
        },
        {
            "name": "crypt_env_search_items",
            "description": "Search items by name within a project+environment scope (no secret values exposed). Returns matching items metadata. Requires scope: 'environment_id', or both 'project' and 'environment'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search term to match against item names" },
                    "include_global": { "type": "string", "enum": ["true", "false", "only"], "description": "true (default) also searches reusable global secrets not yet linked into this environment — these appear with `linked: false` and will NOT be written by generate/inject/fill until linked. false restricts to items actually linked into this environment (what fill/inject will materialize). only returns just the global secrets, ignoring linkage." },
                    "environment_id": { "type": "integer", "description": "Environment ID (scope). Provide this, or both 'project' and 'environment'." },
                    "project": { "type": "string", "description": "Project name (case-insensitive). Used with 'environment' when 'environment_id' is not given." },
                    "environment": { "type": "string", "description": "Environment name within the project (case-insensitive), e.g. production, local, test. Used with 'project'." }
                },
                "required": ["query"]
            }
        },
        {
            "name": "crypt_env_generate_env",
            "description": "Writes a .env file with the real values of the specified secrets, looked up within a project+environment scope. Returns the file path — values never appear in the response. Requires scope: 'environment_id', or both 'project' and 'environment'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "keys": { "type": "array", "items": { "type": "string" }, "description": "Names of the secrets to include" },
                    "environment_id": { "type": "integer", "description": "Environment ID (scope). Provide this, or both 'project' and 'environment'." },
                    "project": { "type": "string", "description": "Project name (case-insensitive). Used with 'environment' when 'environment_id' is not given." },
                    "environment": { "type": "string", "description": "Environment name within the project (case-insensitive), e.g. production, local, test. Used with 'project'." }
                },
                "required": ["keys"]
            }
        },
        {
            "name": "crypt_env_inject_env",
            "description": "Injects a secret as an environment variable into the MCP process, looked up within a project+environment scope. Does not return the value. Requires scope: 'environment_id', or both 'project' and 'environment'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Secret name" },
                    "environment_id": { "type": "integer", "description": "Environment ID (scope). Provide this, or both 'project' and 'environment'." },
                    "project": { "type": "string", "description": "Project name (case-insensitive). Used with 'environment' when 'environment_id' is not given." },
                    "environment": { "type": "string", "description": "Environment name within the project (case-insensitive), e.g. production, local, test. Used with 'project'." }
                },
                "required": ["key"]
            }
        },
        {
            "name": "crypt_env_add_item",
            "description": "Adds a new item to the vault, owned by the given project and linked into the given environment. Requires scope: 'environment_id', or both 'project' and 'environment'. If the key already exists in the target environment, the existing item is updated in place (its previous value is destroyed) — this is the default ('on_conflict': 'update'). If that item is shared with other environments or projects (or is global), the call fails with a conflict instead of silently changing it elsewhere; retry with 'on_conflict': 'replace' to create a new item and repoint just this environment's link, or update the shared item explicitly with crypt_env_update_item. Set 'on_conflict': 'error' to fail on any existing key instead of updating it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "description": "Item type: secret, credential, link, command, note" },
                    "name": { "type": "string" },
                    "value": { "type": "string", "description": "Secret value (for secret/credential types)" },
                    "category": { "type": "string" },
                    "notes": { "type": "string" },
                    "url": { "type": "string" },
                    "username": { "type": "string" },
                    "key": { "type": "string", "description": "Environment variable key this item is linked under. Defaults to 'name' if omitted." },
                    "on_conflict": { "type": "string", "enum": ["update", "replace", "error"], "description": "How to handle an existing item already linked under this key. 'update' (default): re-encrypt onto the existing item, destroying its previous value; fails if the item is shared elsewhere. 'replace': always create a new item and repoint this environment's link to it; the superseded item is deleted only if nothing else still references it. 'error': fail on any existing key." },
                    "environment_id": { "type": "integer", "description": "Environment ID (scope). Provide this, or both 'project' and 'environment'." },
                    "project": { "type": "string", "description": "Project name (case-insensitive). Used with 'environment' when 'environment_id' is not given." },
                    "environment": { "type": "string", "description": "Environment name within the project (case-insensitive), e.g. production, local, test. Used with 'project'." }
                },
                "required": ["type", "name"]
            }
        },
        {
            "name": "crypt_env_update_item",
            "description": "Update an existing vault item. Only the fields provided are changed; omitted fields keep their current values (including secret values). Use crypt_env_list_items to find the item id first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Item id (as returned by list_items or get_item)" },
                    "name": { "type": "string", "description": "New item name" },
                    "value": { "type": "string", "description": "New secret value (for type=secret)" },
                    "url": { "type": "string", "description": "New URL (for type=link or credential)" },
                    "username": { "type": "string", "description": "New username (for type=credential)" },
                    "password": { "type": "string", "description": "New password (for type=credential)" },
                    "title": { "type": "string", "description": "New title (for type=note or link)" },
                    "description": { "type": "string", "description": "New description" },
                    "notes": { "type": "string", "description": "New notes" },
                    "content": { "type": "string", "description": "New content body (for type=note)" },
                    "command": { "type": "string", "description": "New command template (for type=command)" },
                    "shell": { "type": "string", "description": "New shell (for type=command), e.g. bash, powershell" },
                    "categories": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "New list of category names. Replaces the current list."
                    }
                },
                "required": ["id"]
            }
        },
        {
            "name": "crypt_env_delete_item",
            "description": "Permanently delete an item from the vault by id. This action cannot be undone.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Item id to delete (as returned by list_items)" }
                },
                "required": ["id"]
            }
        },
        {
            "name": "crypt_env_update_settings",
            "description": "Updates settings: auto_lock_timeout (minutes) and hotkey. Changing master_password is not allowed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "auto_lock_timeout": { "type": "integer", "description": "Minutes until auto-lock (0 = never)" },
                    "hotkey": { "type": "string", "description": "Global shortcut, e.g. Ctrl+Alt+Z" }
                }
            }
        },
        {
            "name": "crypt_env_fill_env",
            "description": "Fills a .env.example template with real secret values from a project+environment scope, matching keys against that environment's linked variables. Secret values never appear in the response — use crypt_env_list_items first to discover available keys, then write a .env.example, then call this to produce the final .env the service will read. Requires scope: 'environment_id', or both 'project' and 'environment'. Write destination: 'output_path' writes exactly there; 'output_dir' (no output_path) writes to '{output_dir}/.env.<environment-name>'; neither returns the filled content inline. Writes refuse to clobber a file crypt-env did not create — see 'overwrite'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "template": { "type": "string", "description": "Content of the .env.example (lines like KEY= or KEY=description)" },
                    "output_path": { "type": "string", "description": "Absolute path where the filled .env should be written, e.g. /home/user/my-project/.env" },
                    "output_dir": { "type": "string", "description": "Directory to write the filled .env into, using the '.env.<environment-name>' naming convention. Ignored if output_path is given." },
                    "overwrite": { "type": "boolean", "description": "Destructive. Set true ONLY when the user has explicitly confirmed replacing the file at output_path. The previous contents are copied to <path>.bak first. Leave unset otherwise: the call will safely fail with a conflict naming the path if the target exists and was not created by crypt-env." },
                    "environment_id": { "type": "integer", "description": "Environment ID (scope). Provide this, or both 'project' and 'environment'." },
                    "project": { "type": "string", "description": "Project name (case-insensitive). Used with 'environment' when 'environment_id' is not given." },
                    "environment": { "type": "string", "description": "Environment name within the project (case-insensitive), e.g. production, local, test. Used with 'project'." }
                },
                "required": ["template"]
            }
        },
        {
            "name": "crypt_env_doctor",
            "description": "Checks the health of the crypt-env installation: app status, vault lock state, item count, MCP token configuration, and version.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "crypt_env_list_commands",
            "description": "Lists saved commands linked into a project+environment, with name, description, and required placeholders. Requires scope: 'environment_id', or both 'project' and 'environment'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "environment_id": { "type": "integer", "description": "Environment ID (scope). Provide this, or both 'project' and 'environment'." },
                    "project": { "type": "string", "description": "Project name (case-insensitive). Used with 'environment' when 'environment_id' is not given." },
                    "environment": { "type": "string", "description": "Environment name within the project (case-insensitive), e.g. production, local, test. Used with 'project'." }
                }
            }
        },
        {
            "name": "crypt_env_run_command",
            "description": "Returns a command string with {{VAR}} placeholders resolved. Does not execute it. The command is looked up within a project+environment scope. Requires scope: 'environment_id', or both 'project' and 'environment'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Command name" },
                    "params": {
                        "type": "object",
                        "description": "Map of VAR → value to resolve placeholders",
                        "additionalProperties": { "type": "string" }
                    },
                    "environment_id": { "type": "integer", "description": "Environment ID (scope). Provide this, or both 'project' and 'environment'." },
                    "project": { "type": "string", "description": "Project name (case-insensitive). Used with 'environment' when 'environment_id' is not given." },
                    "environment": { "type": "string", "description": "Environment name within the project (case-insensitive), e.g. production, local, test. Used with 'project'." }
                },
                "required": ["name"]
            }
        },
        {
            "name": "crypt_env_share_listen",
            "description": "Start a share session as sender (LAN bridge). Registers mDNS and waits for a peer to connect using the returned pairing code. Shared item ids must already be linked into the given project+environment. Requires scope: 'environment_id', or both 'project' and 'environment'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "IDs of vault items to share (must be linked into the scoped environment)"
                    },
                    "environment_id": { "type": "integer", "description": "Environment ID (scope). Provide this, or both 'project' and 'environment'." },
                    "project": { "type": "string", "description": "Project name (case-insensitive). Used with 'environment' when 'environment_id' is not given." },
                    "environment": { "type": "string", "description": "Environment name within the project (case-insensitive), e.g. production, local, test. Used with 'project'." }
                },
                "required": ["items"]
            }
        },
        {
            "name": "crypt_env_share_connect",
            "description": "Connect to a sender as receiver using the pairing code. Returns a fingerprint that both sides must verify before data flows. Received items are owned by and linked into the given project+environment. Requires scope: 'environment_id', or both 'project' and 'environment'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pairing_code": { "type": "string", "description": "6-digit pairing code from the sender" },
                    "environment_id": { "type": "integer", "description": "Environment ID (scope). Provide this, or both 'project' and 'environment'." },
                    "project": { "type": "string", "description": "Project name (case-insensitive). Used with 'environment' when 'environment_id' is not given." },
                    "environment": { "type": "string", "description": "Environment name within the project (case-insensitive), e.g. production, local, test. Used with 'project'." }
                },
                "required": ["pairing_code"]
            }
        },
        {
            "name": "crypt_env_share_confirm",
            "description": "Confirm (or reject) the fingerprint shown after ECDH. Both sides must call this. Pass confirmed=true only when the fingerprints match.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "confirmed": { "type": "boolean", "description": "true to confirm, false to cancel" }
                },
                "required": ["confirmed"]
            }
        },
        {
            "name": "crypt_env_share_cancel",
            "description": "Cancel the active share session immediately.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "crypt_env_share_status",
            "description": "Get the current share session status: state, fingerprint, direction.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "crypt_env_share_export",
            "description": "Export selected vault items as an AES-256-GCM encrypted .vault package file. IMPORTANT: Show the passphrase to the user immediately. It will not be stored.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "IDs of vault items to export"
                    },
                    "output_path": { "type": "string", "description": "Absolute path for the output .vault file" }
                },
                "required": ["items", "output_path"]
            }
        },
        {
            "name": "crypt_env_share_import",
            "description": "Import items from an encrypted .vault package file into the local vault. Imported items are owned by and linked into the given project+environment. Requires scope: 'environment_id', or both 'project' and 'environment'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path to the .vault package file" },
                    "passphrase": { "type": "string", "description": "12-character passphrase provided by the sender" },
                    "environment_id": { "type": "integer", "description": "Environment ID (scope). Provide this, or both 'project' and 'environment'." },
                    "project": { "type": "string", "description": "Project name (case-insensitive). Used with 'environment' when 'environment_id' is not given." },
                    "environment": { "type": "string", "description": "Environment name within the project (case-insensitive), e.g. production, local, test. Used with 'project'." }
                },
                "required": ["path", "passphrase"]
            }
        },
        {
            "name": "crypt_env_list_categories",
            "description": "List all categories in the vault with their id, name, and color.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "crypt_env_create_category",
            "description": "Create a new category in the vault.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Category name (max 100 characters)" },
                    "color": { "type": "string", "description": "Category color, e.g. #FF5733" },
                    "description": { "type": "string", "description": "Optional description for the category" }
                },
                "required": ["name", "color"]
            }
        },
        {
            "name": "crypt_env_update_category",
            "description": "Edit an existing category by id. Only the fields provided are changed. Pass description as empty string to clear it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Category id (as returned by list_categories)" },
                    "name": { "type": "string", "description": "New name" },
                    "color": { "type": "string", "description": "New color" },
                    "description": { "type": "string", "description": "New description (empty string to clear)" }
                },
                "required": ["id"]
            }
        },
        {
            "name": "crypt_env_delete_category",
            "description": "Delete a category by id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Category id to delete" }
                },
                "required": ["id"]
            }
        },
        {
            "name": "crypt_env_list_projects",
            "description": "List all projects with their environments (name, paths, variable count).",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "crypt_env_inject_environment",
            "description": "Inject an environment's vars into its configured .env path(s). Requires scope: 'environment_id', or both 'project' and 'environment'. If the environment has no configured paths, provide 'output_path' or 'output_dir' to write there instead. Writes refuse to clobber a file crypt-env did not create — see 'overwrite'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "environment_id": { "type": "integer", "description": "Environment ID (scope). Provide this, or both 'project' and 'environment'." },
                    "project": { "type": "string", "description": "Project name (case-insensitive). Used with 'environment' when 'environment_id' is not given." },
                    "environment": { "type": "string", "description": "Environment name within the project (case-insensitive), e.g. production, local, test. Used with 'project'." },
                    "output_path": { "type": "string", "description": "Absolute path to write the .env to, in addition to the environment's configured paths." },
                    "output_dir": { "type": "string", "description": "Directory to write '.env.<environment-name>' into. Used as fallback when the environment has no configured paths and no output_path is given." },
                    "overwrite": { "type": "boolean", "description": "Destructive. Set true ONLY when the user has explicitly confirmed replacing the file at output_path. The previous contents are copied to <path>.bak first. Leave unset otherwise: the call will safely fail with a conflict naming the path if the target exists and was not created by crypt-env." }
                }
            }
        },
        {
            "name": "crypt_env_generate_example_env",
            "description": "Generates a safe-to-commit '.env.example'-style placeholder for an environment: every linked variable key with an empty value (KEY=). Never reads or returns secret values. Requires scope: 'environment_id', or both 'project' and 'environment'. Writes refuse to clobber a file crypt-env did not create — see 'overwrite'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "environment_id": { "type": "integer", "description": "Environment ID (scope). Provide this, or both 'project' and 'environment'." },
                    "project": { "type": "string", "description": "Project name (case-insensitive). Used with 'environment' when 'environment_id' is not given." },
                    "environment": { "type": "string", "description": "Environment name within the project (case-insensitive), e.g. production, local, test. Used with 'project'." },
                    "output_path": { "type": "string", "description": "Absolute path to write the placeholder .env.example to. If omitted, content is returned inline (still placeholders only, never secrets)." },
                    "output_dir": { "type": "string", "description": "Directory to write '.env.example.<environment-name>' into. Ignored if output_path is given." },
                    "overwrite": { "type": "boolean", "description": "Destructive. Set true ONLY when the user has explicitly confirmed replacing the file at output_path. The previous contents are copied to <path>.bak first. Leave unset otherwise: the call will safely fail with a conflict naming the path if the target exists and was not created by crypt-env." }
                }
            }
        },
        {
            "name": "crypt_env_relay_send",
            "description": "Send vault items via internet relay. Returns a code and passphrase. IMPORTANT: Show code and passphrase to user immediately. The passphrase is only shown once.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "item_ids": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "IDs of vault items to share"
                    }
                },
                "required": ["item_ids"]
            }
        },
        {
            "name": "crypt_env_relay_receive",
            "description": "Receive vault items shared via internet relay using a code and passphrase from the sender. Imported items are owned by and linked into the given project+environment. Requires scope: 'environment_id', or both 'project' and 'environment'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "code": { "type": "string", "description": "Relay code provided by the sender (e.g. X7K2-M9P4)" },
                    "passphrase": { "type": "string", "description": "Passphrase provided by the sender" },
                    "environment_id": { "type": "integer", "description": "Environment ID (scope). Provide this, or both 'project' and 'environment'." },
                    "project": { "type": "string", "description": "Project name (case-insensitive). Used with 'environment' when 'environment_id' is not given." },
                    "environment": { "type": "string", "description": "Environment name within the project (case-insensitive), e.g. production, local, test. Used with 'project'." }
                },
                "required": ["code", "passphrase"]
            }
        },
        {
            "name": "crypt_env_list_mcp_servers",
            "description": "List registered MCP servers from Claude config files. Returns name, command, args, and env key names — never env values.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "description": "Which config to read: 'global' (Claude desktop), 'project' (.claude/mcp_servers.json in cwd), or 'all' (both). Default: 'all'.",
                        "enum": ["global", "project", "all"]
                    }
                }
            }
        },
        {
            "name": "crypt_env_add_mcp_server",
            "description": "Register a new MCP server entry in the Claude config. Env keys are stored as empty placeholders — actual values come from the vault at runtime via crypt_env_inject_env.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Server name (unique key in mcpServers)" },
                    "command": { "type": "string", "description": "Command to launch the MCP server" },
                    "args": { "type": "array", "items": { "type": "string" }, "description": "Arguments for the command" },
                    "env": {
                        "type": "object",
                        "description": "Map of env var name to vault item name. Values are stored as empty placeholders in the config — use crypt_env_inject_env at runtime.",
                        "additionalProperties": { "type": "string" }
                    },
                    "scope": { "type": "string", "description": "'global' (Claude desktop) or 'project' (.claude/mcp_servers.json). Default: 'global'.", "enum": ["global", "project"] }
                },
                "required": ["name", "command"]
            }
        },
        {
            "name": "crypt_env_update_mcp_server",
            "description": "Update an existing MCP server entry by name. Only provided fields are changed; omitted fields are preserved.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Server name to update" },
                    "command": { "type": "string", "description": "New command" },
                    "args": { "type": "array", "items": { "type": "string" }, "description": "New args list" },
                    "env": {
                        "type": "object",
                        "description": "New env map (merges with existing). Values are stored as empty placeholders.",
                        "additionalProperties": { "type": "string" }
                    },
                    "scope": { "type": "string", "description": "'global' or 'project'. Default: 'global'.", "enum": ["global", "project"] }
                },
                "required": ["name"]
            }
        },
        {
            "name": "crypt_env_delete_mcp_server",
            "description": "Remove an MCP server entry from the Claude config by name.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Server name to remove" },
                    "scope": { "type": "string", "description": "'global' or 'project'. Default: 'global'.", "enum": ["global", "project"] }
                },
                "required": ["name"]
            }
        },
        {
            "name": "crypt_env_list_environments_by_name",
            "description": "List all environments across all projects, grouped by their real environment name (e.g. production, local, test, or any custom name) rather than guessed from text.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "crypt_env_inject_env_by_name",
            "description": "Inject environment variables for a project directory and environment name. Matches a real environment by its name field (and, when ambiguous, by its configured path living under project_path). NOTE: the previous item-naming-convention fallback (matching items by ENV_KEY prefix or category when no environment matched) has been removed — GET /items and POST /fill now require an explicit project+environment scope that this fallback cannot supply. If no environment matches, this tool now returns an error with next steps instead of guessing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_path": { "type": "string", "description": "Absolute path to the project directory" },
                    "environment": { "type": "string", "description": "Environment name, e.g. production, local, test, or any custom name" },
                    "output_path": { "type": "string", "description": "Unused. Kept for backward compatibility with existing callers." }
                },
                "required": ["project_path", "environment"]
            }
        },
        {
            "name": "crypt_env_import_env_file",
            "description": "Reads a .env file from disk and imports each KEY=value pair as a secret item in the vault, owned by the given project and linked into the given environment. Secret values are read directly by the MCP process — they never appear in this response. Returns only the list of key names and import counts. Requires scope: 'environment_id', or both 'project' and 'environment'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path to the .env file to import, e.g. C:\\projects\\myapp\\.env" },
                    "category": { "type": "string", "description": "Optional category name to assign to all imported items" },
                    "overwrite": { "type": "boolean", "description": "If true, update existing vault items that have the same name or environment key in place, destroying their previous value. Default: false (skip duplicates instead of erroring)." },
                    "environment_id": { "type": "integer", "description": "Environment ID (scope). Provide this, or both 'project' and 'environment'." },
                    "project": { "type": "string", "description": "Project name (case-insensitive). Used with 'environment' when 'environment_id' is not given." },
                    "environment": { "type": "string", "description": "Environment name within the project (case-insensitive), e.g. production, local, test. Used with 'project'." }
                },
                "required": ["path"]
            }
        }
    ])
}

// ─── HTTP helpers ─────────────────────────────────────────────────────────────

fn vault_get(path: &str, token: &str) -> Result<reqwest::blocking::Response, String> {
    mcp_http_client()
        .get(format!("{}{path}", api_base()))
        .header("X-Vault-Token", token)
        .send()
        .map_err(|e| {
            if e.is_connect() {
                "Error: crypt-env app is not running. Open the application and try again.".to_string()
            } else {
                e.to_string()
            }
        })
}

fn vault_post(
    path: &str,
    token: &str,
    body: &serde_json::Value,
) -> Result<reqwest::blocking::Response, String> {
    mcp_http_client()
        .post(format!("{}{path}", api_base()))
        .header("X-Vault-Token", token)
        .json(body)
        .send()
        .map_err(|e| {
            if e.is_connect() {
                "Error: crypt-env app is not running. Open the application and try again.".to_string()
            } else {
                e.to_string()
            }
        })
}

fn vault_put(
    path: &str,
    token: &str,
    body: &serde_json::Value,
) -> Result<reqwest::blocking::Response, String> {
    mcp_http_client()
        .put(format!("{}{path}", api_base()))
        .header("X-Vault-Token", token)
        .json(body)
        .send()
        .map_err(|e| {
            if e.is_connect() {
                "Error: crypt-env app is not running. Open the application and try again.".to_string()
            } else {
                e.to_string()
            }
        })
}

fn vault_delete(path: &str, token: &str) -> Result<reqwest::blocking::Response, String> {
    mcp_http_client()
        .delete(format!("{}{path}", api_base()))
        .header("X-Vault-Token", token)
        .send()
        .map_err(|e| {
            if e.is_connect() {
                "Error: crypt-env app is not running. Open the application and try again.".to_string()
            } else {
                e.to_string()
            }
        })
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn random_hex(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn urlencod(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            ' ' => out.push('+'),
            c => {
                for byte in c.to_string().as_bytes() {
                    out.push_str(&format!("%{:02X}", byte));
                }
            }
        }
    }
    out
}

/// Appends project+environment scope query params to a URL being built, in
/// the same two shapes the REST API's `EnvScopeQuery` accepts: either
/// `environment_id` alone, or `project` + `environment` name pair (both
/// case-insensitive server-side). `environment_id` wins if both are given.
/// No-op if neither shape is present in `args` — the API will then reply
/// with its own 422 VALIDATION_ERROR, which callers already surface.
fn append_scope_params(url: &mut String, sep: &mut char, args: &serde_json::Value) {
    if let Some(id) = args.get("environment_id").and_then(|v| v.as_i64()) {
        url.push_str(&format!("{}environment_id={}", sep, id));
        *sep = '&';
        return;
    }
    if let Some(p) = args.get("project").and_then(|v| v.as_str()) {
        url.push_str(&format!("{}project={}", sep, urlencod(p)));
        *sep = '&';
    }
    if let Some(e) = args.get("environment").and_then(|v| v.as_str()) {
        url.push_str(&format!("{}environment={}", sep, urlencod(e)));
        *sep = '&';
    }
}

/// Valida que el nombre de variable de entorno sea seguro: ^[A-Z][A-Z0-9_]*$
/// y bloquea variables críticas del sistema.
fn is_safe_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() => {}
        _ => return false,
    }
    if !chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
        return false;
    }
    const BLOCKED: &[&str] = &[
        "PATH",
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
        "PYTHONPATH",
        "NODE_OPTIONS",
        "RUBYOPT",
    ];
    if BLOCKED.contains(&key) {
        return false;
    }
    if key.starts_with("LD_") {
        return false;
    }
    true
}

fn cleanup_prev_temp_files() {
    if let Ok(mut files) = TEMP_FILES.lock() {
        files.retain(|p| std::fs::remove_file(p).is_err());
    }
}

fn track_temp_file(path: PathBuf) {
    if let Ok(mut files) = TEMP_FILES.lock() {
        files.push(path);
    }
}

// ─── Tool implementations ─────────────────────────────────────────────────────

fn tool_list_items(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let mut url = "/items".to_string();
    let mut sep = '?';
    if let Some(t) = args.get("type").and_then(|v| v.as_str()) {
        url.push_str(&format!("{}type={}", sep, t));
        sep = '&';
    }
    if let Some(cat) = args.get("category").and_then(|v| v.as_str()) {
        url.push_str(&format!("{}category={}", sep, cat));
        sep = '&';
    }
    if let Some(ig) = args.get("include_global").and_then(|v| v.as_str()) {
        url.push_str(&format!("{}include_global={}", sep, urlencod(ig)));
        sep = '&';
    }
    append_scope_params(&mut url, &mut sep, args);

    let resp = match vault_get(&url, token) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    if resp.status().as_u16() == 403 {
        return tool_err("vault_locked: unlock the vault first");
    }
    if resp.status().as_u16() == 422 {
        let text = resp.text().unwrap_or_default();
        return tool_err(format!("scope required: pass 'environment_id', or both 'project' and 'environment' ({text})"));
    }

    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

fn tool_search_items(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let query = match args.get("query").and_then(|v| v.as_str()) {
        Some(q) => q.to_string(),
        None => return tool_err("required parameter: 'query'"),
    };

    let mut url = format!("/items?search={}", urlencod(&query));
    let mut sep = '&';
    if let Some(ig) = args.get("include_global").and_then(|v| v.as_str()) {
        url.push_str(&format!("{}include_global={}", sep, urlencod(ig)));
        sep = '&';
    }
    append_scope_params(&mut url, &mut sep, args);

    let resp = match vault_get(&url, token) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    if resp.status().as_u16() == 403 {
        return tool_err("vault_locked: unlock the vault first");
    }
    if resp.status().as_u16() == 422 {
        let text = resp.text().unwrap_or_default();
        return tool_err(format!("scope required: pass 'environment_id', or both 'project' and 'environment' ({text})"));
    }

    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

fn tool_get_item(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let id = match args.get("id").and_then(|v| v.as_i64()) {
        Some(i) => i,
        None => return tool_err("required parameter: 'id'"),
    };

    let resp = match vault_get(&format!("/items/{id}"), token) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    if status == 404 {
        return tool_err("item not found");
    }
    if status == 403 {
        return tool_err("vault_locked: unlock the vault first");
    }

    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

fn tool_generate_env(args: &serde_json::Value, token: &str) -> serde_json::Value {
    // Limpiar archivos .env del ciclo anterior antes de crear uno nuevo
    cleanup_prev_temp_files();

    let keys: Vec<String> = match args.get("keys").and_then(|v| v.as_array()) {
        Some(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        None => return tool_err("required parameter: 'keys' (array of strings)"),
    };

    let mut env_lines: Vec<String> = Vec::new();
    let mut n_found = 0usize;

    for key in &keys {
        if !is_safe_env_key(key) {
            env_lines.push(format!("# {key}: invalid or blocked name, skipped"));
            continue;
        }

        let mut search_url = format!("/items?search={}", urlencod(key));
        let mut sep = '&';
        append_scope_params(&mut search_url, &mut sep, args);
        let items_resp = match vault_get(&search_url, token) {
            Ok(r) => r,
            Err(e) => return tool_err(e),
        };

        if items_resp.status().as_u16() == 403 {
            return tool_err("vault_locked: unlock the vault first");
        }
        if items_resp.status().as_u16() == 422 {
            let text = items_resp.text().unwrap_or_default();
            return tool_err(format!("scope required: pass 'environment_id', or both 'project' and 'environment' ({text})"));
        }

        let items_text = match items_resp.text() {
            Ok(t) => t,
            Err(e) => return tool_err(format!("error reading items: {e}")),
        };

        let items_val: serde_json::Value = match serde_json::from_str(&items_text) {
            Ok(v) => v,
            Err(_) => {
                env_lines.push(format!("# {key}: error parsing response"));
                continue;
            }
        };

        let key_lower = key.to_lowercase();
        let found_id = items_val
            .as_array()
            .and_then(|arr| {
                arr.iter().find(|item| {
                    item.get("name")
                        .and_then(|n| n.as_str())
                        .map(|n| n.to_lowercase() == key_lower)
                        .unwrap_or(false)
                })
            })
            .and_then(|item| item.get("id").and_then(|v| v.as_i64()));

        let item_id = match found_id {
            Some(id) => id,
            None => {
                env_lines.push(format!("# {key}: not found in vault"));
                continue;
            }
        };

        let reveal_resp = match vault_post(
            &format!("/items/{item_id}/reveal"),
            token,
            &serde_json::json!({"confirm": true}),
        ) {
            Ok(r) => r,
            Err(e) => return tool_err(e),
        };

        let reveal_text = match reveal_resp.text() {
            Ok(t) => t,
            Err(e) => return tool_err(format!("error reading reveal: {e}")),
        };

        let reveal_val: serde_json::Value = match serde_json::from_str(&reveal_text) {
            Ok(v) => v,
            Err(_) => {
                env_lines.push(format!("# {key}: error parsing value"));
                continue;
            }
        };

        let value = reveal_val
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        env_lines.push(format!("{key}={value}"));
        n_found += 1;
    }

    let content = env_lines.join("\n");
    let filename = format!("crypt_env_{}.env", random_hex(8));
    let path = std::env::temp_dir().join(&filename);

    if let Err(e) = std::fs::write(&path, &content) {
        return tool_err(format!("error writing .env file: {e}"));
    }

    track_temp_file(path.clone());

    let path_str = path.to_string_lossy().to_string();
    tool_ok(
        serde_json::to_string_pretty(&serde_json::json!({
            "path": path_str,
            "count": n_found,
            "note": "This file contains secrets in plaintext. Load it and delete it immediately."
        }))
        .unwrap_or_default(),
    )
}

fn tool_inject_env(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let key = match args.get("key").and_then(|v| v.as_str()) {
        Some(k) => k.to_string(),
        None => return tool_err("required parameter: 'key'"),
    };

    if !is_safe_env_key(&key) {
        return tool_err(format!(
            "invalid or blocked variable name: '{key}'. \
             Must match [A-Z][A-Z0-9_]* and cannot be a critical system variable."
        ));
    }

    let mut search_url = format!("/items?search={}", urlencod(&key));
    let mut sep = '&';
    append_scope_params(&mut search_url, &mut sep, args);
    let items_resp = match vault_get(&search_url, token) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    if items_resp.status().as_u16() == 403 {
        return tool_err("vault_locked: unlock the vault first");
    }
    if items_resp.status().as_u16() == 422 {
        let text = items_resp.text().unwrap_or_default();
        return tool_err(format!("scope required: pass 'environment_id', or both 'project' and 'environment' ({text})"));
    }

    let items_text = match items_resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading items: {e}")),
    };

    let items_val: serde_json::Value = match serde_json::from_str(&items_text) {
        Ok(v) => v,
        Err(_) => return tool_err("error parsing item list"),
    };

    let key_lower = key.to_lowercase();
    let found_id = items_val
        .as_array()
        .and_then(|arr| {
            arr.iter().find(|item| {
                item.get("name")
                    .and_then(|n| n.as_str())
                    .map(|n| n.to_lowercase() == key_lower)
                    .unwrap_or(false)
            })
        })
        .and_then(|item| item.get("id").and_then(|v| v.as_i64()));

    let item_id = match found_id {
        Some(id) => id,
        None => return tool_err(format!("secret '{key}' not found in vault")),
    };

    let reveal_resp = match vault_post(
        &format!("/items/{item_id}/reveal"),
        token,
        &serde_json::json!({"confirm": true}),
    ) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let reveal_text = match reveal_resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading reveal: {e}")),
    };

    let reveal_val: serde_json::Value = match serde_json::from_str(&reveal_text) {
        Ok(v) => v,
        Err(_) => return tool_err("error parsing revealed value"),
    };

    let value = match reveal_val.get("value").and_then(|v| v.as_str()) {
        Some(v) => v.to_string(),
        None => return tool_err("item has no secret value"),
    };

    // Inyectar como variable de entorno
    #[allow(unused_unsafe)]
    unsafe {
        std::env::set_var(&key, &value);
    }

    tool_ok(
        serde_json::to_string_pretty(&serde_json::json!({
            "injected": true,
            "name": key
        }))
        .unwrap_or_default(),
    )
}

fn tool_add_item(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let item_type = match args.get("type").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => return tool_err("required parameter: 'type'"),
    };
    let name = match args.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => return tool_err("required parameter: 'name'"),
    };

    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string();

    let categories = if let Some(cat) = args.get("category").and_then(|v| v.as_str()) {
        serde_json::json!([cat])
    } else {
        serde_json::json!([])
    };

    let mut body = serde_json::json!({
        "id": 0,
        "type": item_type,
        "name": name,
        "categories": categories,
        "created": now_ts
    });

    // Campos opcionales
    for field in &["value", "notes", "url", "username"] {
        if let Some(v) = args.get(field).and_then(|v| v.as_str()) {
            body[field] = serde_json::json!(v);
        }
    }
    if let Some(key) = args.get("key").and_then(|v| v.as_str()) {
        body["key"] = serde_json::json!(key);
    }

    let mut url = "/items".to_string();
    let mut sep = '?';
    append_scope_params(&mut url, &mut sep, args);
    if let Some(mode) = args.get("on_conflict").and_then(|v| v.as_str()) {
        url.push_str(&format!("{sep}on_conflict={mode}"));
    }

    let resp = match vault_post(&url, token, &body) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error leyendo respuesta: {e}")),
    };

    if status == 403 {
        return tool_err("vault_locked: unlock the vault first");
    }
    if status == 422 {
        return tool_err(format!("validation error (scope or field): {text}"));
    }
    if status == 409 {
        // SHARED_ITEM_CONFLICT / KEY_EXISTS / CONFLICT_RETRY — the response
        // body already names the item and the remedy, never a secret value.
        return tool_err(format!("conflict creating item: {text}"));
    }
    if status >= 400 {
        return tool_err(format!("error creating item (HTTP {status}): {text}"));
    }

    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

fn tool_update_settings(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let resp = match vault_put("/settings", token, args) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    if status == 403 {
        return tool_err("vault_locked: unlock the vault first");
    }
    if status >= 400 {
        let text = resp.text().unwrap_or_default();
        return tool_err(format!("error updating settings (HTTP {status}): {text}"));
    }

    tool_ok(
        serde_json::to_string_pretty(&serde_json::json!({ "ok": true })).unwrap_or_default(),
    )
}

fn tool_fill_env(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let template = match args.get("template").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => return tool_err("required parameter: 'template'"),
    };
    let output_path = args.get("output_path").and_then(|v| v.as_str());
    let output_dir = args.get("output_dir").and_then(|v| v.as_str());

    let mut url = "/fill".to_string();
    let mut sep = '?';
    append_scope_params(&mut url, &mut sep, args);

    let overwrite = args.get("overwrite").and_then(|v| v.as_bool()).unwrap_or(false);

    let mut body = serde_json::json!({ "template": template, "overwrite": overwrite });
    if let Some(p) = output_path {
        body["output_path"] = serde_json::json!(p);
    }
    if let Some(d) = output_dir {
        body["output_dir"] = serde_json::json!(d);
    }

    let resp = match vault_post(&url, token, &body) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    if status == 403 {
        return tool_err("vault_locked: unlock the vault first");
    }
    if status == 422 {
        return tool_err(format!("scope or validation error: {text}"));
    }
    if status == 409 {
        return tool_err(format!("target_exists: {text}. Ask the user before retrying with overwrite=true."));
    }
    if status >= 400 {
        return tool_err(format!("fill failed (HTTP {status}): {text}"));
    }

    // The API already wrote the file to output_path/output_dir (if given) —
    // just surface the stats. If neither was given, `text` carries the
    // filled content inline (still no secret leakage risk beyond what the
    // caller already requested by omitting a write destination).
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

fn tool_doctor(_args: &serde_json::Value, _token: &str) -> serde_json::Value {
    let resp = match mcp_http_client_timeout(std::time::Duration::from_secs(3))
        .get(format!("{}/health", api_base()))
        .send()
    {
        Ok(r) => r,
        Err(e) => {
            return tool_ok(
                serde_json::to_string_pretty(&serde_json::json!({
                    "status": "not_running",
                    "error": "crypt-env app is not running. Open the application and try again.",
                    "detail": e.to_string()
                }))
                .unwrap_or_default(),
            )
        }
    };

    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading health response: {e}")),
    };

    let mut health: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return tool_ok(text),
    };

    // Add MCP token file status
    let mcp_token_file_ok = mcp_token_path()
        .map(|p| p.exists())
        .unwrap_or(false);

    health["mcp_server"] = serde_json::json!("running");
    health["mcp_token_file_present"] = serde_json::json!(mcp_token_file_ok);

    tool_ok(serde_json::to_string_pretty(&health).unwrap_or(text))
}

fn tool_list_commands(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let mut url = "/commands".to_string();
    let mut sep = '?';
    append_scope_params(&mut url, &mut sep, args);

    let resp = match vault_get(&url, token) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    if resp.status().as_u16() == 403 {
        return tool_err("vault_locked: unlock the vault first");
    }
    if resp.status().as_u16() == 422 {
        let text = resp.text().unwrap_or_default();
        return tool_err(format!("scope required: pass 'environment_id', or both 'project' and 'environment' ({text})"));
    }

    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

fn tool_run_command(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let cmd_name = match args.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => return tool_err("required parameter: 'name'"),
    };

    let mut list_url = "/commands".to_string();
    let mut sep = '?';
    append_scope_params(&mut list_url, &mut sep, args);

    let list_resp = match vault_get(&list_url, token) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    if list_resp.status().as_u16() == 403 {
        return tool_err("vault_locked: unlock the vault first");
    }
    if list_resp.status().as_u16() == 422 {
        let text = list_resp.text().unwrap_or_default();
        return tool_err(format!("scope required: pass 'environment_id', or both 'project' and 'environment' ({text})"));
    }

    let list_text = match list_resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading commands: {e}")),
    };

    let list_val: serde_json::Value = match serde_json::from_str(&list_text) {
        Ok(v) => v,
        Err(_) => return tool_err("error parsing command list"),
    };

    let name_lower = cmd_name.to_lowercase();
    let found = list_val
        .as_array()
        .and_then(|arr| {
            arr.iter().find(|cmd| {
                cmd.get("name")
                    .and_then(|n| n.as_str())
                    .map(|n| n.to_lowercase() == name_lower)
                    .unwrap_or(false)
            })
        })
        .and_then(|cmd| cmd.get("id").and_then(|v| v.as_i64()));

    let cmd_id = match found {
        Some(id) => id,
        None => return tool_err(format!("command '{cmd_name}' not found")),
    };

    let detail_resp = match vault_get(&format!("/commands/{cmd_id}"), token) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let detail_text = match detail_resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading command detail: {e}")),
    };

    let detail_val: serde_json::Value = match serde_json::from_str(&detail_text) {
        Ok(v) => v,
        Err(_) => return tool_err("error parsing command detail"),
    };

    let template = match detail_val.get("command").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => return tool_err("command has no template"),
    };

    // NOTE: The current command system does not resolve vault secret values at
    // this layer — secrets are managed separately. The {{VAR}} placeholders here
    // are substituted with caller-supplied params, not vault values. However, to
    // avoid echoing any resolved command string (which could contain sensitive
    // caller-supplied data or reveal template structure) back to the LLM host,
    // we execute the command directly and return only exit metadata.
    //
    // Secrets should be passed to the child process as environment variables
    // (set by the vault before execution), not substituted inline into the
    // command string, so they never appear as literal text in this process or
    // its return value.

    // Build the resolved command string for execution only — never returned.
    let mut resolved = template.clone();
    if let Some(params_map) = args.get("params").and_then(|v| v.as_object()) {
        for (var, val) in params_map {
            if let Some(val_str) = val.as_str() {
                let placeholder = format!("{{{{{}}}}}", var);
                resolved = resolved.replace(&placeholder, val_str);
            }
        }
    }

    // Execute the resolved command via the platform shell.
    // On Windows: cmd /C <resolved>
    // On Unix:    sh -c <resolved>
    // The resolved string is never included in the return value.
    #[cfg(target_os = "windows")]
    let child_result = std::process::Command::new("cmd")
        .args(["/C", &resolved])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    #[cfg(not(target_os = "windows"))]
    let child_result = std::process::Command::new("sh")
        .args(["-c", &resolved])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let child = match child_result {
        Ok(c) => c,
        Err(e) => {
            // Do not include the resolved command in the error message.
            return tool_err(format!("failed to start command '{cmd_name}': {e}"));
        }
    };

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => return tool_err(format!("error waiting for command '{cmd_name}': {e}")),
    };

    let exit_code = output.status.code().unwrap_or(-1);

    // Truncate stdout/stderr to 2000 chars each to limit accidental secret leakage
    // through command output. The resolved command itself is never returned.
    const MAX_OUTPUT: usize = 2000;
    let stdout_raw = String::from_utf8_lossy(&output.stdout);
    let stderr_raw = String::from_utf8_lossy(&output.stderr);

    let stdout_str = if stdout_raw.len() > MAX_OUTPUT {
        format!("{}... (truncated)", &stdout_raw[..MAX_OUTPUT])
    } else {
        stdout_raw.into_owned()
    };

    let stderr_str = if stderr_raw.len() > MAX_OUTPUT {
        format!("{}... (truncated)", &stderr_raw[..MAX_OUTPUT])
    } else {
        stderr_raw.into_owned()
    };

    tool_ok(
        serde_json::to_string_pretty(&serde_json::json!({
            "exit_code": exit_code,
            "stdout": stdout_str,
            "stderr": stderr_str
        }))
        .unwrap_or_else(|_| r#"{"exit_code":-1,"stdout":"","stderr":"serialization error"}"#.to_string()),
    )
}

// ─── Share tool implementations ───────────────────────────────────────────────

fn tool_share_listen(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let items = match args.get("items").and_then(|v| v.as_array()) {
        Some(arr) => arr.iter().filter_map(|v| v.as_i64()).collect::<Vec<_>>(),
        None => return tool_err("required parameter: 'items' (array of integers)"),
    };

    if items.is_empty() {
        return tool_err("items list must not be empty");
    }

    let mut url = "/share/listen".to_string();
    let mut sep = '?';
    append_scope_params(&mut url, &mut sep, args);

    let resp = match vault_post(&url, token, &serde_json::json!({ "items": items })) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    if status == 403 { return tool_err("vault_locked: unlock the vault first"); }
    if status == 422 { return tool_err(format!("scope or validation error: {text}")); }
    if status >= 400 { return tool_err(format!("share listen failed (HTTP {status}): {text}")); }

    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

fn tool_share_connect(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let pairing_code = match args.get("pairing_code").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => return tool_err("required parameter: 'pairing_code'"),
    };

    let mut url = "/share/connect".to_string();
    let mut sep = '?';
    append_scope_params(&mut url, &mut sep, args);

    let resp = match vault_post(&url, token, &serde_json::json!({ "pairing_code": pairing_code })) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    if status == 403 { return tool_err("vault_locked: unlock the vault first"); }
    if status == 422 { return tool_err(format!("scope or validation error: {text}")); }
    if status >= 400 { return tool_err(format!("share connect failed (HTTP {status}): {text}")); }

    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

fn tool_share_confirm(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let confirmed = match args.get("confirmed").and_then(|v| v.as_bool()) {
        Some(c) => c,
        None => return tool_err("required parameter: 'confirmed' (boolean)"),
    };

    let resp = match vault_post("/share/confirm", token, &serde_json::json!({ "confirmed": confirmed })) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    if status >= 400 { return tool_err(format!("confirm failed (HTTP {status}): {text}")); }

    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

fn tool_share_cancel(token: &str) -> serde_json::Value {
    let resp = match mcp_http_client()
        .delete(format!("{}/share/session", api_base()))
        .header("X-Vault-Token", token)
        .send()
        .map_err(|e| e.to_string())
    {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    if status >= 400 { return tool_err(format!("cancel failed (HTTP {status}): {text}")); }

    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

fn tool_share_status(token: &str) -> serde_json::Value {
    let resp = match vault_get("/share/status", token) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

fn tool_share_export(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let items = match args.get("items").and_then(|v| v.as_array()) {
        Some(arr) => arr.iter().filter_map(|v| v.as_i64()).collect::<Vec<_>>(),
        None => return tool_err("required parameter: 'items' (array of integers)"),
    };
    let output_path = match args.get("output_path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => return tool_err("required parameter: 'output_path'"),
    };

    let body = serde_json::json!({ "items": items, "output_path": output_path });
    let resp = match vault_post("/share/export", token, &body) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    if status == 403 { return tool_err("vault_locked: unlock the vault first"); }
    if status >= 400 { return tool_err(format!("export failed (HTTP {status}): {text}")); }

    // Surface the response including the passphrase — the tool description
    // instructs the model to show it to the user immediately.
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

fn tool_share_import(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let path = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => return tool_err("required parameter: 'path'"),
    };
    let passphrase = match args.get("passphrase").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => return tool_err("required parameter: 'passphrase'"),
    };

    let mut url = "/share/import".to_string();
    let mut sep = '?';
    append_scope_params(&mut url, &mut sep, args);

    let body = serde_json::json!({ "path": path, "passphrase": passphrase });
    let resp = match vault_post(&url, token, &body) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    if status == 403 { return tool_err("vault_locked: unlock the vault first"); }
    if status == 422 { return tool_err(format!("scope or validation error: {text}")); }
    if status >= 400 { return tool_err(format!("import failed (HTTP {status}): {text}")); }

    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

// ─── Item update / delete ─────────────────────────────────────────────────────

fn tool_update_item(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let id = match args.get("id").and_then(|v| v.as_i64()) {
        Some(i) => i,
        None => return tool_err("required parameter: 'id' (integer)"),
    };

    // Send only the fields provided. The API merges them server-side with the
    // existing encrypted item — secrets are never sent to or seen by the MCP.
    let mut body = serde_json::json!({ "id": id, "type": "", "categories": [], "created": "" });
    for field in &["name", "value", "url", "username", "password", "title",
                   "description", "notes", "content", "command", "shell"] {
        if let Some(v) = args.get(field).and_then(|v| v.as_str()) {
            body[field] = serde_json::json!(v);
        }
    }
    if let Some(cats) = args.get("categories").and_then(|v| v.as_array()) {
        body["categories"] = serde_json::json!(cats);
    }

    let resp = match vault_put(&format!("/items/{id}"), token, &body) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };
    if status == 403 { return tool_err("vault_locked: unlock the vault first"); }
    if status == 404 { return tool_err(format!("item not found: {id}")); }
    if status == 422 { return tool_err(format!("validation error: {text}")); }
    if status >= 400 { return tool_err(format!("error updating item (HTTP {status}): {text}")); }

    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

fn tool_delete_item(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let id = match args.get("id").and_then(|v| v.as_i64()) {
        Some(i) => i,
        None => return tool_err("required parameter: 'id' (integer)"),
    };

    let resp = match vault_delete(&format!("/items/{id}"), token) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    if status == 403 { return tool_err("vault_locked: unlock the vault first"); }
    if status == 404 { return tool_err(format!("item not found: {id}")); }
    if status >= 400 {
        let text = resp.text().unwrap_or_default();
        return tool_err(format!("error deleting item (HTTP {status}): {text}"));
    }

    tool_ok(
        serde_json::to_string_pretty(&serde_json::json!({ "deleted": true, "id": id }))
            .unwrap_or_default(),
    )
}

// ─── Workspace tool implementations ──────────────────────────────────────────

/// GET /projects — every project with its nested environments. Shared by all
/// project/environment tools so the real `name`/`id` fields are queried once
/// instead of each tool re-implementing its own text-sniffing heuristic.
fn fetch_projects(token: &str) -> Result<serde_json::Value, serde_json::Value> {
    let resp = vault_get("/projects", token).map_err(tool_err)?;
    let status = resp.status().as_u16();
    let text = resp
        .text()
        .map_err(|e| tool_err(format!("error reading response: {e}")))?;

    if status == 403 {
        return Err(tool_err("vault_locked: unlock the vault first"));
    }
    if status >= 400 {
        return Err(tool_err(format!("error listing projects (HTTP {status}): {text}")));
    }

    serde_json::from_str(&text).map_err(|_| tool_err("error parsing project list"))
}

fn tool_list_projects(token: &str) -> serde_json::Value {
    match fetch_projects(token) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or_default()),
        Err(e) => e,
    }
}

/// Outcome of resolving an environment identifier from MCP tool args.
#[derive(Debug, PartialEq, Eq)]
struct ResolvedEnvironment {
    id: i64,
    /// True when the caller used the deprecated `id` key instead of `environment_id`.
    /// Callers surface this to the model; remove together with the alias in 1.1.0.
    via_deprecated_id: bool,
}

/// Resolves an environment id from tool args shaped like `crypt_env_inject_environment`'s
/// schema: canonical `environment_id`, or the deprecated `id` alias (accepted through the
/// 1.0.x line, see issue #10), or `project` + `environment` names (case-insensitive,
/// resolved via GET /projects). Shared by every tool that identifies an environment this way.
///
/// Split into a network-fetching half (this function) and a pure matching
/// half (`pick_environment_id`) so the matching logic is unit-testable
/// without a live server.
fn resolve_environment_id(
    args: &serde_json::Value,
    token: &str,
) -> Result<ResolvedEnvironment, serde_json::Value> {
    if let Some(id) = args.get("environment_id").and_then(|v| v.as_i64()) {
        return Ok(ResolvedEnvironment { id, via_deprecated_id: false });
    }
    // DEPRECATED(remove in 1.1.0): environment 'id' alias, issue #10
    if let Some(id) = args.get("id").and_then(|v| v.as_i64()) {
        return Ok(ResolvedEnvironment { id, via_deprecated_id: true });
    }
    // Reject a missing name pair *before* the network call, so an argument
    // error is reported as one (issue #10's canonical message) rather than as
    // whatever connection failure `fetch_projects` happens to hit first.
    if !has_scope_name_pair(args) {
        return Err(missing_scope_err());
    }
    let projects = fetch_projects(token)?;
    pick_environment_id(args, &projects)
}

/// True when both `project` and `environment` name args are present as strings.
fn has_scope_name_pair(args: &serde_json::Value) -> bool {
    args.get("project").and_then(|v| v.as_str()).is_some()
        && args.get("environment").and_then(|v| v.as_str()).is_some()
}

/// The single canonical "no usable scope" error (issue #10 §1, step 4).
fn missing_scope_err() -> serde_json::Value {
    tool_err("required: 'environment_id' (environment id), or 'project' + 'environment' (names)")
}

/// Pure logic behind `resolve_environment_id`: given the already-fetched
/// `GET /projects` payload and the tool args, finds the environment id
/// matching `project` + `environment` (case-insensitive). Does not touch the
/// network or consult `id` in `args` — that shortcut is handled by the
/// caller before this is reached.
fn pick_environment_id(
    args: &serde_json::Value,
    projects: &serde_json::Value,
) -> Result<ResolvedEnvironment, serde_json::Value> {
    let (project, environment) = match (
        args.get("project").and_then(|v| v.as_str()),
        args.get("environment").and_then(|v| v.as_str()),
    ) {
        (Some(p), Some(e)) => (p, e),
        _ => return Err(missing_scope_err()),
    };

    let project_lower = project.to_lowercase();
    let env_lower = environment.to_lowercase();

    let found = projects.as_array().and_then(|arr| {
        arr.iter().find(|p| {
            p.get("name")
                .and_then(|n| n.as_str())
                .map(|n| n.to_lowercase() == project_lower)
                .unwrap_or(false)
        })
    }).and_then(|p| {
        p.get("environments").and_then(|v| v.as_array()).and_then(|envs| {
            envs.iter().find(|e| {
                e.get("name")
                    .and_then(|n| n.as_str())
                    .map(|n| n.to_lowercase() == env_lower)
                    .unwrap_or(false)
            })
        })
    }).and_then(|e| e.get("id").and_then(|v| v.as_i64()));

    found
        .map(|id| ResolvedEnvironment { id, via_deprecated_id: false })
        .ok_or_else(|| tool_err(format!("environment '{environment}' not found in project '{project}'")))
}

fn tool_inject_environment(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let ResolvedEnvironment { id: environment_id, via_deprecated_id } = match resolve_environment_id(args, token) {
        Ok(resolved) => resolved,
        Err(e) => return e,
    };

    let overwrite = args.get("overwrite").and_then(|v| v.as_bool()).unwrap_or(false);

    let mut inject_body = serde_json::json!({ "overwrite": overwrite });
    if let Some(p) = args.get("output_path").and_then(|v| v.as_str()) {
        inject_body["output_path"] = serde_json::json!(p);
    }
    if let Some(d) = args.get("output_dir").and_then(|v| v.as_str()) {
        inject_body["output_dir"] = serde_json::json!(d);
    }

    let resp = match vault_post(
        &format!("/environments/{environment_id}/inject"),
        token,
        &inject_body,
    ) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    if status == 403 { return tool_err("vault_locked: unlock the vault first"); }
    if status == 404 { return tool_err(format!("environment {environment_id} not found")); }
    if status == 409 {
        return tool_err(format!("target_exists: {text}. Ask the user before retrying with overwrite=true."));
    }
    if status >= 400 { return tool_err(format!("inject failed (HTTP {status}): {text}")); }

    let mut result_text = match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => serde_json::to_string_pretty(&v).unwrap_or(text),
        Err(_) => text,
    };
    if via_deprecated_id {
        result_text.push_str("\n\nnote: parameter 'id' is deprecated on this tool; use 'environment_id' instead.");
    }
    tool_ok(result_text)
}

/// Generates a `.env.example`-shaped placeholder for an environment via
/// POST /environments/:id/example — every linked key with an empty value.
/// The API never reads or decrypts the referenced items' actual values for
/// this endpoint, so there is no secret-leakage surface here by construction.
fn tool_generate_example_env(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let ResolvedEnvironment { id: environment_id, via_deprecated_id } = match resolve_environment_id(args, token) {
        Ok(resolved) => resolved,
        Err(e) => return e,
    };

    let overwrite = args.get("overwrite").and_then(|v| v.as_bool()).unwrap_or(false);

    let mut body = serde_json::json!({ "overwrite": overwrite });
    if let Some(p) = args.get("output_path").and_then(|v| v.as_str()) {
        body["output_path"] = serde_json::json!(p);
    }
    if let Some(d) = args.get("output_dir").and_then(|v| v.as_str()) {
        body["output_dir"] = serde_json::json!(d);
    }

    let resp = match vault_post(
        &format!("/environments/{environment_id}/example"),
        token,
        &body,
    ) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    if status == 403 { return tool_err("vault_locked: unlock the vault first"); }
    if status == 404 { return tool_err(format!("environment {environment_id} not found")); }
    if status == 409 {
        return tool_err(format!("target_exists: {text}. Ask the user before retrying with overwrite=true."));
    }
    if status >= 400 { return tool_err(format!("example generation failed (HTTP {status}): {text}")); }

    let mut result_text = match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => serde_json::to_string_pretty(&v).unwrap_or(text),
        Err(_) => text,
    };
    if via_deprecated_id {
        result_text.push_str("\n\nnote: parameter 'id' is deprecated on this tool; use 'environment_id' instead.");
    }
    tool_ok(result_text)
}

// ─── Relay tool implementations ───────────────────────────────────────────────

fn tool_relay_send(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let item_ids: Vec<i64> = match args.get("item_ids").and_then(|v| v.as_array()) {
        Some(arr) => arr.iter().filter_map(|v| v.as_i64()).collect(),
        None => return tool_err("required parameter: 'item_ids' (array of integers)"),
    };

    if item_ids.is_empty() {
        return tool_err("item_ids must not be empty");
    }

    let body = serde_json::json!({ "item_ids": item_ids });
    let resp = match vault_post("/relay/send", token, &body) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    if status == 403 { return tool_err("vault_locked: unlock the vault first"); }
    if status >= 400 { return tool_err(format!("relay send failed (HTTP {status}): {text}")); }

    // Surface the response including code and passphrase — the tool description
    // instructs the model to show both to the user immediately.
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

fn tool_relay_receive(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let code = match args.get("code").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => return tool_err("required parameter: 'code'"),
    };
    let passphrase = match args.get("passphrase").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => return tool_err("required parameter: 'passphrase'"),
    };

    let mut url = "/relay/receive".to_string();
    let mut sep = '?';
    append_scope_params(&mut url, &mut sep, args);

    let body = serde_json::json!({ "code": code, "passphrase": passphrase });
    let resp = match vault_post(&url, token, &body) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    if status == 403 { return tool_err("vault_locked: unlock the vault first"); }
    if status == 422 { return tool_err(format!("scope or validation error: {text}")); }
    if status >= 400 { return tool_err(format!("relay receive failed (HTTP {status}): {text}")); }

    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

// ─── Dispatch ─────────────────────────────────────────────────────────────────

// ─── Category tool implementations ───────────────────────────────────────────

fn tool_list_categories(token: &str) -> serde_json::Value {
    let resp = match vault_get("/categories", token) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    if status == 403 {
        return tool_err("vault_locked: unlock the vault first");
    }
    if status >= 400 {
        return tool_err(format!("error listing categories (HTTP {status}): {text}"));
    }

    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

fn tool_create_category(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let name = match args.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => return tool_err("required parameter: 'name'"),
    };
    let color = match args.get("color").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => return tool_err("required parameter: 'color'"),
    };

    let mut body = serde_json::json!({ "name": name, "color": color });
    if let Some(desc) = args.get("description").and_then(|v| v.as_str()) {
        body["description"] = serde_json::json!(desc);
    }
    let resp = match vault_post("/categories", token, &body) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    if status == 403 {
        return tool_err("vault_locked: unlock the vault first");
    }
    if status >= 400 {
        return tool_err(format!("error creating category (HTTP {status}): {text}"));
    }

    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

fn tool_update_category(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let id = match args.get("id").and_then(|v| v.as_str()) {
        Some(i) => i.to_string(),
        None => return tool_err("required parameter: 'id'"),
    };

    let mut body = serde_json::json!({});
    if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
        body["name"] = serde_json::json!(name);
    }
    if let Some(color) = args.get("color").and_then(|v| v.as_str()) {
        body["color"] = serde_json::json!(color);
    }
    if let Some(desc) = args.get("description").and_then(|v| v.as_str()) {
        body["description"] = serde_json::json!(desc);
    }

    let path = format!("/categories/{}", urlencod(&id));
    let resp = match vault_put(&path, token, &body) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();
    let text = match resp.text() {
        Ok(t) => t,
        Err(e) => return tool_err(format!("error reading response: {e}")),
    };

    if status == 403 {
        return tool_err("vault_locked: unlock the vault first");
    }
    if status == 404 {
        return tool_err(format!("category not found: {id}"));
    }
    if status >= 400 {
        return tool_err(format!("error updating category (HTTP {status}): {text}"));
    }

    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(v) => tool_ok(serde_json::to_string_pretty(&v).unwrap_or(text)),
        Err(_) => tool_ok(text),
    }
}

fn tool_delete_category(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let id = match args.get("id").and_then(|v| v.as_str()) {
        Some(i) => i.to_string(),
        None => return tool_err("required parameter: 'id'"),
    };

    let path = format!("/categories/{}", urlencod(&id));
    let resp = match vault_delete(&path, token) {
        Ok(r) => r,
        Err(e) => return tool_err(e),
    };

    let status = resp.status().as_u16();

    if status == 403 {
        return tool_err("vault_locked: unlock the vault first");
    }
    if status == 404 {
        return tool_err(format!("category not found: {id}"));
    }
    if status == 204 {
        return tool_ok(serde_json::to_string_pretty(&serde_json::json!({ "deleted": true }))
            .unwrap_or_default());
    }
    if status >= 400 {
        let text = resp.text().unwrap_or_default();
        return tool_err(format!("error deleting category (HTTP {status}): {text}"));
    }

    tool_ok(serde_json::to_string_pretty(&serde_json::json!({ "deleted": true }))
        .unwrap_or_default())
}

// ─── MCP config file helpers ──────────────────────────────────────────────────

/// Returns the path to the Claude desktop MCP config file on the current platform.
fn claude_desktop_config_path() -> Result<std::path::PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA")
            .map(|d| {
                std::path::PathBuf::from(d)
                    .join("Claude")
                    .join("claude_desktop_config.json")
            })
            .map_err(|_| "APPDATA environment variable not set".to_string())
    }

    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .map(|d| {
                std::path::PathBuf::from(d)
                    .join("Library")
                    .join("Application Support")
                    .join("Claude")
                    .join("claude_desktop_config.json")
            })
            .map_err(|_| "HOME environment variable not set".to_string())
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::env::var("HOME")
            .map(|d| {
                std::path::PathBuf::from(d)
                    .join(".config")
                    .join("Claude")
                    .join("claude_desktop_config.json")
            })
            .map_err(|_| "HOME environment variable not set".to_string())
    }
}

/// Returns the path to the project-level MCP servers config (.claude/mcp_servers.json in cwd).
fn project_mcp_config_path() -> Result<std::path::PathBuf, String> {
    std::env::current_dir()
        .map(|cwd| cwd.join(".claude").join("mcp_servers.json"))
        .map_err(|e| format!("cannot determine current directory: {e}"))
}

/// Reads a JSON config file, returning an empty object if the file does not exist.
fn read_json_config(path: &std::path::Path) -> Result<serde_json::Value, String> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("error reading {}: {e}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|e| format!("error parsing {}: {e}", path.display()))
}

/// Writes a JSON config atomically: write to a .tmp file then rename.
fn write_json_config_atomic(path: &std::path::Path, value: &serde_json::Value) -> Result<(), String> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create directory {}: {e}", parent.display()))?;
    }
    let tmp_path = path.with_extension("tmp");
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| format!("error serializing config: {e}"))?;
    std::fs::write(&tmp_path, &text)
        .map_err(|e| format!("error writing temp file {}: {e}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path)
        .map_err(|e| format!("error renaming temp file to {}: {e}", path.display()))
}

/// Strips env values from a server entry for safe output (only returns key names).
fn safe_server_entry(name: &str, entry: &serde_json::Value) -> serde_json::Value {
    let command = entry.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let args: Vec<serde_json::Value> = entry.get("args")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let env_keys: Vec<String> = entry.get("env")
        .and_then(|v| v.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    serde_json::json!({
        "name": name,
        "command": command,
        "args": args,
        "env_keys": env_keys
    })
}

// ─── MCP server management tool implementations ───────────────────────────────

fn tool_list_mcp_servers(args: &serde_json::Value, _token: &str) -> serde_json::Value {
    let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("all");

    let mut result = serde_json::json!({ "global": [], "project": [] });

    // Global scope
    if scope == "global" || scope == "all" {
        match claude_desktop_config_path() {
            Err(e) => return tool_err(format!("cannot determine global config path: {e}")),
            Ok(path) => {
                match read_json_config(&path) {
                    Err(e) => return tool_err(e),
                    Ok(config) => {
                        let servers: Vec<serde_json::Value> = config
                            .get("mcpServers")
                            .and_then(|v| v.as_object())
                            .map(|m| {
                                m.iter()
                                    .map(|(name, entry)| safe_server_entry(name, entry))
                                    .collect()
                            })
                            .unwrap_or_default();
                        result["global"] = serde_json::json!(servers);
                        result["global_path"] = serde_json::json!(path.to_string_lossy());
                    }
                }
            }
        }
    }

    // Project scope
    if scope == "project" || scope == "all" {
        match project_mcp_config_path() {
            Err(e) => return tool_err(format!("cannot determine project config path: {e}")),
            Ok(path) => {
                match read_json_config(&path) {
                    Err(e) => return tool_err(e),
                    Ok(config) => {
                        // Project config may be { "mcpServers": {...} } or directly the servers map
                        let servers_map = config.get("mcpServers")
                            .and_then(|v| v.as_object())
                            .or_else(|| config.as_object());
                        let servers: Vec<serde_json::Value> = servers_map
                            .map(|m| {
                                m.iter()
                                    .map(|(name, entry)| safe_server_entry(name, entry))
                                    .collect()
                            })
                            .unwrap_or_default();
                        result["project"] = serde_json::json!(servers);
                        result["project_path"] = serde_json::json!(path.to_string_lossy());
                    }
                }
            }
        }
    }

    tool_ok(serde_json::to_string_pretty(&result).unwrap_or_default())
}

fn tool_add_mcp_server(args: &serde_json::Value, _token: &str) -> serde_json::Value {
    let name = match args.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => return tool_err("required parameter: 'name'"),
    };
    let command = match args.get("command").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => return tool_err("required parameter: 'command'"),
    };
    let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("global");

    let cmd_args: Vec<serde_json::Value> = args.get("args")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Build env section with empty placeholders — never real values
    let env_obj: serde_json::Map<String, serde_json::Value> = args.get("env")
        .and_then(|v| v.as_object())
        .map(|m| {
            m.keys()
                .map(|k| (k.clone(), serde_json::Value::String(String::new())))
                .collect()
        })
        .unwrap_or_default();

    let config_path = match scope {
        "project" => match project_mcp_config_path() {
            Ok(p) => p,
            Err(e) => return tool_err(e),
        },
        _ => match claude_desktop_config_path() {
            Ok(p) => p,
            Err(e) => return tool_err(e),
        },
    };

    let mut config = match read_json_config(&config_path) {
        Ok(c) => c,
        Err(e) => return tool_err(e),
    };

    // Ensure mcpServers key exists
    if config.get("mcpServers").is_none() {
        config["mcpServers"] = serde_json::json!({});
    }

    let servers = match config.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        Some(m) => m,
        None => return tool_err("config has invalid 'mcpServers' structure"),
    };

    if servers.contains_key(&name) {
        return tool_err(format!("server '{name}' already exists — use crypt_env_update_mcp_server to modify it"));
    }

    let mut entry = serde_json::json!({ "command": command, "args": cmd_args });
    if !env_obj.is_empty() {
        entry["env"] = serde_json::Value::Object(env_obj.clone());
    }
    servers.insert(name.clone(), entry);

    if let Err(e) = write_json_config_atomic(&config_path, &config) {
        return tool_err(e);
    }

    let env_keys: Vec<String> = env_obj.keys().cloned().collect();
    let note = if env_keys.is_empty() {
        "Server registered. No env vars configured.".to_string()
    } else {
        format!(
            "Server registered. Env keys stored as empty placeholders: {:?}. \
             Run 'crypt_env_inject_env' for each key before launching the MCP server.",
            env_keys
        )
    };

    tool_ok(serde_json::to_string_pretty(&serde_json::json!({
        "added": true,
        "name": name,
        "scope": scope,
        "config_path": config_path.to_string_lossy(),
        "note": note
    })).unwrap_or_default())
}

fn tool_update_mcp_server(args: &serde_json::Value, _token: &str) -> serde_json::Value {
    let name = match args.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => return tool_err("required parameter: 'name'"),
    };
    let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("global");

    let config_path = match scope {
        "project" => match project_mcp_config_path() {
            Ok(p) => p,
            Err(e) => return tool_err(e),
        },
        _ => match claude_desktop_config_path() {
            Ok(p) => p,
            Err(e) => return tool_err(e),
        },
    };

    let mut config = match read_json_config(&config_path) {
        Ok(c) => c,
        Err(e) => return tool_err(e),
    };

    // Ensure mcpServers exists
    if config.get("mcpServers").is_none() {
        config["mcpServers"] = serde_json::json!({});
    }

    let servers = match config.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        Some(m) => m,
        None => return tool_err("config has invalid 'mcpServers' structure"),
    };

    if !servers.contains_key(&name) {
        return tool_err(format!("server '{name}' not found — use crypt_env_add_mcp_server to create it"));
    }

    let entry = servers.entry(name.clone()).or_insert_with(|| serde_json::json!({}));

    if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
        entry["command"] = serde_json::json!(cmd);
    }
    if let Some(cmd_args) = args.get("args").and_then(|v| v.as_array()) {
        entry["args"] = serde_json::json!(cmd_args);
    }
    if let Some(env_map) = args.get("env").and_then(|v| v.as_object()) {
        // Merge: existing keys preserved, new keys added as empty placeholders
        let existing_env = entry
            .get("env")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let mut merged = existing_env;
        for k in env_map.keys() {
            merged.insert(k.clone(), serde_json::Value::String(String::new()));
        }
        entry["env"] = serde_json::Value::Object(merged);
    }

    if let Err(e) = write_json_config_atomic(&config_path, &config) {
        return tool_err(e);
    }

    tool_ok(serde_json::to_string_pretty(&serde_json::json!({
        "updated": true,
        "name": name,
        "scope": scope,
        "config_path": config_path.to_string_lossy()
    })).unwrap_or_default())
}

fn tool_delete_mcp_server(args: &serde_json::Value, _token: &str) -> serde_json::Value {
    let name = match args.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => return tool_err("required parameter: 'name'"),
    };
    let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("global");

    let config_path = match scope {
        "project" => match project_mcp_config_path() {
            Ok(p) => p,
            Err(e) => return tool_err(e),
        },
        _ => match claude_desktop_config_path() {
            Ok(p) => p,
            Err(e) => return tool_err(e),
        },
    };

    let mut config = match read_json_config(&config_path) {
        Ok(c) => c,
        Err(e) => return tool_err(e),
    };

    let servers = match config.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        Some(m) => m,
        None => return tool_err(format!("server '{name}' not found (mcpServers is empty or missing)")),
    };

    if servers.remove(&name).is_none() {
        return tool_err(format!("server '{name}' not found"));
    }

    if let Err(e) = write_json_config_atomic(&config_path, &config) {
        return tool_err(e);
    }

    tool_ok(serde_json::to_string_pretty(&serde_json::json!({
        "deleted": true,
        "name": name,
        "scope": scope,
        "config_path": config_path.to_string_lossy()
    })).unwrap_or_default())
}

// ─── Environment-aware workspace injection tool implementations ───────────────

fn tool_list_environments_by_name(token: &str) -> serde_json::Value {
    let projects = match fetch_projects(token) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let arr = match projects.as_array() {
        Some(a) => a,
        None => return tool_err("unexpected project response format"),
    };

    // Real query: group by each environment's actual `name` field instead of
    // guessing an environment from project/workspace name text.
    let mut groups: std::collections::HashMap<String, Vec<serde_json::Value>> =
        std::collections::HashMap::new();

    for project in arr {
        let project_id = project.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let project_name = project.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let envs = project.get("environments").and_then(|v| v.as_array()).cloned().unwrap_or_default();

        for env in envs {
            let group_key = env.get("name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            let entry = serde_json::json!({
                "projectId": project_id,
                "projectName": project_name,
                "environmentId": env.get("id"),
                "environmentName": env.get("name"),
                "paths": env.get("paths"),
                "varCount": env.get("vars").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0),
            });
            groups.entry(group_key).or_default().push(entry);
        }
    }

    tool_ok(serde_json::to_string_pretty(&groups).unwrap_or_default())
}

fn tool_inject_env_by_name(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let project_path = match args.get("project_path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => return tool_err("required parameter: 'project_path'"),
    };
    let environment = match args.get("environment").and_then(|v| v.as_str()) {
        Some(e) => e.to_lowercase(),
        None => return tool_err("required parameter: 'environment'"),
    };
    let output_path = args.get("output_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{project_path}/.env"));

    let projects = match fetch_projects(token) {
        Ok(v) => v,
        Err(e) => return e,
    };

    // Real query: match environments by their actual `name` field (not a
    // sniffed guess), scoped first to ones whose configured paths live under
    // project_path; if none match by path, fall back to any environment with
    // that exact name as long as it's unambiguous.
    let collect_matches = |require_path: bool| -> Vec<(i64, i64, String, String)> {
        let mut out = Vec::new();
        let Some(arr) = projects.as_array() else { return out; };
        for project in arr {
            let project_id = match project.get("id").and_then(|v| v.as_i64()) { Some(i) => i, None => continue };
            let project_name = project.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let envs = project.get("environments").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            for env in envs {
                let env_id = match env.get("id").and_then(|v| v.as_i64()) { Some(i) => i, None => continue };
                let env_name = env.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if env_name.to_lowercase() != environment {
                    continue;
                }
                if require_path {
                    let path_matches = env.get("paths").and_then(|v| v.as_array())
                        .map(|paths| paths.iter().any(|p| {
                            p.as_str().map(|s| s.starts_with(&project_path)).unwrap_or(false)
                        }))
                        .unwrap_or(false);
                    if !path_matches {
                        continue;
                    }
                }
                out.push((project_id, env_id, project_name.clone(), env_name.clone()));
            }
        }
        out
    };

    let mut candidates = collect_matches(true);
    if candidates.is_empty() {
        candidates = collect_matches(false);
    }

    if candidates.len() > 1 {
        let options: Vec<String> = candidates.iter()
            .map(|(pid, eid, pname, ename)| format!("{pname} / {ename} (project {pid}, environment {eid})"))
            .collect();
        return tool_err(format!(
            "ambiguous match for environment '{environment}': {}. Use crypt_env_inject_environment with a specific 'environment_id'.",
            options.join(", ")
        ));
    }

    if let Some((project_id, env_id, project_name, env_name)) = candidates.into_iter().next() {
        // Step 3a: environment found — inject it
        let inject_resp = match vault_post(
            &format!("/environments/{env_id}/inject"),
            token,
            &serde_json::json!({}),
        ) {
            Ok(r) => r,
            Err(e) => return tool_err(e),
        };

        let inject_status = inject_resp.status().as_u16();
        let inject_text = match inject_resp.text() {
            Ok(t) => t,
            Err(e) => return tool_err(format!("error reading inject response: {e}")),
        };

        if inject_status == 403 { return tool_err("vault_locked: unlock the vault first"); }
        if inject_status >= 400 { return tool_err(format!("inject failed (HTTP {inject_status}): {inject_text}")); }

        let mut inject_result: serde_json::Value = serde_json::from_str(&inject_text)
            .unwrap_or(serde_json::json!({}));
        inject_result["method"] = serde_json::json!("environment");
        inject_result["projectId"] = serde_json::json!(project_id);
        inject_result["projectName"] = serde_json::json!(project_name);
        inject_result["environmentId"] = serde_json::json!(env_id);
        inject_result["environmentName"] = serde_json::json!(env_name);

        return tool_ok(serde_json::to_string_pretty(&inject_result).unwrap_or_default());
    }

    // Step 3b (formerly the item-naming-convention fallback): no environment
    // matched by name under project_path. This fallback used to scan the
    // entire vault with an unscoped GET /items and hand-build a /fill
    // template — both endpoints now hard-require a project+environment
    // scope (see docs/reference.md MCP notes), and there is no such scope
    // to supply here by design (that's the whole reason this branch exists).
    // Rather than send a request that is now guaranteed to 422, fail fast
    // with actionable next steps.
    let _ = output_path; // kept for signature/doc parity; no longer used on this path
    tool_err(format!(
        "no environment named '{environment}' found under project_path '{project_path}', and the item-naming-convention fallback is no longer available: GET /items and POST /fill now require an explicit project+environment scope, which this fallback cannot supply. Use crypt_env_list_projects or crypt_env_list_environments_by_name to find the right project+environment, then call crypt_env_inject_environment directly, or create an environment named '{environment}' in this project."
    ))
}

// ─── Import .env file tool implementation ─────────────────────────────────────

fn build_item_body(id: i64, key: &str, value: &str, category: &Option<String>, now_ts: &str) -> serde_json::Value {
    let categories = if let Some(cat) = category {
        serde_json::json!([cat])
    } else {
        serde_json::json!([])
    };
    serde_json::json!({
        "id": id,
        "type": "secret",
        "name": key,
        "value": value,
        "categories": categories,
        "created": now_ts
    })
}

fn tool_import_env_file(args: &serde_json::Value, token: &str) -> serde_json::Value {
    let path = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => return tool_err("required parameter: 'path'"),
    };
    let category: Option<String> = args.get("category").and_then(|v| v.as_str()).map(|s| s.to_string());
    let overwrite = args.get("overwrite").and_then(|v| v.as_bool()).unwrap_or(false);

    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => return tool_err(format!("cannot read file: {e}")),
    };

    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string();

    // Parse pairs from file
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut skipped_invalid: Vec<String> = Vec::new();

    for line in contents.lines() {
        let trimmed = line.trim();
        // Skip blank lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Split on first '=' only
        let eq_pos = match trimmed.find('=') {
            Some(pos) => pos,
            None => continue,
        };
        let raw_key = trimmed[..eq_pos].trim();
        let raw_val = trimmed[eq_pos + 1..].trim();

        if raw_key.is_empty() {
            continue;
        }

        // Strip surrounding quotes from value
        let value = if (raw_val.starts_with('"') && raw_val.ends_with('"'))
            || (raw_val.starts_with('\'') && raw_val.ends_with('\''))
        {
            raw_val[1..raw_val.len() - 1].to_string()
        } else {
            raw_val.to_string()
        };

        if !is_safe_env_key(raw_key) {
            skipped_invalid.push(raw_key.to_string());
            continue;
        }

        pairs.push((raw_key.to_string(), value));
    }

    let mut imported: u32 = 0;
    let mut updated: u32 = 0;
    let mut skipped_existing: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut keys: Vec<String> = Vec::new();

    for (key, value) in &pairs {
        // Search for existing item by key name
        let mut search_url = format!("/items?search={}", urlencod(key));
        let mut search_sep = '&';
        append_scope_params(&mut search_url, &mut search_sep, args);
        let search_resp = match vault_get(&search_url, token) {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("{key}: {e}"));
                continue;
            }
        };

        if search_resp.status().as_u16() == 403 {
            return tool_err("vault_locked: unlock the vault first");
        }
        if search_resp.status().as_u16() == 422 {
            let text = search_resp.text().unwrap_or_default();
            return tool_err(format!("scope required: pass 'environment_id', or both 'project' and 'environment' ({text})"));
        }

        let search_text = match search_resp.text() {
            Ok(t) => t,
            Err(e) => {
                errors.push(format!("{key}: error reading search response: {e}"));
                continue;
            }
        };

        let search_val: serde_json::Value = match serde_json::from_str(&search_text) {
            Ok(v) => v,
            Err(e) => {
                errors.push(format!("{key}: error parsing search response: {e}"));
                continue;
            }
        };

        let key_lower = key.to_lowercase();
        let existing = search_val.as_array().and_then(|arr| {
            arr.iter().find(|item| {
                item.get("name")
                    .and_then(|n| n.as_str())
                    .map(|n| n.to_lowercase() == key_lower)
                    .unwrap_or(false)
            })
        });

        if let Some(existing_item) = existing {
            if !overwrite {
                skipped_existing.push(key.clone());
                continue;
            }
            // Update existing item
            let existing_id = match existing_item.get("id").and_then(|v| v.as_i64()) {
                Some(id) => id,
                None => {
                    errors.push(format!("{key}: existing item has no id"));
                    continue;
                }
            };
            let body = build_item_body(existing_id, key, value, &category, "");
            let resp = match vault_put(&format!("/items/{existing_id}"), token, &body) {
                Ok(r) => r,
                Err(e) => {
                    errors.push(format!("{key}: {e}"));
                    continue;
                }
            };
            let status = resp.status().as_u16();
            if status == 403 {
                return tool_err("vault_locked: unlock the vault first");
            }
            if status >= 400 {
                let text = resp.text().unwrap_or_default();
                errors.push(format!("{key}: update failed (HTTP {status}): {text}"));
                continue;
            }
            updated += 1;
            keys.push(key.clone());
        } else {
            // Create new item, linked into the scoped environment. The
            // name-based search above can miss a key that is linked under a
            // renamed item (name != key), so this POST can still collide on
            // the environment key even when `existing` was None — map
            // `overwrite` onto `on_conflict` explicitly rather than relying
            // on the server default, so that path is covered too:
            // overwrite=true -> update in place; overwrite=false -> error,
            // treated below as a skip like the by-name check above. This is
            // what keeps bulk import from mass-producing orphans (issue #9).
            let body = build_item_body(0, key, value, &category, &now_ts);
            let mut create_url = "/items".to_string();
            let mut create_sep = '?';
            append_scope_params(&mut create_url, &mut create_sep, args);
            let on_conflict = if overwrite { "update" } else { "error" };
            create_url.push_str(&format!("{create_sep}on_conflict={on_conflict}"));
            let resp = match vault_post(&create_url, token, &body) {
                Ok(r) => r,
                Err(e) => {
                    errors.push(format!("{key}: {e}"));
                    continue;
                }
            };
            let status = resp.status().as_u16();
            if status == 403 {
                return tool_err("vault_locked: unlock the vault first");
            }
            if status == 422 {
                let text = resp.text().unwrap_or_default();
                return tool_err(format!("scope required: pass 'environment_id', or both 'project' and 'environment' ({text})"));
            }
            if status == 409 {
                // Racing/renamed-item collision the name search couldn't see.
                // Same report bucket as the by-name skip path above.
                skipped_existing.push(key.clone());
                continue;
            }
            if status >= 400 {
                let text = resp.text().unwrap_or_default();
                errors.push(format!("{key}: create failed (HTTP {status}): {text}"));
                continue;
            }
            imported += 1;
            keys.push(key.clone());
        }
    }

    tool_ok(
        serde_json::to_string_pretty(&serde_json::json!({
            "imported": imported,
            "updated": updated,
            "skipped_existing": skipped_existing,
            "skipped_invalid": skipped_invalid,
            "errors": errors,
            "keys": keys
        }))
        .unwrap_or_default(),
    )
}

fn dispatch(
    method: &str,
    params: &serde_json::Value,
    token: &str,
) -> Result<serde_json::Value, RpcError> {
    match method {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": MCP_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "crypt-env", "version": "0.1.0" }
        })),
        "ping" => Ok(serde_json::json!({})),
        "tools/list" => Ok(serde_json::json!({ "tools": tool_definitions() })),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::Value::Object(Default::default()));
            Ok(handle_tool_call(name, &args, token))
        }
        _ => Err(RpcError {
            code: -32601,
            message: format!("Method not found: {method}"),
        }),
    }
}

fn handle_tool_call(name: &str, args: &serde_json::Value, token: &str) -> serde_json::Value {
    match name {
        "crypt_env_list_items" => tool_list_items(args, token),
        "crypt_env_get_item" => tool_get_item(args, token),
        "crypt_env_search_items" => tool_search_items(args, token),
        "crypt_env_fill_env" => tool_fill_env(args, token),
        "crypt_env_generate_env" => tool_generate_env(args, token),
        "crypt_env_inject_env" => tool_inject_env(args, token),
        "crypt_env_add_item" => tool_add_item(args, token),
        "crypt_env_update_item" => tool_update_item(args, token),
        "crypt_env_delete_item" => tool_delete_item(args, token),
        "crypt_env_update_settings" => tool_update_settings(args, token),
        "crypt_env_doctor" => tool_doctor(args, token),
        "crypt_env_list_commands" => tool_list_commands(args, token),
        "crypt_env_run_command" => tool_run_command(args, token),
        "crypt_env_share_listen" => tool_share_listen(args, token),
        "crypt_env_share_connect" => tool_share_connect(args, token),
        "crypt_env_share_confirm" => tool_share_confirm(args, token),
        "crypt_env_share_cancel" => tool_share_cancel(token),
        "crypt_env_share_status" => tool_share_status(token),
        "crypt_env_share_export" => tool_share_export(args, token),
        "crypt_env_share_import" => tool_share_import(args, token),
        "crypt_env_list_categories" => tool_list_categories(token),
        "crypt_env_create_category" => tool_create_category(args, token),
        "crypt_env_update_category" => tool_update_category(args, token),
        "crypt_env_delete_category" => tool_delete_category(args, token),
        "crypt_env_list_projects" => tool_list_projects(token),
        "crypt_env_inject_environment" => tool_inject_environment(args, token),
        "crypt_env_generate_example_env" => tool_generate_example_env(args, token),
        "crypt_env_relay_send" => tool_relay_send(args, token),
        "crypt_env_relay_receive" => tool_relay_receive(args, token),
        "crypt_env_list_mcp_servers" => tool_list_mcp_servers(args, token),
        "crypt_env_add_mcp_server" => tool_add_mcp_server(args, token),
        "crypt_env_update_mcp_server" => tool_update_mcp_server(args, token),
        "crypt_env_delete_mcp_server" => tool_delete_mcp_server(args, token),
        "crypt_env_list_environments_by_name" => tool_list_environments_by_name(token),
        "crypt_env_inject_env_by_name" => tool_inject_env_by_name(args, token),
        "crypt_env_import_env_file" => tool_import_env_file(args, token),
        _ => tool_err(format!("unknown tool: {name}")),
    }
}

// ─── I/O ──────────────────────────────────────────────────────────────────────

fn writeln_json(stdout: &std::io::Stdout, val: &impl Serialize) {
    let mut lock = stdout.lock();
    let _ = serde_json::to_writer(&mut lock, val);
    let _ = writeln!(lock);
    let _ = lock.flush();
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    // Resolve the REST endpoint once, before any request. A malformed
    // `CRYPTENV_API_URL` is fatal here rather than at first use.
    match resolve_api_base() {
        Ok(base) => {
            let _ = API_BASE_CACHE.set(base);
        }
        Err(e) => {
            eprintln!("[crypt-env-mcp] {e}");
            std::process::exit(1);
        }
    }

    let token = match read_mcp_token() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("[crypt-env-mcp] {e}");
            std::process::exit(1);
        }
    };

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) if l.trim().is_empty() => continue,
            Ok(l) => l,
            Err(_) => break,
        };

        let req: RpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = RpcResponse {
                    jsonrpc: "2.0",
                    id: serde_json::Value::Null,
                    result: None,
                    error: Some(RpcError {
                        code: -32700,
                        message: format!("Parse error: {e}"),
                    }),
                };
                writeln_json(&stdout, &resp);
                continue;
            }
        };

        // Notificaciones (sin id) — no requieren respuesta
        if req.id.is_none() {
            continue;
        }

        let id = req.id.clone().unwrap_or(serde_json::Value::Null);
        let result = dispatch(&req.method, &req.params, &token);
        let resp = match result {
            Ok(r) => RpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(r),
                error: None,
            },
            Err(e) => RpcResponse {
                jsonrpc: "2.0",
                id,
                result: None,
                error: Some(e),
            },
        };
        writeln_json(&stdout, &resp);
    }
}

// ─── Tests (issues #10, #11 — MCP unit coverage) ──────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─── append_scope_params ────────────────────────────────────────────────

    #[test]
    fn append_scope_params_uses_environment_id_alone_when_present() {
        let mut url = "/items".to_string();
        let mut sep = '?';
        let args = serde_json::json!({ "environment_id": 42, "project": "demo", "environment": "production" });
        append_scope_params(&mut url, &mut sep, &args);
        assert_eq!(url, "/items?environment_id=42");
    }

    #[test]
    fn append_scope_params_uses_project_and_environment_when_no_id() {
        let mut url = "/items".to_string();
        let mut sep = '?';
        let args = serde_json::json!({ "project": "demo", "environment": "production" });
        append_scope_params(&mut url, &mut sep, &args);
        assert_eq!(url, "/items?project=demo&environment=production");
    }

    #[test]
    fn append_scope_params_is_a_noop_when_nothing_present() {
        let mut url = "/items".to_string();
        let mut sep = '?';
        let args = serde_json::json!({});
        append_scope_params(&mut url, &mut sep, &args);
        assert_eq!(url, "/items");
    }

    // ─── is_safe_env_key ────────────────────────────────────────────────────

    #[test]
    fn is_safe_env_key_accepts_a_normal_uppercase_key() {
        assert!(is_safe_env_key("DB_HOST"));
    }

    #[test]
    fn is_safe_env_key_rejects_blocked_system_variables() {
        assert!(!is_safe_env_key("PATH"));
        assert!(!is_safe_env_key("LD_PRELOAD"));
        assert!(!is_safe_env_key("LD_ANYTHING"));
    }

    #[test]
    fn is_safe_env_key_rejects_lowercase_and_invalid_leading_chars() {
        assert!(!is_safe_env_key("db_host"));
        assert!(!is_safe_env_key("1KEY"));
        assert!(!is_safe_env_key(""));
    }

    // ─── pick_environment_id ────────────────────────────────────────────────

    fn sample_projects() -> serde_json::Value {
        serde_json::json!([
            {
                "id": 1,
                "name": "Demo",
                "environments": [
                    { "id": 10, "name": "Production" },
                    { "id": 11, "name": "local" },
                ],
            },
        ])
    }

    #[test]
    fn pick_environment_id_matches_case_insensitively() {
        let args = serde_json::json!({ "project": "demo", "environment": "PRODUCTION" });
        let resolved = pick_environment_id(&args, &sample_projects()).unwrap();
        assert_eq!(resolved.id, 10);
        // Name-pair resolution is never the deprecated `id` alias path.
        assert!(!resolved.via_deprecated_id);
    }

    #[test]
    fn pick_environment_id_errors_on_unknown_project() {
        let args = serde_json::json!({ "project": "ghost", "environment": "production" });
        assert!(pick_environment_id(&args, &sample_projects()).is_err());
    }

    #[test]
    fn pick_environment_id_errors_on_known_project_unknown_environment() {
        let args = serde_json::json!({ "project": "demo", "environment": "ghost" });
        assert!(pick_environment_id(&args, &sample_projects()).is_err());
    }

    #[test]
    fn pick_environment_id_errors_when_project_or_environment_args_missing() {
        let args = serde_json::json!({ "project": "demo" });
        assert!(pick_environment_id(&args, &sample_projects()).is_err());
    }

    // ─── tool-list schema invariants (issue #10) ────────────────────────────

    const CANON_ENVIRONMENT_ID_DESC: &str =
        "Environment ID (scope). Provide this, or both 'project' and 'environment'.";
    const CANON_PROJECT_DESC: &str =
        "Project name (case-insensitive). Used with 'environment' when 'environment_id' is not given.";
    const CANON_ENVIRONMENT_DESC: &str =
        "Environment name within the project (case-insensitive), e.g. production, local, test. Used with 'project'.";

    fn tool_name(tool: &serde_json::Value) -> &str {
        tool.get("name").and_then(|v| v.as_str()).unwrap_or("<unnamed>")
    }

    /// A tool is treated as environment-scoped iff its `inputSchema.properties`
    /// contains both `project` and `environment`. `crypt_env_inject_env_by_name` is
    /// excluded by construction: it only has `project_path`, not `project`.
    fn environment_scoped_tools(tools: &serde_json::Value) -> Vec<&serde_json::Value> {
        tools
            .as_array()
            .expect("tool_definitions() must return a JSON array")
            .iter()
            .filter(|t| match t.pointer("/inputSchema/properties") {
                Some(props) => props.get("project").is_some() && props.get("environment").is_some(),
                None => false,
            })
            .collect()
    }

    #[test]
    fn environment_scoped_tool_count_is_stable() {
        let tools = tool_definitions();
        let scoped = environment_scoped_tools(&tools);
        assert_eq!(
            scoped.len(),
            15,
            "expected exactly 15 environment-scoped tools (project+environment present); found {}: {:?}",
            scoped.len(),
            scoped.iter().map(|t| tool_name(t)).collect::<Vec<_>>()
        );
    }

    /// Test 1 (plan §1): every environment-scoped tool declares `environment_id`
    /// and none declares a bare `id`.
    #[test]
    fn every_environment_scoped_tool_declares_environment_id() {
        let tools = tool_definitions();
        for tool in environment_scoped_tools(&tools) {
            let name = tool_name(tool);
            let props = tool
                .pointer("/inputSchema/properties")
                .unwrap_or_else(|| panic!("tool {name} has no inputSchema.properties"));
            assert!(
                props.get("environment_id").is_some(),
                "tool {name} is environment-scoped but does not declare 'environment_id'"
            );
            assert!(
                props.get("id").is_none(),
                "tool {name} declares a bare 'id' for an environment"
            );
        }
    }

    /// Test 2 (plan §1): the three canonical description strings are byte-identical
    /// across every environment-scoped tool.
    #[test]
    fn environment_scope_descriptions_are_canonical() {
        let tools = tool_definitions();
        for tool in environment_scoped_tools(&tools) {
            let name = tool_name(tool);
            let props = tool
                .pointer("/inputSchema/properties")
                .unwrap_or_else(|| panic!("tool {name} has no inputSchema.properties"));

            let environment_id_desc = props
                .pointer("/environment_id/description")
                .and_then(|v| v.as_str());
            assert_eq!(
                environment_id_desc,
                Some(CANON_ENVIRONMENT_ID_DESC),
                "tool {name} has a non-canonical 'environment_id' description"
            );

            let project_desc = props.pointer("/project/description").and_then(|v| v.as_str());
            assert_eq!(
                project_desc,
                Some(CANON_PROJECT_DESC),
                "tool {name} has a non-canonical 'project' description"
            );

            let environment_desc = props.pointer("/environment/description").and_then(|v| v.as_str());
            assert_eq!(
                environment_desc,
                Some(CANON_ENVIRONMENT_DESC),
                "tool {name} has a non-canonical 'environment' description"
            );
        }
    }

    /// Test 3 (plan §1): the five tools where `id` correctly means an item, category,
    /// or workspace id keep declaring bare `id` and do not pick up `environment_id`.
    /// Guards against an over-eager find-and-replace.
    #[test]
    fn id_tools_keep_bare_id() {
        let tools = tool_definitions();
        let arr = tools.as_array().expect("tool_definitions() must return a JSON array");
        let id_tools = [
            "crypt_env_get_item",
            "crypt_env_update_item",
            "crypt_env_delete_item",
            "crypt_env_update_category",
            "crypt_env_delete_category",
        ];
        for name in id_tools {
            let tool = arr
                .iter()
                .find(|t| tool_name(t) == name)
                .unwrap_or_else(|| panic!("tool {name} not found in tool_definitions()"));
            let props = tool
                .pointer("/inputSchema/properties")
                .unwrap_or_else(|| panic!("tool {name} has no inputSchema.properties"));
            assert!(
                props.get("id").is_some(),
                "tool {name} should still declare bare 'id'"
            );
            assert!(
                props.get("environment_id").is_none(),
                "tool {name} should not declare 'environment_id'"
            );
        }
    }

    /// Test 4 (plan §1): pure resolver behaviour, no network — `fetch_projects()` is
    /// only reached on the name-pair path, which none of these cases hit.
    #[test]
    fn resolve_environment_id_prefers_canonical_key() {
        let resolved = resolve_environment_id(&serde_json::json!({ "environment_id": 7 }), "")
            .expect("environment_id alone should resolve");
        assert_eq!(resolved, ResolvedEnvironment { id: 7, via_deprecated_id: false });

        let resolved = resolve_environment_id(&serde_json::json!({ "id": 7 }), "")
            .expect("deprecated id alias should resolve");
        assert_eq!(resolved, ResolvedEnvironment { id: 7, via_deprecated_id: true });

        let resolved = resolve_environment_id(
            &serde_json::json!({ "environment_id": 7, "id": 9 }),
            "",
        )
        .expect("environment_id should win when both keys are present");
        assert_eq!(resolved, ResolvedEnvironment { id: 7, via_deprecated_id: false });
    }

    // ─── Endpoint resolution mirror (cli-remote-endpoint-config task 5.1) ──

    #[test]
    fn mcp_resolve_api_base_defaults_when_unset_or_empty() {
        assert_eq!(resolve_api_base_from(None).unwrap(), DEFAULT_API_BASE);
        assert_eq!(resolve_api_base_from(Some("  ")).unwrap(), DEFAULT_API_BASE);
    }

    #[test]
    fn mcp_resolve_api_base_honors_override_and_trims_trailing_slash() {
        assert_eq!(
            resolve_api_base_from(Some("https://127.0.0.1:47821/")).unwrap(),
            "https://127.0.0.1:47821"
        );
    }

    #[test]
    fn mcp_resolve_api_base_rejects_malformed_values() {
        for raw in ["not-a-url", "ftp://127.0.0.1", "127.0.0.1:47821"] {
            assert!(resolve_api_base_from(Some(raw)).is_err(), "{raw} should be rejected");
        }
    }

    #[test]
    fn mcp_host_is_loopback_matches_cli_semantics() {
        assert!(host_is_loopback("127.0.0.1"));
        assert!(host_is_loopback("localhost"));
        assert!(host_is_loopback("::1"));
        assert!(!host_is_loopback("vault.example.com"));
        assert!(!host_is_loopback("0.0.0.0"));
    }

    #[test]
    fn mcp_env_var_smoke_sets_process_endpoint() {
        // `CRYPTENV_API_URL` set → the resolver returns it verbatim (minus a
        // trailing slash); this is the value `api_base()` would cache in `main`.
        let resolved = resolve_api_base_from(Some("https://127.0.0.1:59999")).unwrap();
        assert_eq!(resolved, "https://127.0.0.1:59999");
    }

    /// Test 5 (plan §1): missing scope names the canonical key in the error and
    /// drops the old "'id' (environment id)" wording.
    #[test]
    fn resolve_environment_id_missing_scope_names_the_canonical_key() {
        let err = resolve_environment_id(&serde_json::json!({}), "")
            .expect_err("empty args must fail to resolve");
        let msg = err
            .pointer("/content/0/text")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        assert!(
            msg.contains("environment_id"),
            "error should mention 'environment_id'; got: {msg}"
        );
        assert!(
            !msg.contains("'id' (environment id)"),
            "error should not use the old wording; got: {msg}"
        );
    }
}
