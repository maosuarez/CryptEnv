use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router, middleware};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use crate::crypto;
// `DbWorkspaceVar` went unused when issue #4 replaced the workspace-relay
// handlers with the project-relay ones.
use crate::db::DbCategory;
use crate::envfile;
use crate::fsguard;
use crate::project::{self, EnvironmentInput, ProjectInput};
use crate::share::{ShareState, ShareSessionState};
use crate::share::relay;
use crate::share::package::PlainItem;
use crate::tls;
use crate::tokens;
use crate::vault::{SharedState, VaultItem};

// ─── Estado compartido de la API ──────────────────────────────────────────────

struct RateLimitState {
    attempts: u32,
    window_start: Instant,
}

pub struct ApiState {
    vault: SharedState,
    session_token: Arc<Mutex<Option<String>>>,
    token_expires: Arc<Mutex<Option<Instant>>>,
    unlock_rate: Mutex<RateLimitState>,
    share: Arc<ShareState>,
}

impl ApiState {
    /// Builds a fresh state: no active session, rate limiter reset, new share
    /// session. Used by `start_server` and by `crate::test_support::router`
    /// so both build `ApiState` identically — no duplicated initialisation to
    /// drift. `pub(crate)` (not `pub`): visible to the in-crate test harness
    /// without widening the crate's public API.
    pub(crate) fn new(vault: SharedState) -> Self {
        ApiState {
            vault,
            session_token: Arc::new(Mutex::new(None)),
            token_expires: Arc::new(Mutex::new(None)),
            unlock_rate: Mutex::new(RateLimitState {
                attempts: 0,
                window_start: Instant::now(),
            }),
            share: Arc::new(ShareState::new()),
        }
    }
}

// ─── Tipos de respuesta ───────────────────────────────────────────────────────

#[derive(Serialize)]
struct ErrorBody {
    error: String,
    code: String,
}

#[derive(Serialize)]
struct UnlockResponse {
    token: String,
}

#[derive(Serialize)]
struct CategoryResponse {
    id: String,
    name: String,
    color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Serialize)]
struct CommandDetail {
    id: i64,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shell: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    placeholders: Vec<String>,
}

/// `/commands`-list-only wrapper adding the same `isGlobal`/`linked`
/// discriminators as `ScopedItem`, kept off the shared `CommandDetail` (used
/// unscoped by `GET /commands/:id`, which this change deliberately leaves
/// alone — see plan §3/§4).
#[derive(Serialize)]
struct ScopedCommand {
    #[serde(flatten)]
    detail: CommandDetail,
    #[serde(rename = "isGlobal")]
    is_global: bool,
    linked: bool,
}

#[derive(Serialize)]
struct RevealResponse {
    value: String,
}

// ─── CORS guard ───────────────────────────────────────────────────────────────

/// Rechaza requests con Origin distinto de "null" o ausente (browser cross-origin).
/// Añade Access-Control-Allow-Origin: null a todas las respuestas.
async fn cors_guard(
    request: axum::extract::Request,
    next: middleware::Next,
) -> axum::response::Response {
    let allowed = match request.headers().get("origin") {
        Some(origin) => origin.as_bytes() == b"null",
        None => true,
    };
    if !allowed {
        return (StatusCode::FORBIDDEN, "forbidden origin").into_response();
    }
    let mut response = next.run(request).await;
    if let Ok(val) = HeaderValue::from_str("null") {
        response.headers_mut().insert("access-control-allow-origin", val);
    }
    response
}

// ─── Helpers de respuesta y auth ──────────────────────────────────────────────

fn err_json(status: StatusCode, msg: &str, code: &str) -> impl IntoResponse {
    (status, Json(ErrorBody { error: msg.to_string(), code: code.to_string() }))
}

/// Builds the `envfile` marker line for a resolved environment: looks up
/// its project's name for the `(project: ..., environment: ...)` text.
/// Falls back to "unknown" if the project lookup fails — this is
/// informational text only (§4.3 of the issue #8 plan), never parsed back,
/// so a lookup failure here must never block the write itself.
async fn build_marker(state: &ApiState, env: &project::Environment) -> String {
    let project_name = {
        let vault = state.vault.lock().await;
        vault.db.get_project_name(env.project_id).await.ok().flatten()
    };
    envfile::marker_line(project_name.as_deref().unwrap_or("unknown"), &env.name)
}

/// Maps an `envfile::EnvFileError` to the HTTP response shape shared by
/// `/fill` and `/environments/:id/example` (§1.3 of the issue #8 plan).
fn err_envfile(e: envfile::EnvFileError) -> axum::response::Response {
    match e {
        envfile::EnvFileError::TargetExists(_) => {
            err_json(StatusCode::CONFLICT, &e.to_string(), "TARGET_EXISTS").into_response()
        }
        envfile::EnvFileError::BackupExists(_) => {
            err_json(StatusCode::CONFLICT, &e.to_string(), "BACKUP_EXISTS").into_response()
        }
        envfile::EnvFileError::Io(..) => {
            err_json(StatusCode::INTERNAL_SERVER_ERROR, &format!("cannot write file: {e}"), "INTERNAL_ERROR")
                .into_response()
        }
    }
}

/// Verifica el header X-Vault-Token.
/// Acepta session token (con expiración) o MCP token estático (sin expiración).
async fn verify_token(headers: &HeaderMap, state: &ApiState) -> Result<(), StatusCode> {
    let provided = headers
        .get("x-vault-token")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // --- Verificar session token (con expiración) ---
    {
        let stored = state.session_token.lock().await;
        let expires = state.token_expires.lock().await;

        if let (Some(s), Some(exp)) = (stored.as_deref(), *expires) {
            if Instant::now() < exp && bool::from(s.as_bytes().ct_eq(provided.as_bytes())) {
                let vault = state.vault.lock().await;
                return if vault.key.is_some() { Ok(()) } else { Err(StatusCode::FORBIDDEN) };
            }
        }
    }

    // --- Fallback: MCP token estático (sin expiración) ---
    {
        let vault = state.vault.lock().await;
        if vault.key.is_none() {
            return Err(StatusCode::FORBIDDEN);
        }
        let mcp_token = vault.db.get_setting("mcp_token").await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        match mcp_token {
            Some(t) if bool::from(t.as_bytes().ct_eq(provided.as_bytes())) => Ok(()),
            _ => Err(StatusCode::UNAUTHORIZED),
        }
    }
}

/// Toma el lock del vault, copia key + raw data, suelta el lock, descifra sin lock.
async fn decrypt_all_items(state: &ApiState) -> Result<Vec<VaultItem>, StatusCode> {
    // Fase 1: tomar lock, extraer key y datos crudos
    let (key, raw) = {
        let vault = state.vault.lock().await;
        let key = vault.key.as_ref().ok_or(StatusCode::FORBIDDEN)?.clone();
        let raw = vault
            .db
            .list_items()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        (key, raw)
        // lock se suelta aquí
    };

    // Fase 2: descifrar sin lock
    let items: Vec<VaultItem> = raw
        .into_iter()
        .filter_map(|(id, _, data, _, is_global)| crate::vault::decrypt_item(&key, id, &data, is_global).ok())
        .collect();

    Ok(items)
}

/// Elimina los campos sensibles antes de retornar un item al cliente.
fn redact_item(mut item: VaultItem) -> VaultItem {
    item.value = None;
    item.password = None;
    item.content = None;
    item
}

/// Extrae placeholders del formato {{VAR}} de un template.
fn extract_placeholders(template: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut s = template;
    while let Some(start) = s.find("{{") {
        let rest = &s[start + 2..];
        if let Some(end) = rest.find("}}") {
            let name = &rest[..end];
            if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                let ph = format!("{{{{{}}}}}", name);
                if !result.contains(&ph) {
                    result.push(ph);
                }
            }
            s = &rest[end + 2..];
        } else {
            break;
        }
    }
    result
}

fn now_ts_str() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

// ─── Input validation ─────────────────────────────────────────────────────────

const VALID_ITEM_TYPES: &[&str] = &["secret", "credential", "link", "note", "command"];

/// Returns an HTTP 422 response with the standard error body.
fn err_validation(field: &str, reason: &str) -> axum::response::Response {
    err_json(
        StatusCode::UNPROCESSABLE_ENTITY,
        &format!("{field}: {reason}"),
        "VALIDATION_ERROR",
    )
    .into_response()
}

// ─── Project + environment scoping ────────────────────────────────────────────
//
// Every endpoint scoped to a single project+environment accepts the same two
// shapes, mirrored from the CLI's existing `project inject --id` /
// `--project --environment` convention: either `environment_id` alone, or a
// case-insensitive `project` name + `environment` name pair. This struct is
// reused verbatim as the axum `Query` extractor on every such endpoint.
#[derive(Deserialize)]
struct EnvScopeQuery {
    environment_id: Option<i64>,
    project: Option<String>,
    environment: Option<String>,
    /// Only consumed by `handle_list_commands` — see `IncludeGlobal`. Present
    /// here (rather than only on `ItemsQuery`) because `/commands` shares
    /// this extractor; other handlers reusing `EnvScopeQuery` simply ignore
    /// an unused query param, matching the existing per-handler duplication
    /// style instead of refactoring the shared extractor in a bug-fix PR.
    include_global: Option<String>,
}

/// Discovery-endpoint tri-state for whether globally-reusable, unlinked
/// items are unioned into the response. Materialization endpoints
/// (`/fill`, `/environments/:id/inject`, `/environments/:id/example`,
/// `/share/listen`) never consult this — they stay strictly linkage-based.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum IncludeGlobal {
    With,
    Without,
    Only,
}

impl IncludeGlobal {
    /// `None` (param omitted) defaults to `With` — see plan §4.5: a
    /// default-`false` fix is invisible to callers who don't know the
    /// param exists, which is precisely the bug being fixed.
    fn parse(raw: Option<&str>) -> Result<IncludeGlobal, axum::response::Response> {
        match raw {
            None | Some("true") | Some("with") => Ok(IncludeGlobal::With),
            Some("false") | Some("without") => Ok(IncludeGlobal::Without),
            Some("only") => Ok(IncludeGlobal::Only),
            Some(_) => Err(err_validation(
                "include_global",
                "must be one of: true, false, only",
            )),
        }
    }
}

/// Union (or restriction) of `items` against the `linked` id set, per `mode`.
/// Pure function — no `ApiState`, no lock, no crypto — fully unit-testable
/// without a vault or an HTTP server. Stamps `linked` on every returned item.
fn scope_items(items: Vec<VaultItem>, linked: &HashSet<i64>, mode: IncludeGlobal) -> Vec<ScopedItem> {
    items
        .into_iter()
        .filter_map(|item| {
            let is_linked = linked.contains(&item.id);
            let is_global = item.is_global.unwrap_or(false);
            let include = match mode {
                IncludeGlobal::With => is_linked || is_global,
                IncludeGlobal::Without => is_linked,
                IncludeGlobal::Only => is_global,
            };
            include.then(|| ScopedItem { linked: is_linked, item })
        })
        .collect()
}

