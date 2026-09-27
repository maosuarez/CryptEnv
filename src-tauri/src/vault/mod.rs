use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{Manager, State};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use crate::biometric;
use crate::crypto::{self, CryptoKey};
use crate::db::{DbCategory, LinkMode, LinkOutcome, VaultDb};

pub mod import;
pub mod share_commands;

// ─── Serializable types (shared with frontend) ────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct VaultItem {
    #[serde(default)]
    pub id: i64,
    #[serde(rename = "type")]
    pub item_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default)]
    pub created: String,
    /// Marks this item as reusable across projects/environments — only
    /// global items are offered as vault-item references when linking an
    /// environment variable (see project::mod for the inject-time lookup).
    /// `Option` (not `bool`) so that HTTP partial updates that omit this
    /// field leave the existing value untouched instead of clearing it.
    #[serde(default, rename = "isGlobal", skip_serializing_if = "Option::is_none")]
    pub is_global: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
}

#[derive(Serialize)]
pub struct UnlockPayload {
    pub items: Vec<VaultItem>,
    pub categories: Vec<Category>,
}

// ─── Application state ────────────────────────────────────────────────────────

pub struct VaultState {
    pub db: VaultDb,
    /// Zeroizing guarantees the 32 bytes are overwritten when set to None or dropped.
    pub key: Option<Zeroizing<[u8; 32]>>,
    /// Monotonic timestamp of the last vault operation. None when the vault is locked.
    pub last_activity: Option<std::time::Instant>,
}

impl VaultState {
    pub fn new(db: VaultDb) -> Self {
        VaultState {
            db,
            key: None,
            last_activity: None,
        }
    }

    /// Record activity. Call this on every vault operation that accesses the key.
    pub fn touch(&mut self) {
        self.last_activity = Some(std::time::Instant::now());
    }
}

pub type SharedState = Arc<Mutex<VaultState>>;

// ─── Crypto helpers ───────────────────────────────────────────────────────────

/// `is_global` lives in its own plaintext DB column (queryable without
/// decryption) rather than inside the ciphertext — this reassembles it onto
/// the decrypted item.
pub(crate) fn decrypt_item(
    key: &CryptoKey,
    row_id: i64,
    data: &str,
    is_global: bool,
) -> Result<VaultItem, String> {
    let json = crypto::decrypt(key, data)?;
    let mut item: VaultItem =
        serde_json::from_slice(&json).map_err(|e| format!("deserialize item: {e}"))?;
    item.id = row_id;
    item.is_global = Some(is_global);
    Ok(item)
}

/// Strips `is_global` before serializing — it's metadata, not a secret, and
/// must never enter the ciphertext (see `decrypt_item`).
pub(crate) fn encrypt_item(key: &CryptoKey, item: &VaultItem) -> Result<String, String> {
    let mut for_blob = item.clone();
    for_blob.is_global = None;
    let json = serde_json::to_vec(&for_blob).map_err(|e| format!("serialize item: {e}"))?;
    crypto::encrypt(key, &json)
}

/// One-time (gated by `settings['migrated_literals_v1']`), needs the vault
/// key so it can't run during the pre-unlock schema migrations — converts
/// legacy literal-only environment vars (predating "every var is a real
/// item") into real encrypted secret items, owned by their environment's
/// project. Called from every unlock entry point (password + biometric +
/// the HTTP/CLI/MCP unlock path in `api::handle_unlock`).
pub(crate) async fn migrate_literal_vars_to_items(db: &VaultDb, key: &CryptoKey) -> Result<(), String> {
    if db.get_setting("migrated_literals_v1").await?.is_some() {
        return Ok(());
    }

    let now = epoch_to_iso8601(
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    );

    for (env_var_id, key_name, literal, project_id) in db.list_unmigrated_literal_vars().await? {
        let item = VaultItem {
            id: 0,
            item_type: "secret".to_string(),
            name: Some(key_name),
            value: Some(literal),
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
            created: now.clone(),
            is_global: Some(false),
        };
        let encrypted = encrypt_item(key, &item)?;
        let new_id = db.upsert_item(0, "secret", &encrypted, &item.created, false).await?;
        db.add_item_owner(new_id, project_id).await?;
        db.set_environment_var_item(env_var_id, new_id).await?;
    }

    db.set_setting("migrated_literals_v1", "true").await?;
    Ok(())
}

// ─── Tauri commands ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn vault_is_setup(state: State<'_, SharedState>) -> Result<bool, String> {
    let s = state.lock().await;
    s.db.is_initialized().await
}

/// Shared unlock logic: derives key from password bytes, loads items + categories.
/// The caller is responsible for verifying the password is correct before calling this
/// (either via `unlock_vault_crypto` or by knowing it came from a trusted DPAPI blob).
async fn do_unlock(
    password_bytes: &[u8],
    s: &mut VaultState,
) -> Result<UnlockPayload, String> {
    let key = {
        let db = &s.db;
        if db.is_initialized().await? {
            let (salt, token) = db
                .get_meta()
                .await?
                .ok_or_else(|| "vault_meta row missing".to_string())?;
            crypto::unlock_vault_crypto(password_bytes, &salt, &token)?
        } else {
            let (salt, token, k) = crypto::init_vault_crypto(password_bytes)?;
            db.init_vault(&salt, &token).await?;
            k
        }
    };

    s.key = Some(Zeroizing::new(key));
    s.touch();

    migrate_literal_vars_to_items(&s.db, &key).await?;

    let raw = s.db.list_items().await?;
    let items: Vec<VaultItem> = raw
        .into_iter()
        .filter_map(|(id, _, data, _, is_global)| decrypt_item(&key, id, &data, is_global).ok())
        .collect();

    let cats = s
        .db
        .list_categories()
        .await?
        .into_iter()
        .map(|c| Category {
            id: c.cid,
            name: c.name,
            color: c.color,
            description: c.description,
        })
        .collect();

    Ok(UnlockPayload {
        items,
        categories: cats,
    })
}

#[tauri::command]
pub async fn vault_unlock(
    password: String,
    state: State<'_, SharedState>,
) -> Result<UnlockPayload, String> {
    let mut s = state.lock().await;
    do_unlock(password.as_bytes(), &mut s).await
}

/// Internal lock used by both the Tauri command and the background auto-lock task.
pub async fn lock_vault(shared: &SharedState) {
    let mut s = shared.lock().await;
    s.key = None;
    s.last_activity = None;
}

#[tauri::command]
pub async fn vault_lock(state: State<'_, SharedState>) -> Result<(), String> {
    lock_vault(&state).await;
    Ok(())
}

/// Refreshes the backend idle timer from frontend user activity so the
/// background auto-lock doesn't fire while the user is actively working.
/// No-op when locked — it must never extend or revive a locked session.
#[tauri::command]
pub async fn vault_touch(state: State<'_, SharedState>) -> Result<(), String> {
    let mut s = state.lock().await;
    if s.key.is_some() {
        s.touch();
    }
    Ok(())
}

#[tauri::command]
pub async fn vault_get_items(state: State<'_, SharedState>) -> Result<Vec<VaultItem>, String> {
    let mut s = state.lock().await;
    let key = s.key.as_ref().ok_or("vault is locked")?.clone();
    s.touch();
    let raw = s.db.list_items().await?;
    Ok(raw
        .into_iter()
        .filter_map(|(id, _, data, _, is_global)| decrypt_item(&key, id, &data, is_global).ok())
        .collect())
}

#[tauri::command]
pub async fn vault_save_item(
    item: VaultItem,
    state: State<'_, SharedState>,
) -> Result<VaultItem, String> {
    let mut s = state.lock().await;
    let key = s.key.as_ref().ok_or("vault is locked")?.clone();
    s.touch();
    let is_global = item.is_global.unwrap_or(false);
    let encrypted = encrypt_item(&key, &item)?;
    let new_id = s
        .db
        .upsert_item(item.id, &item.item_type, &encrypted, &item.created, is_global)
        .await?;
    let mut saved = item;
    saved.id = new_id;
    saved.is_global = Some(is_global);
    Ok(saved)
}

