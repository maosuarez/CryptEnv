//! Tauri commands for whole-project sharing via the encrypted relay (issue
//! #4). Modelled on `vault::share_commands::{share_relay_send, share_relay_receive}`
//! — same `guard.touch()` calls, same bundled-default fallback for the relay
//! URL/anon key (`DEFAULT_RELAY_URL`/`DEFAULT_RELAY_ANON_KEY`). Note the
//! existing inconsistency with the HTTP handlers (which hard-fail with
//! `NOT_CONFIGURED` instead of falling back to a bundled default) is
//! preserved rather than "fixed" here — that's a separate concern from this
//! feature.

use tauri::State;
use serde::Serialize;

use crate::project::relay::{build_project_bundle, receive_project_bundle};
use crate::share::relay;
use crate::vault::SharedState;

const DEFAULT_RELAY_URL: &str = match option_env!("CRYPTENV_RELAY_URL") {
    Some(v) => v,
    None => "",
};
const DEFAULT_RELAY_ANON_KEY: &str = match option_env!("CRYPTENV_RELAY_ANON_KEY") {
    Some(v) => v,
    None => "",
};

#[derive(Serialize)]
pub struct RelayShareResult {
    pub code: String,
    pub passphrase: String,
    pub project: String,
    #[serde(rename = "environmentCount")]
    pub environment_count: usize,
    #[serde(rename = "itemCount")]
    pub item_count: usize,
}

#[tauri::command]
pub async fn project_relay_send(
    project_id: i64,
    environment_ids: Vec<i64>,
    vault_state: State<'_, SharedState>,
) -> Result<RelayShareResult, String> {
    let (supabase_url, anon_key, bundle) = {
        let mut guard = vault_state.lock().await;
        let k = guard.key.as_ref().ok_or("vault is locked")?;
        let vault_key: [u8; 32] = **k;
        guard.touch();

        let supabase_url = guard
            .db
            .get_setting("relay_supabase_url")
            .await?
            .unwrap_or_else(|| DEFAULT_RELAY_URL.to_string());
        let anon_key = guard
            .db
            .get_setting("relay_supabase_anon_key")
            .await?
            .unwrap_or_else(|| DEFAULT_RELAY_ANON_KEY.to_string());

        if supabase_url.is_empty() || anon_key.is_empty() {
            return Err("Supabase relay not configured. Go to Settings → Internet Sharing to add your Supabase URL and API key.".to_string());
        }

        let bundle = build_project_bundle(&guard.db, &vault_key, project_id, &environment_ids).await?;
        (supabase_url, anon_key, bundle)
    };

    let project_name = bundle.name.clone();
    let environment_count = bundle.environments.len();
    let item_count = bundle.items.len();

    let code = relay::generate_share_code();
    let passphrase = crate::share::crypto::generate_passphrase();

    let relay_key = relay::derive_relay_key_async(&code, &passphrase).await.map_err(|e| e.to_string())?;
    let payload = relay::encrypt_project(&bundle, &relay_key).map_err(|e| e.to_string())?;

    let code_clone = code.clone();
    let url_clone = supabase_url.clone();
    let key_clone = anon_key.clone();
    tokio::task::spawn_blocking(move || relay::relay_upload(&url_clone, &key_clone, &code_clone, &payload))
        .await
        .map_err(|e| format!("relay send failed: {e}"))?
        .map_err(|e| format!("relay send failed: {e}"))?;

    { vault_state.lock().await.touch(); }

    Ok(RelayShareResult { code, passphrase, project: project_name, environment_count, item_count })
}

#[derive(Serialize)]
pub struct ProjectReceiveResult {
    pub project: String,
    pub environments: Vec<String>,
    #[serde(rename = "itemCount")]
    pub item_count: usize,
}

#[tauri::command]
pub async fn project_relay_receive(
    code: String,
    passphrase: String,
    project_name_override: Option<String>,
    vault_state: State<'_, SharedState>,
) -> Result<ProjectReceiveResult, String> {
    let (vault_key, supabase_url, anon_key) = {
        let mut guard = vault_state.lock().await;
        let k = guard.key.as_ref().ok_or("vault is locked")?;
        let vault_key: [u8; 32] = **k;
        guard.touch();

        let supabase_url = guard
            .db
            .get_setting("relay_supabase_url")
            .await?
            .unwrap_or_else(|| DEFAULT_RELAY_URL.to_string());
        let anon_key = guard
            .db
            .get_setting("relay_supabase_anon_key")
            .await?
            .unwrap_or_else(|| DEFAULT_RELAY_ANON_KEY.to_string());

        if supabase_url.is_empty() || anon_key.is_empty() {
            return Err("Supabase relay not configured. Go to Settings → Internet Sharing to add your Supabase URL and API key.".to_string());
        }

        (vault_key, supabase_url, anon_key)
    };

    let relay_key = relay::derive_relay_key_async(&code, &passphrase).await.map_err(|e| e.to_string())?;

    let code_clone = code.clone();
    let url_clone = supabase_url.clone();
    let key_clone = anon_key.clone();
    let payload = tokio::task::spawn_blocking(move || relay::relay_claim(&url_clone, &key_clone, &code_clone))
        .await
        .map_err(|e| format!("relay receive failed: {e}"))?
        .map_err(|e| format!("relay receive failed: {e}"))?;

    let bundle = relay::decrypt_project(&payload, &relay_key)
        .map_err(|e| format!("relay receive failed: could not decrypt payload — {e}"))?;

    // Burn-after-read already happened server-side: `relay_claim` deletes the row.
    let mut guard = vault_state.lock().await;
    guard.touch();
    let result = receive_project_bundle(&guard.db, &vault_key, bundle, project_name_override).await?;

    Ok(ProjectReceiveResult {
        project: result.project_name,
        environments: result.environment_names,
        item_count: result.item_count,
    })
}