/// Applies the `/items` type/category/search filters over `ScopedItem`s.
/// `type_filter`/`cat_filter`/`search_filter` are expected pre-lowercased by
/// the caller, matching the pre-existing filter behaviour byte-for-byte.
fn filter_scoped_items(
    items: Vec<ScopedItem>,
    type_filter: Option<&str>,
    cat_filter: Option<&str>,
    search_filter: Option<&str>,
) -> Vec<ScopedItem> {
    items
        .into_iter()
        .filter(|s| {
            if let Some(t) = type_filter {
                if s.item.item_type.to_lowercase() != t {
                    return false;
                }
            }
            if let Some(cat) = cat_filter {
                let found = s
                    .item
                    .categories
                    .iter()
                    .flatten()
                    .any(|c| c.to_lowercase() == cat);
                if !found {
                    return false;
                }
            }
            if let Some(q) = search_filter {
                let name_match = s
                    .item
                    .name
                    .as_deref()
                    .map(|n| n.to_lowercase().contains(q))
                    .unwrap_or(false);
                let title_match = s
                    .item
                    .title
                    .as_deref()
                    .map(|t| t.to_lowercase().contains(q))
                    .unwrap_or(false);
                if !name_match && !title_match {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// API-response-only wrapper adding the `linked` discriminator on top of
/// `VaultItem`. MUST NOT be merged into `VaultItem` — that struct is what
/// gets AES-GCM encrypted (`vault::encrypt_item`), so a view-only field on
/// it risks being persisted into ciphertext by any round-trip write path.
#[derive(Serialize)]
struct ScopedItem {
    #[serde(flatten)]
    item: VaultItem,
    linked: bool,
}

/// Resolves the environment for a scoped request. A missing/unmatched
/// identifier is a 422 (matching the existing validation-error convention,
/// see `err_validation`); an *ambiguous* case-insensitive match — more than
/// one project or environment satisfying the given name — is a distinct 409
/// `AMBIGUOUS_SCOPE` instead, since the request itself is well-formed and the
/// fix (`environment_id`) is deterministic. Every endpoint that scopes a
/// request via `EnvScopeQuery` goes through this single function, so this is
/// the one place the 409 mapping needs to live.
async fn resolve_scope(
    state: &ApiState,
    environment_id: Option<i64>,
    project: Option<&str>,
    environment: Option<&str>,
) -> Result<project::Environment, axum::response::Response> {
    let vault = state.vault.lock().await;
    project::resolve_environment(&vault.db, environment_id, project, environment)
        .await
        .map_err(|msg| {
            if msg.starts_with(project::AMBIGUOUS_MATCH_PREFIX) {
                err_json(StatusCode::CONFLICT, &msg, "AMBIGUOUS_SCOPE").into_response()
            } else {
                err_validation("project/environment", &msg)
            }
        })
}

/// Validates fields for a POST /items (create) request.
/// All required fields must be present and non-empty; optional fields are
/// validated only when present.
fn validate_create(body: &VaultItem) -> Result<(), axum::response::Response> {
    // name: required, non-empty, max 255
    match body.name.as_deref() {
        None | Some("") => return Err(err_validation("name", "required and must not be empty")),
        Some(n) if n.len() > 255 => return Err(err_validation("name", "must be 255 characters or fewer")),
        _ => {}
    }

    // type: required, must be a known variant
    if body.item_type.is_empty() {
        return Err(err_validation("type", "required"));
    }
    if !VALID_ITEM_TYPES.contains(&body.item_type.as_str()) {
        return Err(err_validation(
            "type",
            &format!(
                "must be one of: {}",
                VALID_ITEM_TYPES.join(", ")
            ),
        ));
    }

    // value: required, non-empty
    match body.value.as_deref() {
        None | Some("") => return Err(err_validation("value", "required and must not be empty")),
        _ => {}
    }

    // categories: each entry max 100 chars
    for cat in body.categories.iter().flatten() {
        if cat.len() > 100 {
            return Err(err_validation("category", "each entry must be 100 characters or fewer"));
        }
    }

    Ok(())
}

/// Validates fields for a PUT /items/:id (update) request.
/// All fields are optional, but any field that is present must satisfy its rule.
fn validate_update(body: &VaultItem) -> Result<(), axum::response::Response> {
    // name: if present must be non-empty and max 255
    if let Some(n) = body.name.as_deref() {
        if n.is_empty() {
            return Err(err_validation("name", "must not be empty"));
        }
        if n.len() > 255 {
            return Err(err_validation("name", "must be 255 characters or fewer"));
        }
    }

    // type: if present (non-empty string sent) must be a known variant
    if !body.item_type.is_empty() && !VALID_ITEM_TYPES.contains(&body.item_type.as_str()) {
        return Err(err_validation(
            "type",
            &format!(
                "must be one of: {}",
                VALID_ITEM_TYPES.join(", ")
            ),
        ));
    }

    // value: if present must be non-empty
    if let Some(v) = body.value.as_deref() {
        if v.is_empty() {
            return Err(err_validation("value", "must not be empty"));
        }
    }

    // categories: each entry max 100 chars
    for cat in body.categories.iter().flatten() {
        if cat.len() > 100 {
            return Err(err_validation("category", "each entry must be 100 characters or fewer"));
        }
    }

    Ok(())
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct UnlockBody {
    master_password: String,
}

async fn handle_unlock(
    State(state): State<Arc<ApiState>>,
    Json(body): Json<UnlockBody>,
) -> impl IntoResponse {
    // Rate limiting: máx 5 intentos de unlock por ventana de 60 segundos
    {
        let mut rate = state.unlock_rate.lock().await;
        let now = Instant::now();
        if now.duration_since(rate.window_start) >= Duration::from_secs(60) {
            rate.window_start = now;
            rate.attempts = 0;
        }
        if rate.attempts >= 5 {
            let elapsed = now.duration_since(rate.window_start).as_secs();
            let retry_after = 60u64.saturating_sub(elapsed);
            let mut resp = err_json(
                StatusCode::TOO_MANY_REQUESTS,
                "demasiados intentos, reintenta más tarde",
                "RATE_LIMITED",
            )
            .into_response();
            if let Ok(val) = HeaderValue::from_str(&retry_after.to_string()) {
                resp.headers_mut().insert("retry-after", val);
            }
            return resp;
        }
        rate.attempts += 1;
    }

    let mut vault = state.vault.lock().await;

    // Verificar que la bóveda está inicializada
    let (salt, token) = match vault.db.get_meta().await {
        Ok(Some(m)) => m,
        Ok(None) => {
            return err_json(
                StatusCode::BAD_REQUEST,
                "bóveda no inicializada",
                "BAD_REQUEST",
            )
            .into_response()
        }
        Err(e) => {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR")
                .into_response()
        }
    };

    let key = match crypto::unlock_vault_crypto(body.master_password.as_bytes(), &salt, &token) {
        Ok(k) => k,
        Err(_) => {
            return err_json(
                StatusCode::BAD_REQUEST,
                "contraseña incorrecta",
                "BAD_REQUEST",
            )
            .into_response()
        }
    };

    vault.key = Some(Zeroizing::new(key));

    if let Err(e) = crate::vault::migrate_literal_vars_to_items(&vault.db, &key).await {
        return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response();
    }

    // Leer timeout de la DB (minutos, default 5)
    let minutes = match vault.db.get_setting("auto_lock_timeout").await {
        Ok(Some(v)) => v.parse::<u64>().unwrap_or(5),
        _ => 5,
    };

    drop(vault);

    // Generar token de sesión: 16 bytes = 32 chars hex
    let mut token_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut token_bytes);
    let session_token: String = token_bytes.iter().map(|b| format!("{:02x}", b)).collect();

    let expires = Instant::now() + Duration::from_secs(minutes * 60);

    *state.session_token.lock().await = Some(session_token.clone());
    *state.token_expires.lock().await = Some(expires);

    (StatusCode::OK, Json(UnlockResponse { token: session_token })).into_response()
}

#[derive(Deserialize)]
struct ItemsQuery {
    #[serde(rename = "type")]
    item_type: Option<String>,
    category: Option<String>,
    search: Option<String>,
    environment_id: Option<i64>,
    project: Option<String>,
    environment: Option<String>,
    /// Tri-state `true|false|only`, default `true` — see `IncludeGlobal`.
    include_global: Option<String>,
}

/// The set of item ids linked into an environment's `environment_vars` — the
/// scope every /items, /commands, /fill lookup below is restricted to.
fn environment_item_ids(env: &project::Environment) -> HashSet<i64> {
    env.vars.iter().map(|v| v.item_id).collect()
}

async fn handle_list_items(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Query(params): Query<ItemsQuery>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("no autorizado", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("bóveda bloqueada", "VAULT_LOCKED"),
            _ => ("error interno", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let env = match resolve_scope(
        &state,
        params.environment_id,
        params.project.as_deref(),
        params.environment.as_deref(),
    )
    .await
    {
        Ok(e) => e,
        Err(resp) => return resp,
    };
    let allowed_ids = environment_item_ids(&env);

    let mode = match IncludeGlobal::parse(params.include_global.as_deref()) {
        Ok(m) => m,
        Err(resp) => return resp,
    };

    let items = match decrypt_all_items(&state).await {
        Ok(i) => i,
        Err(StatusCode::FORBIDDEN) => {
            return err_json(StatusCode::FORBIDDEN, "bóveda bloqueada", "VAULT_LOCKED")
                .into_response()
        }
        Err(_) => {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, "error interno", "INTERNAL_ERROR")
                .into_response()
        }
    };

    // Union linked items with globals (per `mode`) before applying the
    // type/category/search filters, so `search` also searches globals.
    let scoped = scope_items(items, &allowed_ids, mode);

    let type_filter = params.item_type.as_deref().map(|s| s.to_lowercase());
    let cat_filter = params.category.as_deref().map(|s| s.to_lowercase());
    let search_filter = params.search.as_deref().map(|s| s.to_lowercase());

    let filtered: Vec<ScopedItem> = filter_scoped_items(
        scoped,
        type_filter.as_deref(),
        cat_filter.as_deref(),
        search_filter.as_deref(),
    )
    .into_iter()
    .map(|s| ScopedItem { item: redact_item(s.item), linked: s.linked })
    .collect();

    (StatusCode::OK, Json(filtered)).into_response()
}

async fn handle_get_item(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("no autorizado", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("bóveda bloqueada", "VAULT_LOCKED"),
            _ => ("error interno", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let items = match decrypt_all_items(&state).await {
        Ok(i) => i,
        Err(StatusCode::FORBIDDEN) => {
            return err_json(StatusCode::FORBIDDEN, "bóveda bloqueada", "VAULT_LOCKED")
                .into_response()
        }
        Err(_) => {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, "error interno", "INTERNAL_ERROR")
                .into_response()
        }
    };

    match items.into_iter().find(|item| item.id == id) {
        Some(item) => (StatusCode::OK, Json(redact_item(item))).into_response(),
        None => err_json(StatusCode::NOT_FOUND, "item no encontrado", "NOT_FOUND").into_response(),
    }
}

/// Body for `POST /items`: the item fields (flattened, same shape as before)
/// plus an optional `key` override for the environment variable this item is
/// linked under. When omitted, `key` defaults to the item's `name`.
#[derive(Deserialize)]
struct CreateItemBody {
    #[serde(flatten)]
    item: VaultItem,
    key: Option<String>,
}

/// Separate extractor (rather than folding `on_conflict` into `EnvScopeQuery`,
/// which every scoped GET/PUT/DELETE endpoint also uses) so that only
/// `POST /items` gives the parameter any meaning — everywhere else it would
/// be silently accepted and ignored, which is worse than a second struct.
#[derive(Deserialize)]
struct ConflictQuery {
    on_conflict: Option<String>,
}

fn parse_link_mode(raw: Option<&str>) -> Result<crate::db::LinkMode, axum::response::Response> {
    match raw {
        None => Ok(crate::db::LinkMode::Update),
        Some("update") => Ok(crate::db::LinkMode::Update),
        Some("replace") => Ok(crate::db::LinkMode::Replace),
        Some("error") => Ok(crate::db::LinkMode::Error),
        Some(_) => Err(err_validation("on_conflict", "must be one of: update, replace, error")),
    }
}

async fn handle_create_item(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Query(scope): Query<EnvScopeQuery>,
    Query(conflict): Query<ConflictQuery>,
    Json(mut body): Json<CreateItemBody>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("no autorizado", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("bóveda bloqueada", "VAULT_LOCKED"),
            _ => ("error interno", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let mode = match parse_link_mode(conflict.on_conflict.as_deref()) {
        Ok(m) => m,
        Err(resp) => return resp,
    };

    let env = match resolve_scope(
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

    if let Err(resp) = validate_create(&body.item) {
        return resp;
    }

    let key_name = body
        .key
        .take()
        .filter(|k| !k.is_empty())
        .or_else(|| body.item.name.clone())
        .unwrap_or_default();
    if key_name.is_empty() {
        return err_validation("key", "required and must not be empty (defaults to item name)");
    }

    // Asegurar timestamp de creación
    if body.item.created.is_empty() {
        body.item.created = now_ts_str();
    }

    let outcome = {
        let vault = state.vault.lock().await;
        let key = match vault.key.as_ref() {
            Some(k) => k.clone(),
            None => {
                return err_json(StatusCode::FORBIDDEN, "bóveda bloqueada", "VAULT_LOCKED")
                    .into_response()
            }
        };

        crate::vault::create_or_update_env_item(
            &vault.db,
            &key,
            &body.item,
            env.project_id,
            env.id,
            &key_name,
            mode,
        )
        .await
    };

    match outcome {
        Ok(crate::vault::UpsertOutcome::Created(item)) => {
            (StatusCode::CREATED, Json(redact_item(item))).into_response()
        }
        Ok(crate::vault::UpsertOutcome::Updated(item)) => {
            (StatusCode::OK, Json(redact_item(item))).into_response()
        }
        Ok(crate::vault::UpsertOutcome::Conflict { item_id, reason }) => match reason {
            crate::vault::ConflictReason::Shared => err_json(
                StatusCode::CONFLICT,
                &format!(
                    "key '{key_name}' is linked to item {item_id}, which is shared (global, multi-linked, \
                     or multi-owned). Use ?on_conflict=replace to repoint this link to a new copy, or \
                     PUT /items/{item_id} to change the shared value everywhere."
                ),
                "SHARED_ITEM_CONFLICT",
            )
            .into_response(),
            crate::vault::ConflictReason::KeyExists => err_json(
                StatusCode::CONFLICT,
                &format!("key '{key_name}' already exists in this environment"),
                "KEY_EXISTS",
            )
            .into_response(),
            crate::vault::ConflictReason::StateChanged => err_json(
                StatusCode::CONFLICT,
                &format!("key '{key_name}' changed concurrently, retry the request"),
                "CONFLICT_RETRY",
            )
            .into_response(),
        },
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response(),
    }
}

async fn handle_update_item(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<VaultItem>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("no autorizado", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("bóveda bloqueada", "VAULT_LOCKED"),
            _ => ("error interno", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    if let Err(resp) = validate_update(&body) {
        return resp;
    }

    // Decrypt existing item so we can merge — secrets stay server-side
    let items = match decrypt_all_items(&state).await {
        Ok(i) => i,
        Err(StatusCode::FORBIDDEN) => {
            return err_json(StatusCode::FORBIDDEN, "bóveda bloqueada", "VAULT_LOCKED")
                .into_response()
        }
        Err(_) => {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, "error interno", "INTERNAL_ERROR")
                .into_response()
        }
    };

    let existing = match items.into_iter().find(|item| item.id == id) {
        Some(e) => e,
        None => return err_json(StatusCode::NOT_FOUND, "item no encontrado", "NOT_FOUND").into_response(),
    };

    // Merge: body fields override existing; None/empty keeps the existing value.
    // Secret fields (value, password, content) are preserved from the encrypted
    // store when the caller does not supply them — they never leave this process.
    let merged = VaultItem {
        id,
        item_type: if body.item_type.is_empty() { existing.item_type } else { body.item_type },
        name:        body.name.or(existing.name),
        value:       body.value.or(existing.value),
        url:         body.url.or(existing.url),
        username:    body.username.or(existing.username),
        password:    body.password.or(existing.password),
        title:       body.title.or(existing.title),
        description: body.description.or(existing.description),
        command:     body.command.or(existing.command),
        shell:       body.shell.or(existing.shell),
        notes:       body.notes.or(existing.notes),
        content:     body.content.or(existing.content),
        // None = not sent → keep existing. Some([]) = explicitly clear. Some([...]) = replace.
        categories: body.categories.or(existing.categories),
        created: existing.created,
        is_global: body.is_global.or(existing.is_global),
    };

    let vault = state.vault.lock().await;
    let key = match vault.key.as_ref() {
        Some(k) => k.clone(),
        None => {
            return err_json(StatusCode::FORBIDDEN, "bóveda bloqueada", "VAULT_LOCKED")
                .into_response()
        }
    };

    let encrypted = match crate::vault::encrypt_item(&key, &merged) {
        Ok(e) => e,
        Err(_) => {
            return err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                "error cifrando item",
                "INTERNAL_ERROR",
            )
            .into_response()
        }
    };

    let is_global = merged.is_global.unwrap_or(false);
    match vault
        .db
        .upsert_item(id, &merged.item_type, &encrypted, &merged.created, is_global)
        .await
    {
        Ok(_) => {}
        Err(e) => {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR")
                .into_response()
        }
    }

    (StatusCode::OK, Json(redact_item(merged))).into_response()
}

async fn handle_delete_item(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("no autorizado", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("bóveda bloqueada", "VAULT_LOCKED"),
            _ => ("error interno", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    // Verificar que el item existe
    let items = match decrypt_all_items(&state).await {
        Ok(i) => i,
        Err(StatusCode::FORBIDDEN) => {
            return err_json(StatusCode::FORBIDDEN, "bóveda bloqueada", "VAULT_LOCKED")
                .into_response()
        }
        Err(_) => {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, "error interno", "INTERNAL_ERROR")
                .into_response()
        }
    };

    if items.iter().find(|item| item.id == id).is_none() {
        return err_json(StatusCode::NOT_FOUND, "item no encontrado", "NOT_FOUND").into_response();
    }

    let vault = state.vault.lock().await;
    match vault.db.delete_item(id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response()
        }
    }
}

/// Read-only, redacted report of items with no `environment_vars` reference
/// and `is_global = 0` (issue #9 §3.6). Deliberately no REST prune endpoint —
/// a static MCP token should not be able to bulk-delete vault rows; the
/// destructive half stays behind the GUI's confirmation
/// (`vault_prune_orphan_items`), matching every other destructive vault
/// operation. Lets `crypt-env doctor` surface a one-line orphan count.
async fn handle_list_orphans(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("no autorizado", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("bóveda bloqueada", "VAULT_LOCKED"),
            _ => ("error interno", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let vault = state.vault.lock().await;
    let key = match vault.key.as_ref() {
        Some(k) => k.clone(),
        None => {
            return err_json(StatusCode::FORBIDDEN, "bóveda bloqueada", "VAULT_LOCKED")
                .into_response()
        }
    };

    match crate::vault::list_orphan_items(&vault.db, &key).await {
        Ok(items) => {
            let redacted: Vec<VaultItem> = items.into_iter().map(redact_item).collect();
            (StatusCode::OK, Json(redacted)).into_response()
        }
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response(),
    }
}

async fn handle_list_categories(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Categorías no requieren key, solo token válido
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("no autorizado", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => {
                // Para categorías, FORBIDDEN (vault sin key) también es aceptable
                // pero el verify_token ya verifica key, así que devolvemos el error
                ("bóveda bloqueada", "VAULT_LOCKED")
            }
            _ => ("error interno", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let vault = state.vault.lock().await;
    match vault.db.list_categories().await {
        Ok(cats) => {
            let response: Vec<CategoryResponse> = cats
                .into_iter()
                .map(|c| CategoryResponse { id: c.cid, name: c.name, color: c.color, description: c.description })
                .collect();
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response()
        }
    }
}

#[derive(Deserialize)]
struct CreateCategoryBody {
    name: String,
    color: String,
    description: Option<String>,
}

async fn handle_create_category(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(body): Json<CreateCategoryBody>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("no autorizado", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("bóveda bloqueada", "VAULT_LOCKED"),
            _ => ("error interno", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    if body.name.is_empty() {
        return err_validation("name", "required and must not be empty");
    }
    if body.name.len() > 100 {
        return err_validation("name", "must be 100 characters or fewer");
    }
    if body.color.is_empty() {
        return err_validation("color", "required and must not be empty");
    }

    // Generate a UUID-like cid using random bytes
    let mut id_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut id_bytes);
    let cid = id_bytes.iter().map(|b| format!("{:02x}", b)).collect::<String>();

    let cat = DbCategory { cid: cid.clone(), name: body.name.clone(), color: body.color.clone(), description: body.description.clone() };

    let vault = state.vault.lock().await;
    match vault.db.insert_category(&cat).await {
        Ok(()) => (
            StatusCode::CREATED,
            Json(CategoryResponse { id: cid, name: body.name, color: body.color, description: body.description }),
        )
            .into_response(),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response(),
    }
}

#[derive(Deserialize)]
struct UpdateCategoryBody {
    name: Option<String>,
    color: Option<String>,
    description: Option<String>,
}

async fn handle_update_category(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<UpdateCategoryBody>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("no autorizado", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("bóveda bloqueada", "VAULT_LOCKED"),
            _ => ("error interno", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    // Fetch existing category to merge fields
    let vault = state.vault.lock().await;
    let cats = match vault.db.list_categories().await {
        Ok(c) => c,
        Err(e) => {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR")
                .into_response()
        }
    };

    let existing = match cats.into_iter().find(|c| c.cid == id) {
        Some(c) => c,
        None => {
            return err_json(StatusCode::NOT_FOUND, "category not found", "NOT_FOUND")
                .into_response()
        }
    };

    let new_name = body.name.unwrap_or(existing.name);
    let new_color = body.color.unwrap_or(existing.color);
    // None means "keep existing"; Some("") clears the description
    let new_description = match body.description {
        Some(d) if d.is_empty() => None,
        Some(d) => Some(d),
        None => existing.description,
    };

    if new_name.is_empty() {
        return err_validation("name", "must not be empty");
    }
    if new_name.len() > 100 {
        return err_validation("name", "must be 100 characters or fewer");
    }

    let updated = DbCategory { cid: id.clone(), name: new_name.clone(), color: new_color.clone(), description: new_description.clone() };
    match vault.db.update_category(&updated).await {
        Ok(true) => (
            StatusCode::OK,
            Json(CategoryResponse { id, name: new_name, color: new_color, description: new_description }),
        )
            .into_response(),
        Ok(false) => {
            err_json(StatusCode::NOT_FOUND, "category not found", "NOT_FOUND").into_response()
        }
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response(),
    }
}

async fn handle_delete_category(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("no autorizado", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("bóveda bloqueada", "VAULT_LOCKED"),
            _ => ("error interno", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let vault = state.vault.lock().await;
    match vault.db.delete_category(&id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => {
            err_json(StatusCode::NOT_FOUND, "category not found", "NOT_FOUND").into_response()
        }
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response(),
    }
}

async fn handle_list_commands(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Query(scope): Query<EnvScopeQuery>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("no autorizado", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("bóveda bloqueada", "VAULT_LOCKED"),
            _ => ("error interno", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let env = match resolve_scope(
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
    let allowed_ids = environment_item_ids(&env);

    let mode = match IncludeGlobal::parse(scope.include_global.as_deref()) {
        Ok(m) => m,
        Err(resp) => return resp,
    };

    let items = match decrypt_all_items(&state).await {
        Ok(i) => i,
        Err(StatusCode::FORBIDDEN) => {
            return err_json(StatusCode::FORBIDDEN, "bóveda bloqueada", "VAULT_LOCKED")
                .into_response()
        }
        Err(_) => {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, "error interno", "INTERNAL_ERROR")
                .into_response()
        }
    };

    let commands: Vec<ScopedCommand> = scope_items(items, &allowed_ids, mode)
        .into_iter()
        .filter(|s| s.item.item_type == "command")
        .map(|s| {
            let is_global = s.item.is_global.unwrap_or(false);
            let linked = s.linked;
            let item = s.item;
            let template = item.command.as_deref().unwrap_or("");
            let placeholders = extract_placeholders(template);
            ScopedCommand {
                detail: CommandDetail {
                    id: item.id,
                    name: item.name.unwrap_or_default(),
                    description: item.description,
                    shell: item.shell,
                    command: item.command,
                    placeholders,
                },
                is_global,
                linked,
            }
        })
        .collect();

    (StatusCode::OK, Json(commands)).into_response()
}

async fn handle_get_command(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("no autorizado", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("bóveda bloqueada", "VAULT_LOCKED"),
            _ => ("error interno", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let items = match decrypt_all_items(&state).await {
        Ok(i) => i,
        Err(StatusCode::FORBIDDEN) => {
            return err_json(StatusCode::FORBIDDEN, "bóveda bloqueada", "VAULT_LOCKED")
                .into_response()
        }
        Err(_) => {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, "error interno", "INTERNAL_ERROR")
                .into_response()
        }
    };

    let found = items
        .into_iter()
        .find(|item| item.item_type == "command" && item.id == id);

    match found {
        Some(item) => {
            let template = item.command.as_deref().unwrap_or("");
            let placeholders = extract_placeholders(template);
            let detail = CommandDetail {
                id: item.id,
                name: item.name.unwrap_or_default(),
                description: item.description,
                shell: item.shell,
                command: item.command,
                placeholders,
            };
            (StatusCode::OK, Json(detail)).into_response()
        }
        None => {
            err_json(StatusCode::NOT_FOUND, "comando no encontrado", "NOT_FOUND").into_response()
        }
    }
}

#[derive(Deserialize)]
struct RevealBody {
    confirm: Option<bool>,
}

async fn handle_reveal_item(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<RevealBody>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("no autorizado", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("bóveda bloqueada", "VAULT_LOCKED"),
            _ => ("error interno", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    if body.confirm != Some(true) {
        return err_json(
            StatusCode::BAD_REQUEST,
            "se requiere confirm: true",
            "BAD_REQUEST",
        )
        .into_response();
    }

    let items = match decrypt_all_items(&state).await {
        Ok(i) => i,
        Err(StatusCode::FORBIDDEN) => {
            return err_json(StatusCode::FORBIDDEN, "bóveda bloqueada", "VAULT_LOCKED")
                .into_response()
        }
        Err(_) => {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, "error interno", "INTERNAL_ERROR")
                .into_response()
        }
    };

    match items.into_iter().find(|item| item.id == id) {
        Some(item) => {
            eprintln!("[reveal] item#{} at {}", id, now_ts_str());

            let value = item
                .value
                .or(item.password)
                .or(item.content)
                .unwrap_or_default();

            (StatusCode::OK, Json(RevealResponse { value })).into_response()
        }
        None => err_json(StatusCode::NOT_FOUND, "item no encontrado", "NOT_FOUND").into_response(),
    }
}

async fn handle_get_settings(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        return err_json(code, "no autorizado", "UNAUTHORIZED").into_response();
    }
    let vault = state.vault.lock().await;
    let timeout = vault.db.get_setting("auto_lock_timeout").await
        .unwrap_or_default()
        .unwrap_or_else(|| "5".into());
    let hotkey = vault.db.get_setting("hotkey").await
        .unwrap_or_default()
        .unwrap_or_else(|| "Ctrl+Alt+Z".into());
    drop(vault);

    (StatusCode::OK, Json(serde_json::json!({
        "auto_lock_timeout": timeout.parse::<i64>().unwrap_or(5),
        "hotkey": hotkey,
    }))).into_response()
}

#[derive(Deserialize)]
struct UpdateSettingsBody {
    auto_lock_timeout: Option<i64>,
    hotkey: Option<String>,
}

async fn handle_put_settings(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(body): Json<UpdateSettingsBody>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        return err_json(code, "no autorizado", "UNAUTHORIZED").into_response();
    }
    let vault = state.vault.lock().await;
    if let Some(t) = body.auto_lock_timeout {
        if let Err(e) = vault.db.set_setting("auto_lock_timeout", &t.to_string()).await {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response();
        }
    }
    if let Some(h) = body.hotkey {
        if let Err(e) = vault.db.set_setting("hotkey", &h).await {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response();
        }
    }
    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}

// ─── /health ─────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct HealthResponse {
    version: &'static str,
    status: &'static str,
    vault_locked: bool,
    mcp_token_configured: bool,
}

async fn handle_health(State(state): State<Arc<ApiState>>) -> impl IntoResponse {
    let vault = state.vault.lock().await;
    let vault_locked = vault.key.is_none();
    let mcp_token_configured = vault
        .db
        .get_setting("mcp_token")
        .await
        .ok()
        .flatten()
        .map(|t| !t.is_empty())
        .unwrap_or(false);
    drop(vault);

    (
        StatusCode::OK,
        Json(HealthResponse {
            version: env!("CARGO_PKG_VERSION"),
            status: "running",
            vault_locked,
            mcp_token_configured,
        }),
    )
        .into_response()
}

// ─── TempEnvFile — RAII guard for secret-bearing files ───────────────────────
//
// Ensures that a file containing plaintext secrets is always zeroed and deleted
// when it goes out of scope — even on panic or early error return.
//
// Usage pattern:
//   let guard = TempEnvFile::create(path, content)?;
//   // ... any fallible work ...
//   let path = guard.persist(); // disarms the guard; caller now owns the file
//
// If `persist()` is never called (error path or panic), `Drop` wipes the file.

struct TempEnvFile {
    path: std::path::PathBuf,
    /// Byte length of the content written, used for the zero-overwrite pass.
    content_len: usize,
    /// Set to true by `persist()` to suppress cleanup in `Drop`.
    persisted: bool,
    /// Whether a file already existed at `path` before this guard committed
    /// to it (per `envfile::Committed::pre_existed`). Determines what
    /// `Drop` may safely do to it on the error path — see `Drop` below.
    pre_existed: bool,
    /// Path of the `.bak` copy `envfile::commit` created, if any — surfaced
    /// to the caller in the response so they know it exists.
    backup: Option<std::path::PathBuf>,
}

impl TempEnvFile {
    /// Delegates to `envfile::commit` for the existence gate, `.bak`
    /// backup and marker/permission policy, and records whether the path
    /// existed before this write so `Drop` knows whether it may delete the
    /// inode outright or must only zero its contents.
    ///
    /// This replaces the previous write-then-`chmod` ordering, where a
    /// failed `set_permissions` returned `Err` *after* the target had
    /// already been truncated, leaving a secret file at umask permissions
    /// with no guard armed. `envfile::commit` sets the mode at open time.
    ///
    /// On Windows, `%APPDATA%` and other per-user directories already
    /// inherit restrictive NTFS ACLs from their parent — only the owning
    /// user account and SYSTEM have access. The standard library provides
    /// no portable API for setting Windows ACLs from Rust, so no
    /// additional permission change is needed or applied there.
    fn create_guarded(
        path: std::path::PathBuf,
        content: &str,
        marker: &str,
        opts: &envfile::WriteOptions,
    ) -> Result<Self, envfile::EnvFileError> {
        let committed = envfile::commit(&path, content, marker, opts)?;
        Ok(Self {
            path,
            content_len: content.len(),
            persisted: false,
            pre_existed: committed.pre_existed,
            backup: committed.backup,
        })
    }

    /// Disarm the guard: the file will NOT be deleted on drop.
    /// Returns the path so the caller can report it.
    fn persist(mut self) -> std::path::PathBuf {
        self.persisted = true;
        // We consume `self` so Drop still runs, but the persisted flag prevents
        // any cleanup. Clone the path before consumption.
        self.path.clone()
    }
}

impl Drop for TempEnvFile {
    fn drop(&mut self) {
        if self.persisted {
            return;
        }
        // Overwrite with zeros first to hinder file-system recovery of secrets.
        // Use max(content_len, 1) so we always issue at least one write attempt
        // even if content_len is somehow zero. A zero pass is defense-in-depth,
        // not an erasure guarantee, on journaling/copy-on-write filesystems.
        let zeros = vec![0u8; self.content_len.max(1)];
        let _ = std::fs::write(&self.path, &zeros);

        if self.pre_existed {
            // The original bytes are already gone by this point (and
            // preserved in a `.bak` if the gate required one), so leaving
            // our plaintext on disk is the worse of the two failures — but
            // deleting a path we did not create would destroy the inode,
            // its permissions and its existence, which callers may depend
            // on. Truncate to zero length instead of removing it.
            let _ = std::fs::File::create(&self.path);
        } else {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

// ─── /fill ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct FillBody {
    template: String,
    /// When provided, write the filled .env directly to this exact path.
    /// The response will contain stats but not the secret content.
    output_path: Option<String>,
    /// When provided (and `output_path` is not), write to
    /// `{output_dir}/.env.<environment-name>` instead of returning content
    /// inline — the default-filename-with-environment-suffix convention.
    output_dir: Option<String>,
    /// If the resolved target exists and was not created by crypt-env,
    /// the write is refused with `409 TARGET_EXISTS` unless this is `true`
    /// — in which case the prior contents are copied to `<path>.bak` first.
    #[serde(default)]
    overwrite: bool,
}

#[derive(Serialize)]
struct FillResponse {
    /// Only present when neither output_path nor output_dir is specified.
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    /// Only present when writing to disk (output_path or output_dir).
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    injected: usize,
    not_found: usize,
    missing_keys: Vec<String>,
    /// Path of the `.bak` copy, present only when `overwrite: true` caused
    /// an existing unmanaged file to be backed up before being replaced.
    #[serde(skip_serializing_if = "Option::is_none")]
    backup: Option<String>,
}

async fn handle_fill(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Query(scope): Query<EnvScopeQuery>,
    Json(body): Json<FillBody>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let env = match resolve_scope(
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

    // Resolve and validate the write target *before* any decryption happens
    // (issue #7, objective 3): on rejection, no plaintext has been produced
    // for this request, on disk or in memory.
    //
    // `output_path` is exact caller intent for *this* request and is passed
    // through verbatim — validating it is issue #8's territory (clobber /
    // no-clobber), not this one. `output_dir` only ever decides the
    // *filename* inside it, and that filename is derived from the
    // environment's `name` — untrusted, persisted data — so it goes through
    // `fsguard::resolve_within`, which guarantees the result cannot land
    // outside the caller-supplied directory.
    let write_target: Option<PathBuf> = if let Some(out) = body.output_path.as_deref() {
        Some(PathBuf::from(out))
    } else if let Some(dir) = body.output_dir.as_deref() {
        match fsguard::resolve_within(dir, &format!(".env.{}", env.name)) {
            Ok(p) => Some(p),
            Err(fsguard::ContainmentError::BaseUnusable(msg)) => {
                return err_json(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("cannot use output directory: {msg}"),
                    "INTERNAL_ERROR",
                )
                .into_response();
            }
            Err(e) => {
                // Never echo the environment name or the resolved path (see
                // plan §4/D5) — the id is enough to identify the row, and a
                // rejection firing at all means a hostile name reached a
                // sink and layer 2 caught it.
                eprintln!(
                    "[api] /fill rejected: environment id {} — output_dir not contained ({e:?})",
                    env.id
                );
                return err_json(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "output_dir: environment name does not resolve to a path contained within the requested directory",
                    "PATH_NOT_CONTAINED",
                )
                .into_response();
            }
        }
    } else {
        None
    };

    let items = match decrypt_all_items(&state).await {
        Ok(i) => i,
        Err(StatusCode::FORBIDDEN) => {
            return err_json(StatusCode::FORBIDDEN, "vault locked", "VAULT_LOCKED")
                .into_response()
        }
        Err(_) => {
            return err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal error",
                "INTERNAL_ERROR",
            )
            .into_response()
        }
    };

    // Fill strictly from this environment's linked vars (key -> decrypted
    // value) — a template key that happens to match an item name elsewhere
    // in the vault, but isn't linked into this environment, is NOT filled.
    let items_by_id: HashMap<i64, VaultItem> = items.into_iter().map(|i| (i.id, i)).collect();
    let key_to_value: HashMap<String, String> = env
        .vars
        .iter()
        .filter_map(|v| {
            items_by_id.get(&v.item_id).map(|item| {
                let value = item
                    .value
                    .as_deref()
                    .or(item.password.as_deref())
                    .or(item.content.as_deref())
                    .unwrap_or("")
                    .to_string();
                (v.key.to_lowercase(), value)
            })
        })
        .collect();

    let mut new_lines: Vec<String> = Vec::new();
    let mut injected = 0usize;
    let mut not_found = 0usize;
    let mut missing_keys: Vec<String> = Vec::new();

    for line in body.template.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            new_lines.push(line.to_string());
            continue;
        }
        if let Some(eq_pos) = trimmed.find('=') {
            let key = &trimmed[..eq_pos];
            if !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '_') {
                let key_lower = key.to_lowercase();
                if let Some(value) = key_to_value.get(&key_lower) {
                    new_lines.push(format!("{key}={value}"));
                    injected += 1;
                    continue;
                } else {
                    // Not linked into this environment: preserve the
                    // original line untouched rather than blanking the
                    // value out — this is very likely a local-only value
                    // (port, feature flag, teammate-shared literal) that
                    // was never vault-managed, and blanking it is silent,
                    // irreversible data loss. Only report it as missing.
                    missing_keys.push(key.to_string());
                    not_found += 1;
                    new_lines.push(line.to_string());
                    continue;
                }
            }
        }
        new_lines.push(line.to_string());
    }

    let mut filled = new_lines.join("\n");
    if body.template.ends_with('\n') {
        filled.push('\n');
    }

    // When writing to disk: write via RAII guard, return stats only (no
    // secret content in the response). `write_target` was already resolved
    // and validated above, before decryption.
    //
    // The guard zeros and deletes the file if any error occurs before persist().
    // On success, persist() disarms the guard so the caller can consume the file.
    if let Some(path) = write_target {
        // Only the explicit `output_path` branch needs a directory created
        // here: it is exact caller intent with no interpolated name in it.
        // The `output_dir` branch's base was already created inside
        // `fsguard::resolve_within` above, on the caller-supplied directory
        // alone (issue #7, objective 4 — no `create_dir_all` ever sees a
        // path with an interpolated name in it).
        if body.output_path.is_some() {
            if let Some(parent) = path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return err_json(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        &format!("cannot create directory: {e}"),
                        "INTERNAL_ERROR",
                    )
                    .into_response();
                }
            }
        }

        let marker = build_marker(&state, &env).await;
        let opts = envfile::WriteOptions { overwrite: body.overwrite, mode: envfile::FileMode::Private0600 };
        let guard = match TempEnvFile::create_guarded(path, &filled, &marker, &opts) {
            Ok(g) => g,
            Err(e) => return err_envfile(e),
        };
        let backup = guard.backup.as_ref().map(|p| p.to_string_lossy().into_owned());
        // Disarm: caller is now responsible for the file.
        let final_path = guard.persist();
        return (
            StatusCode::OK,
            Json(FillResponse {
                content: None,
                path: Some(final_path.to_string_lossy().into_owned()),
                injected,
                not_found,
                missing_keys,
                backup,
            }),
        )
            .into_response();
    }

    // Neither output_path nor output_dir: return the content inline (CLI / programmatic use).
    (
        StatusCode::OK,
        Json(FillResponse {
            content: Some(filled),
            path: None,
            injected,
            not_found,
            missing_keys,
            backup: None,
        }),
    )
        .into_response()
}

// ─── Share handlers ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ShareListenBody {
    items: Vec<i64>,
}

#[derive(Serialize)]
struct ShareListenResponse {
    pairing_code: String,
    fingerprint: Option<String>,
    expires_in: u64,
    status: &'static str,
}

async fn handle_share_listen(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Query(scope): Query<EnvScopeQuery>,
    Json(body): Json<ShareListenBody>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    if body.items.is_empty() {
        return err_json(StatusCode::UNPROCESSABLE_ENTITY, "items list must not be empty", "VALIDATION_ERROR").into_response();
    }

    // Resolve the project+environment this share is scoped to. Sharing is
    // restricted to items already linked into that environment's vars — the
    // same scoping rule GET/POST /items and GET /commands apply — so a
    // sender can only ever share what already belongs to the project they
    // said they were sharing from.
    let env = match resolve_scope(
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
    let allowed_ids = environment_item_ids(&env);
    if let Some(bad_id) = body.items.iter().find(|id| !allowed_ids.contains(id)) {
        return err_validation(
            "items",
            &format!("item {bad_id} is not linked to environment '{}'", env.name),
        );
    }

    // Extract vault key
    let vault_key: [u8; 32] = {
        let guard = state.vault.lock().await;
        match guard.key.as_ref() {
            Some(k) => **k,
            None => return err_json(StatusCode::FORBIDDEN, "vault locked", "VAULT_LOCKED").into_response(),
        }
    };

    let pairing_code = match crate::share::start_listen_session(
        state.share.clone(),
        body.items,
        vault_key,
        state.vault.clone(),
        Some(env.project_id),
        Some(env.id),
    )
    .await
    {
        Ok(code) => code,
        Err(e) => {
            return err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
                "INTERNAL_ERROR",
            )
            .into_response()
        }
    };

    (
        StatusCode::OK,
        Json(ShareListenResponse {
            pairing_code,
            fingerprint: None,
            expires_in: 300,
            status: "listening",
        }),
    )
        .into_response()
}

#[derive(Deserialize)]
struct ShareConnectBody {
    pairing_code: String,
}

#[derive(Serialize)]
struct ShareConnectResponse {
    fingerprint: String,
    status: &'static str,
}

async fn handle_share_connect(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Query(scope): Query<EnvScopeQuery>,
    Json(body): Json<ShareConnectBody>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    // Resolve the project+environment received items should land in — see
    // `share::import_plain_items_into_vault`, which owns and links each
    // imported item into this environment once the transfer completes.
    let env = match resolve_scope(
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

    let vault_key: [u8; 32] = {
        let guard = state.vault.lock().await;
        match guard.key.as_ref() {
            Some(k) => **k,
            None => return err_json(StatusCode::FORBIDDEN, "vault locked", "VAULT_LOCKED").into_response(),
        }
    };

    let fingerprint = match crate::share::connect_to_peer(
        state.share.clone(),
        body.pairing_code,
        vault_key,
        state.vault.clone(),
        Some(env.project_id),
        Some(env.id),
    )
    .await
    {
        Ok(fp) => fp,
        Err(e) => {
            return err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
                "INTERNAL_ERROR",
            )
            .into_response()
        }
    };

    (
        StatusCode::OK,
        Json(ShareConnectResponse {
            fingerprint,
            status: "awaiting_confirmation",
        }),
    )
        .into_response()
}

#[derive(Deserialize)]
struct ShareConfirmBody {
    confirmed: bool,
}

#[derive(Serialize)]
struct ShareConfirmResponse {
    status: String,
}

async fn handle_share_confirm(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(body): Json<ShareConfirmBody>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    match crate::share::confirm_fingerprint(&state.share, body.confirmed).await {
        Ok(()) => {
            let status = if body.confirmed { "active" } else { "cancelled" };
            (StatusCode::OK, Json(ShareConfirmResponse { status: status.to_string() })).into_response()
        }
        Err(e) => err_json(StatusCode::BAD_REQUEST, &e.to_string(), "BAD_REQUEST").into_response(),
    }
}

#[derive(Serialize)]
struct ShareStatusResponse {
    state: String,
    fingerprint: Option<String>,
    direction: Option<String>,
    received_names: Option<Vec<String>>,
    /// Received names that were NOT linked into the target environment
    /// because their key already had a different item linked there — see
    /// `share::ImportOutcome`. Same availability rule as `received_names`.
    skipped_keys: Option<Vec<String>>,
}

async fn handle_share_status(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let guard = state.share.session.lock().await;
    match guard.as_ref() {
        None => (
            StatusCode::OK,
            Json(ShareStatusResponse {
                state: "none".to_string(),
                fingerprint: None,
                direction: None,
                received_names: None,
                skipped_keys: None,
            }),
        )
            .into_response(),
        Some(s) => {
            let state_str = match &s.state {
                ShareSessionState::Failed(msg) => format!("failed: {msg}"),
                other => other.as_str().to_string(),
            };
            let done_receiving = s.state == ShareSessionState::Done && s.direction == crate::share::ShareDirection::Receiving;
            let received = if done_receiving { Some(s.received_names.clone()) } else { None };
            let skipped = if done_receiving { Some(s.skipped_keys.clone()) } else { None };
            (
                StatusCode::OK,
                Json(ShareStatusResponse {
                    state: state_str,
                    fingerprint: s.fingerprint.clone(),
                    direction: Some(s.direction.as_str().to_string()),
                    received_names: received,
                    skipped_keys: skipped,
                }),
            )
                .into_response()
        }
    }
}

#[derive(Serialize)]
struct ShareCancelResponse {
    cancelled: bool,
}

async fn handle_share_cancel(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    match crate::share::cancel_session(&state.share).await {
        Ok(()) => (StatusCode::OK, Json(ShareCancelResponse { cancelled: true })).into_response(),
        Err(e) => err_json(StatusCode::BAD_REQUEST, &e.to_string(), "BAD_REQUEST").into_response(),
    }
}

#[derive(Deserialize)]
struct ShareExportBody {
    items: Vec<i64>,
    output_path: String,
}

#[derive(Serialize)]
struct ShareExportResponse {
    passphrase: String,
    path: String,
}

async fn handle_share_export(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(body): Json<ShareExportBody>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    if body.items.is_empty() {
        return err_json(StatusCode::UNPROCESSABLE_ENTITY, "items list must not be empty", "VALIDATION_ERROR").into_response();
    }

    let output_path = std::path::PathBuf::from(&body.output_path);
    if let Some(parent) = output_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("cannot create output directory: {e}"),
                "INTERNAL_ERROR",
            )
            .into_response();
        }
    }

    match crate::share::export_package(&body.items, &output_path, &state.vault).await {
        Ok(passphrase) => (
            StatusCode::OK,
            Json(ShareExportResponse {
                passphrase,
                path: body.output_path,
            }),
        )
            .into_response(),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string(), "INTERNAL_ERROR")
            .into_response(),
    }
}

#[derive(Deserialize)]
struct ShareImportBody {
    path: String,
    passphrase: String,
}

#[derive(Serialize)]
struct ShareImportResponse {
    imported: usize,
    item_names: Vec<String>,
    /// Names imported but not linked into the environment because their key
    /// was already linked to a different item there, or was unsafe (empty /
    /// contained `=` or a newline) — see `share::ImportOutcome`.
    skipped_keys: Vec<String>,
}

async fn handle_share_import(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Query(scope): Query<EnvScopeQuery>,
    Json(body): Json<ShareImportBody>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    // Imported items must land in a resolvable project+environment, same as
    // /share/connect and /relay/receive, so they're actually findable
    // afterwards via GET /items / search / MCP instead of only by guessing
    // their numeric id.
    let env = match resolve_scope(
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

    let path = std::path::PathBuf::from(&body.path);
    match crate::share::import_package(&path, &body.passphrase, &state.vault, Some((env.project_id, env.id))).await {
        Ok(outcome) => {
            let count = outcome.names.len();
            (
                StatusCode::OK,
                Json(ShareImportResponse {
                    imported: count,
                    item_names: outcome.names,
                    skipped_keys: outcome.skipped_keys,
                }),
            )
                .into_response()
        }
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string(), "INTERNAL_ERROR")
            .into_response(),
    }
}

// ─── Workspace handlers ───────────────────────────────────────────────────────

async fn handle_list_projects(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let vault = state.vault.lock().await;
    match project::list_projects(&vault.db).await {
        Ok(projects) => (StatusCode::OK, Json(projects)).into_response(),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response(),
    }
}

async fn handle_save_project(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(body): Json<ProjectInput>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    if body.name.is_empty() {
        return err_json(StatusCode::UNPROCESSABLE_ENTITY, "name: required and must not be empty", "VALIDATION_ERROR")
            .into_response();
    }

    let is_new = body.id == 0;
    let vault = state.vault.lock().await;
    match project::save_project(&vault.db, body).await {
        Ok(id) => {
            let status = if is_new { StatusCode::CREATED } else { StatusCode::OK };
            (status, Json(serde_json::json!({ "id": id }))).into_response()
        }
        // A concurrent request may have already created a project with the
        // same case-insensitive name (enforced by the DB's unique index,
        // surfaced here via the stable `"conflict:"` sentinel from
        // `db::upsert_project` — never sqlx's own error text, which is not a
        // stable string and would leak SQL identifiers on any mismatch) —
        // report it as a distinguishable conflict so callers like the CLI's
        // auto-create fallback can re-fetch and reuse the existing project
        // instead of treating this as a hard failure.
        Err(e) if e.starts_with("conflict:") => {
            err_json(StatusCode::CONFLICT, "a project with this name already exists", "CONFLICT")
                .into_response()
        }
        // `project::save_project` runs `validate_project_name` as its first
        // statement (issue #7's choke point) and prefixes its message with
        // "name: " on failure — surface that as a caller error, not a
        // server fault. Safe to echo: `validate_project_name` returns the
        // rule that was broken and never the input `name` itself.
        Err(e) if e.starts_with("name: ") => {
            err_json(StatusCode::UNPROCESSABLE_ENTITY, &e, "VALIDATION_ERROR").into_response()
        }
        // Never echo `e` here: on any other failure it may carry raw sqlx/SQL
        // text (table, column, index names), which CLAUDE.md forbids in an
        // API response. Log the detail server-side instead — never the
        // input `name`, never any var value.
        Err(e) => {
            eprintln!("handle_save_project: {e}");
            err_json(StatusCode::INTERNAL_SERVER_ERROR, "internal error", "INTERNAL_ERROR").into_response()
        }
    }
}

async fn handle_delete_project(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let vault = state.vault.lock().await;

    let projects = match project::list_projects(&vault.db).await {
        Ok(p) => p,
        Err(e) => {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response()
        }
    };
    if projects.iter().find(|p| p.id == id).is_none() {
        return err_json(StatusCode::NOT_FOUND, "project not found", "NOT_FOUND").into_response();
    }

    match project::delete_project(&vault.db, id).await {
        Ok(impact) => (StatusCode::OK, Json(impact)).into_response(),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response(),
    }
}

async fn handle_preview_delete_project(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let vault = state.vault.lock().await;
    match project::project_delete_preview(&vault.db, id).await {
        Ok(impact) => (StatusCode::OK, Json(impact)).into_response(),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response(),
    }
}

async fn handle_save_environment(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(body): Json<EnvironmentInput>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    if body.name.is_empty() {
        return err_json(StatusCode::UNPROCESSABLE_ENTITY, "name: required and must not be empty", "VALIDATION_ERROR")
            .into_response();
    }
    if body.project_id <= 0 {
        return err_validation("projectId", "required and must reference an existing project");
    }

    let is_new = body.id == 0;
    let vault = state.vault.lock().await;

    match project::list_projects(&vault.db).await {
        Ok(projects) => {
            if !projects.iter().any(|p| p.id == body.project_id) {
                return err_validation("projectId", "must reference an existing project");
            }
        }
        Err(e) => return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response(),
    }

    match project::save_environment(&vault.db, body).await {
        Ok(id) => {
            let status = if is_new { StatusCode::CREATED } else { StatusCode::OK };
            (status, Json(serde_json::json!({ "id": id }))).into_response()
        }
        // `project::save_environment` runs `validate_environment_name` as
        // its first statement (issue #7's choke point — the same check the
        // Tauri command, CLI, and an imported `.cryptenv-proj` template all
        // go through) and prefixes its message with "name: " on failure.
        // Safe to echo: the message names the rule, never the input `name`.
        Err(e) if e.starts_with("name: ") => {
            err_json(StatusCode::UNPROCESSABLE_ENTITY, &e, "VALIDATION_ERROR").into_response()
        }
        // Same "conflict:" sentinel contract as `handle_save_project` — set
        // either by `db::upsert_environment`'s unique-index violation (ASCII
        // case) or `project::ensure_no_case_collision`'s app-level
        // Unicode-aware pre-check (non-ASCII case). Both return the exact
        // same string, so one match arm covers both layers.
        Err(e) if e.starts_with("conflict:") => err_json(
            StatusCode::CONFLICT,
            "an environment with this name already exists in this project",
            "CONFLICT",
        )
        .into_response(),
        // Never echo `e`: on any other failure it may carry raw sqlx/SQL text
        // (table, column, index names), which CLAUDE.md forbids in an API
        // response. Log the detail server-side instead — never the input
        // `name`, never any var value.
        Err(e) => {
            eprintln!("handle_save_environment: {e}");
            err_json(StatusCode::INTERNAL_SERVER_ERROR, "internal error", "INTERNAL_ERROR").into_response()
        }
    }
}

async fn handle_delete_environment(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let vault = state.vault.lock().await;
    match project::delete_environment(&vault.db, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response(),
    }
}

/// Body for `POST /environments/:id/inject`. Both fields optional — an empty
/// body (`{}`) preserves the pre-existing behavior of writing to exactly the
/// environment's configured `paths[]`. Mirrors `/fill`'s filename resolution
/// (see `FillBody`): `output_path` is an explicit exact path (added to
/// `paths[]`, never replacing it); `output_dir` is only consulted when
/// `paths[]` is empty and no `output_path` was given, writing a single
/// `{output_dir}/.env.<environment-name>` file.
#[derive(Deserialize, Default)]
#[serde(default)]
struct InjectBody {
    output_path: Option<String>,
    output_dir: Option<String>,
    /// If any caller-supplied path (`output_path`/`output_dir`-derived) is
    /// `Foreign`, the write is refused with `409 TARGET_EXISTS` unless this
    /// is `true` — in which case the prior contents are copied to
    /// `<path>.bak` first. Owner-configured `environment.paths[]` are never
    /// gated by this flag (see `project::inject_environment`'s doc comment).
    overwrite: bool,
}

async fn handle_inject_environment(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<InjectBody>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let vault = state.vault.lock().await;

    let vault_key: [u8; 32] = match vault.key.as_ref() {
        Some(k) => **k,
        None => {
            return err_json(StatusCode::FORBIDDEN, "vault locked", "VAULT_LOCKED").into_response()
        }
    };

    match project::inject_environment(&vault.db, &vault_key, id, body.output_path, body.output_dir, body.overwrite).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) if e == "environment not found" => {
            err_json(StatusCode::NOT_FOUND, &e, "NOT_FOUND").into_response()
        }
        Err(e) if e == "environment has no paths configured" => {
            err_json(StatusCode::UNPROCESSABLE_ENTITY, &e, "VALIDATION_ERROR").into_response()
        }
        Err(e) if e.starts_with(envfile::TARGET_EXISTS_PREFIX) => {
            err_json(StatusCode::CONFLICT, &e, "TARGET_EXISTS").into_response()
        }
        Err(e) if e.starts_with(envfile::BACKUP_EXISTS_PREFIX) => {
            err_json(StatusCode::CONFLICT, &e, "BACKUP_EXISTS").into_response()
        }
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response(),
    }
}

// ─── /tokens (management: create, list, revoke) ────────────────────────────
//
// Unlike `/token/vars` below, managing tokens is a vault-owner operation —
// gated by the normal full-session `verify_token`, same as `/projects` or
// `/items`.

async fn handle_create_token(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(body): Json<tokens::CreateTokenInput>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let vault = state.vault.lock().await;
    let vault_key: [u8; 32] = match vault.key.as_ref() {
        Some(k) => **k,
        None => return err_json(StatusCode::FORBIDDEN, "vault locked", "VAULT_LOCKED").into_response(),
    };

    match tokens::create_token(&vault.db, &vault_key, body).await {
        Ok(created) => (StatusCode::CREATED, Json(created)).into_response(),
        Err(e) if e == "environment not found" => {
            err_json(StatusCode::NOT_FOUND, &e, "NOT_FOUND").into_response()
        }
        Err(e) => err_json(StatusCode::UNPROCESSABLE_ENTITY, &e, "VALIDATION_ERROR").into_response(),
    }
}

async fn handle_list_tokens(State(state): State<Arc<ApiState>>, headers: HeaderMap) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let vault = state.vault.lock().await;
    match tokens::list_tokens(&vault.db).await {
        Ok(list) => (StatusCode::OK, Json(list)).into_response(),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response(),
    }
}

async fn handle_revoke_token(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let vault = state.vault.lock().await;
    match tokens::revoke_token(&vault.db, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response(),
    }
}

// ─── /token/vars ────────────────────────────────────────────────────────────
//
// Auth for this route is deliberately NOT `verify_token`: a scoped access
// token (`crate::tokens`) is a different, narrower principal than the
// full-vault session/MCP tokens `verify_token` checks. It never sees this
// endpoint's sibling routes — no `/items`, no `/projects`, nothing that
// could enumerate the vault — only the single environment it was cut for.

#[derive(Serialize)]
struct TokenVarsResponse {
    #[serde(rename = "environmentId")]
    environment_id: i64,
    vars: HashMap<String, String>,
}

async fn handle_token_vars(State(state): State<Arc<ApiState>>, headers: HeaderMap) -> impl IntoResponse {
    let provided = match headers.get("x-access-token").and_then(|v| v.to_str().ok()) {
        Some(t) => t.to_string(),
        None => {
            return err_json(StatusCode::UNAUTHORIZED, "missing X-Access-Token header", "UNAUTHORIZED")
                .into_response()
        }
    };

    let vault = state.vault.lock().await;
    // Captured before `authenticate` so a `session`-mode token can be
    // checked against it — a `standalone`-mode token ignores this and
    // unwraps its own key instead (see `tokens::authenticate`).
    let session_key: Option<[u8; 32]> = vault.key.as_ref().map(|k| **k);

    let (token_row, env_key) = match tokens::authenticate(&vault.db, &provided, session_key.as_ref()).await {
        Ok(v) => v,
        Err(tokens::TokenAuthError::NotFound) => {
            return err_json(StatusCode::UNAUTHORIZED, "invalid token", "UNAUTHORIZED").into_response()
        }
        Err(tokens::TokenAuthError::Revoked) => {
            return err_json(StatusCode::UNAUTHORIZED, "token revoked", "UNAUTHORIZED").into_response()
        }
        Err(tokens::TokenAuthError::Expired) => {
            return err_json(StatusCode::UNAUTHORIZED, "token expired", "UNAUTHORIZED").into_response()
        }
        Err(tokens::TokenAuthError::SessionRequired) => {
            return err_json(
                StatusCode::FORBIDDEN,
                "this token requires an unlocked vault (session-bound mode)",
                "VAULT_LOCKED",
            )
            .into_response()
        }
        Err(tokens::TokenAuthError::Corrupt(e)) => {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response()
        }
    };

    match project::resolve_environment_values(&vault.db, &env_key, token_row.environment_id).await {
        Ok(vars) => (
            StatusCode::OK,
            Json(TokenVarsResponse { environment_id: token_row.environment_id, vars }),
        )
            .into_response(),
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response(),
    }
}

// ─── /environments/:id/example ────────────────────────────────────────────────
//
// Generates a `.env`-shaped file listing an environment's variable KEYS with
// empty values — safe to commit to source control, since it never touches
// (let alone decrypts) the referenced items' actual secret values.

/// Mirrors `InjectBody`'s dual write mode: explicit `output_path`, or
/// `output_dir` (written as `{output_dir}/.env.example.<environment-name>`),
/// or neither (content returned inline — still just placeholders, never
/// secrets, so inline return carries no risk here).
#[derive(Deserialize, Default)]
#[serde(default)]
struct ExampleBody {
    output_path: Option<String>,
    output_dir: Option<String>,
    /// See `FillBody::overwrite` / `InjectBody::overwrite` — same policy.
    overwrite: bool,
}

#[derive(Serialize)]
struct ExampleResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    keys: Vec<String>,
    /// Path of the `.bak` copy, present only when `overwrite: true` caused
    /// an existing unmanaged file to be backed up before being replaced.
    #[serde(skip_serializing_if = "Option::is_none")]
    backup: Option<String>,
}

async fn handle_environment_example(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<ExampleBody>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    let env = {
        let vault = state.vault.lock().await;
        match project::resolve_environment(&vault.db, Some(id), None, None).await {
            Ok(e) => e,
            Err(_) => {
                return err_json(StatusCode::NOT_FOUND, "environment not found", "NOT_FOUND")
                    .into_response()
            }
        }
    };

    let mut keys: Vec<String> = env.vars.iter().map(|v| v.key.clone()).collect();
    keys.sort();

    // Placeholder content only — every value is empty, the actual item
    // values are never read or decrypted here.
    let content = keys.iter().map(|k| format!("{k}=")).collect::<Vec<_>>().join("\n") + "\n";

    // Same containment split as `/fill` (issue #7): `output_path` is exact
    // caller intent, passed through verbatim; `output_dir` only picks the
    // filename inside it, and that filename is derived from the untrusted,
    // persisted environment `name`, so it goes through `fsguard`. Content
    // here is placeholder keys only (never decrypted), so there is no
    // decrypt-ordering concern like `/fill`'s objective 3 — but the code
    // shape is kept the same so the two sinks stay diff-comparable.
    let write_target: Option<PathBuf> = if let Some(p) = body.output_path.as_deref() {
        Some(PathBuf::from(p))
    } else if let Some(dir) = body.output_dir.as_deref() {
        match fsguard::resolve_within(dir, &format!(".env.example.{}", env.name)) {
            Ok(p) => Some(p),
            Err(fsguard::ContainmentError::BaseUnusable(msg)) => {
                return err_json(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("cannot use output directory: {msg}"),
                    "INTERNAL_ERROR",
                )
                .into_response();
            }
            Err(e) => {
                eprintln!(
                    "[api] /environments/{{id}}/example rejected: environment id {} — output_dir not contained ({e:?})",
                    env.id
                );
                return err_json(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "output_dir: environment name does not resolve to a path contained within the requested directory",
                    "PATH_NOT_CONTAINED",
                )
                .into_response();
            }
        }
    } else {
        None
    };

    if let Some(path) = write_target {
        if body.output_path.is_some() {
            if let Some(parent) = path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return err_json(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        &format!("cannot create directory: {e}"),
                        "INTERNAL_ERROR",
                    )
                    .into_response();
                }
            }
        }
        // No RAII zero-wipe needed here — the content holds no secret value,
        // only key names. It still gets the same existence gate and marker
        // as `/fill` and `/inject`: an unmanaged file at this path is just
        // as real a target for silent clobbering as a secret-bearing one.
        // `FileMode::Inherit` (not `Private0600`) because this file exists
        // to be committed and shared — forcing owner-only permissions on a
        // `.env.example` would be surprising on a shared build machine.
        //
        // `path` is already containment-checked above (issue #7): the
        // `output_dir` branch routes through `fsguard::resolve_within`, so
        // the reported path is the post-canonicalization one.
        let marker = build_marker(&state, &env).await;
        let opts = envfile::WriteOptions { overwrite: body.overwrite, mode: envfile::FileMode::Inherit };
        let committed = match envfile::commit(&path, &content, &marker, &opts) {
            Ok(c) => c,
            Err(e) => return err_envfile(e),
        };
        let backup = committed.backup.map(|p| p.to_string_lossy().into_owned());
        let resolved = path.to_string_lossy().into_owned();
        return (StatusCode::OK, Json(ExampleResponse { content: None, path: Some(resolved), keys, backup })).into_response();
    }

    (StatusCode::OK, Json(ExampleResponse { content: Some(content), path: None, keys, backup: None })).into_response()
}

// ─── Relay handlers ───────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct RelaySendBody {
    item_ids: Vec<i64>,
}

#[derive(serde::Serialize)]
struct RelaySendResponse {
    code: String,
    passphrase: String,
}

async fn handle_relay_send(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(body): Json<RelaySendBody>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    if body.item_ids.is_empty() {
        return err_json(
            StatusCode::UNPROCESSABLE_ENTITY,
            "item_ids must not be empty",
            "VALIDATION_ERROR",
        )
        .into_response();
    }

    // Extract everything needed while holding the lock, then release it before
    // the blocking relay upload.
    let (_vault_key, supabase_url, anon_key, plain_items) = {
        let vault = state.vault.lock().await;
        let k = match vault.key.as_ref() {
            Some(k) => **k,
            None => {
                return err_json(StatusCode::FORBIDDEN, "vault locked", "VAULT_LOCKED")
                    .into_response()
            }
        };

        let supabase_url = match vault.db.get_setting("relay_supabase_url").await {
            Ok(Some(u)) => u,
            Ok(None) => {
                return err_json(
                    StatusCode::BAD_REQUEST,
                    "relay not configured: set relay_supabase_url in Settings",
                    "NOT_CONFIGURED",
                )
                .into_response()
            }
            Err(e) => {
                return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR")
                    .into_response()
            }
        };

        let anon_key = match vault.db.get_setting("relay_supabase_anon_key").await {
            Ok(Some(k)) => k,
            Ok(None) => {
                return err_json(
                    StatusCode::BAD_REQUEST,
                    "relay not configured: set relay_supabase_anon_key in Settings",
                    "NOT_CONFIGURED",
                )
                .into_response()
            }
            Err(e) => {
                return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR")
                    .into_response()
            }
        };

        let raw = match vault.db.list_items().await {
            Ok(r) => r,
            Err(e) => {
                return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR")
                    .into_response()
            }
        };

        let mut items: Vec<PlainItem> = Vec::new();
        for (id, _, data, _, _) in &raw {
            if !body.item_ids.contains(id) {
                continue;
            }
            let json = match crypto::decrypt(&k, data) {
                Ok(j) => j,
                Err(_) => continue,
            };
            let item: VaultItem = match serde_json::from_slice(&json) {
                Ok(i) => i,
                Err(_) => continue,
            };
            items.push(PlainItem {
                item_type: item.item_type,
                name: item.name.unwrap_or_default(),
                value: item.value,
                username: item.username,
                password: item.password,
                url: item.url,
                notes: item.notes,
                category: item.categories.and_then(|v| v.into_iter().next()),
                command: item.command,
            });
        }

        (k, supabase_url, anon_key, items)
    };

    let code = relay::generate_share_code();
    let passphrase = crate::share::crypto::generate_passphrase();

    let relay_key = match relay::derive_relay_key(&code, &passphrase) {
        Ok(k) => k,
        Err(e) => {
            return err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
                "INTERNAL_ERROR",
            )
            .into_response()
        }
    };

    let payload = match relay::encrypt_items(&plain_items, &relay_key) {
        Ok(p) => p,
        Err(e) => {
            return err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
                "INTERNAL_ERROR",
            )
            .into_response()
        }
    };

    let code_clone = code.clone();
    let upload_result = tokio::task::spawn_blocking(move || {
        relay::relay_upload(&supabase_url, &anon_key, &code_clone, &payload)
    })
    .await;

    match upload_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            return err_json(
                StatusCode::BAD_GATEWAY,
                &format!("relay upload failed: {e}"),
                "RELAY_ERROR",
            )
            .into_response()
        }
        Err(e) => {
            return err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("task error: {e}"),
                "INTERNAL_ERROR",
            )
            .into_response()
        }
    }

    (StatusCode::OK, Json(RelaySendResponse { code, passphrase })).into_response()
}