/// Pure logic shared by the `vault_create_project_item` Tauri command and the
/// HTTP `POST /items` handler: encrypts and inserts a new item, then grants
/// ownership to `project_id` atomically. New items are never global by
/// default — the caller must explicitly toggle that afterwards.
pub async fn create_project_item(
    db: &VaultDb,
    key: &CryptoKey,
    item: &VaultItem,
    project_id: i64,
) -> Result<i64, String> {
    let encrypted = encrypt_item(key, item)?;
    let new_id = db
        .upsert_item(0, &item.item_type, &encrypted, &item.created, false)
        .await?;
    db.add_item_owner(new_id, project_id).await?;
    Ok(new_id)
}

/// `db`'s conflict classification, re-exported so callers outside this crate
/// (namely `api`) match on the `vault` type rather than importing from `db`
/// directly — `vault` is the layer that owns the create-or-update decision,
/// `db` only executes it.
pub use crate::db::ConflictReason;

/// Result of `create_or_update_env_item` — mirrors `db::LinkOutcome` but
/// carries the full (decrypted, redaction-pending) `VaultItem` on success
/// instead of bare ids, since the HTTP response needs the whole shape.
pub enum UpsertOutcome {
    Created(VaultItem),
    Updated(VaultItem),
    Conflict { item_id: i64, reason: ConflictReason },
}

/// Orchestrates `POST /items`'s create-or-update-on-collision behaviour
/// (issue #9): inspect the current state of `env_key` in `environment_id`,
/// decide free / exclusive / shared, encrypt (preserving the existing row's
/// `created` on the update branch so the ciphertext and the column agree),
/// then hand the encrypted blob to `db::create_or_link_item`'s single
/// transaction. This is the only layer that holds the vault key on this
/// path — `db` never sees a `CryptoKey`.
pub async fn create_or_update_env_item(
    db: &VaultDb,
    key: &CryptoKey,
    item: &VaultItem,
    project_id: i64,
    environment_id: i64,
    env_key: &str,
    mode: LinkMode,
) -> Result<UpsertOutcome, String> {
    let conflict = db.inspect_env_key(environment_id, env_key).await?;

    // Short-circuit without touching the key: no need to encrypt anything to
    // return a conflict.
    if let Some(c) = &conflict {
        if mode == LinkMode::Error {
            return Ok(UpsertOutcome::Conflict {
                item_id: c.item_id,
                reason: ConflictReason::KeyExists,
            });
        }
        if mode == LinkMode::Update {
            let exclusive = !c.is_global && c.link_count == 1 && c.owner_ids == [project_id];
            if !exclusive {
                return Ok(UpsertOutcome::Conflict {
                    item_id: c.item_id,
                    reason: ConflictReason::Shared,
                });
            }
        }
    }

    // Build the item to persist. On the `Update` branch, stamp the existing
    // row's `created` onto the blob so it matches `items.created`, which the
    // update branch of `create_or_link_item` does not rewrite. On create /
    // replace, keep the caller's `created` (already defaulted by the handler).
    let mut to_persist = item.clone();
    if mode == LinkMode::Update {
        if let Some(c) = &conflict {
            to_persist.created = c.created.clone();
        }
    }

    let encrypted = encrypt_item(key, &to_persist)?;

    let outcome = db
        .create_or_link_item(
            environment_id,
            project_id,
            env_key,
            &to_persist.item_type,
            &encrypted,
            &to_persist.created,
            mode,
            conflict.map(|c| c.item_id),
        )
        .await?;

    Ok(match outcome {
        LinkOutcome::Created { item_id, is_global } => {
            let mut saved = to_persist;
            saved.id = item_id;
            saved.is_global = Some(is_global);
            UpsertOutcome::Created(saved)
        }
        LinkOutcome::Updated { item_id, is_global } => {
            let mut saved = to_persist;
            saved.id = item_id;
            saved.is_global = Some(is_global);
            UpsertOutcome::Updated(saved)
        }
        LinkOutcome::Conflict { item_id, reason } => UpsertOutcome::Conflict { item_id, reason },
    })
}

/// Creates a new item and atomically grants ownership to `project_id` — the
/// "add a typed variable inside a project" primitive. New items are never
/// global by default (the caller must explicitly toggle that afterwards).
#[tauri::command]
pub async fn vault_create_project_item(
    item: VaultItem,
    project_id: i64,
    state: State<'_, SharedState>,
) -> Result<VaultItem, String> {
    let mut s = state.lock().await;
    let key = s.key.as_ref().ok_or("vault is locked")?.clone();
    s.touch();
    let new_id = create_project_item(&s.db, &key, &item, project_id).await?;
    let mut saved = item;
    saved.id = new_id;
    saved.is_global = Some(false);
    Ok(saved)
}

// ─── Project scaffolding from templates ───────────────────────────────────────

/// Upper bound on variables a single scaffold request may create.
const MAX_TEMPLATE_VARS: usize = 200;

/// Stored in place of an empty value so an un-edited environment fails loudly
/// at the consuming service instead of silently running with an empty secret.
pub const TEMPLATE_PLACEHOLDER: &str = "CHANGE_ME";

/// One `KEY=value` pair to scaffold. Deliberately no `Debug` derive: the value
/// is secret material and must never be formatted into logs or errors.
#[derive(Deserialize)]
pub struct TemplateVarInput {
    pub key: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Deserialize)]
pub struct ProjectFromTemplatesInput {
    pub project: crate::project::ProjectInput,
    #[serde(default)]
    pub vars: Vec<TemplateVarInput>,
}

fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Rejects the request before any write. Errors name keys/indices only.
fn validate_template_input(input: &ProjectFromTemplatesInput) -> Result<(), String> {
    crate::project::validate_project_name(&input.project.name)?;
    if input.project.id != 0 {
        return Err("templates can only be applied when creating a new project".to_string());
    }
    if input.vars.len() > MAX_TEMPLATE_VARS {
        return Err(format!("too many variables ({} > {MAX_TEMPLATE_VARS})", input.vars.len()));
    }
    let mut seen = std::collections::HashSet::new();
    for (i, v) in input.vars.iter().enumerate() {
        if !is_valid_env_key(&v.key) {
            return Err(format!("invalid variable key at #{}: '{}' (expected [A-Z_][A-Z0-9_]*)", i + 1, v.key));
        }
        if !seen.insert(v.key.as_str()) {
            return Err(format!("duplicate variable key '{}'", v.key));
        }
    }
    Ok(())
}

/// Creates a project, its `default` environment, and one project-scoped
/// secret item per variable linked into that environment — all or nothing.
/// `db` helpers each run on the pool (no shared transaction), so failure
/// after the project exists is undone by `delete_project`, which also
/// removes the items this project solely owns.
pub async fn create_project_from_templates(
    db: &VaultDb,
    key: &CryptoKey,
    input: ProjectFromTemplatesInput,
) -> Result<i64, String> {
    scaffold_project(db, key, input, None).await
}

/// `fail_after` is a test-only failpoint: fail right after creating that many
/// items, to exercise the rollback path.
async fn scaffold_project(
    db: &VaultDb,
    key: &CryptoKey,
    input: ProjectFromTemplatesInput,
    fail_after: Option<usize>,
) -> Result<i64, String> {
    validate_template_input(&input)?;
    let ProjectFromTemplatesInput { project, vars } = input;
    let project_id = crate::project::save_project(db, project).await?;

    match populate_default_environment(db, key, project_id, vars, fail_after).await {
        Ok(()) => Ok(project_id),
        Err(e) => match db.delete_project(project_id).await {
            Ok(_) => Err(e),
            Err(_) => Err(format!(
                "{e} (rollback failed: delete project #{project_id} manually)"
            )),
        },
    }
}

