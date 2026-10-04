//! `POST /exec`, `DELETE /exec/:runId` and `POST /generate-env`.
//!
//! The MCP principal never receives secret values. These routes resolve the
//! values inside the backend, hand them to the child process environment (or a
//! private 0600 file) and return only redacted, bounded output / a path.
//! See `openspec/changes/mcp-command-execution-hardening`.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use zeroize::Zeroizing;

use super::approvals::{respond_pending, ApprovalKind, ApprovalPayload, ApprovalSummary};
use super::auth::{AuthedPrincipal, Principal};
use super::{
    decrypt_all_items, environment_item_ids, err_json, err_validation, scope_items, ApiState, CoreError,
    EnvScopeQuery, IncludeGlobal,
};
use crate::exec::tempfiles::GENERATED_TTL;
use crate::exec::wsl::WslTarget;
use crate::exec::{self, redact::Redactor, ExecSpec};
use crate::project;
use crate::vault::VaultItem;

/// Commands running at once.
pub(super) const MAX_CONCURRENT_RUNS: usize = 4;
/// Requests allowed to wait for a free slot; more get 429.
pub(super) const MAX_QUEUED_RUNS: usize = 8;

// ─── POST /exec ───────────────────────────────────────────────────────────────

/// A key to inject, optionally scoped differently from the command itself.
#[derive(Deserialize)]
#[serde(untagged)]
enum InjectKey {
    Name(String),
    Scoped {
        key: String,
        environment_id: Option<i64>,
        project: Option<String>,
        environment: Option<String>,
    },
}

/// An injection key together with the scope it was selected in.
struct WantedKey {
    key: String,
    environment_id: Option<i64>,
    project: Option<String>,
    environment: Option<String>,
}

#[derive(Deserialize, Default)]
struct ClientContext {
    os: Option<String>,
    #[serde(rename = "wslDistro")]
    wsl_distro: Option<String>,
    cwd: Option<String>,
    home: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct ExecBody {
    #[serde(rename = "commandId")]
    command_id: i64,
    #[serde(default)]
    params: BTreeMap<String, String>,
    #[serde(default, rename = "injectKeys")]
    inject_keys: Vec<InjectKey>,
    cwd: Option<String>,
    #[serde(default, rename = "clientContext")]
    client_context: ClientContext,
    /// Caller-chosen id so the run can be cancelled with `DELETE /exec/:runId`.
    #[serde(rename = "runId")]
    run_id: Option<String>,
}

#[derive(Serialize)]
struct ExecResponse {
    #[serde(rename = "runId")]
    run_id: String,
    #[serde(rename = "exitCode")]
    exit_code: Option<i32>,
    #[serde(rename = "timedOut")]
    timed_out: bool,
    cancelled: bool,
    stdout: String,
    stderr: String,
    #[serde(rename = "stdoutTruncated")]
    stdout_truncated: bool,
    #[serde(rename = "stderrTruncated")]
    stderr_truncated: bool,
}

fn valid_run_id(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Decrements the queue counter even when the request future is dropped.
struct WaitGuard<'a>(&'a AtomicUsize);
impl Drop for WaitGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Cancels the run and forgets it when the request ends for any reason,
/// including the HTTP client (the MCP process) disconnecting.
struct RunGuard {
    state: Arc<ApiState>,
    id: String,
    cancel: Arc<AtomicBool>,
}
impl Drop for RunGuard {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Ok(mut runs) = self.state.runs.lock() {
            runs.remove(&self.id);
        }
    }
}

fn is_blank(s: &Option<String>) -> bool {
    s.as_deref().map(|v| v.trim().is_empty()).unwrap_or(true)
}

fn secret_value(item: &VaultItem) -> String {
    item.value
        .as_deref()
        .or(item.password.as_deref())
        .or(item.content.as_deref())
        .unwrap_or("")
        .to_string()
}