#[derive(serde::Deserialize)]
struct RelayReceiveBody {
    code: String,
    passphrase: String,
}

#[derive(serde::Serialize)]
struct RelayReceiveResponse {
    names: Vec<String>,
    /// Names imported but not linked into the environment because their key
    /// was already linked to a different item there, or was unsafe (empty /
    /// contained `=` or a newline) — see `share::ImportOutcome`.
    skipped_keys: Vec<String>,
}

async fn handle_relay_receive(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Query(scope): Query<EnvScopeQuery>,
    Json(body): Json<RelayReceiveBody>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    if body.code.is_empty() || body.passphrase.is_empty() {
        return err_json(
            StatusCode::UNPROCESSABLE_ENTITY,
            "code and passphrase are required",
            "VALIDATION_ERROR",
        )
        .into_response();
    }

    // Imported items must land in a resolvable project+environment, same as
    // /share/connect and /share/import, so they're actually findable
    // afterwards via GET /items / search / MCP instead of only by guessing
    // their numeric id.
    let env = match resolve_scope(
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

    // Derive relay key before spawning blocking tasks (no I/O needed)
    let relay_key = match relay::derive_relay_key(&body.code, &body.passphrase) {
        Ok(k) => k,
        Err(e) => {
            return err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
                "INTERNAL_ERROR",
            )
            .into_response()
        }
    };

    // Extract relay settings while holding the lock
    let (vault_key, supabase_url, anon_key) = {
        let vault = state.vault.lock().await;
        let k = match vault.key.as_ref() {
            Some(k) => **k,
            None => {
                return err_json(StatusCode::FORBIDDEN, "vault locked", "VAULT_LOCKED")
                    .into_response()
            }
        };

        let supabase_url = match vault.db.get_setting("relay_supabase_url").await {
            Ok(Some(u)) => u,
            Ok(None) => {
                return err_json(
                    StatusCode::BAD_REQUEST,
                    "relay not configured: set relay_supabase_url in Settings",
                    "NOT_CONFIGURED",
                )
                .into_response()
            }
            Err(e) => {
                return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR")
                    .into_response()
            }
        };

        let anon_key_val = match vault.db.get_setting("relay_supabase_anon_key").await {
            Ok(Some(k)) => k,
            Ok(None) => {
                return err_json(
                    StatusCode::BAD_REQUEST,
                    "relay not configured: set relay_supabase_anon_key in Settings",
                    "NOT_CONFIGURED",
                )
                .into_response()
            }
            Err(e) => {
                return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR")
                    .into_response()
            }
        };

        (k, supabase_url, anon_key_val)
    };

    // Download the relay payload (blocking)
    let url_clone = supabase_url.clone();
    let key_clone = anon_key.clone();
    let code_clone = body.code.clone();
    let download_result = tokio::task::spawn_blocking(move || {
        relay::relay_download(&url_clone, &key_clone, &code_clone)
    })
    .await;

    let payload = match download_result {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            return err_json(
                StatusCode::BAD_GATEWAY,
                &format!("relay download failed: {e}"),
                "RELAY_ERROR",
            )
            .into_response()
        }
        Err(e) => {
            return err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("task error: {e}"),
                "INTERNAL_ERROR",
            )
            .into_response()
        }
    };

    let plain_items = match relay::decrypt_payload(&payload, &relay_key) {
        Ok(items) => items,
        Err(e) => {
            return err_json(
                StatusCode::UNPROCESSABLE_ENTITY,
                &format!("decrypt failed (wrong passphrase?): {e}"),
                "DECRYPT_ERROR",
            )
            .into_response()
        }
    };

    // Delete after first use (burn-after-read, best-effort)
    let url_clone2 = supabase_url.clone();
    let key_clone2 = anon_key.clone();
    let code_clone2 = body.code.clone();
    let _ = tokio::task::spawn_blocking(move || {
        relay::relay_delete(&url_clone2, &key_clone2, &code_clone2)
    })
    .await;

    // Import items into the vault, owned by and linked into the resolved
    // project/environment — reuses the exact same helper LAN share and
    // `.vault` package import use, instead of duplicating item-creation +
    // linking logic here.
    let vault = state.vault.lock().await;
    let outcome = match crate::share::import_plain_items_into_vault(
        &plain_items,
        &vault_key,
        &vault.db,
        Some((env.project_id, env.id)),
    )
    .await
    {
        Ok(o) => o,
        Err(e) => {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string(), "INTERNAL_ERROR")
                .into_response()
        }
    };

    (
        StatusCode::OK,
        Json(RelayReceiveResponse { names: outcome.names, skipped_keys: outcome.skipped_keys }),
    )
        .into_response()
}

