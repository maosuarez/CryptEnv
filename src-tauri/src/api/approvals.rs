//! Human approval for sensitive MCP operations.
//!
//! The MCP principal cannot perform exfiltration-capable operations directly
//! (`relay_send`, `share_export`, MCP host config writes, `generate_env`).
//! Instead the API stores a pending approval with a non-secret summary and
//! tells the desktop app. Only the user, through the Tauri commands
//! `approval_list` / `approval_resolve` (reachable only from the GUI webview,
//! never over REST), can approve it. On approval the backend performs the
//! operation with the unlocked vault's own rights; the relay code, passphrase
//! and export passphrase go to the GUI and **never** into any REST response.
//!
//! Approvals live in memory only. They expire after [`APPROVAL_TTL`], at most
//! [`MAX_PENDING`] are pending at once, and each is bound to the vault lock
//! epoch: a lock/unlock cycle discards them.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::auth::AuthedPrincipal;
use super::exec_routes::GenerateEnvRequest;
use super::mcp_servers::{self, McpServerOp};
use super::{err_json, ApiState};

/// How long a request waits for the user.
pub(crate) const APPROVAL_TTL: Duration = Duration::from_secs(120);
/// Maximum simultaneously pending approvals.
pub(crate) const MAX_PENDING: usize = 8;
/// Finished approvals stay pollable this long, bounded by [`MAX_KEPT`].
const RESOLVED_RETENTION: Duration = Duration::from_secs(10 * 60);
const MAX_KEPT: usize = 64;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApprovalKind {
    RelaySend,
    ShareExport,
    McpServerAdd,
    McpServerUpdate,
    McpServerDelete,
    GenerateEnv,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}

/// What the user sees. Contains no secret value.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct ApprovalSummary {
    pub operation: String,
    pub kind: ApprovalKind,
    /// Item (or server) names affected.
    pub items: Vec<String>,
    pub item_count: usize,
    /// Extra non-secret lines, e.g. the command line an MCP server entry would run.
    pub details: Vec<String>,
    pub destination: Option<String>,
    pub requested_by: &'static str,
}

/// The request to execute on approval. Holds item ids and destinations, never
/// derived secrets.
#[derive(Clone, Debug)]
pub(crate) enum ApprovalPayload {
    RelaySend { item_ids: Vec<i64> },
    ShareExport { item_ids: Vec<i64>, output_path: String },
    McpServer(McpServerOp),
    GenerateEnv(GenerateEnvRequest),
}