async fn populate_default_environment(
    db: &VaultDb,
    key: &CryptoKey,
    project_id: i64,
    vars: Vec<TemplateVarInput>,
    fail_after: Option<usize>,
) -> Result<(), String> {
    let env = db
        .list_environments(project_id)
        .await?
        .into_iter()
        .find(|e| e.is_default)
        .ok_or("new project has no default environment")?;

    let now = epoch_to_iso8601(
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    );

    let mut env_vars = Vec::with_capacity(vars.len());
    for (i, v) in vars.into_iter().enumerate() {
        if fail_after == Some(i) {
            return Err(format!("failed to create item for '{}'", v.key));
        }
        let value = if v.value.is_empty() { TEMPLATE_PLACEHOLDER.to_string() } else { v.value };
        let item = VaultItem {
            id: 0,
            item_type: "secret".to_string(),
            name: Some(v.key.clone()),
            value: Some(value),
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
            created: now.clone(),
            is_global: Some(false),
        };
        // Map the error so nothing derived from the value can surface.
        let item_id = create_project_item(db, key, &item, project_id)
            .await
            .map_err(|_| format!("failed to create item for '{}'", v.key))?;
        env_vars.push(crate::project::EnvironmentVar { id: 0, key: v.key, item_id });
    }

    crate::project::save_environment(
        db,
        crate::project::EnvironmentInput {
            id: env.id,
            project_id,
            name: env.name,
            is_default: env.is_default,
            paths: env.paths,
            vars: env_vars,
        },
    )
    .await?;
    Ok(())
}

/// Scaffolds a new project from the GUI's template picker (see
/// `openspec/specs/project-templates`). GUI-only: not exposed over REST/MCP/CLI.
#[tauri::command]
pub async fn project_create_from_templates(
    input: ProjectFromTemplatesInput,
    state: State<'_, SharedState>,
) -> Result<i64, String> {
    let mut s = state.lock().await;
    let key = s.key.as_ref().ok_or("vault is locked")?.clone();
    s.touch();
    create_project_from_templates(&s.db, &key, input).await
}

/// Result of toggling `isGlobal` on an item: at most one of the two shapes is
/// populated. `updated` — the flag was simply flipped (item had ≤1 owner).
/// `forked` — un-globaling an item with >1 owners splits it into one
/// independent copy per owning project; the original shared row is gone.
#[derive(Serialize)]
pub struct GlobalToggleResult {
    pub updated: Option<VaultItem>,
    pub forked: Vec<VaultItem>,
}

/// Pure logic behind `vault_set_item_global`: toggles `isGlobal`, or — when
/// un-globaling an item with more than one owning project — forks it into
/// one independent copy per owner. Split out the same way `create_project_item`
/// already was, so this is unit-testable without a Tauri runtime; the
/// `#[tauri::command]` below is now a thin wrapper delegating to it with
/// identical behaviour.
pub async fn set_item_global(
    db: &VaultDb,
    key: &CryptoKey,
    id: i64,
    global: bool,
) -> Result<GlobalToggleResult, String> {
    let owners = db.list_owning_projects(id).await?;

    // Marking global, or un-globaling something with ≤1 owner: no fork needed.
    if global || owners.len() <= 1 {
        db.set_item_global(id, global).await?;
        let raw = db.list_items().await?;
        let updated = raw
            .into_iter()
            .find(|(row_id, ..)| *row_id == id)
            .and_then(|(row_id, _, data, _, is_global)| decrypt_item(key, row_id, &data, is_global).ok());
        return Ok(GlobalToggleResult { updated, forked: vec![] });
    }

    // Un-globaling a multi-owner item: fork one independent copy per owner.
    let raw = db.list_items().await?;
    let (_, item_type, data, created, _) = raw
        .into_iter()
        .find(|(row_id, ..)| *row_id == id)
        .ok_or("item not found")?;
    let original = decrypt_item(key, id, &data, true)?;

    let mut forked = Vec::with_capacity(owners.len());
    for project_id in &owners {
        let mut copy = original.clone();
        copy.id = 0;
        copy.is_global = Some(false);
        let encrypted = encrypt_item(key, &copy)?;
        let new_id = db.upsert_item(0, &item_type, &encrypted, &created, false).await?;
        db.add_item_owner(new_id, *project_id).await?;
        db.repoint_env_var_item(*project_id, id, new_id).await?;
        copy.id = new_id;
        forked.push(copy);
    }

    db.delete_item(id).await?;

    Ok(GlobalToggleResult { updated: None, forked })
}

#[tauri::command]
pub async fn vault_set_item_global(
    id: i64,
    global: bool,
    state: State<'_, SharedState>,
) -> Result<GlobalToggleResult, String> {
    let mut s = state.lock().await;
    let key = s.key.as_ref().ok_or("vault is locked")?.clone();
    s.touch();
    set_item_global(&s.db, &key, id, global).await
}

/// Which projects currently reference (own) this item — used for "used by N
/// projects" warnings before a destructive delete-everywhere action.
#[derive(Serialize)]
pub struct ItemOwner {
    #[serde(rename = "projectId")]
    pub project_id: i64,
    #[serde(rename = "projectName")]
    pub project_name: String,
}

#[tauri::command]
pub async fn vault_get_item_owners(
    id: i64,
    state: State<'_, SharedState>,
) -> Result<Vec<ItemOwner>, String> {
    let s = state.lock().await;
    let owner_ids = s.db.list_owning_projects(id).await?;
    let projects = s.db.list_projects().await?;
    Ok(projects
        .into_iter()
        .filter(|p| owner_ids.contains(&p.id))
        .map(|p| ItemOwner { project_id: p.id, project_name: p.name })
        .collect())
}

// ─── Issue #9: orphan report / prune (report by default, delete only on
// explicit confirmation — see plan §3.6) ───────────────────────────────────

/// Decrypts only the ids `db::list_orphan_item_ids` returns and redacts them,
/// so the caller can judge what it is about to delete without any secret
/// value leaving this process.
pub async fn list_orphan_items(db: &VaultDb, key: &CryptoKey) -> Result<Vec<VaultItem>, String> {
    let ids = db.list_orphan_item_ids().await?;
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let raw = db.list_items().await?;
    Ok(raw
        .into_iter()
        .filter(|(id, ..)| ids.contains(id))
        .filter_map(|(id, _, data, _, is_global)| decrypt_item(key, id, &data, is_global).ok())
        .collect())
}

/// Deletes the given ids via `db::delete_items_cascade`, which re-checks the
/// orphan predicate per id inside its own transaction — a concurrently
/// re-linked or globalized item is left alone rather than destroyed.
pub async fn prune_orphan_items(db: &VaultDb, ids: &[i64]) -> Result<(), String> {
    db.delete_items_cascade(ids).await
}

#[tauri::command]
pub async fn vault_list_orphan_items(
    state: State<'_, SharedState>,
) -> Result<Vec<VaultItem>, String> {
    let mut s = state.lock().await;
    let key = s.key.as_ref().ok_or("vault is locked")?.clone();
    s.touch();
    list_orphan_items(&s.db, &key).await
}

#[tauri::command]
pub async fn vault_prune_orphan_items(
    ids: Vec<i64>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let mut s = state.lock().await;
    s.key.as_ref().ok_or("vault is locked")?;
    s.touch();
    prune_orphan_items(&s.db, &ids).await
}

#[tauri::command]
pub async fn vault_delete_item(id: i64, state: State<'_, SharedState>) -> Result<(), String> {
    let mut s = state.lock().await;
    s.key.as_ref().ok_or("vault is locked")?;
    s.touch();
    s.db.delete_item(id).await
}