// ─── Project relay handlers (issue #4 — share a whole project) ───────────────
// Replaces the deleted `/workspaces/*/relay/*` pair: same relay transport,
// but the payload is a `ProjectBundle` (structure + values for N
// environments at once) instead of a single flat item list or a
// frozen-table-backed workspace. See `project::relay` for the orchestration
// (build/receive) and `share::relay::ProjectBundle` for the wire format.

#[derive(serde::Deserialize)]
struct ProjectRelaySendBody {
    environment_ids: Vec<i64>,
}

#[derive(serde::Serialize)]
struct ProjectRelaySendResponse {
    code: String,
    passphrase: String,
    project: String,
    environment_count: usize,
    item_count: usize,
}

/// Share a whole project (selected environments, structure + decrypted
/// values, deduped items) via the relay. `environment_ids` is the sender's
/// explicit selection — the GUI/CLI default non-default environments to
/// unchecked (D4), but that's a client-side safety default, not enforced
/// here; this endpoint shares exactly what it's asked to.
async fn handle_project_relay_send(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<ProjectRelaySendBody>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    if body.environment_ids.is_empty() {
        return err_json(
            StatusCode::UNPROCESSABLE_ENTITY,
            "environment_ids must not be empty",
            "VALIDATION_ERROR",
        )
        .into_response();
    }

    let (supabase_url, anon_key, bundle) = {
        let vault = state.vault.lock().await;
        let k = match vault.key.as_ref() {
            Some(k) => **k,
            None => {
                return err_json(StatusCode::FORBIDDEN, "vault locked", "VAULT_LOCKED")
                    .into_response()
            }
        };

        let supabase_url = match vault.db.get_setting("relay_supabase_url").await {
            Ok(Some(u)) => u,
            Ok(None) => {
                return err_json(
                    StatusCode::BAD_REQUEST,
                    "relay not configured: set relay_supabase_url in Settings",
                    "NOT_CONFIGURED",
                )
                .into_response()
            }
            Err(e) => {
                return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR")
                    .into_response()
            }
        };
        let anon_key = match vault.db.get_setting("relay_supabase_anon_key").await {
            Ok(Some(k)) => k,
            Ok(None) => {
                return err_json(
                    StatusCode::BAD_REQUEST,
                    "relay not configured: set relay_supabase_anon_key in Settings",
                    "NOT_CONFIGURED",
                )
                .into_response()
            }
            Err(e) => {
                return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR")
                    .into_response()
            }
        };

        let bundle = match project::relay::build_project_bundle(&vault.db, &k, id, &body.environment_ids).await {
            Ok(b) => b,
            Err(e) if e.contains("not found") => {
                return err_json(StatusCode::NOT_FOUND, &e, "NOT_FOUND").into_response()
            }
            Err(e) if e.contains("too large") => {
                return err_json(StatusCode::PAYLOAD_TOO_LARGE, &e, "PAYLOAD_TOO_LARGE").into_response()
            }
            Err(e) => {
                return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR")
                    .into_response()
            }
        };

        (supabase_url, anon_key, bundle)
    };

    let project_name = bundle.name.clone();
    let environment_count = bundle.environments.len();
    let item_count = bundle.items.len();

    let code = relay::generate_share_code();
    let passphrase = crate::share::crypto::generate_passphrase();

    let relay_key = match relay::derive_relay_key(&code, &passphrase) {
        Ok(k) => k,
        Err(e) => {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string(), "INTERNAL_ERROR")
                .into_response()
        }
    };
    let payload = match relay::encrypt_project(&bundle, &relay_key) {
        Ok(p) => p,
        Err(e) => {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string(), "INTERNAL_ERROR")
                .into_response()
        }
    };

    let code_clone = code.clone();
    let upload_result = tokio::task::spawn_blocking(move || {
        relay::relay_upload(&supabase_url, &anon_key, &code_clone, &payload)
    })
    .await;
    match upload_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            return err_json(
                StatusCode::BAD_GATEWAY,
                &format!("relay upload failed: {e}"),
                "RELAY_ERROR",
            )
            .into_response()
        }
        Err(e) => {
            return err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("task error: {e}"),
                "INTERNAL_ERROR",
            )
            .into_response()
        }
    }

    (
        StatusCode::OK,
        Json(ProjectRelaySendResponse {
            code,
            passphrase,
            project: project_name,
            environment_count,
            item_count,
        }),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
struct ProjectRelayReceiveBody {
    code: String,
    passphrase: String,
    #[serde(default)]
    project_name_override: Option<String>,
}

#[derive(serde::Serialize)]
struct ProjectRelayReceiveResponse {
    project: String,
    environments: Vec<String>,
    item_count: usize,
}

/// Receive a whole project shared via the relay: recreates it as a brand-new
/// project (never merges into an existing one — D5), in one transaction
/// (D7). A case-insensitive project-name collision is reported as `409
/// CONFLICT` so the caller can retry with `project_name_override`.
async fn handle_project_relay_receive(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    Json(body): Json<ProjectRelayReceiveBody>,
) -> impl IntoResponse {
    if let Err(code) = verify_token(&headers, &state).await {
        let (msg, err_code) = match code {
            StatusCode::UNAUTHORIZED => ("unauthorized", "UNAUTHORIZED"),
            StatusCode::FORBIDDEN => ("vault locked", "VAULT_LOCKED"),
            _ => ("internal error", "INTERNAL_ERROR"),
        };
        return err_json(code, msg, err_code).into_response();
    }

    if body.code.is_empty() || body.passphrase.is_empty() {
        return err_json(
            StatusCode::UNPROCESSABLE_ENTITY,
            "code and passphrase are required",
            "VALIDATION_ERROR",
        )
        .into_response();
    }

    let relay_key = match relay::derive_relay_key(&body.code, &body.passphrase) {
        Ok(k) => k,
        Err(e) => {
            return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string(), "INTERNAL_ERROR")
                .into_response()
        }
    };

    let (vault_key, supabase_url, anon_key) = {
        let vault = state.vault.lock().await;
        let k = match vault.key.as_ref() {
            Some(k) => **k,
            None => {
                return err_json(StatusCode::FORBIDDEN, "vault locked", "VAULT_LOCKED")
                    .into_response()
            }
        };
        let supabase_url = match vault.db.get_setting("relay_supabase_url").await {
            Ok(Some(u)) => u,
            Ok(None) => {
                return err_json(
                    StatusCode::BAD_REQUEST,
                    "relay not configured: set relay_supabase_url in Settings",
                    "NOT_CONFIGURED",
                )
                .into_response()
            }
            Err(e) => {
                return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR")
                    .into_response()
            }
        };
        let anon_key = match vault.db.get_setting("relay_supabase_anon_key").await {
            Ok(Some(k)) => k,
            Ok(None) => {
                return err_json(
                    StatusCode::BAD_REQUEST,
                    "relay not configured: set relay_supabase_anon_key in Settings",
                    "NOT_CONFIGURED",
                )
                .into_response()
            }
            Err(e) => {
                return err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR")
                    .into_response()
            }
        };
        (k, supabase_url, anon_key)
    };

    let url_clone = supabase_url.clone();
    let key_clone = anon_key.clone();
    let code_clone = body.code.clone();
    let download_result = tokio::task::spawn_blocking(move || {
        relay::relay_download(&url_clone, &key_clone, &code_clone)
    })
    .await;
    let payload = match download_result {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            return err_json(
                StatusCode::BAD_GATEWAY,
                &format!("relay download failed: {e}"),
                "RELAY_ERROR",
            )
            .into_response()
        }
        Err(e) => {
            return err_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("task error: {e}"),
                "INTERNAL_ERROR",
            )
            .into_response()
        }
    };

    let bundle = match relay::decrypt_project(&payload, &relay_key) {
        Ok(b) => b,
        Err(e) => {
            return err_json(
                StatusCode::UNPROCESSABLE_ENTITY,
                &format!("decrypt failed (wrong passphrase or not a project package?): {e}"),
                "DECRYPT_ERROR",
            )
            .into_response()
        }
    };

    // Burn-after-read (best-effort).
    let url_clone2 = supabase_url.clone();
    let key_clone2 = anon_key.clone();
    let code_clone2 = body.code.clone();
    let _ = tokio::task::spawn_blocking(move || {
        relay::relay_delete(&url_clone2, &key_clone2, &code_clone2)
    })
    .await;

    let vault = state.vault.lock().await;
    let result = project::relay::receive_project_bundle(&vault.db, &vault_key, bundle, body.project_name_override)
        .await;

    match result {
        Ok(r) => (
            StatusCode::OK,
            Json(ProjectRelayReceiveResponse {
                project: r.project_name,
                environments: r.environment_names,
                item_count: r.item_count,
            }),
        )
            .into_response(),
        // Matches #12's "conflict:" string-prefix convention for db/project
        // layer errors — see docs/plans/issue-4 D5. Swap for the shared
        // `PROJECT_NAME_CONFLICT` constant once that branch merges.
        Err(e) if e.starts_with("conflict:") => {
            err_json(StatusCode::CONFLICT, &e, "CONFLICT").into_response()
        }
        Err(e) => err_json(StatusCode::INTERNAL_SERVER_ERROR, &e, "INTERNAL_ERROR").into_response(),
    }
}

