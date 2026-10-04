//! Shared test harness — see `docs/plans/issue-11-test-coverage-and-postman.md`
//! §3.2 for the design rationale.
//!
//! Only ever compiled under `#[cfg(test)]` (declared as `#[cfg(test)] mod
//! test_support;` in `lib.rs`), so nothing here reaches a release build.
//!
//! **Sibling issue plans: this is the fixture API. Use it; do not invent
//! another.** `unwrap()`/`expect()` are permitted here — CLAUDE.md's ban on
//! `unwrap()` applies to production code; a panicking fixture is a failing
//! test, which is correct behaviour.
//!
//! Dependency direction: this module sits *above* `db`, `vault`, `project`
//! and `api`, and is imported by their test modules only. It does not create
//! a `db -> api` edge; those modules remain unaware of `api`/`test_support`
//! in all non-test builds.
//!
//! Risk: fixture coupling (see plan §6.2). `TestVault`'s seeded shape (3
//! items in "production", 1 global item, "demo" project with "production" +
//! "local" environments) is depended on by multiple sibling issues. Tests
//! MUST assert against `v.item_ids` / `v.project_id` / `v.env_id`, never
//! against literal ids or item counts. Additions for a single test go
//! through `seed_item`/`seed_project`/`link_var`, never by editing the
//! shared builder below.

use std::sync::Arc;
use tokio::sync::Mutex;
use crate::crypto::VaultKey;

use crate::db::VaultDb;
use crate::vault::{SharedState, VaultItem, VaultState};

/// A fully wired vault backed by a real (tempdir) SQLite file, ready to be
/// driven through the HTTP router via `req()`, or inspected directly via its
/// `state`/`dir` fields.
///
/// `dir` MUST stay alive for the lifetime of the test: dropping it deletes
/// the sqlite file out from under `state`. Never destructure it away.
pub struct TestVault {
    pub dir: tempfile::TempDir,
    pub state: SharedState,
    /// Value for the `X-Vault-Token` header — a live *session* token seeded
    /// straight into `api`, so tests authenticate as the user without an
    /// `/unlock` round-trip. Use `mcp_token` to act as the MCP principal.
    pub token: String,
    /// The static `settings['mcp_token']`: authenticates as the MCP principal.
    pub mcp_token: String,
    /// The API state every `router(v)` is built on, so seeded sessions,
    /// approvals and runs are shared across the requests of one test.
    pub api: Arc<crate::api::ApiState>,
    pub master_password: String,
    /// Project "demo". Zero when built via `unlocked_vault_empty()`.
    pub project_id: i64,
    /// Environment "production" of "demo" (`isDefault = true`). Zero when
    /// built via `unlocked_vault_empty()`.
    pub env_id: i64,
    /// All seeded items, in insertion order: `DB_HOST`, `DB_PASSWORD`,
    /// `API_KEY` (all linked into "production"), then `SHARED_TOKEN` (the
    /// global item, linked to nothing). Empty when built via
    /// `unlocked_vault_empty()`.
    pub item_ids: Vec<i64>,
}

const MASTER_PASSWORD: &str = "test-master-password-1";
const MCP_TOKEN: &str = "test-mcp-token-0123456789abcdef";
const SESSION_TOKEN: &str = "test-session-token-0123456789ab";

async fn open_db() -> (tempfile::TempDir, VaultDb) {
    let dir = tempfile::tempdir().expect("create tempdir for test vault");
    let db_path = dir.path().join("vault.db");
    let db = VaultDb::open(db_path.to_str().expect("tempdir path is valid utf8"))
        .await
        .expect("open test vault db");
    (dir, db)
}

/// Initialises vault crypto (salt + verify token + key), persists them, and
/// seeds `settings['mcp_token']` so `token` authenticates immediately.
/// Returns the raw 32-byte key.
async fn init_crypto(db: &VaultDb) -> VaultKey {
    let (salt, verify_token, key) =
        crate::crypto::init_vault_crypto(MASTER_PASSWORD.as_bytes()).expect("init vault crypto");
    db.init_vault(&salt, &verify_token)
        .await
        .expect("persist vault_meta");
    db.set_setting("mcp_token", MCP_TOKEN)
        .await
        .expect("seed mcp_token setting");
    key
}

