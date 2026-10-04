//! Startup recovery: when the vault database cannot be opened the app still
//! starts, into a recovery screen (move the damaged database aside, or quit)
//! instead of panicking at every launch.

use serde::Serialize;
use tauri::{Manager, State};

/// Managed by `lib.rs` setup. In `Recovery` mode no `VaultState` exists, so
/// every vault command answers "state not managed" and only the commands
/// below are usable.
pub enum AppMode {
    Normal,
    Recovery { error: String },
}

#[derive(Serialize)]
pub struct AppModeInfo {
    /// `"normal"` or `"recovery"`.
    pub mode: &'static str,
    /// Why the database could not be opened. Never contains key material.
    pub error: Option<String>,
}

/// Available in both modes; the frontend asks this before anything else.
#[tauri::command]
pub fn app_mode(mode: State<'_, AppMode>) -> AppModeInfo {
    match &*mode {
        AppMode::Normal => AppModeInfo { mode: "normal", error: None },
        AppMode::Recovery { error } => AppModeInfo { mode: "recovery", error: Some(error.clone()) },
    }
}

/// Renames the unopenable database (and its WAL/SHM files) to
/// `vault.db.corrupt-<timestamp>` and restarts the app, which then creates a
/// fresh vault. The original is never deleted. Refused outside recovery mode.
#[tauri::command]
pub fn recovery_move_aside(app: tauri::AppHandle, mode: State<'_, AppMode>) -> Result<(), String> {
    if !matches!(&*mode, AppMode::Recovery { .. }) {
        return Err("not in recovery mode".to_string());
    }
    let app_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("path error: {e}"))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string();
    crate::db::move_db_aside(&app_dir.join("vault.db"), &stamp)?;
    app.restart()
}

#[tauri::command]
pub fn recovery_quit(app: tauri::AppHandle) {
    app.exit(0);
}