// ─── Función pública de arranque ──────────────────────────────────────────────

/// Builds the plain `axum::Router` with all 36 routes plus the `cors_guard`
/// middleware layer, given an already-constructed `ApiState`. `pub(crate)`
/// (not `pub`): reachable from `api::tests` (a descendant module) and from
/// `crate::test_support::router` (same crate), but never part of the crate's
/// external public API — no production caller outside this crate can build a
/// router or bind it to a socket other than the one fixed inside
/// `start_server` below.
pub(crate) fn build_router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route("/health", get(handle_health))
        .route("/unlock", post(handle_unlock))
        .route("/fill", post(handle_fill))
        .route("/items", get(handle_list_items))
        .route("/items", post(handle_create_item))
        .route("/items/:id", get(handle_get_item))
        .route("/items/:id", put(handle_update_item))
        .route("/items/:id", delete(handle_delete_item))
        .route("/items/:id/reveal", post(handle_reveal_item))
        .route("/maintenance/orphans", get(handle_list_orphans))
        .route("/categories", get(handle_list_categories))
        .route("/categories", post(handle_create_category))
        .route("/categories/:id", put(handle_update_category))
        .route("/categories/:id", delete(handle_delete_category))
        .route("/commands", get(handle_list_commands))
        .route("/commands/:id", get(handle_get_command))
        .route("/settings", get(handle_get_settings))
        .route("/settings", put(handle_put_settings))
        .route("/share/listen", post(handle_share_listen))
        .route("/share/connect", post(handle_share_connect))
        .route("/share/confirm", post(handle_share_confirm))
        .route("/share/status", get(handle_share_status))
        .route("/share/session", delete(handle_share_cancel))
        .route("/share/export", post(handle_share_export))
        .route("/share/import", post(handle_share_import))
        .route("/projects", get(handle_list_projects))
        .route("/projects", post(handle_save_project))
        .route("/projects/:id", delete(handle_delete_project))
        .route("/projects/:id/preview-delete", get(handle_preview_delete_project))
        .route("/environments", post(handle_save_environment))
        .route("/environments/:id", delete(handle_delete_environment))
        .route("/environments/:id/inject", post(handle_inject_environment))
        .route("/environments/:id/example", post(handle_environment_example))
        .route("/tokens", post(handle_create_token))
        .route("/tokens", get(handle_list_tokens))
        .route("/tokens/:id", delete(handle_revoke_token))
        .route("/token/vars", get(handle_token_vars))
        .route("/relay/send", post(handle_relay_send))
        .route("/relay/receive", post(handle_relay_receive))
        .route("/projects/:id/relay/send", post(handle_project_relay_send))
        .route("/projects/relay/receive", post(handle_project_relay_receive))
        .with_state(state)
        .layer(middleware::from_fn(cors_guard))
}

