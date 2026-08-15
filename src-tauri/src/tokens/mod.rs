//! Scoped, non-interactive access tokens — the credential a headless caller
//! (a CI job, a `crypt-env` CLI invocation, an autonomous agent) uses to read
//! one environment's values without going through the interactive
//! password-unlock flow. Shared by the Tauri commands below and the HTTP
//! handler in `api::mod`, same split as `project::mod`.
//!
//! Every token is bound to exactly one environment — never a whole project
//! or the whole vault — so a leaked token can't be used to enumerate or read
//! anything beyond what it was cut for.
//!
//! Two modes (`DbAccessToken.mode`):
//! - `"session"` (default): the token only authenticates requests against an
//!   already-unlocked vault (the master password was entered, its key is
//!   live in memory). No decryption capability travels with the token
//!   itself.
//! - `"standalone"` (opt-in per token): the vault key for that one
//!   environment is wrapped under a key derived from the token
//!   (`crypto::wrap_vault_key`) and stored alongside it, so the token can
//!   decrypt on its own — no unlocked session required. This is the
//!   credential handed to something that truly runs headless.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::crypto::{self, CryptoKey};
use crate::db::{DbAccessToken, VaultDb};
use crate::vault::SharedState;

// ─── Frontend-facing types ─────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum TokenMode {
    Session,
    Standalone,
}

impl TokenMode {
    fn as_db_str(self) -> &'static str {
        match self {
            TokenMode::Session => "session",
            TokenMode::Standalone => "standalone",
        }
    }
}

#[derive(Deserialize)]
pub struct CreateTokenInput {
    pub name: String,
    #[serde(rename = "environmentId")]
    pub environment_id: i64,
    #[serde(default = "default_mode")]
    pub mode: TokenMode,
    /// Optional RFC3339/ISO8601 expiry. `None` = no expiry.
    #[serde(default)]
    pub expires: Option<String>,
}

fn default_mode() -> TokenMode {
    TokenMode::Session
}

/// Returned once, at creation time — the plaintext token is never
/// recoverable afterwards, only its hash is stored.
#[derive(Serialize)]
pub struct CreatedToken {
    pub id: i64,
    pub token: String,
}

#[derive(Serialize, Clone)]
pub struct TokenSummary {
    pub id: i64,
    pub name: String,
    #[serde(rename = "environmentId")]
    pub environment_id: i64,
    pub mode: TokenMode,
    pub created: String,
    pub expires: Option<String>,
    pub revoked: Option<String>,
    #[serde(rename = "lastUsed")]
    pub last_used: Option<String>,
}

impl From<DbAccessToken> for TokenSummary {
    fn from(t: DbAccessToken) -> Self {
        TokenSummary {
            id: t.id,
            name: t.name,
            environment_id: t.environment_id,
            mode: if t.mode == "standalone" { TokenMode::Standalone } else { TokenMode::Session },
            created: t.created,
            expires: t.expires,
            revoked: t.revoked,
            last_used: t.last_used,
        }
    }
}

/// Why a presented token failed to authenticate — lets the API map each
/// case to the right HTTP status without leaking which case it was in the
/// response body (all get a generic 401/403 there).
pub enum TokenAuthError {
    NotFound,
    Revoked,
    Expired,
    /// A `session` token was presented but the vault isn't currently
    /// unlocked in this process.
    SessionRequired,
    Corrupt(String),
}

// ─── Pure logic (no Tauri/Axum coupling) ───────────────────────────────────

/// Creates a token scoped to one environment. `vault_key` must be the
/// caller's currently-unlocked vault key — required even for `session`
/// tokens (to prove the caller is actually unlocked right now), and used to
/// seal the wrapped copy for `standalone` tokens.
pub async fn create_token(
    db: &VaultDb,
    vault_key: &CryptoKey,
    input: CreateTokenInput,
) -> Result<CreatedToken, String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("token name cannot be empty".into());
    }
    if db.get_environment(input.environment_id).await?.is_none() {
        return Err("environment not found".into());
    }

    let token = crypto::generate_access_token();
    let hash = crypto::hash_access_token(&token);

    let wrapped_key = match input.mode {
        TokenMode::Standalone => Some(crypto::wrap_vault_key(&token, vault_key)?),
        TokenMode::Session => None,
    };

    let id = db
        .insert_access_token(
            name,
            input.environment_id,
            input.mode.as_db_str(),
            &hash,
            wrapped_key.as_deref(),
            input.expires.as_deref(),
        )
        .await?;

    Ok(CreatedToken { id, token })
}

pub async fn list_tokens(db: &VaultDb) -> Result<Vec<TokenSummary>, String> {
    let rows = db.list_access_tokens().await?;
    Ok(rows.into_iter().map(TokenSummary::from).collect())
}

pub async fn revoke_token(db: &VaultDb, id: i64) -> Result<(), String> {
    db.revoke_access_token(id).await
}

/// Authenticates a presented token and resolves the vault key it grants
/// access to, scoped to its bound `environment_id`.
///
/// `session_key`: the calling process's currently-unlocked vault key, if
/// any — required to satisfy a `mode = "session"` token, ignored for
/// `mode = "standalone"` (which carries its own decryption capability).
///
/// On success, updates `last_used` and returns `(token row, resolved key)`.
pub async fn authenticate(
    db: &VaultDb,
    presented_token: &str,
    session_key: Option<&CryptoKey>,
) -> Result<(DbAccessToken, CryptoKey), TokenAuthError> {
    let hash = crypto::hash_access_token(presented_token);
    let row = db
        .get_access_token_by_hash(&hash)
        .await
        .map_err(TokenAuthError::Corrupt)?
        .ok_or(TokenAuthError::NotFound)?;

    if row.revoked.is_some() {
        return Err(TokenAuthError::Revoked);
    }
    if let Some(exp) = &row.expires {
        if exp.as_str() < now_epoch_secs().as_str() {
            return Err(TokenAuthError::Expired);
        }
    }

    let key = if row.mode == "standalone" {
        let wrapped = row.wrapped_key.as_deref().ok_or_else(|| {
            TokenAuthError::Corrupt("standalone token missing wrapped key".to_string())
        })?;
        crypto::unwrap_vault_key(presented_token, wrapped).map_err(TokenAuthError::Corrupt)?
    } else {
        *session_key.ok_or(TokenAuthError::SessionRequired)?
    };

    let _ = db.touch_access_token(row.id).await;

    Ok((row, key))
}

/// Matches the plain epoch-seconds format `db::now_ts()` stamps every
/// `access_tokens` timestamp column with, so string comparison against
/// `expires` sorts correctly.
fn now_epoch_secs() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string()
}

// ─── Tauri commands ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn tokens_create(
    state: State<'_, SharedState>,
    input: CreateTokenInput,
) -> Result<CreatedToken, String> {
    let mut vault = state.lock().await;
    let key = *vault.key.as_deref().ok_or("vault is locked")?;
    vault.touch();
    create_token(&vault.db, &key, input).await
}

#[tauri::command]
pub async fn tokens_list(state: State<'_, SharedState>) -> Result<Vec<TokenSummary>, String> {
    let mut vault = state.lock().await;
    vault.key.as_ref().ok_or("vault is locked")?;
    vault.touch();
    list_tokens(&vault.db).await
}

#[tauri::command]
pub async fn tokens_revoke(state: State<'_, SharedState>, id: i64) -> Result<(), String> {
    let mut vault = state.lock().await;
    vault.key.as_ref().ok_or("vault is locked")?;
    vault.touch();
    revoke_token(&vault.db, id).await
}