struct Approval {
    id: String,
    summary: ApprovalSummary,
    payload: Option<ApprovalPayload>,
    created: Instant,
    epoch: u64,
    status: ApprovalStatus,
    /// Non-secret outcome, filled after execution.
    meta: Value,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ApprovalError {
    /// 8 approvals are already pending.
    Full,
    NotFound,
    /// Already approved, denied or expired.
    NotPending,
}

/// A serialisable snapshot of one approval (no payload).
#[derive(Clone, Debug, Serialize)]
pub(crate) struct ApprovalView {
    pub id: String,
    pub kind: ApprovalKind,
    pub status: ApprovalStatus,
    pub summary: ApprovalSummary,
    /// Seconds left to decide; 0 once resolved.
    #[serde(rename = "expiresIn")]
    pub expires_in: u64,
    pub meta: Value,
}

#[derive(Default)]
pub(crate) struct ApprovalStore {
    entries: Vec<Approval>,
}

fn random_id() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl ApprovalStore {
    /// Drops approvals from an older lock epoch, expires overdue pending ones
    /// and trims old finished ones.
    pub(crate) fn sweep(&mut self, now: Instant, epoch: u64) {
        self.entries.retain(|e| e.epoch == epoch);
        for e in self.entries.iter_mut() {
            if e.status == ApprovalStatus::Pending && now.duration_since(e.created) >= APPROVAL_TTL {
                e.status = ApprovalStatus::Expired;
                e.payload = None;
            }
        }
        self.entries.retain(|e| {
            e.status == ApprovalStatus::Pending || now.duration_since(e.created) < RESOLVED_RETENTION
        });
        while self.entries.len() > MAX_KEPT {
            if let Some(i) = self.entries.iter().position(|e| e.status != ApprovalStatus::Pending) {
                self.entries.remove(i);
            } else {
                break;
            }
        }
    }

    pub(crate) fn create(
        &mut self,
        summary: ApprovalSummary,
        payload: ApprovalPayload,
        now: Instant,
        epoch: u64,
    ) -> Result<String, ApprovalError> {
        self.sweep(now, epoch);
        let pending = self.entries.iter().filter(|e| e.status == ApprovalStatus::Pending).count();
        if pending >= MAX_PENDING {
            return Err(ApprovalError::Full);
        }
        let id = random_id();
        self.entries.push(Approval {
            id: id.clone(),
            summary,
            payload: Some(payload),
            created: now,
            epoch,
            status: ApprovalStatus::Pending,
            meta: json!({}),
        });
        Ok(id)
    }

    fn view_of(e: &Approval, now: Instant) -> ApprovalView {
        let left = APPROVAL_TTL.saturating_sub(now.duration_since(e.created)).as_secs();
        ApprovalView {
            id: e.id.clone(),
            kind: e.summary.kind,
            status: e.status,
            summary: e.summary.clone(),
            expires_in: if e.status == ApprovalStatus::Pending { left } else { 0 },
            meta: e.meta.clone(),
        }
    }

    pub(crate) fn view(&self, id: &str, now: Instant) -> Option<ApprovalView> {
        self.entries.iter().find(|e| e.id == id).map(|e| Self::view_of(e, now))
    }

    pub(crate) fn pending(&self, now: Instant) -> Vec<ApprovalView> {
        self.entries
            .iter()
            .filter(|e| e.status == ApprovalStatus::Pending)
            .map(|e| Self::view_of(e, now))
            .collect()
    }

    /// Decides a pending approval. Approving hands the payload to the caller
    /// (which executes it outside the store lock); denying returns `None`.
    pub(crate) fn decide(&mut self, id: &str, approve: bool) -> Result<Option<ApprovalPayload>, ApprovalError> {
        let e = self.entries.iter_mut().find(|e| e.id == id).ok_or(ApprovalError::NotFound)?;
        if e.status != ApprovalStatus::Pending {
            return Err(ApprovalError::NotPending);
        }
        if approve {
            e.status = ApprovalStatus::Approved;
            e.meta = json!({ "done": false });
            Ok(e.payload.take())
        } else {
            e.status = ApprovalStatus::Denied;
            e.payload = None;
            Ok(None)
        }
    }

    pub(crate) fn finish(&mut self, id: &str, meta: Value) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.meta = meta;
        }
    }
}

/// Secret outcome of an approved operation. Goes to the GUI only.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ApprovalSecret {
    Relay { code: String, passphrase: String },
    Export { path: String, passphrase: String },
}

/// Returned to the GUI by `approval_resolve`.
#[derive(Serialize)]
pub(crate) struct ApprovalResolution {
    pub id: String,
    pub status: ApprovalStatus,
    pub summary: ApprovalSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<ApprovalSecret>,
    /// Non-secret outcome (also what the MCP caller can poll).
    pub meta: Value,
}

/// Callback the desktop shell installs to learn about new approvals.
pub(crate) type Notifier = Arc<dyn Fn(&ApprovalView) + Send + Sync>;

struct Executed {
    secret: Option<ApprovalSecret>,
    meta: Value,
}

impl ApiState {
    /// Installs the new-approval callback (event emit + window focus).
    pub(crate) fn set_approval_notifier<F>(&self, notifier: F)
    where
        F: Fn(&ApprovalView) + Send + Sync + 'static,
    {
        let _ = self.notifier.set(Arc::new(notifier));
    }

    async fn current_epoch(&self) -> u64 {
        self.vault.lock().await.epoch
    }

    /// Stores a pending approval and notifies the GUI.
    pub(crate) async fn request_approval(
        &self,
        summary: ApprovalSummary,
        payload: ApprovalPayload,
    ) -> Result<ApprovalView, ApprovalError> {
        let epoch = self.current_epoch().await;
        let now = Instant::now();
        let view = {
            let mut store = self.approvals.lock().await;
            let id = store.create(summary, payload, now, epoch)?;
            store.view(&id, now).ok_or(ApprovalError::NotFound)?
        };
        if let Some(notify) = self.notifier.get() {
            notify(&view);
        }
        Ok(view)
    }

    /// Non-secret snapshot for polling (REST) or display (GUI).
    pub(crate) async fn approval_view(&self, id: &str) -> Option<ApprovalView> {
        let epoch = self.current_epoch().await;
        let now = Instant::now();
        let mut store = self.approvals.lock().await;
        store.sweep(now, epoch);
        store.view(id, now)
    }

