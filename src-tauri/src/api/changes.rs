//! `vault_changed` signal for the desktop GUI.
//!
//! The REST API is the single door CLI, TUI and MCP use to write vault data.
//! After an authenticated request on a data-changing route succeeds, the
//! installed notifier fires so the open window can re-read its state. The
//! signal is empty by design: no ids, names or values travel with it — the
//! GUI refetches through its own authenticated commands.

use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use std::sync::Arc;

use super::ApiState;

/// Callback the desktop shell installs to emit the `vault_changed` event.
pub(crate) type ChangeNotifier = Arc<dyn Fn() + Send + Sync>;

/// Routes whose success changes data the GUI displays (items, categories,
/// projects, environments, imports). Reads, reveals, file injection and
/// approvals are excluded: they leave the vault's contents as they were.
pub(super) const CHANGING_ROUTES: &[(&str, &str)] = &[
    ("POST", "/items"),
    ("PUT", "/items/:id"),
    ("DELETE", "/items/:id"),
    ("POST", "/categories"),
    ("PUT", "/categories/:id"),
    ("DELETE", "/categories/:id"),
    ("POST", "/projects"),
    ("DELETE", "/projects/:id"),
    ("POST", "/environments"),
    ("DELETE", "/environments/:id"),
    ("POST", "/share/import"),
    ("POST", "/relay/receive"),
    ("POST", "/projects/relay/receive"),
];

/// True when a successful `method path` request changes vault data.
pub(super) fn changes_vault_data(method: &str, path: &str) -> bool {
    CHANGING_ROUTES.iter().any(|(m, p)| *m == method && *p == path)
}

impl ApiState {
    /// Installs the `vault_changed` callback (the desktop shell emits the event).
    pub(crate) fn set_change_notifier<F>(&self, notifier: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        let _ = self.change_notifier.set(Arc::new(notifier));
    }

    fn notify_changed(&self) {
        if let Some(notify) = self.change_notifier.get() {
            notify();
        }
    }
}

/// Route layer: signals only after the handler (which authenticates and
/// writes) answered 200/201/204. A 202 means "waiting for approval" — nothing
/// was written yet — and every error status means nothing changed.
pub(super) async fn notify_on_success(
    State(state): State<Arc<ApiState>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let response = next.run(request).await;
    if matches!(response.status(), StatusCode::OK | StatusCode::CREATED | StatusCode::NO_CONTENT) {
        state.notify_changed();
    }
    response
}
