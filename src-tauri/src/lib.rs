use tauri::{Emitter, Manager};

pub mod api;
pub mod biometric;
pub mod cli;
pub mod crypto;
pub mod db;
pub mod envfile;
pub mod exec;
pub mod fsguard;
pub mod hotkey;
pub mod mcp;
pub mod project;
pub mod share;
pub mod shellfmt;
pub mod tls;
pub mod vault;
pub mod wsl;

#[cfg(test)]
mod test_support;

use vault::{
    app_complete_setup, app_generate_mcp_config, app_get_system_info, app_is_first_run,
    biometric_check, biometric_disable, biometric_enroll, biometric_is_enrolled, biometric_unlock,
    lock_vault, vault_change_password, vault_delete_item, vault_export_backup,
    vault_generate_mcp_token, vault_get_categories, vault_get_items, vault_get_mcp_token,
    vault_get_settings, vault_import_backup, vault_import_backup_data, vault_import_items,
    vault_is_setup, vault_list, vault_lock, vault_parse_import, vault_save_categories, vault_save_item,
    vault_save_settings, vault_unlock, vault_wipe, vault_create_project_item, vault_set_item_global,
    vault_get_item_owners, vault_list_orphan_items, vault_prune_orphan_items, project_create_from_templates,
    vault_pause_hotkey, vault_touch, SharedState, VaultState,
};
use api::approvals::{approval_list, approval_resolve};
use vault::share_commands::{
    share_cancel, share_confirm_fingerprint, share_export_file, share_import_file,
    share_poll_status, relay_schema_version, share_relay_receive, share_relay_send, share_start_receive,
    share_start_send, SharedShareState,
};
use project::{
    environment_delete, environment_inject, environment_inject_preview, environment_save,
    project_delete, project_export, project_import, project_list, project_pick_env_path,
    project_preview_delete, project_save, project_check_root, project_pick_root_dir, project_write_yaml,
};
use project::relay_commands::{project_relay_receive, project_relay_send};
use wsl::{wsl_configure_client, wsl_detect, wsl_distro_home, wsl_list_distros, wsl_remove_client};

struct PendingUpdate(std::sync::Mutex<Option<tauri_plugin_updater::Update>>);