    pub(crate) async fn pending_approvals(&self) -> Vec<ApprovalView> {
        let epoch = self.current_epoch().await;
        let now = Instant::now();
        let mut store = self.approvals.lock().await;
        store.sweep(now, epoch);
        store.pending(now)
    }

    /// Approves or denies. On approval the stored payload runs here, with the
    /// session rights of the unlocked vault.
    pub(crate) async fn resolve_approval(&self, id: &str, approve: bool) -> Result<ApprovalResolution, ApprovalError> {
        let epoch = self.current_epoch().await;
        let now = Instant::now();
        let (summary, payload) = {
            let mut store = self.approvals.lock().await;
            store.sweep(now, epoch);
            let summary = store.view(id, now).ok_or(ApprovalError::NotFound)?.summary;
            let payload = store.decide(id, approve)?;
            (summary, payload)
        };

        let Some(payload) = payload else {
            return Ok(ApprovalResolution {
                id: id.to_string(),
                status: ApprovalStatus::Denied,
                summary,
                secret: None,
                meta: json!({ "done": true }),
            });
        };

        let (secret, meta) = match self.execute(&payload).await {
            Ok(done) => (done.secret, done.meta),
            // The message is non-secret by construction (see `execute`).
            Err(message) => (None, json!({ "done": true, "ok": false, "error": message })),
        };
        self.approvals.lock().await.finish(id, meta.clone());
        Ok(ApprovalResolution {
            id: id.to_string(),
            status: ApprovalStatus::Approved,
            summary,
            secret,
            meta,
        })
    }

    /// Runs an approved payload. The `Err` text never contains a passphrase,
    /// relay code or secret value.
    async fn execute(&self, payload: &ApprovalPayload) -> Result<Executed, String> {
        match payload {
            ApprovalPayload::RelaySend { item_ids } => {
                let (code, passphrase) = self.relay_send_core(item_ids).await.map_err(|e| e.message)?;
                Ok(Executed {
                    secret: Some(ApprovalSecret::Relay { code, passphrase }),
                    meta: json!({ "done": true, "ok": true, "item_count": item_ids.len() }),
                })
            }
            ApprovalPayload::ShareExport { item_ids, output_path } => {
                let passphrase = self.share_export_core(item_ids, output_path).await.map_err(|e| e.message)?;
                Ok(Executed {
                    secret: Some(ApprovalSecret::Export { path: output_path.clone(), passphrase }),
                    meta: json!({ "done": true, "ok": true, "item_count": item_ids.len(), "path": output_path }),
                })
            }
            ApprovalPayload::McpServer(op) => {
                let result = mcp_servers::apply(op)?;
                Ok(Executed { secret: None, meta: json!({ "done": true, "ok": true, "result": result }) })
            }
            ApprovalPayload::GenerateEnv(req) => {
                let generated = self.generate_env_core(req).await.map_err(|e| e.message)?;
                Ok(Executed {
                    secret: None,
                    meta: json!({
                        "done": true,
                        "ok": true,
                        "path": generated.path,
                        "count": generated.count,
                        "expires_in_seconds": generated.expires_in_seconds,
                    }),
                })
            }
        }
    }

    /// Housekeeping tick: expires approvals and deletes expired generated files.
    pub(crate) async fn maintenance(&self) {
        let (epoch, generated) = {
            let vault = self.vault.lock().await;
            (vault.epoch, vault.generated.clone())
        };
        self.approvals.lock().await.sweep(Instant::now(), epoch);
        generated.sweep_expired();
    }
}

/// 202 response for an MCP request that needs the user's approval.
pub(super) async fn respond_pending(
    state: &ApiState,
    summary: ApprovalSummary,
    payload: ApprovalPayload,
) -> Response {
    match state.request_approval(summary, payload).await {
        Ok(view) => (
            StatusCode::ACCEPTED,
            Json(json!({
                "approvalId": view.id,
                "status": "pending",
                "expiresIn": view.expires_in,
            })),
        )
            .into_response(),
        Err(ApprovalError::Full) => err_json(
            StatusCode::TOO_MANY_REQUESTS,
            "too many pending approvals; ask the user to resolve them first",
            "APPROVALS_FULL",
        )
        .into_response(),
        Err(_) => err_json(StatusCode::INTERNAL_SERVER_ERROR, "internal error", "INTERNAL_ERROR").into_response(),
    }
}