pub(super) async fn handle_exec(
    State(state): State<Arc<ApiState>>,
    AuthedPrincipal(_principal): AuthedPrincipal,
    Query(scope): Query<EnvScopeQuery>,
    Json(body): Json<ExecBody>,
) -> Response {
    // Reject malformed injection requests before any value is decrypted.
    let mut wanted: Vec<WantedKey> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for k in &body.inject_keys {
        let (key, id, project, environment) = match k {
            InjectKey::Name(n) => (n.clone(), None, None, None),
            InjectKey::Scoped { key, environment_id, project, environment } => {
                (key.clone(), *environment_id, project.clone(), environment.clone())
            }
        };
        if !exec::is_safe_env_key(&key) {
            return err_validation("injectKeys", &exec::ExecError::InvalidKey(key).to_string());
        }
        if seen.insert(key.clone()) {
            wanted.push(WantedKey { key, environment_id: id, project, environment });
        }
    }
    if let Some(id) = &body.run_id {
        if !valid_run_id(id) {
            return err_validation("runId", "must be 8-64 characters of [A-Za-z0-9_-]");
        }
    }

    // Queue: MAX_CONCURRENT_RUNS at once, MAX_QUEUED_RUNS waiting, then 429.
    let permit = match state.exec_slots.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            if state.exec_waiting.fetch_add(1, Ordering::SeqCst) >= MAX_QUEUED_RUNS {
                state.exec_waiting.fetch_sub(1, Ordering::SeqCst);
                return err_json(
                    StatusCode::TOO_MANY_REQUESTS,
                    "too many commands running; retry shortly",
                    "EXEC_BUSY",
                )
                .into_response();
            }
            let _waiting = WaitGuard(&state.exec_waiting);
            match state.exec_slots.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => {
                    return err_json(StatusCode::INTERNAL_SERVER_ERROR, "internal error", "INTERNAL_ERROR")
                        .into_response()
                }
            }
        }
    };

    // Scope of the command itself.
    let env = match super::resolve_scope(
        &state,
        scope.environment_id,
        scope.project.as_deref(),
        scope.environment.as_deref(),
    )
    .await
    {
        Ok(e) => e,
        Err(resp) => return resp,
    };

    // Decrypted outside the vault lock (`decrypt_all_items` copies the key out).
    let items = match decrypt_all_items(&state).await {
        Ok(i) => i,
        Err(StatusCode::FORBIDDEN) => {
            return err_json(StatusCode::FORBIDDEN, "vault locked", "VAULT_LOCKED").into_response()
        }
        Err(_) => {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, "internal error", "INTERNAL_ERROR").into_response()
        }
    };
    let epoch = state.vault.lock().await.epoch;

    let command_item = scope_items(items.clone(), &environment_item_ids(&env), IncludeGlobal::With)
        .into_iter()
        .map(|s| s.item)
        .find(|i| i.item_type == "command" && i.id == body.command_id);
    let Some(template) = command_item.and_then(|i| i.command) else {
        return err_json(StatusCode::NOT_FOUND, "command not found in this scope", "NOT_FOUND").into_response();
    };

    // Validates every parameter; nothing has started yet.
    let resolved = match exec::params::substitute(&template, &body.params) {
        Ok(r) => r,
        Err(e) => return err_validation("params", &e.to_string()),
    };

    // Resolve injected values. Each key is looked up by item name within its
    // scope (the command's scope unless the key carries its own).
    let mut injected: Vec<(String, Zeroizing<String>)> = Vec::new();
    for WantedKey { key, environment_id, project, environment } in wanted {
        let key_env = if environment_id.is_none() && is_blank(&project) && is_blank(&environment) {
            env.clone()
        } else {
            match super::resolve_scope(&state, environment_id, project.as_deref(), environment.as_deref()).await {
                Ok(e) => e,
                Err(resp) => return resp,
            }
        };
        let lower = key.to_lowercase();
        let found = scope_items(items.clone(), &environment_item_ids(&key_env), IncludeGlobal::With)
            .into_iter()
            .map(|s| s.item)
            .find(|i| i.name.as_deref().map(|n| n.to_lowercase() == lower).unwrap_or(false));
        match found {
            Some(item) => injected.push((key, Zeroizing::new(secret_value(&item)))),
            None => {
                return err_json(
                    StatusCode::NOT_FOUND,
                    &format!("secret '{key}' not found in this scope"),
                    "NOT_FOUND",
                )
                .into_response()
            }
        }
    }
    drop(items);

    // Working directory / WSL target.
    let ctx = &body.client_context;
    let wsl_target = match (ctx.os.as_deref(), ctx.wsl_distro.as_deref()) {
        (Some("linux"), Some(distro)) if cfg!(windows) => Some(WslTarget {
            distro: distro.to_string(),
            cwd: body.cwd.clone().or_else(|| ctx.cwd.clone()),
            home: ctx.home.clone(),
        }),
        _ => None,
    };
    let cwd: Option<PathBuf> = if wsl_target.is_some() {
        None
    } else {
        match body.cwd.as_ref().or(ctx.cwd.as_ref()) {
            Some(c) => {
                let p = PathBuf::from(c);
                if !p.is_absolute() || !p.is_dir() {
                    return err_validation("cwd", "must be an absolute path to an existing directory");
                }
                Some(p)
            }
            None => None,
        }
    };

    let redactor = Redactor::new(&injected);
    let spec = ExecSpec {
        command: resolved,
        cwd,
        env: injected,
        wsl: wsl_target,
        timeout: Duration::from_millis(state.exec_timeout_ms.load(Ordering::SeqCst)),
    };

    // Register the run so DELETE /exec/:runId (or a dropped connection) can kill it.
    let run_id = body.run_id.clone().unwrap_or_else(|| {
        use rand::RngCore;
        let mut b = [0u8; 8];
        rand::thread_rng().fill_bytes(&mut b);
        b.iter().map(|x| format!("{x:02x}")).collect()
    });
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let Ok(mut runs) = state.runs.lock() else {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, "internal error", "INTERNAL_ERROR").into_response();
        };
        if runs.contains_key(&run_id) {
            return err_json(StatusCode::CONFLICT, "runId already in use", "CONFLICT").into_response();
        }
        runs.insert(run_id.clone(), cancel.clone());
    }
    let _guard = RunGuard { state: state.clone(), id: run_id.clone(), cancel: cancel.clone() };

    let job_cancel = cancel.clone();
    let mut job = tokio::task::spawn_blocking(move || exec::run(&spec, &job_cancel, &redactor));

    // A lock/unlock (epoch change) while the command runs kills it.
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let joined = loop {
        tokio::select! {
            r = &mut job => break r,
            _ = tick.tick() => {
                if state.vault.lock().await.epoch != epoch {
                    cancel.store(true, Ordering::SeqCst);
                }
            }
        }
    };
    drop(permit);

    match joined {
        Ok(Ok(out)) => (
            StatusCode::OK,
            Json(ExecResponse {
                run_id,
                exit_code: out.exit_code,
                timed_out: out.timed_out,
                cancelled: out.cancelled,
                stdout: out.stdout,
                stderr: out.stderr,
                stdout_truncated: out.stdout_truncated,
                stderr_truncated: out.stderr_truncated,
            }),
        )
            .into_response(),
        // `ExecError` text never carries the command or a value.
        Ok(Err(e)) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string(), "EXEC_FAILED").into_response(),
        Err(_) => err_json(StatusCode::INTERNAL_SERVER_ERROR, "internal error", "INTERNAL_ERROR").into_response(),
    }
}