fn plain_secret(name: &str, value: &str, is_global: bool) -> VaultItem {
    VaultItem {
        id: 0,
        item_type: "secret".to_string(),
        name: Some(name.to_string()),
        value: Some(value.to_string()),
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
        created: "0".to_string(),
        is_global: Some(is_global),
    }
}

/// Shared builder for `unlocked_vault()` / `unlocked_vault_empty()` /
/// `locked_vault()`. `seed_data = false` gives crypto + `mcp_token` only —
/// no project/environments/items.
async fn build(seed_data: bool) -> TestVault {
    let (dir, db) = open_db().await;
    let key = init_crypto(&db).await;

    let mut project_id = 0i64;
    let mut env_id = 0i64;
    let mut item_ids = Vec::new();

    if seed_data {
        project_id = db
            .upsert_project(0, "demo", None, "generic")
            .await
            .expect("insert project 'demo'");
        let prod_id = db
            .upsert_environment(0, project_id, "production", true)
            .await
            .expect("insert environment 'production'");
        db.upsert_environment(0, project_id, "local", false)
            .await
            .expect("insert environment 'local'");
        env_id = prod_id;

        for (name, value) in [
            ("DB_HOST", "localhost"),
            ("DB_PASSWORD", "hunter2"),
            ("API_KEY", "sk-test-key"),
        ] {
            let item = plain_secret(name, value, false);
            let encrypted = crate::vault::encrypt_item(&key, &item).expect("encrypt seeded item");
            let item_id = db
                .upsert_item(0, &item.item_type, &encrypted, &item.created, false)
                .await
                .expect("insert seeded item");
            db.add_item_owner(item_id, project_id)
                .await
                .expect("grant item ownership to 'demo'");
            db.upsert_environment_var(prod_id, name, item_id)
                .await
                .expect("link seeded item into 'production'");
            item_ids.push(item_id);
        }

        let global_item = plain_secret("SHARED_TOKEN", "shared-value", true);
        let encrypted =
            crate::vault::encrypt_item(&key, &global_item).expect("encrypt global item");
        let global_id = db
            .upsert_item(0, &global_item.item_type, &encrypted, &global_item.created, true)
            .await
            .expect("insert global item");
        item_ids.push(global_id);
    }

    let state: SharedState = Arc::new(Mutex::new(VaultState::new(db)));
    {
        let mut s = state.lock().await;
        s.set_key(Some(key.clone()));
        s.touch();
    }

    let api = Arc::new(crate::api::ApiState::new(state.clone()));
    api.seed_session(SESSION_TOKEN).await;
    // Generated MCP files go into the test's own tempdir.
    state.lock().await.generated.init(dir.path().join("mcp-tmp"));

    TestVault {
        dir,
        state,
        token: SESSION_TOKEN.to_string(),
        mcp_token: MCP_TOKEN.to_string(),
        api,
        master_password: MASTER_PASSWORD.to_string(),
        project_id,
        env_id,
        item_ids,
    }
}

/// Tempdir + `VaultDb::open` + `init_vault_crypto` + key installed in
/// `VaultState` + project "demo" with environments "production" (default)
/// and "local" + 3 items (`DB_HOST`, `DB_PASSWORD`, `API_KEY`) linked into
/// "production" + 1 global item (`SHARED_TOKEN`, `isGlobal = true`, linked to
/// nothing) + `settings['mcp_token']` seeded so `token` authenticates
/// immediately.
pub async fn unlocked_vault() -> TestVault {
    build(true).await
}

/// Same crypto setup, no projects / environments / items. For tests that
/// need to assert creation from empty, or 422-on-unresolvable-scope.
pub async fn unlocked_vault_empty() -> TestVault {
    build(false).await
}

/// Locked vault (`state.key == None`) for 403 VAULT_LOCKED assertions. Same
/// seeded shape as `unlocked_vault()` otherwise, so a test can still build
/// scope query params against `v.env_id` before asserting the lock rejects
/// the request.
pub async fn locked_vault() -> TestVault {
    let v = build(true).await;
    v.state.lock().await.set_key(None);
    v
}