#[tauri::command]
pub async fn vault_get_categories(
    state: State<'_, SharedState>,
) -> Result<Vec<Category>, String> {
    let s = state.lock().await;
    Ok(s.db
        .list_categories()
        .await?
        .into_iter()
        .map(|c| Category {
            id: c.cid,
            name: c.name,
            color: c.color,
            description: c.description,
        })
        .collect())
}

#[tauri::command]
pub async fn vault_list(
    state: State<'_, SharedState>,
) -> Result<serde_json::Value, String> {
    let mut s = state.lock().await;
    let key = s.key.as_ref().ok_or("vault is locked")?.clone();
    s.touch();

    let items: Vec<VaultItem> = s.db.list_items().await?
        .into_iter()
        .filter_map(|(id, _, data, _, is_global)| decrypt_item(&key, id, &data, is_global).ok())
        .collect();

    let categories: Vec<Category> = s.db.list_categories().await?
        .into_iter()
        .map(|c| Category {
            id: c.cid,
            name: c.name,
            color: c.color,
            description: c.description,
        })
        .collect();

    Ok(serde_json::json!({
        "items": items,
        "categories": categories,
    }))
}

#[tauri::command]
pub async fn vault_save_categories(
    cats: Vec<Category>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let s = state.lock().await;
    let db_cats: Vec<DbCategory> = cats
        .into_iter()
        .map(|c| DbCategory {
            cid: c.id,
            name: c.name,
            color: c.color,
            description: c.description,
        })
        .collect();
    s.db.save_categories(&db_cats).await
}

#[tauri::command]
pub async fn vault_get_settings(
    state: State<'_, SharedState>,
) -> Result<serde_json::Value, String> {
    let s = state.lock().await;
    let timeout = s
        .db
        .get_setting("auto_lock_timeout")
        .await?
        .unwrap_or_else(|| "5".into());
    let hotkey = s
        .db
        .get_setting("hotkey")
        .await?
        .unwrap_or_else(|| crate::hotkey::default_hotkey().into());
    Ok(serde_json::json!({
        "autoLockTimeout": timeout.parse::<i64>().unwrap_or(5),
        "hotkey": hotkey
    }))
}

/// Non-sensitive runtime diagnostics shown on the LockScreen and in Settings.
/// Never carries keys, tokens, or vault contents — only the app version and
/// where the encrypted database lives on disk.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AppSystemInfo {
    pub version: String,
    pub app_dir: String,
    pub db_path: String,
    pub os: String,
}

/// Pure builder for [`AppSystemInfo`], split out from the command so it can be
/// unit-tested without a Tauri runtime. `vault.db` matches the file opened in
/// `lib.rs::run` setup.
pub fn build_system_info(version: &str, app_dir: &std::path::Path) -> AppSystemInfo {
    AppSystemInfo {
        version: version.to_string(),
        app_dir: app_dir.display().to_string(),
        db_path: app_dir.join("vault.db").display().to_string(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
    }
}

/// Available while locked: exposes no secret material.
#[tauri::command]
pub fn app_get_system_info(app: tauri::AppHandle) -> Result<AppSystemInfo, String> {
    let app_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("path error: {e}"))?;
    Ok(build_system_info(&app.package_info().version.to_string(), &app_dir))
}

#[tauri::command]
pub async fn vault_change_password(
    current_password: String,
    new_password: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let mut s = state.lock().await;
    s.key.as_ref().ok_or("vault is locked")?;

    // Verify current password against stored meta
    let (salt, token) = s
        .db
        .get_meta()
        .await?
        .ok_or("vault_meta missing")?;
    let old_key = crypto::unlock_vault_crypto(current_password.as_bytes(), &salt, &token)?;

    // Generate new crypto material
    let (new_salt, new_token, new_key) = crypto::init_vault_crypto(new_password.as_bytes())?;

    // Re-encrypt every item blob
    let raw = s.db.list_items().await?;
    let mut re_encrypted: Vec<(i64, String)> = Vec::with_capacity(raw.len());
    for (id, _, data, _, _) in &raw {
        let plaintext = crypto::decrypt(&old_key, data)?;
        let new_data = crypto::encrypt(&new_key, &plaintext)?;
        re_encrypted.push((*id, new_data));
    }

    // Atomic DB update: new salt/token + all re-encrypted items
    s.db.rekey(&new_salt, &new_token, re_encrypted).await?;

    // Update in-memory key
    s.key = Some(Zeroizing::new(new_key));
    s.touch();

    Ok(())
}

#[tauri::command]
pub async fn vault_wipe(state: State<'_, SharedState>) -> Result<(), String> {
    let mut s = state.lock().await;
    s.key = None;
    s.last_activity = None;
    s.db.wipe_and_reset().await
}

#[tauri::command]
pub async fn vault_save_settings(
    auto_lock_timeout: i64,
    hotkey: String,
    #[allow(unused_variables)]
    relay_supabase_url: Option<String>,
    #[allow(unused_variables)]
    relay_supabase_anon_key: Option<String>,
    state: State<'_, SharedState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // Apply the shortcut first so an invalid or OS-conflicting combination is
    // reported to the user and never persisted.
    let hotkey = crate::hotkey::normalize_hotkey(&hotkey);
    crate::hotkey::replace_app_hotkey(&app, &hotkey)?;

    let s = state.lock().await;
    s.db
        .set_setting("auto_lock_timeout", &auto_lock_timeout.to_string())
        .await?;
    s.db.set_setting("hotkey", &hotkey).await?;
    if let Some(url) = relay_supabase_url {
        if !url.is_empty() {
            s.db.set_setting("relay_supabase_url", &url).await?;
        }
    }
    if let Some(key) = relay_supabase_anon_key {
        if !key.is_empty() {
            s.db.set_setting("relay_supabase_anon_key", &key).await?;
        }
    }
    Ok(())
}

/// Suppresses the global window-toggle shortcut while Settings records a new
/// combination, so pressing the current hotkey doesn't hide the window.
#[tauri::command]
pub fn vault_pause_hotkey(
    paused: bool,
    hotkey_state: State<'_, crate::hotkey::HotkeyState>,
) -> Result<(), String> {
    hotkey_state.set_paused(paused);
    Ok(())
}

#[tauri::command]
pub async fn vault_generate_mcp_token(
    state: State<'_, SharedState>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let mut s = state.lock().await;
    s.key.as_ref().ok_or("vault is locked")?;
    s.touch();

    // Generar token de 32 bytes hex
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();

    // Guardar en DB
    s.db.set_setting("mcp_token", &token).await?;

    // Write token file for the MCP binary.
    // On Windows, %APPDATA% inherits user-only NTFS ACLs; no further ACL work needed.
    // On Unix, restrict to 0o600 (owner read/write only) after the write.
    let app_dir = app.path().app_data_dir()
        .map_err(|e| format!("path error: {e}"))?;
    let token_path = app_dir.join("mcp_token");
    std::fs::write(&token_path, &token)
        .map_err(|e| format!("write mcp token file: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("set mcp token file permissions: {e}"))?;
    }

    Ok(token)
}

#[tauri::command]
pub async fn vault_get_mcp_token(
    state: State<'_, SharedState>,
) -> Result<Option<String>, String> {
    let mut s = state.lock().await;
    s.key.as_ref().ok_or("vault is locked")?;
    s.touch();
    s.db.get_setting("mcp_token").await
}

// ─── Backup types ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct BackupItem {
    id: i64,
    item_type: String,
    /// Raw AES-GCM encrypted blob from the DB (hex-encoded nonce || ciphertext).
    data: String,
    created: String,
    #[serde(default)]
    is_global: bool,
}

#[derive(Serialize, Deserialize)]
struct BackupFile {
    version: u32,
    created_at: u64,
    /// Hex-encoded Argon2id salt used to derive the vault key.
    salt: String,
    /// AES-GCM verify token that proves the master password is correct.
    token: String,
    items: Vec<BackupItem>,
    categories: Vec<serde_json::Value>,
}

// ─── Backup commands ──────────────────────────────────────────────────────────

