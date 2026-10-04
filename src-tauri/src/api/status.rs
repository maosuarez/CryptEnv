//! Visible status of the local REST server.
//!
//! `start_server` publishes its state here so the GUI can tell the user when the
//! API (and with it the CLI, TUI and MCP) is unavailable and why, and offer to
//! regenerate the TLS material and restart the server. Reasons are fixed
//! strings: details (paths, OS error text) go only to the local log.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use axum_server::Handle;
use serde::Serialize;
use tauri::Manager;

use super::ApiState;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ApiStatus {
    Starting,
    Running,
    Stopped,
    /// `code`: `port_in_use`, `tls_error`, `bind_error` or `server_error`.
    Failed { code: &'static str, reason: String },
}

static API_STATUS: RwLock<ApiStatus> = RwLock::new(ApiStatus::Starting);
static SERVER_HANDLE: Mutex<Option<Handle>> = Mutex::new(None);

pub(super) fn set_status(status: ApiStatus) {
    let mut guard = API_STATUS.write().unwrap_or_else(|p| p.into_inner());
    *guard = status;
}

pub fn current_status() -> ApiStatus {
    API_STATUS.read().unwrap_or_else(|p| p.into_inner()).clone()
}

pub(super) fn set_handle(handle: Handle) {
    let mut guard = SERVER_HANDLE.lock().unwrap_or_else(|p| p.into_inner());
    *guard = Some(handle);
}

fn take_handle() -> Option<Handle> {
    SERVER_HANDLE.lock().unwrap_or_else(|p| p.into_inner()).take()
}

/// Waits (bounded) until `done(current_status())` holds.
async fn wait_for(done: impl Fn(&ApiStatus) -> bool) {
    for _ in 0..50 {
        if done(&current_status()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Stops the running server (if any), discards the TLS material so a fresh pair
/// is generated, and starts the server again.
async fn regenerate_and_restart(
    api_state: Arc<ApiState>,
    app_data_dir: PathBuf,
) -> Result<ApiStatus, String> {
    if let Some(handle) = take_handle() {
        handle.shutdown();
        wait_for(|s| !matches!(s, ApiStatus::Running)).await;
    }
    if let Err(e) = crate::tls::reset_tls_material(&app_data_dir) {
        eprintln!("[api] Could not reset TLS material: {e}");
        return Err("could not reset the TLS certificate".to_string());
    }
    set_status(ApiStatus::Starting);
    tauri::async_runtime::spawn(super::start_server(api_state, app_data_dir));
    wait_for(|s| !matches!(s, ApiStatus::Starting)).await;
    Ok(current_status())
}

/// Current state of the local REST server.
#[tauri::command]
pub fn api_status() -> ApiStatus {
    current_status()
}

/// Regenerates the TLS certificate and restarts the REST server. Clients that
/// copied the old certificate (e.g. WSL) must re-run `crypt-env setup wsl`.
#[tauri::command]
pub async fn tls_regenerate(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<ApiState>>,
) -> Result<ApiStatus, String> {
    let dir = app.path().app_data_dir().map_err(|e| {
        eprintln!("[api] Cannot resolve app data dir: {e}");
        "could not locate the application data directory".to_string()
    })?;
    regenerate_and_restart(state.inner().clone(), dir).await
}