/// `DELETE /exec/:runId`: kills a running command's whole process tree.
pub(super) async fn handle_cancel_exec(
    State(state): State<Arc<ApiState>>,
    AuthedPrincipal(_principal): AuthedPrincipal,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    let flag = state.runs.lock().ok().and_then(|runs| runs.get(&run_id).cloned());
    match flag {
        Some(f) => {
            f.store(true, Ordering::SeqCst);
            (StatusCode::OK, Json(serde_json::json!({ "cancelled": true }))).into_response()
        }
        None => err_json(StatusCode::NOT_FOUND, "run not found", "NOT_FOUND").into_response(),
    }
}

// ─── POST /generate-env ───────────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
pub(crate) struct GenerateEnvRequest {
    pub keys: Vec<String>,
    pub environment_id: Option<i64>,
    pub project: Option<String>,
    pub environment: Option<String>,
}

pub(crate) struct GeneratedEnv {
    pub path: String,
    pub count: usize,
    pub expires_in_seconds: u64,
}

impl ApiState {
    /// Resolves the keys inside the backend and writes them to a private
    /// 0600 file. The path (never the content) is the only output.
    pub(crate) async fn generate_env_core(&self, req: &GenerateEnvRequest) -> Result<GeneratedEnv, CoreError> {
        let env = {
            let vault = self.vault.lock().await;
            project::resolve_environment(
                &vault.db,
                req.environment_id,
                req.project.as_deref(),
                req.environment.as_deref(),
            )
            .await
            .map_err(|e| CoreError::new(StatusCode::UNPROCESSABLE_ENTITY, "VALIDATION_ERROR", e))?
        };
        let items = decrypt_all_items(self)
            .await
            .map_err(|_| CoreError::new(StatusCode::FORBIDDEN, "VAULT_LOCKED", "vault locked".to_string()))?;
        let scoped: Vec<VaultItem> = scope_items(items, &environment_item_ids(&env), IncludeGlobal::With)
            .into_iter()
            .map(|s| s.item)
            .collect();

        let mut lines: Vec<Zeroizing<String>> = Vec::new();
        let mut count = 0usize;
        for key in &req.keys {
            if !exec::is_safe_env_key(key) {
                lines.push(Zeroizing::new(format!("# {key}: invalid or blocked name, skipped")));
                continue;
            }
            let lower = key.to_lowercase();
            let found = scoped
                .iter()
                .find(|i| i.name.as_deref().map(|n| n.to_lowercase() == lower).unwrap_or(false));
            match found {
                Some(item) => {
                    let value = Zeroizing::new(secret_value(item));
                    if value.contains(['\n', '\r', '\0']) {
                        // A line break would inject extra variables into the file.
                        lines.push(Zeroizing::new(format!("# {key}: value contains a line break, skipped")));
                    } else {
                        lines.push(Zeroizing::new(format!("{key}={}", value.as_str())));
                        count += 1;
                    }
                }
                None => lines.push(Zeroizing::new(format!("# {key}: not found in vault"))),
            }
        }
        let mut content = Zeroizing::new(String::new());
        for l in &lines {
            content.push_str(l);
            content.push('\n');
        }

        let generated = self.vault.lock().await.generated.clone();
        let path = generated
            .create(&content)
            .map_err(|e| CoreError::new(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", e))?;
        Ok(GeneratedEnv {
            path: path.to_string_lossy().into_owned(),
            count,
            expires_in_seconds: GENERATED_TTL.as_secs(),
        })
    }
}

pub(super) async fn handle_generate_env(
    State(state): State<Arc<ApiState>>,
    AuthedPrincipal(principal): AuthedPrincipal,
    Json(body): Json<GenerateEnvRequest>,
) -> Response {
    if body.keys.is_empty() || body.keys.len() > 128 {
        return err_validation("keys", "provide between 1 and 128 key names");
    }
    match principal {
        Principal::Session => match state.generate_env_core(&body).await {
            Ok(g) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "path": g.path,
                    "count": g.count,
                    "expiresInSeconds": g.expires_in_seconds,
                    "note": "This file contains secrets in plaintext. It is deleted after 10 minutes, when the vault locks, and at app exit.",
                })),
            )
                .into_response(),
            Err(e) => e.into_response(),
        },
        Principal::Mcp => {
            let summary = ApprovalSummary {
                operation: format!("Write a plaintext .env with {} secret(s) to a private temporary file", body.keys.len()),
                kind: ApprovalKind::GenerateEnv,
                items: body.keys.clone(),
                item_count: body.keys.len(),
                details: vec!["File is private (0600), deleted after 10 minutes, at vault lock and at app exit".to_string()],
                destination: Some("private temporary directory of crypt-env".to_string()),
                requested_by: "mcp",
            };
            respond_pending(&state, summary, ApprovalPayload::GenerateEnv(body)).await
        }
    }
}