/// `GET /approvals/:id`: status and non-secret metadata only.
pub(super) async fn handle_get_approval(
    State(state): State<Arc<ApiState>>,
    AuthedPrincipal(_principal): AuthedPrincipal,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.approval_view(&id).await {
        Some(v) => (
            StatusCode::OK,
            Json(json!({
                "id": v.id,
                "kind": v.kind,
                "status": v.status,
                "summary": v.summary,
                "expiresIn": v.expires_in,
                "meta": v.meta,
            })),
        )
            .into_response(),
        None => err_json(StatusCode::NOT_FOUND, "approval not found or expired", "NOT_FOUND").into_response(),
    }
}

// ─── Tauri commands (GUI only) ────────────────────────────────────────────────

fn command_error(e: ApprovalError) -> String {
    match e {
        ApprovalError::Full => "too many pending approvals".to_string(),
        ApprovalError::NotFound => "approval not found or expired".to_string(),
        ApprovalError::NotPending => "approval already resolved".to_string(),
    }
}

/// Pending approvals for the approval modal.
#[tauri::command]
pub async fn approval_list(state: tauri::State<'_, Arc<ApiState>>) -> Result<Vec<serde_json::Value>, String> {
    let views = state.pending_approvals().await;
    views.iter().map(|v| serde_json::to_value(v).map_err(|e| e.to_string())).collect()
}

/// Approves or denies a request. On approval, returns the relay code /
/// passphrase (when any) for the GUI to show.
#[tauri::command]
pub async fn approval_resolve(
    state: tauri::State<'_, Arc<ApiState>>,
    id: String,
    approve: bool,
) -> Result<serde_json::Value, String> {
    let resolution = state.resolve_approval(&id, approve).await.map_err(command_error)?;
    serde_json::to_value(&resolution).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> ApprovalSummary {
        ApprovalSummary {
            operation: "Send 1 item".to_string(),
            kind: ApprovalKind::RelaySend,
            items: vec!["DB_PASSWORD".to_string()],
            item_count: 1,
            details: vec![],
            destination: Some("relay".to_string()),
            requested_by: "mcp",
        }
    }

    fn payload() -> ApprovalPayload {
        ApprovalPayload::RelaySend { item_ids: vec![1] }
    }

    #[test]
    fn pending_approval_expires_after_120_seconds() {
        let mut store = ApprovalStore::default();
        let t0 = Instant::now();
        let id = store.create(summary(), payload(), t0, 1).unwrap();
        store.sweep(t0 + Duration::from_secs(119), 1);
        assert_eq!(store.view(&id, t0).unwrap().status, ApprovalStatus::Pending);
        store.sweep(t0 + Duration::from_secs(120), 1);
        assert_eq!(store.view(&id, t0).unwrap().status, ApprovalStatus::Expired);
        assert!(matches!(store.decide(&id, true), Err(ApprovalError::NotPending)), "expired cannot be approved");
    }

    #[test]
    fn at_most_eight_pending_then_full() {
        let mut store = ApprovalStore::default();
        let t0 = Instant::now();
        for _ in 0..MAX_PENDING {
            store.create(summary(), payload(), t0, 1).unwrap();
        }
        assert_eq!(store.create(summary(), payload(), t0, 1), Err(ApprovalError::Full));
        // Resolving one frees a slot.
        let first = store.pending(t0)[0].id.clone();
        store.decide(&first, false).unwrap();
        assert!(store.create(summary(), payload(), t0, 1).is_ok());
    }

    #[test]
    fn lock_epoch_change_discards_approvals() {
        let mut store = ApprovalStore::default();
        let t0 = Instant::now();
        let id = store.create(summary(), payload(), t0, 1).unwrap();
        store.sweep(t0, 2);
        assert!(store.view(&id, t0).is_none());
        assert!(matches!(store.decide(&id, true), Err(ApprovalError::NotFound)));
    }

    #[test]
    fn approve_hands_out_the_payload_once() {
        let mut store = ApprovalStore::default();
        let t0 = Instant::now();
        let id = store.create(summary(), payload(), t0, 1).unwrap();
        assert!(matches!(store.decide(&id, true), Ok(Some(ApprovalPayload::RelaySend { .. }))));
        assert!(matches!(store.decide(&id, true), Err(ApprovalError::NotPending)));
        assert_eq!(store.view(&id, t0).unwrap().status, ApprovalStatus::Approved);
    }

    #[test]
    fn deny_discards_the_payload() {
        let mut store = ApprovalStore::default();
        let t0 = Instant::now();
        let id = store.create(summary(), payload(), t0, 1).unwrap();
        assert!(matches!(store.decide(&id, false), Ok(None)));
        assert_eq!(store.view(&id, t0).unwrap().status, ApprovalStatus::Denied);
    }
}
