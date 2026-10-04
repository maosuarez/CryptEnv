//! MCP host configuration writes (`claude_desktop_config.json` and the
//! project-level `.claude/mcp_servers.json`).
//!
//! These writes used to happen inside the MCP binary, which let an agent plant
//! a persistent command in the host config with no human in the loop. They now
//! live in the backend so the approval gate cannot be bypassed: for the MCP
//! principal, `POST/PUT/DELETE /mcp-servers` only create a pending approval.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::approvals::{ApprovalKind, ApprovalPayload, ApprovalSummary};
use super::auth::{AuthedPrincipal, Principal};
use super::{err_json, err_validation, ApiState};

/// Fields shared by add / update / delete requests.
#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct McpServerRequest {
    pub name: String,
    pub command: Option<String>,
    pub args: Option<Vec<Value>>,
    /// Only the key names are honoured: values are always written as empty
    /// placeholders, never real secrets.
    pub env: Option<serde_json::Map<String, Value>>,
    /// `global` (Claude desktop config) or `project` (`<cwd>/.claude/mcp_servers.json`).
    pub scope: Option<String>,
    /// Required for `project` scope: the caller's working directory (absolute).
    pub cwd: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) enum McpServerOp {
    Add(McpServerRequest),
    Update(McpServerRequest),
    Delete(McpServerRequest),
}

impl McpServerOp {
    fn request(&self) -> &McpServerRequest {
        match self {
            McpServerOp::Add(r) | McpServerOp::Update(r) | McpServerOp::Delete(r) => r,
        }
    }

    fn verb(&self) -> &'static str {
        match self {
            McpServerOp::Add(_) => "Add",
            McpServerOp::Update(_) => "Update",
            McpServerOp::Delete(_) => "Delete",
        }
    }

    pub(crate) fn kind(&self) -> ApprovalKind {
        match self {
            McpServerOp::Add(_) => ApprovalKind::McpServerAdd,
            McpServerOp::Update(_) => ApprovalKind::McpServerUpdate,
            McpServerOp::Delete(_) => ApprovalKind::McpServerDelete,
        }
    }
}

fn claude_desktop_config_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA")
            .map(|d| PathBuf::from(d).join("Claude").join("claude_desktop_config.json"))
            .map_err(|_| "APPDATA environment variable not set".to_string())
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .map(|d| {
                PathBuf::from(d)
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
            .map(|d| PathBuf::from(d).join(".config").join("Claude").join("claude_desktop_config.json"))
            .map_err(|_| "HOME environment variable not set".to_string())
    }
}

/// Resolves the config file a request targets.
fn config_path(req: &McpServerRequest) -> Result<PathBuf, String> {
    match req.scope.as_deref().unwrap_or("global") {
        "project" => {
            let cwd = req.cwd.as_deref().ok_or("scope 'project' requires 'cwd'")?;
            let cwd = Path::new(cwd);
            if !cwd.is_absolute() {
                return Err("'cwd' must be an absolute path".to_string());
            }
            Ok(cwd.join(".claude").join("mcp_servers.json"))
        }
        "global" => claude_desktop_config_path(),
        _ => Err("scope must be 'global' or 'project'".to_string()),
    }
}

fn validate(op: &McpServerOp) -> Result<(), String> {
    let req = op.request();
    if req.name.trim().is_empty() || req.name.len() > 128 || req.name.chars().any(|c| c.is_control()) {
        return Err("name: required, at most 128 characters, no control characters".to_string());
    }
    if let McpServerOp::Add(_) = op {
        match req.command.as_deref() {
            Some(c) if !c.trim().is_empty() && c.len() <= 1024 && !c.chars().any(|c| c.is_control()) => {}
            _ => return Err("command: required for add, at most 1024 characters".to_string()),
        }
    }
    if let Some(c) = req.command.as_deref() {
        if c.len() > 1024 || c.chars().any(|c| c.is_control()) {
            return Err("command: at most 1024 characters, no control characters".to_string());
        }
    }
    config_path(req).map(|_| ())
}