/// The plain axum Router with state — no TLS, no socket. `v.api` was built
/// via the same `ApiState::new` that `start_server` uses, so tests and the
/// real server construct state identically.
pub fn router(v: &TestVault) -> axum::Router {
    crate::api::build_router(v.api.clone())
}

/// One request through `ServiceExt::oneshot`. `token: None` omits the
/// `X-Vault-Token` header entirely (for 401 assertions). Returns the status
/// plus the parsed JSON body (`Value::Null` for empty bodies).
pub async fn req(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> (axum::http::StatusCode, serde_json::Value) {
    use tower::ServiceExt;

    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        builder = builder.header("x-vault-token", t);
    }
    let axum_body = match &body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            axum::body::Body::from(serde_json::to_vec(v).expect("serialize request body"))
        }
        None => axum::body::Body::empty(),
    };
    let request = builder.body(axum_body).expect("build test request");

    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("router did not return a response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

/// Real `POST /unlock` round-trip returning a session token, for the tests
/// that need session semantics (expiry, rate limiting) rather than the
/// static `mcp_token`.
pub async fn session_token(app: &axum::Router, v: &TestVault) -> String {
    let body = serde_json::json!({ "master_password": v.master_password });
    let (status, json) = req(app, "POST", "/unlock", None, Some(body)).await;
    assert_eq!(
        status,
        axum::http::StatusCode::OK,
        "unlock round-trip failed: {json:?}"
    );
    json.get("token")
        .and_then(|t| t.as_str())
        .expect("unlock response carries a token field")
        .to_string()
}

/// Encrypts and inserts a standalone item — does not link it into any
/// environment or grant project ownership. Pair with `link_var` and/or a
/// direct `db.add_item_owner` call (via `v.state`) when a test needs those.
pub async fn seed_item(v: &TestVault, name: &str, value: &str, is_global: bool) -> i64 {
    let state = v.state.lock().await;
    let key = state
        .key
        .clone()
        .expect("vault must be unlocked to seed_item");
    let item = plain_secret(name, value, is_global);
    let encrypted = crate::vault::encrypt_item(&key, &item).expect("encrypt item");
    state
        .db
        .upsert_item(0, &item.item_type, &encrypted, &item.created, is_global)
        .await
        .expect("insert item")
}

/// Creates a new project with one environment per name in `envs` (the first
/// is `isDefault = true`, matching `unlocked_vault()`'s own convention).
/// Returns `(project_id, environment_ids)` in the same order as `envs`.
pub async fn seed_project(v: &TestVault, name: &str, envs: &[&str]) -> (i64, Vec<i64>) {
    let state = v.state.lock().await;
    let project_id = state
        .db
        .upsert_project(0, name, None, "generic")
        .await
        .expect("insert project");
    let mut env_ids = Vec::with_capacity(envs.len());
    for (i, env_name) in envs.iter().enumerate() {
        let id = state
            .db
            .upsert_environment(0, project_id, env_name, i == 0)
            .await
            .expect("insert environment");
        env_ids.push(id);
    }
    (project_id, env_ids)
}

/// Links `item_id` into `env_id` under `key`, without touching any other var
/// already set on that environment. Returns the `environment_vars` row id.
pub async fn link_var(v: &TestVault, env_id: i64, key: &str, item_id: i64) -> i64 {
    let state = v.state.lock().await;
    state
        .db
        .upsert_environment_var(env_id, key, item_id)
        .await
        .expect("link environment var")
}

/// Reads an item straight out of the DB and decrypts it — for asserting what
/// was actually persisted, independent of what the handler echoed back.
pub async fn read_item(v: &TestVault, id: i64) -> VaultItem {
    let state = v.state.lock().await;
    let key = state
        .key
        .clone()
        .expect("vault must be unlocked to read_item");
    let raw = state.db.list_items().await.expect("list_items");
    let (row_id, _, data, _, is_global) = raw
        .into_iter()
        .find(|(row_id, ..)| *row_id == id)
        .expect("item exists in db");
    crate::vault::decrypt_item(&key, row_id, &data, is_global).expect("decrypt item")
}