pub async fn start_server(vault: SharedState, app_data_dir: PathBuf) {
    let api_state = Arc::new(ApiState::new(vault));
    let app = build_router(api_state);

    const ADDR: &str = "127.0.0.1:47821";

    // Ensure a valid self-signed TLS certificate is present (generated on first
    // launch, regenerated if within 30 days of expiry).
    let tls_config = match tls::ensure_tls_config(&app_data_dir).await {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("[api] Failed to initialise TLS certificate: {e}");
            return;
        }
    };

    let addr: std::net::SocketAddr = match ADDR.parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[api] Invalid bind address {ADDR}: {e}");
            return;
        }
    };

    eprintln!("[api] Listening on https://{ADDR} (TLS)");

    if let Err(e) = axum_server::bind_rustls(addr, tls_config)
        .serve(app.into_make_service())
        .await
    {
        eprintln!("[api] REST server error: {e}");
    }
}

#[cfg(test)]
mod tests;

// ─── Tests: issue #13, global-item scoped visibility ─────────────────────────
//
// Pure-function tests: plain `VaultItem` values and a `project::Environment`
// with synthetic `vars`, no database, no key, no async. `VaultItem`
// deliberately has no `#[derive(Debug)]` (it holds decrypted plaintext
// secrets — CLAUDE.md forbids secrets in logs/errors, and a stray `{:?}` on
// assertion failure would leak one into CI output), so assertions below
// compare individual fields rather than whole structs.
#[cfg(test)]
mod scope_tests {
    use super::*;