/// Export all vault items and categories to a `.cenvbak` file.
/// The backup preserves the vault's existing salt and verify token so that
/// restoring only requires the original master password — no separate backup password.
/// Returns the number of items written.
#[tauri::command]
pub async fn vault_export_backup(
    path: String,
    state: State<'_, SharedState>,
) -> Result<usize, String> {
    let mut s = state.lock().await;
    s.key.as_ref().ok_or("vault is locked")?;
    s.touch();

    // Read vault crypto material
    let (salt, token) = s
        .db
        .get_meta()
        .await?
        .ok_or("vault_meta missing")?;

    // Read all raw (already-encrypted) item rows
    let raw_items = s.db.list_items().await?;
    let items: Vec<BackupItem> = raw_items
        .iter()
        .map(|(id, item_type, data, created, is_global)| BackupItem {
            id: *id,
            item_type: item_type.clone(),
            data: data.clone(),
            created: created.clone(),
            is_global: *is_global,
        })
        .collect();
    let item_count = items.len();

    // Read categories as generic JSON values
    let db_cats = s.db.list_categories().await?;
    let categories: Vec<serde_json::Value> = db_cats
        .iter()
        .map(|c| serde_json::json!({ "id": c.cid, "name": c.name, "color": c.color, "description": c.description }))
        .collect();

    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let backup = BackupFile {
        version: 1,
        created_at,
        salt,
        token,
        items,
        categories,
    };

    let json = serde_json::to_string_pretty(&backup)
        .map_err(|e| format!("serialize backup: {e}"))?;

    std::fs::write(&path, json).map_err(|e| format!("write backup: {e}"))?;

    Ok(item_count)
}

/// Import a `.cenvbak` file by filesystem path.
///
/// - `merge = false`: wipes the current vault and restores the backup exactly,
///   keeping the original crypto material so the same master password applies.
/// - `merge = true`: re-encrypts each imported item with the *current* vault key
///   and upserts into the live vault. Categories are merged (no duplicates by id).
///
/// Returns the number of items restored.
#[tauri::command]
pub async fn vault_import_backup(
    path: String,
    master_password: String,
    merge: bool,
    state: State<'_, SharedState>,
) -> Result<usize, String> {
    let json = std::fs::read_to_string(&path)
        .map_err(|e| format!("read backup: {e}"))?;
    do_restore_backup(&json, &master_password, merge, &state).await
}

/// Import a `.cenvbak` by passing its JSON content directly.
/// Used by the frontend file picker where only file content (not the path) is accessible.
#[tauri::command]
pub async fn vault_import_backup_data(
    data: String,
    master_password: String,
    merge: bool,
    state: State<'_, SharedState>,
) -> Result<usize, String> {
    do_restore_backup(&data, &master_password, merge, &state).await
}

async fn do_restore_backup(
    json: &str,
    master_password: &str,
    merge: bool,
    state: &State<'_, SharedState>,
) -> Result<usize, String> {
    let backup: BackupFile = serde_json::from_str(json)
        .map_err(|e| format!("parse backup: {e}"))?;

    if backup.version != 1 {
        return Err(format!("unsupported backup version: {}", backup.version));
    }

    // Verify the master password against the backup's crypto material.
    let backup_key = crypto::unlock_vault_crypto(
        master_password.as_bytes(),
        &backup.salt,
        &backup.token,
    )
    .map_err(|_| "incorrect master password for this backup".to_string())?;

    let mut s = state.lock().await;

    if !merge {
        // Wipe the current vault and restore the backup's crypto material verbatim.
        // After init_vault the in-memory key equals the backup key.
        s.key = None;
        s.db.wipe_and_reset().await?;
        s.db.init_vault(&backup.salt, &backup.token).await?;
        s.key = Some(Zeroizing::new(backup_key));
        s.touch();

        // Insert items using their original encrypted blobs (already keyed with backup_key).
        for item in &backup.items {
            s.db
                .upsert_item(0, &item.item_type, &item.data, &item.created, item.is_global)
                .await?;
        }

        // Restore categories wholesale.
        let db_cats: Vec<DbCategory> = backup
            .categories
            .iter()
            .filter_map(|v| {
                Some(DbCategory {
                    cid: v.get("id")?.as_str()?.to_string(),
                    name: v.get("name")?.as_str()?.to_string(),
                    color: v.get("color")?.as_str()?.to_string(),
                    description: v.get("description").and_then(|x| x.as_str()).map(|s| s.to_string()),
                })
            })
            .collect();
        s.db.save_categories(&db_cats).await?;
    } else {
        // Merge: re-encrypt each item from backup_key → current vault key.
        let current_key = s.key.as_ref().ok_or("vault is locked")?.clone();
        s.touch();

        for item in &backup.items {
            let plaintext = crypto::decrypt(&backup_key, &item.data)
                .map_err(|e| format!("decrypt backup item {}: {e}", item.id))?;
            let new_data = crypto::encrypt(&current_key, &plaintext)
                .map_err(|e| format!("re-encrypt item {}: {e}", item.id))?;
            s.db
                .upsert_item(0, &item.item_type, &new_data, &item.created, item.is_global)
                .await?;
        }

        // Merge categories: keep existing, append any new ids from the backup.
        let existing_cats = s.db.list_categories().await?;
        let existing_ids: std::collections::HashSet<String> =
            existing_cats.iter().map(|c| c.cid.clone()).collect();

        let mut merged = existing_cats;
        for v in &backup.categories {
            let cid = match v.get("id").and_then(|x| x.as_str()) {
                Some(id) => id.to_string(),
                None => continue,
            };
            if !existing_ids.contains(&cid) {
                merged.push(DbCategory {
                    cid,
                    name: v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    color: v.get("color").and_then(|x| x.as_str()).unwrap_or("#888").to_string(),
                    description: v.get("description").and_then(|x| x.as_str()).map(|s| s.to_string()),
                });
            }
        }
        s.db.save_categories(&merged).await?;
    }

    Ok(backup.items.len())
}

// ─── Import from password managers ───────────────────────────────────────────

#[derive(Deserialize, Serialize)]
pub struct ImportItem {
    pub name: String,
    pub value: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub url: Option<String>,
    pub notes: Option<String>,
    pub item_type: String,
}

#[derive(Deserialize)]
pub struct ParseImportArgs {
    pub content: String,
    pub format: String,
}

/// Parse file content into a preview list of ImportItems without touching vault state.
#[tauri::command]
pub async fn vault_parse_import(args: ParseImportArgs) -> Result<Vec<ImportItem>, String> {
    let items = match args.format.as_str() {
        "env"       => import::parse_env_file(&args.content),
        "bitwarden" => import::parse_bitwarden_csv(&args.content),
        "1password" => import::parse_1password_csv(&args.content),
        "csv"       => import::parse_csv_generic(&args.content),
        other       => return Err(format!("unknown import format: {other}")),
    };
    Ok(items)
}

/// Import a list of items into the vault, skipping any whose name already exists.
/// Returns the count of items actually inserted.
#[tauri::command]
pub async fn vault_import_items(
    items: Vec<ImportItem>,
    state: State<'_, SharedState>,
) -> Result<usize, String> {
    let mut s = state.lock().await;
    let key = s.key.as_ref().ok_or("vault is locked")?.clone();
    s.touch();
    let key = &key;

    // Build the set of existing item names to detect duplicates.
    let raw = s.db.list_items().await?;
    let existing_names: std::collections::HashSet<String> = raw
        .iter()
        .filter_map(|(id, _, data, _, is_global)| {
            decrypt_item(key, *id, data, *is_global).ok().and_then(|v| v.name)
        })
        .collect();

    // Epoch-seconds timestamp shared by all items in this import batch.
    let epoch_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let now = epoch_to_iso8601(epoch_secs);

    let mut inserted = 0usize;

    for imp in items {
        if existing_names.contains(&imp.name) {
            continue;
        }

        let vault_item = VaultItem {
            id: 0,
            item_type: imp.item_type.clone(),
            name: Some(imp.name),
            value: imp.value,
            username: imp.username,
            password: imp.password,
            url: imp.url,
            notes: imp.notes,
            title: None,
            description: None,
            command: None,
            shell: None,
            content: None,
            categories: None,
            created: now.clone(),
            is_global: None,
        };

        let encrypted = encrypt_item(key, &vault_item)?;
        s.db
            .upsert_item(0, &vault_item.item_type, &encrypted, &vault_item.created, false)
            .await?;
        inserted += 1;
    }

    Ok(inserted)
}