/// Non-secret description of the operation for the approval modal. The full
/// command line is shown because it is exactly what would run on the host.
pub(crate) fn summarize(op: &McpServerOp) -> ApprovalSummary {
    let req = op.request();
    let scope = req.scope.as_deref().unwrap_or("global");
    let mut details = Vec::new();
    if let Some(cmd) = &req.command {
        let args: Vec<String> = req
            .args
            .iter()
            .flatten()
            .map(|a| a.as_str().map(|s| s.to_string()).unwrap_or_else(|| a.to_string()))
            .collect();
        details.push(format!("command: {} {}", cmd, args.join(" ")).trim_end().to_string());
    }
    if let Some(env) = &req.env {
        if !env.is_empty() {
            details.push(format!("env keys: {}", env.keys().cloned().collect::<Vec<_>>().join(", ")));
        }
    }
    ApprovalSummary {
        operation: format!("{} MCP server '{}' ({scope} config)", op.verb(), req.name),
        kind: op.kind(),
        items: vec![req.name.clone()],
        item_count: 1,
        details,
        destination: config_path(req).ok().map(|p| p.to_string_lossy().into_owned()),
        requested_by: "mcp",
    }
}

fn read_json_config(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("error reading {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("error parsing {}: {e}", path.display()))
}

/// Writes atomically: temp file, then rename.
fn write_json_config_atomic(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create directory {}: {e}", parent.display()))?;
    }
    let tmp_path = path.with_extension("tmp");
    let text = serde_json::to_string_pretty(value).map_err(|e| format!("error serializing config: {e}"))?;
    std::fs::write(&tmp_path, &text).map_err(|e| format!("error writing temp file {}: {e}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path).map_err(|e| format!("error renaming temp file to {}: {e}", path.display()))
}

fn blank_env(env: &serde_json::Map<String, Value>) -> serde_json::Map<String, Value> {
    env.keys().map(|k| (k.clone(), Value::String(String::new()))).collect()
}

/// Applies `op` to the config file at `path`. Pure file logic, no secrets.
pub(crate) fn apply_to_path(path: &Path, op: &McpServerOp) -> Result<Value, String> {
    let req = op.request();
    let mut config = read_json_config(path)?;
    let name = req.name.clone();

    match op {
        McpServerOp::Delete(_) => {
            let servers = config
                .get_mut("mcpServers")
                .and_then(|v| v.as_object_mut())
                .ok_or_else(|| format!("server '{name}' not found (mcpServers is empty or missing)"))?;
            if servers.remove(&name).is_none() {
                return Err(format!("server '{name}' not found"));
            }
        }
        McpServerOp::Add(_) | McpServerOp::Update(_) => {
            if config.get("mcpServers").is_none() {
                config["mcpServers"] = json!({});
            }
            let servers = config
                .get_mut("mcpServers")
                .and_then(|v| v.as_object_mut())
                .ok_or("config has invalid 'mcpServers' structure")?;
            if let McpServerOp::Add(_) = op {
                if servers.contains_key(&name) {
                    return Err(format!("server '{name}' already exists; use update to modify it"));
                }
                let command = req.command.clone().unwrap_or_default();
                let mut entry = json!({ "command": command, "args": req.args.clone().unwrap_or_default() });
                if let Some(env) = req.env.as_ref().filter(|e| !e.is_empty()) {
                    entry["env"] = Value::Object(blank_env(env));
                }
                servers.insert(name.clone(), entry);
            } else {
                let entry = servers
                    .get_mut(&name)
                    .ok_or_else(|| format!("server '{name}' not found; use add to create it"))?;
                if let Some(cmd) = &req.command {
                    entry["command"] = json!(cmd);
                }
                if let Some(args) = &req.args {
                    entry["args"] = json!(args);
                }
                if let Some(env) = &req.env {
                    // Merge: existing keys preserved, new keys added as empty placeholders.
                    let mut merged = entry.get("env").and_then(|v| v.as_object()).cloned().unwrap_or_default();
                    for k in env.keys() {
                        merged.entry(k.clone()).or_insert_with(|| Value::String(String::new()));
                    }
                    entry["env"] = Value::Object(merged);
                }
            }
        }
    }

    write_json_config_atomic(path, &config)?;
    let key = match op {
        McpServerOp::Add(_) => "added",
        McpServerOp::Update(_) => "updated",
        McpServerOp::Delete(_) => "deleted",
    };
    Ok(json!({
        key: true,
        "name": name,
        "scope": req.scope.as_deref().unwrap_or("global"),
        "config_path": path.to_string_lossy(),
    }))
}

/// Resolves the path and applies `op`. Used by the direct (session) route and
/// by an approved request.
pub(crate) fn apply(op: &McpServerOp) -> Result<Value, String> {
    validate(op)?;
    let path = config_path(op.request())?;
    apply_to_path(&path, op)
}

/// Session callers act immediately; the MCP principal gets a pending approval.
async fn dispatch(state: &Arc<ApiState>, principal: Principal, op: McpServerOp) -> axum::response::Response {
    if let Err(msg) = validate(&op) {
        return err_validation("mcp-server", &msg);
    }
    match principal {
        Principal::Mcp => {
            let summary = summarize(&op);
            super::approvals::respond_pending(state, summary, ApprovalPayload::McpServer(op)).await
        }
        Principal::Session => match apply(&op) {
            Ok(v) => (StatusCode::OK, Json(v)).into_response(),
            Err(e) => err_json(StatusCode::BAD_REQUEST, &e, "BAD_REQUEST").into_response(),
        },
    }
}

pub(super) async fn handle_add_mcp_server(
    State(state): State<Arc<ApiState>>,
    AuthedPrincipal(principal): AuthedPrincipal,
    Json(body): Json<McpServerRequest>,
) -> impl IntoResponse {
    dispatch(&state, principal, McpServerOp::Add(body)).await
}

pub(super) async fn handle_update_mcp_server(
    State(state): State<Arc<ApiState>>,
    AuthedPrincipal(principal): AuthedPrincipal,
    Json(body): Json<McpServerRequest>,
) -> impl IntoResponse {
    dispatch(&state, principal, McpServerOp::Update(body)).await
}

pub(super) async fn handle_delete_mcp_server(
    State(state): State<Arc<ApiState>>,
    AuthedPrincipal(principal): AuthedPrincipal,
    Query(query): Query<McpServerRequest>,
) -> impl IntoResponse {
    dispatch(&state, principal, McpServerOp::Delete(query)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(name: &str, dir: &Path) -> McpServerRequest {
        McpServerRequest {
            name: name.to_string(),
            command: Some("node".to_string()),
            args: Some(vec![json!("server.js")]),
            env: Some(serde_json::Map::from_iter([("API_KEY".to_string(), json!("real-secret"))])),
            scope: Some("project".to_string()),
            cwd: Some(dir.to_string_lossy().into_owned()),
        }
    }

    #[test]
    fn add_update_delete_roundtrip_never_stores_env_values() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".claude").join("mcp_servers.json");

        apply_to_path(&path, &McpServerOp::Add(req("s", tmp.path()))).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("real-secret"), "env value must be a placeholder: {text}");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["mcpServers"]["s"]["command"], "node");
        assert_eq!(v["mcpServers"]["s"]["env"]["API_KEY"], "");

        assert!(apply_to_path(&path, &McpServerOp::Add(req("s", tmp.path()))).is_err(), "duplicate add");

        let mut upd = req("s", tmp.path());
        upd.command = Some("deno".to_string());
        apply_to_path(&path, &McpServerOp::Update(upd)).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["s"]["command"], "deno");

        apply_to_path(&path, &McpServerOp::Delete(req("s", tmp.path()))).unwrap();
        assert!(apply_to_path(&path, &McpServerOp::Delete(req("s", tmp.path()))).is_err());
    }

    #[test]
    fn project_scope_needs_an_absolute_cwd() {
        let mut r = req("s", Path::new("/tmp"));
        r.cwd = None;
        assert!(validate(&McpServerOp::Add(r.clone())).is_err());
        r.cwd = Some("relative".to_string());
        assert!(validate(&McpServerOp::Add(r)).is_err());
    }

    #[test]
    fn summary_shows_the_command_line_and_env_key_names_only() {
        let tmp = tempfile::tempdir().unwrap();
        let s = summarize(&McpServerOp::Add(req("s", tmp.path())));
        let text = serde_json::to_string(&s).unwrap();
        assert!(text.contains("node server.js"));
        assert!(text.contains("API_KEY"));
        assert!(!text.contains("real-secret"));
    }
}