    /// Builds a minimal `VaultItem` fixture. `is_global` mirrors the
    /// `Option<bool>` shape of the real field (`None` behaves like `false`
    /// for scoping purposes, same as `unwrap_or(false)` in `scope_items`).
    fn item(id: i64, name: &str, is_global: Option<bool>) -> VaultItem {
        VaultItem {
            id,
            item_type: "secret".to_string(),
            name: Some(name.to_string()),
            value: None,
            url: None,
            username: None,
            password: None,
            title: None,
            description: None,
            command: None,
            shell: None,
            categories: None,
            notes: None,
            content: None,
            created: "2026-01-01T00:00:00Z".to_string(),
            is_global,
        }
    }

    fn env_with_vars(pairs: &[(&str, i64)]) -> project::Environment {
        project::Environment {
            id: 1,
            project_id: 1,
            name: "test".to_string(),
            is_default: true,
            paths: vec![],
            vars: pairs
                .iter()
                .map(|(key, item_id)| project::EnvironmentVar {
                    id: 0,
                    key: key.to_string(),
                    item_id: *item_id,
                })
                .collect(),
            created: "2026-01-01T00:00:00Z".to_string(),
            updated: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn global_unlinked_item_visible_by_default() {
        let item_a = item(1, "A", Some(false)); // linked, non-global
        let item_b = item(2, "B", Some(true)); // unlinked, global
        let env = env_with_vars(&[("A_KEY", 1)]);
        let linked = environment_item_ids(&env);

        let result = scope_items(vec![item_a, item_b], &linked, IncludeGlobal::With);

        assert_eq!(result.len(), 2, "default mode must return both linked and unlinked-global items");
        let b = result.iter().find(|s| s.item.id == 2).expect("item B must be present");
        assert_eq!(b.item.is_global, Some(true));
        assert!(!b.linked, "unlinked global item must report linked: false");
    }

    #[test]
    fn linked_item_reports_linked_true() {
        let item_a = item(1, "A", Some(false));
        let env = env_with_vars(&[("A_KEY", 1)]);
        let linked = environment_item_ids(&env);

        let result = scope_items(vec![item_a], &linked, IncludeGlobal::With);

        assert_eq!(result.len(), 1);
        assert!(result[0].linked, "linked item must report linked: true");
        assert_eq!(result[0].item.is_global, Some(false));
    }

    #[test]
    fn global_and_linked_item_appears_once() {
        let item_c = item(3, "C", Some(true)); // both global and linked
        let env = env_with_vars(&[("C_KEY", 3)]);
        let linked = environment_item_ids(&env);

        let result = scope_items(vec![item_c], &linked, IncludeGlobal::With);

        assert_eq!(result.len(), 1, "item that is both global and linked must appear exactly once (dedup guard)");
        assert!(result[0].linked);
        assert_eq!(result[0].item.is_global, Some(true));
    }

    #[test]
    fn include_global_false_matches_legacy_scope() {
        let item_a = item(1, "A", Some(false)); // linked, non-global
        let item_b = item(2, "B", Some(true)); // unlinked, global
        let env = env_with_vars(&[("A_KEY", 1)]);
        let linked = environment_item_ids(&env);

        let result = scope_items(vec![item_a, item_b], &linked, IncludeGlobal::Without);

        assert_eq!(result.len(), 1, "Without must return exactly the linked set, matching the pre-change filter");
        assert_eq!(result[0].item.id, 1);
        assert!(result[0].linked);
    }

    #[test]
    fn include_global_only_returns_globals_regardless_of_link() {
        let item_a = item(1, "A", Some(false)); // linked, non-global
        let item_b = item(2, "B", Some(true)); // unlinked, global
        let item_c = item(3, "C", Some(true)); // linked, global
        let env = env_with_vars(&[("A_KEY", 1), ("C_KEY", 3)]);
        let linked = environment_item_ids(&env);

        let result = scope_items(vec![item_a, item_b, item_c], &linked, IncludeGlobal::Only);

        let ids: HashSet<i64> = result.iter().map(|s| s.item.id).collect();
        assert_eq!(ids, HashSet::from([2, 3]), "Only must return all is_global items regardless of linkage, excluding non-global A");
    }

    #[test]
    fn search_and_type_filters_apply_to_unioned_globals() {
        let item_a = item(1, "DB_HOST", Some(false)); // linked
        let item_b = item(2, "API_KEY", Some(true)); // unlinked, global
        let env = env_with_vars(&[("DB_HOST", 1)]);
        let linked = environment_item_ids(&env);

        let scoped = scope_items(vec![item_a, item_b], &linked, IncludeGlobal::With);
        assert_eq!(scoped.len(), 2, "union must include both before filtering");

        let filtered = filter_scoped_items(scoped, None, None, Some("api"));

        assert_eq!(filtered.len(), 1, "search must narrow within the union, including unioned globals");
        assert_eq!(filtered[0].item.id, 2);
    }

    #[test]
    fn invalid_include_global_value_is_rejected() {
        // `axum::response::Response` (the `Err` side) implements neither
        // `Debug` nor `PartialEq`, so `assert_eq!`/`unwrap()` on the whole
        // `Result` won't compile — `matches!` pattern-matches without
        // requiring either trait.
        assert!(IncludeGlobal::parse(Some("bogus")).is_err());
        assert!(matches!(IncludeGlobal::parse(None), Ok(IncludeGlobal::With)));
        assert!(matches!(IncludeGlobal::parse(Some("true")), Ok(IncludeGlobal::With)));
        assert!(matches!(IncludeGlobal::parse(Some("with")), Ok(IncludeGlobal::With)));
        assert!(matches!(IncludeGlobal::parse(Some("false")), Ok(IncludeGlobal::Without)));
        assert!(matches!(IncludeGlobal::parse(Some("without")), Ok(IncludeGlobal::Without)));
        assert!(matches!(IncludeGlobal::parse(Some("only")), Ok(IncludeGlobal::Only)));
    }
}