/// Format Unix epoch seconds as an ISO-8601 UTC string: "2006-01-02T15:04:05Z".
fn epoch_to_iso8601(secs: u64) -> String {
    let sec  = secs % 60;
    let min  = (secs / 60) % 60;
    let hour = (secs / 3600) % 24;
    let days = secs / 86400;
    let (year, month, day) = days_since_epoch_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// Convert days-since-Unix-epoch (1970-01-01) to (year, month, day).
/// Uses the algorithm from http://howardhinnant.github.io/date_algorithms.html.
fn days_since_epoch_to_ymd(mut days: u64) -> (u64, u64, u64) {
    days += 719468;
    let era = days / 146097;
    let doe = days % 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y   = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp  = (5 * doy + 2) / 153;
    let d   = doy - (153 * mp + 2) / 5 + 1;
    let m   = if mp < 10 { mp + 3 } else { mp - 9 };
    let y   = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ─── First-run / setup commands ──────────────────────────────────────────────

#[tauri::command]
pub async fn app_is_first_run(state: State<'_, SharedState>) -> Result<bool, String> {
    let s = state.lock().await;
    let val = s.db.get_setting("setup_complete").await?;
    Ok(val.as_deref() != Some("true"))
}

#[tauri::command]
pub async fn app_complete_setup(state: State<'_, SharedState>) -> Result<(), String> {
    let s = state.lock().await;
    s.db.set_setting("setup_complete", "true").await
}

#[tauri::command]
pub async fn app_generate_mcp_config(
    target_path: String,
    state: State<'_, SharedState>,
) -> Result<String, String> {
    let token = {
        let s = state.lock().await;
        s.key.as_ref().ok_or("vault is locked")?;
        s.db.get_setting("mcp_token")
            .await?
            .ok_or_else(|| "MCP token not found — open Settings to generate one".to_string())?
    };
    fn home_dir() -> Option<std::path::PathBuf> {
        if let Ok(p) = std::env::var("USERPROFILE") {
            if !p.is_empty() { return Some(std::path::PathBuf::from(p)); }
        }
        if let Ok(p) = std::env::var("HOME") {
            if !p.is_empty() { return Some(std::path::PathBuf::from(p)); }
        }
        None
    }

    let resolved: std::path::PathBuf = if target_path.is_empty() {
        let home = home_dir().ok_or("cannot determine home directory")?;
        home.join(".mcp.json")
    } else if target_path.starts_with('~') {
        let home = home_dir().ok_or("cannot determine home directory")?;
        let rest = target_path
            .trim_start_matches('~')
            .trim_start_matches('/')
            .trim_start_matches('\\');
        if rest.is_empty() { home.join(".mcp.json") } else { home.join(rest) }
    } else {
        std::path::PathBuf::from(&target_path)
    };

    let mut root: serde_json::Value = if resolved.exists() {
        let content = std::fs::read_to_string(&resolved)
            .map_err(|e| format!("read {}: {e}", resolved.display()))?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    {
        let obj = root.as_object_mut().ok_or("existing .mcp.json is not a JSON object")?;
        let mcp_servers = obj.entry("mcpServers").or_insert_with(|| serde_json::json!({}));
        if let Some(servers) = mcp_servers.as_object_mut() {
            servers.insert(
                "cryptenv".to_string(),
                serde_json::json!({
                    "command": "crypt-env-mcp",
                    "env": { "CRYPTENV_TOKEN": token }
                }),
            );
        }
    }

    if let Some(parent) = resolved.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create dirs {}: {e}", parent.display()))?;
    }

    let json_str = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("serialize mcp config: {e}"))?;
    std::fs::write(&resolved, &json_str)
        .map_err(|e| format!("write {}: {e}", resolved.display()))?;

    Ok(resolved.to_string_lossy().into_owned())
}

// ─── Biometric commands ───────────────────────────────────────────────────────

#[tauri::command]
pub async fn biometric_check(state: State<'_, SharedState>) -> Result<String, String> {
    let _ = state; // state not needed; availability is system-wide
    let status = biometric::check_availability().await;
    Ok(status.as_str().to_string())
}

#[tauri::command]
pub async fn biometric_is_enrolled(state: State<'_, SharedState>) -> Result<bool, String> {
    let s = state.lock().await;
    let blob = s.db.get_setting("biometric_blob").await?;
    Ok(blob.map(|v| !v.is_empty()).unwrap_or(false))
}

#[tauri::command]
pub async fn biometric_enroll(
    password: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    // 1. Verify the supplied password against the stored vault meta.
    {
        let s = state.lock().await;
        let (salt, token) = s
            .db
            .get_meta()
            .await?
            .ok_or_else(|| "vault not initialized".to_string())?;
        crypto::unlock_vault_crypto(password.as_bytes(), &salt, &token)?;
    }

    // 2. Request Windows Hello consent before storing anything.
    let verified = biometric::request_verification("Enroll CryptEnv biometric unlock").await?;
    if !verified {
        return Err("Windows Hello verification was not completed".to_string());
    }

    // 3. DPAPI-protect the password bytes, then hex-encode for DB storage.
    #[cfg(target_os = "windows")]
    let hex_blob = {
        use zeroize::Zeroizing;
        let pw_bytes = Zeroizing::new(password.into_bytes());
        let blob = biometric::dpapi_protect(&pw_bytes)?;
        crypto::hex_encode(&blob)
    };

    #[cfg(not(target_os = "windows"))]
    let hex_blob: String = {
        return Err("biometric unlock is not available on this platform".to_string());
        #[allow(unreachable_code)]
        String::new()
    };

    let s = state.lock().await;
    s.db.set_setting("biometric_blob", &hex_blob).await?;
    Ok(())
}

#[tauri::command]
pub async fn biometric_unlock(state: State<'_, SharedState>) -> Result<UnlockPayload, String> {
    // 1. Retrieve the stored DPAPI blob.
    let hex_blob = {
        let s = state.lock().await;
        s.db
            .get_setting("biometric_blob")
            .await?
            .filter(|v| !v.is_empty())
            .ok_or_else(|| "biometric unlock is not enrolled".to_string())?
    };

    // 2. Request Windows Hello consent.
    let verified = biometric::request_verification("Unlock CryptEnv vault").await?;
    if !verified {
        return Err("Windows Hello verification was not completed".to_string());
    }

    // 3. Decode + DPAPI-unprotect to recover the master password bytes.
    #[cfg(target_os = "windows")]
    let password_bytes = {
        let raw = crypto::hex_decode(&hex_blob)
            .map_err(|_| "biometric blob is corrupt".to_string())?;
        biometric::dpapi_unprotect(&raw)?
    };

    #[cfg(not(target_os = "windows"))]
    {
        let _ = hex_blob;
        return Err("biometric unlock is not available on this platform".to_string());
    }

    // 4. Unlock using the recovered password, same path as vault_unlock.
    #[cfg(target_os = "windows")]
    {
        let mut s = state.lock().await;
        do_unlock(&password_bytes, &mut s).await
    }
}

#[tauri::command]
pub async fn biometric_disable(state: State<'_, SharedState>) -> Result<(), String> {
    let s = state.lock().await;
    s.db.set_setting("biometric_blob", "").await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbEnvironmentVar;

    async fn test_db() -> (tempfile::TempDir, VaultDb, [u8; 32]) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        let db = VaultDb::open(path.to_str().unwrap()).await.unwrap();
        let (salt, token, key) = crypto::init_vault_crypto(b"test-master-password-1").unwrap();
        db.init_vault(&salt, &token).await.unwrap();
        (dir, db, key)
    }

    #[test]
    fn system_info_reports_version_and_db_path_under_app_dir() {
        let dir = tempfile::tempdir().unwrap();
        let info = build_system_info("1.2.3", dir.path());
        assert_eq!(info.version, "1.2.3");
        assert!(!info.app_dir.is_empty());
        assert_eq!(std::path::Path::new(&info.app_dir), dir.path());
        assert_eq!(std::path::Path::new(&info.db_path), dir.path().join("vault.db"));
        assert!(!info.os.is_empty());
    }

    #[test]
    fn system_info_serializes_camel_case() {
        let dir = tempfile::tempdir().unwrap();
        let json = serde_json::to_value(build_system_info("1.0.0", dir.path())).unwrap();
        for key in ["version", "appDir", "dbPath", "os"] {
            assert!(json.get(key).is_some(), "missing key {key}");
        }
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

    async fn literal_var(db: &VaultDb, project_name: &str, key_name: &str, literal: &str) -> (i64, i64) {
        let project_id = db.upsert_project(0, project_name, None, "generic").await.unwrap();
        let env_id = db.upsert_environment(0, project_id, "production", true).await.unwrap();
        db.set_environment_vars(
            env_id,
            &[DbEnvironmentVar {
                id: 0,
                environment_id: env_id,
                key: key_name.to_string(),
                item_id: None,
                literal: Some(literal.to_string()),
            }],
        )
        .await
        .unwrap();
        (project_id, env_id)
    }

    // ─── migrate_literal_vars_to_items ────────────────────────────────────

    #[tokio::test]
    async fn migrate_converts_literal_var_into_real_item() {
        let (_dir, db, key) = test_db().await;
        let (project_id, env_id) = literal_var(&db, "proj-a", "LITERAL_KEY", "literal-secret").await;

        migrate_literal_vars_to_items(&db, &key).await.unwrap();

        let vars = db.get_environment_vars(env_id).await.unwrap();
        assert_eq!(vars.len(), 1);
        assert!(vars[0].literal.is_none(), "literal must be cleared after migration");
        let item_id = vars[0].item_id.expect("item_id must be set after migration");

        let owners = db.list_owning_projects(item_id).await.unwrap();
        assert_eq!(owners, vec![project_id]);

        let raw = db.list_items().await.unwrap();
        let (_, _, data, _, is_global) = raw.into_iter().find(|(id, ..)| *id == item_id).unwrap();
        let item = decrypt_item(&key, item_id, &data, is_global).unwrap();
        assert_eq!(item.name.as_deref(), Some("LITERAL_KEY"));
        assert_eq!(item.value.as_deref(), Some("literal-secret"));
        assert_eq!(item.is_global, Some(false));
    }

    #[tokio::test]
    async fn migrate_gives_each_project_its_own_owned_item() {
        let (_dir, db, key) = test_db().await;
        let (project_a, _) = literal_var(&db, "proj-a", "KEY_A", "secret-a").await;
        let (project_b, _) = literal_var(&db, "proj-b", "KEY_B", "secret-b").await;

        migrate_literal_vars_to_items(&db, &key).await.unwrap();

        let raw = db.list_items().await.unwrap();
        assert_eq!(raw.len(), 2, "one item per migrated literal var");
        for (id, ..) in &raw {
            let owners = db.list_owning_projects(*id).await.unwrap();
            assert!(owners == vec![project_a] || owners == vec![project_b]);
        }
    }

    #[tokio::test]
    async fn migrate_is_a_noop_once_already_migrated() {
        let (_dir, db, key) = test_db().await;
        db.set_setting("migrated_literals_v1", "true").await.unwrap();
        let (_project_id, env_id) = literal_var(&db, "proj-a", "LITERAL_KEY", "literal-secret").await;

        migrate_literal_vars_to_items(&db, &key).await.unwrap();

        let vars = db.get_environment_vars(env_id).await.unwrap();
        assert_eq!(vars.len(), 1);
        assert!(vars[0].item_id.is_none(), "already-migrated gate must skip the rewrite");
        assert_eq!(vars[0].literal.as_deref(), Some("literal-secret"));
        assert!(db.list_items().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn migrate_running_twice_is_idempotent() {
        let (_dir, db, key) = test_db().await;
        literal_var(&db, "proj-a", "LITERAL_KEY", "literal-secret").await;

        migrate_literal_vars_to_items(&db, &key).await.unwrap();
        let first_count = db.list_items().await.unwrap().len();

        migrate_literal_vars_to_items(&db, &key).await.unwrap();
        let second_count = db.list_items().await.unwrap().len();

        assert_eq!(first_count, 1);
        assert_eq!(second_count, 1, "second call must not create duplicate items");
    }

    // ─── create_project_item ──────────────────────────────────────────────

    #[tokio::test]
    async fn create_project_item_grants_ownership() {
        let (_dir, db, key) = test_db().await;
        let project_id = db.upsert_project(0, "proj", None, "generic").await.unwrap();
        let item = plain_secret("NEW_ITEM", "value-1", false);

        let new_id = create_project_item(&db, &key, &item, project_id).await.unwrap();

        assert_eq!(db.list_owning_projects(new_id).await.unwrap(), vec![project_id]);
    }

    #[tokio::test]
    async fn create_project_item_is_never_global_regardless_of_input() {
        let (_dir, db, key) = test_db().await;
        let project_id = db.upsert_project(0, "proj", None, "generic").await.unwrap();
        let mut item = plain_secret("NEW_ITEM", "value-1", false);
        item.is_global = Some(true); // caller tries to sneak in isGlobal=true

        let new_id = create_project_item(&db, &key, &item, project_id).await.unwrap();

        let raw = db.list_items().await.unwrap();
        let (_, _, _, _, is_global) = raw.into_iter().find(|(id, ..)| *id == new_id).unwrap();
        assert!(!is_global, "create_project_item must force is_global=false in the DB column");
    }

    #[tokio::test]
    async fn create_project_item_twice_yields_two_owned_items() {
        let (_dir, db, key) = test_db().await;
        let project_id = db.upsert_project(0, "proj", None, "generic").await.unwrap();
        let item_a = plain_secret("ITEM_A", "value-a", false);
        let item_b = plain_secret("ITEM_B", "value-b", false);

        let id_a = create_project_item(&db, &key, &item_a, project_id).await.unwrap();
        let id_b = create_project_item(&db, &key, &item_b, project_id).await.unwrap();

        assert_ne!(id_a, id_b);
        let owned = db.list_owned_item_ids(project_id).await.unwrap();
        assert!(owned.contains(&id_a) && owned.contains(&id_b));
    }

    // ─── set_item_global (fork logic) ─────────────────────────────────────

    #[tokio::test]
    async fn set_item_global_true_updates_in_place() {
        let (_dir, db, key) = test_db().await;
        let project_id = db.upsert_project(0, "proj", None, "generic").await.unwrap();
        let item = plain_secret("ITEM", "value", false);
        let id = create_project_item(&db, &key, &item, project_id).await.unwrap();

        let result = set_item_global(&db, &key, id, true).await.unwrap();

        assert!(result.forked.is_empty());
        let updated = result.updated.expect("updated item present");
        assert_eq!(updated.is_global, Some(true));
    }

    #[tokio::test]
    async fn set_item_global_false_with_single_owner_updates_in_place() {
        let (_dir, db, key) = test_db().await;
        let project_id = db.upsert_project(0, "proj", None, "generic").await.unwrap();
        let mut item = plain_secret("ITEM", "value", true);
        item.is_global = Some(true);
        let encrypted = encrypt_item(&key, &item).unwrap();
        let id = db.upsert_item(0, &item.item_type, &encrypted, &item.created, true).await.unwrap();
        db.add_item_owner(id, project_id).await.unwrap();

        let result = set_item_global(&db, &key, id, false).await.unwrap();

        assert!(result.forked.is_empty(), "single owner must not fork");
        assert_eq!(result.updated.unwrap().is_global, Some(false));
    }

    #[tokio::test]
    async fn set_item_global_false_with_multiple_owners_forks_per_owner() {
        let (_dir, db, key) = test_db().await;
        let project_a = db.upsert_project(0, "proj-a", None, "generic").await.unwrap();
        let project_b = db.upsert_project(0, "proj-b", None, "generic").await.unwrap();
        let env_a = db.upsert_environment(0, project_a, "production", true).await.unwrap();
        let env_b = db.upsert_environment(0, project_b, "production", true).await.unwrap();

        let mut item = plain_secret("SHARED", "shared-value", true);
        item.is_global = Some(true);
        let encrypted = encrypt_item(&key, &item).unwrap();
        let original_id = db.upsert_item(0, &item.item_type, &encrypted, &item.created, true).await.unwrap();
        db.add_item_owner(original_id, project_a).await.unwrap();
        db.add_item_owner(original_id, project_b).await.unwrap();
        db.upsert_environment_var(env_a, "SHARED", original_id).await.unwrap();
        db.upsert_environment_var(env_b, "SHARED", original_id).await.unwrap();

        let result = set_item_global(&db, &key, original_id, false).await.unwrap();

        assert!(result.updated.is_none());
        assert_eq!(result.forked.len(), 2, "one fork per owning project");
        for copy in &result.forked {
            assert_eq!(copy.is_global, Some(false));
            assert_eq!(copy.name.as_deref(), Some("SHARED"));
            assert_eq!(copy.value.as_deref(), Some("shared-value"));
        }

        // Each project's environment var must now point at ITS OWN fork, not
        // the deleted original nor the other project's fork.
        let vars_a = db.get_environment_vars(env_a).await.unwrap();
        let vars_b = db.get_environment_vars(env_b).await.unwrap();
        let forked_ids: Vec<i64> = result.forked.iter().map(|c| c.id).collect();
        assert!(forked_ids.contains(&vars_a[0].item_id.unwrap()));
        assert!(forked_ids.contains(&vars_b[0].item_id.unwrap()));
        assert_ne!(vars_a[0].item_id, vars_b[0].item_id);
    }

    #[tokio::test]
    async fn set_item_global_false_multi_owner_deletes_the_original() {
        let (_dir, db, key) = test_db().await;
        let project_a = db.upsert_project(0, "proj-a", None, "generic").await.unwrap();
        let project_b = db.upsert_project(0, "proj-b", None, "generic").await.unwrap();

        let mut item = plain_secret("SHARED", "shared-value", true);
        item.is_global = Some(true);
        let encrypted = encrypt_item(&key, &item).unwrap();
        let original_id = db.upsert_item(0, &item.item_type, &encrypted, &item.created, true).await.unwrap();
        db.add_item_owner(original_id, project_a).await.unwrap();
        db.add_item_owner(original_id, project_b).await.unwrap();

        set_item_global(&db, &key, original_id, false).await.unwrap();

        let raw = db.list_items().await.unwrap();
        assert!(
            raw.iter().find(|(id, ..)| *id == original_id).is_none(),
            "original shared row must be deleted after forking"
        );
    }

    // ── project_create_from_templates ──

    fn tpl_input(name: &str, vars: &[(&str, &str)]) -> ProjectFromTemplatesInput {
        ProjectFromTemplatesInput {
            project: crate::project::ProjectInput {
                id: 0,
                name: name.to_string(),
                description: None,
                template: "node,postgres".to_string(),
                categories: vec![],
                initial_environment: None,
            },
            vars: vars
                .iter()
                .map(|(k, v)| TemplateVarInput { key: k.to_string(), value: v.to_string() })
                .collect(),
        }
    }

    async fn assert_nothing_created(db: &VaultDb) {
        assert!(db.list_projects().await.unwrap().is_empty(), "no project must remain");
        assert!(db.list_items().await.unwrap().is_empty(), "no item must remain");
    }

    #[tokio::test]
    async fn scaffold_creates_project_items_and_linked_default_env() {
        let (_dir, db, key) = test_db().await;
        let id = create_project_from_templates(
            &db,
            &key,
            tpl_input("shop", &[("NODE_ENV", "development"), ("POSTGRES_PASSWORD", "")]),
        )
        .await
        .unwrap();

        let projects = db.list_projects().await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].template, "node,postgres");

        let envs = db.list_environments(id).await.unwrap();
        assert_eq!(envs.len(), 1);
        assert!(envs[0].is_default);
        let vars = db.get_environment_vars(envs[0].id).await.unwrap();
        assert_eq!(vars.len(), 2);

        let raw = db.list_items().await.unwrap();
        assert_eq!(raw.len(), 2);
        for v in &vars {
            let item_id = v.item_id.expect("var must link an item");
            let (rid, _, data, _, is_global) = raw.iter().find(|(rid, ..)| *rid == item_id).unwrap();
            assert!(!is_global);
            assert!(!data.contains("development"), "value must be encrypted at rest");
            let item = decrypt_item(&key, *rid, data, *is_global).unwrap();
            assert_eq!(item.item_type, "secret");
            assert_eq!(item.name.as_deref(), Some(v.key.as_str()));
            let expected = if v.key == "NODE_ENV" { "development" } else { TEMPLATE_PLACEHOLDER };
            assert_eq!(item.value.as_deref(), Some(expected));
            assert_eq!(db.list_owning_projects(item_id).await.unwrap(), vec![id]);
        }
    }

    #[tokio::test]
    async fn scaffold_with_no_vars_creates_empty_default_env() {
        let (_dir, db, key) = test_db().await;
        let id = create_project_from_templates(&db, &key, tpl_input("empty", &[])).await.unwrap();
        let envs = db.list_environments(id).await.unwrap();
        assert!(db.get_environment_vars(envs[0].id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn scaffold_rejects_invalid_input_without_writing() {
        let (_dir, db, key) = test_db().await;
        for input in [
            tpl_input("p", &[("my-key", "secret-value-1")]),
            tpl_input("p", &[("1ABC", "secret-value-1")]),
            tpl_input("p", &[("lower", "secret-value-1")]),
            tpl_input("p", &[("A", "secret-value-1"), ("A", "secret-value-2")]),
            tpl_input("bad/name", &[("A", "secret-value-1")]),
        ] {
            let err = create_project_from_templates(&db, &key, input).await.unwrap_err();
            assert!(!err.contains("secret-value"), "error leaked a value: {err}");
        }
        let too_many: Vec<(String, String)> =
            (0..=MAX_TEMPLATE_VARS).map(|i| (format!("K{i}"), "v".to_string())).collect();
        let refs: Vec<(&str, &str)> = too_many.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert!(create_project_from_templates(&db, &key, tpl_input("p", &refs)).await.is_err());
        let mut existing = tpl_input("p", &[]);
        existing.project.id = 7;
        assert!(create_project_from_templates(&db, &key, existing).await.is_err());
        assert_nothing_created(&db).await;
    }

    #[tokio::test]
    async fn scaffold_rolls_back_on_mid_way_failure() {
        let (_dir, db, key) = test_db().await;
        let input = tpl_input(
            "rollback",
            &[("A", "secret-value-a"), ("B", "secret-value-b"), ("C", "secret-value-c")],
        );
        let err = scaffold_project(&db, &key, input, Some(2)).await.unwrap_err();
        assert!(err.contains("'C'"));
        assert!(!err.contains("secret-value"), "error leaked a value: {err}");
        assert_nothing_created(&db).await;
    }
}