#[tauri::command]
async fn check_for_update(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_updater::UpdaterExt;
    match app.updater().map_err(|e| e.to_string())?.check().await {
        Ok(Some(update)) => {
            let version = update.version.clone();
            let pending = app.state::<PendingUpdate>();
            *pending
                .0
                .lock()
                .map_err(|_| "update state unavailable".to_string())? = Some(update);
            Ok(Some(version))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    let pending = app.state::<PendingUpdate>();
    let update = pending
        .0
        .lock()
        .map_err(|_| "update state unavailable".to_string())?
        .take()
        .ok_or_else(|| "No update available — run check first".to_string())?;
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Multiple crates in the dependency tree enable different rustls crypto
    // providers (ring + aws-lc-rs), so rustls cannot auto-select one.
    // Install ring explicitly before any TLS code runs.
    let _ = rustls::crypto::ring::default_provider().install_default();

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let app_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");
            std::fs::create_dir_all(&app_dir).expect("failed to create app data dir");

            let db_path = app_dir.join("vault.db");
            let db_path_str = db_path.to_str().expect("invalid db path");

            let db = tauri::async_runtime::block_on(db::VaultDb::open(db_path_str))
                .expect("failed to open vault database");

            let saved_hotkey = tauri::async_runtime::block_on(db.get_setting("hotkey"))
                .ok()
                .flatten();

            let state: SharedState =
                std::sync::Arc::new(tokio::sync::Mutex::new(VaultState::new(db)));
            app.manage(state.clone());

            // The share slot lives inside the vault state so locking cancels it.
            let share_state: SharedShareState =
                tauri::async_runtime::block_on(async { state.lock().await.share.clone() });
            app.manage(share_state);

            app.manage(PendingUpdate(std::sync::Mutex::new(None)));

            // Private directory for plaintext files generated for MCP; anything a
            // previous run left behind is swept here.
            let generated =
                tauri::async_runtime::block_on(async { state.lock().await.generated.clone() });
            generated.init(app_dir.join("mcp-tmp"));

            let api_state = std::sync::Arc::new(api::ApiState::new(state.clone()));
            app.manage(api_state.clone());

            // A pending MCP approval must be seen by the user: tell the webview and
            // bring the window forward (it may be hidden to the tray).
            let approval_handle = app.handle().clone();
            api_state.set_approval_notifier(move |view| {
                let _ = approval_handle.emit("approval://requested", view);
                if let Some(window) = approval_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                    let _ = window.request_user_attention(Some(tauri::UserAttentionType::Critical));
                }
            });

            let api_app_dir = app_dir.clone();
            tauri::async_runtime::spawn(api::start_server(api_state.clone(), api_app_dir));

            // Background auto-lock: every 30 s, check idle time against the
            // configured timeout. Timeout = 0 means "never lock".
            let auto_lock_state = state.clone();
            let auto_lock_handle = app.handle().clone();
            let maintenance_state = api_state.clone();
            tauri::async_runtime::spawn(async move {
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(30));
                // The first tick fires immediately; skip it so we don't lock
                // right at startup before the user has a chance to unlock.
                interval.tick().await;

                loop {
                    interval.tick().await;

                    // Expire stale MCP approvals and delete expired generated files.
                    maintenance_state.maintenance().await;

                    // Acquire lock, compute whether we should auto-lock, then
                    // release before calling lock_vault (which re-acquires).
                    let should_lock = {
                        let s = auto_lock_state.lock().await;

                        // Only relevant while the vault is unlocked.
                        let last = match s.last_activity {
                            Some(t) => t,
                            None => continue,
                        };

                        let timeout_mins: u64 = s
                            .db
                            .get_setting("auto_lock_timeout")
                            .await
                            .ok()
                            .flatten()
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(5);

                        // 0 means "never auto-lock".
                        if timeout_mins == 0 {
                            continue;
                        }

                        last.elapsed() >= std::time::Duration::from_secs(timeout_mins * 60)
                    };

                    if should_lock {
                        lock_vault(&auto_lock_state).await;
                        // Let the frontend drop to the lock screen right away
                        // instead of discovering the lock on its next call.
                        let _ = auto_lock_handle.emit("vault_locked", ());
                    }
                }
            });

            // Register the persisted hotkey; fall back to the platform default
            // when unset or when the stored combination can't be registered.
            app.manage(hotkey::HotkeyState::default());
            let registered = saved_hotkey
                .as_deref()
                .map(|hk| hotkey::register_app_hotkey(app.handle(), hk))
                .unwrap_or_else(|| Err("unset".into()));
            if registered.is_err() {
                hotkey::register_app_hotkey(app.handle(), hotkey::default_hotkey())?;
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            shellfmt::shell_format_assignment,
            vault_is_setup,
            vault_unlock,
            vault_lock,
            vault_touch,
            vault_list,
            vault_get_items,
            vault_save_item,
            vault_delete_item,
            vault_create_project_item,
            vault_set_item_global,
            vault_get_item_owners,
            vault_list_orphan_items,
            vault_prune_orphan_items,
            vault_get_categories,
            vault_save_categories,
            vault_get_settings,
            vault_save_settings,
            vault_pause_hotkey,
            vault_change_password,
            vault_wipe,
            vault_generate_mcp_token,
            vault_get_mcp_token,
            vault_export_backup,
            vault_import_backup,
            vault_import_backup_data,
            vault_parse_import,
            vault_import_items,
            biometric_check,
            biometric_is_enrolled,
            biometric_enroll,
            biometric_unlock,
            biometric_disable,
            share_start_send,
            share_start_receive,
            share_poll_status,
            share_confirm_fingerprint,
            share_cancel,
            share_export_file,
            share_import_file,
            share_relay_send,
            share_relay_receive,
            relay_schema_version,
            project_list,
            project_save,
            project_create_from_templates,
            project_delete,
            project_preview_delete,
            project_export,
            project_import,
            project_pick_env_path,
            project_pick_root_dir,
            project_check_root,
            project_write_yaml,
            environment_save,
            environment_delete,
            environment_inject,
            wsl_list_distros,
            wsl_distro_home,
            wsl_detect,
            wsl_configure_client,
            wsl_remove_client,
            environment_inject_preview,
            project_relay_send,
            project_relay_receive,
            check_for_update,
            install_update,
            app_is_first_run,
            app_complete_setup,
            app_generate_mcp_config,
            app_get_system_info,
            approval_list,
            approval_resolve,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // Plaintext files generated for MCP must not outlive the app.
            if let tauri::RunEvent::Exit = event {
                let state = app_handle.state::<SharedState>();
                tauri::async_runtime::block_on(async {
                    state.lock().await.generated.purge_all();
                });
            }
        });
}
