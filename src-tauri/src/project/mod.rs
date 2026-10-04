use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tauri::State;
use zeroize::Zeroizing;

use crate::db::{DbEnvironmentVar, ProjectDeleteImpact, VaultDb};
use crate::envfile;
use crate::vault::SharedState;

pub mod manifest;
pub mod relay;
pub mod relay_commands;

// ─── Frontend-facing types ────────────────────────────────────────────────────

/// Every variable is a real vault item now — no more bare literals. `item_id`
/// points into the shared `items` table; ownership (`item_projects`) is
/// granted automatically by `save_environment` below.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EnvironmentVar {
    #[serde(default)]
    pub id: i64,
    pub key: String,
    #[serde(rename = "itemId")]
    pub item_id: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Environment {
    #[serde(default)]
    pub id: i64,
    #[serde(rename = "projectId")]
    pub project_id: i64,
    pub name: String,
    #[serde(rename = "isDefault")]
    pub is_default: bool,
    pub paths: Vec<String>,
    pub vars: Vec<EnvironmentVar>,
    pub created: String,
    pub updated: String,
}

#[derive(Deserialize)]
pub struct EnvironmentInput {
    #[serde(default)]
    pub id: i64,
    /// `#[serde(default)]` so a missing field produces a friendly 422
    /// (`handle_save_environment` validates `project_id > 0` and that it
    /// references an existing project) instead of a raw JSON-deserialize
    /// rejection.
    #[serde(default, rename = "projectId")]
    pub project_id: i64,
    pub name: String,
    #[serde(default, rename = "isDefault")]
    pub is_default: bool,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub vars: Vec<EnvironmentVar>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Project {
    #[serde(default)]
    pub id: i64,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub template: String,
    pub created: String,
    pub updated: String,
    pub environments: Vec<Environment>,
    /// Category NAMES (same convention as `VaultItem.categories`) — reuses
    /// the existing categories table for project tags/language.
    #[serde(default)]
    pub categories: Vec<String>,
    /// Project root directory as the vault host sees it — the directory
    /// holding `.crypt-env.yaml`. Relative environment paths resolve
    /// against it (see [`resolve_env_path`]).
    #[serde(rename = "rootPath", default, skip_serializing_if = "Option::is_none")]
    pub root_path: Option<String>,
}

#[derive(Deserialize)]
pub struct ProjectInput {
    #[serde(default)]
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub template: String,
    #[serde(default)]
    pub categories: Vec<String>,
    /// Name of the environment auto-created with a NEW project. Blank or
    /// omitted means "default"; ignored when updating an existing project.
    #[serde(default, rename = "initialEnvironment")]
    pub initial_environment: Option<String>,
    /// Project root directory. Omitted = keep the current one, `""` =
    /// clear it, anything else must be an absolute path (host view).
    #[serde(default, rename = "rootPath")]
    pub root_path: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct InjectResult {
    pub paths: Vec<String>,
    pub written: Vec<String>,
    /// Owner-configured paths (`environment.paths[]`) that were `Foreign`
    /// (pre-existing, not carrying the crypt-env marker) at the time of
    /// this inject. Written anyway — configured paths are grandfathered,
    /// never hard-gated (see §4.4 of the issue #8 plan) — but reported so
    /// the caller can see it. A path only ever appears here once: writing
    /// through leaves the marker in place, so the next inject finds it
    /// `Managed` and it drops off this list.
    #[serde(rename = "unmanagedPaths")]
    pub unmanaged_paths: Vec<String>,
    /// `.bak` paths created because a write target was `Foreign` — see
    /// `envfile::commit`. Empty unless a pre-existing unmanaged file was
    /// just overwritten.
    pub backups: Vec<String>,
    /// Keys that were NOT written: the referenced vault item could not be
    /// decrypted, or the key is not a valid dotenv variable name. Names
    /// only — never values. The other keys are written regardless.
    #[serde(rename = "failedKeys")]
    pub failed_keys: Vec<String>,
}

/// Decrypts one stored item and returns its injectable value, zeroizing the
/// intermediate plaintext. The error carries no item content.
fn decrypt_item_value(vault_key: &[u8; 32], data: &str) -> Result<Zeroizing<String>, String> {
    let json = Zeroizing::new(crate::crypto::decrypt(vault_key, data)?);
    let item: crate::vault::VaultItem =
        serde_json::from_slice(&json).map_err(|_| "parse item".to_string())?;
    Ok(Zeroizing::new(item.value.or(item.password).or(item.content).unwrap_or_default()))
}

/// Result of `environment_inject_preview` — resolves and inspects paths
/// without decrypting anything or writing a byte. Lets the GUI show a
/// confirm dialog naming the exact files that would be overwritten before
/// the user commits to `overwrite: true`.
#[derive(Serialize)]
pub struct InjectPreview {
    pub paths: Vec<String>,
    pub foreign: Vec<String>,
    /// Targets that are symlinks; injecting into them is refused.
    pub symlinks: Vec<String>,
}

/// Where a write-target path came from, for gating purposes (§4.4 of the
/// issue #8 plan): a path the vault owner saved into `environment.paths[]`
/// through the GUI is pre-consented and only ever reported if unmanaged; a
/// path an API/MCP caller invented on this one request is hard-gated.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PathOrigin {
    Configured,
    CallerSupplied,
}

// ─── Pure logic (no Tauri/Axum coupling) ──────────────────────────────────────
// Shared by the Tauri commands below and the HTTP handlers in `api::mod`, which
// previously duplicated this logic almost verbatim.

pub async fn list_projects(db: &VaultDb) -> Result<Vec<Project>, String> {
    let db_projects = db.list_projects().await?;
    let mut result = Vec::with_capacity(db_projects.len());
    for p in db_projects {
        let db_envs = db.list_environments(p.id).await?;
        let mut environments = Vec::with_capacity(db_envs.len());
        for e in db_envs {
            let db_vars = db.get_environment_vars(e.id).await?;
            environments.push(Environment {
                id: e.id,
                project_id: e.project_id,
                name: e.name,
                is_default: e.is_default,
                paths: e.paths,
                // Vars still awaiting the post-unlock literal→item migration
                // (item_id not yet set) are skipped rather than exposed broken.
                vars: db_vars
                    .into_iter()
                    .filter_map(|v| v.item_id.map(|item_id| EnvironmentVar { id: v.id, key: v.key, item_id }))
                    .collect(),
                created: e.created,
                updated: e.updated,
            });
        }
        let categories = db.list_project_categories(p.id).await?;
        result.push(Project {
            id: p.id,
            name: p.name,
            description: p.description,
            template: p.template,
            created: p.created,
            updated: p.updated,
            environments,
            categories,
            root_path: p.root_path,
        });
    }
    Ok(result)
}

/// Resolves category NAMES (as sent by the frontend, same convention as
/// `VaultItem.categories`) to their stable ids for storage.
async fn category_names_to_ids(db: &VaultDb, names: &[String]) -> Result<Vec<String>, String> {
    let all = db.list_categories().await?;
    Ok(all
        .into_iter()
        .filter(|c| names.contains(&c.name))
        .map(|c| c.cid)
        .collect())
}

// ─── Name validation (issue #7) ────────────────────────────────────────────────
//
// Two rules, deliberately asymmetric (see the plan §3/step 2 and §4/D2):
// environment names are machine identifiers that end up as a filename
// component, so they get a strict allowlist; project names are human labels
// ("My App" is normal) so they get a laxer deny-list instead. Both share the
// hostile-character core below, which mirrors (but does not import — this
// module has no dependency on `fsguard`, and `fsguard`'s public surface is
// deliberately just `resolve_within`/`ContainmentError`) `fsguard`'s
// step-1..3 rules. This is the choke point: called from `save_environment`
// and `save_project` below, so every caller — HTTP, the Tauri commands, an
// imported `.cryptenv-proj` template, and the CLI — goes through the same
// check before a name is ever persisted.

/// Hostile-character core shared by both name validators: separators,
/// control characters, NUL, `.`/`..` as a whole name, NTFS alternate-data-
/// stream `:` and the rest of the Win32-illegal set, trailing dot/space, and
/// Windows reserved device names. Returns the specific rule that was broken
/// (never echoes `name` itself — see plan §4/D5).
fn reject_filesystem_hostile(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("must not be empty or whitespace-only".to_string());
    }
    if name.contains('\0') || name.chars().any(|c| c.is_control()) {
        return Err("must not contain control characters or a NUL byte".to_string());
    }
    if name.contains('/') || name.contains('\\') {
        return Err("must not contain '/' or '\\'".to_string());
    }
    if name == "." || name == ".." {
        return Err("must not be '.' or '..'".to_string());
    }
    if name.contains(':') || name.chars().any(|c| matches!(c, '<' | '>' | '"' | '|' | '?' | '*')) {
        return Err("must not contain ':', '<', '>', '\"', '|', '?', or '*'".to_string());
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err("must not end with a trailing '.' or space".to_string());
    }
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let base = name.split('.').next().unwrap_or(name);
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(base)) {
        return Err("must not be a reserved device name (CON, PRN, AUX, NUL, COM1-9, LPT1-9)".to_string());
    }
    Ok(())
}

/// True for the unnamed root environment: the empty name `""`, or the legacy
/// `"default"` name that older projects were created with. Both map to the
/// canonical, extensionless `.env` file.
pub fn is_root_environment(name: &str) -> bool {
    name.is_empty() || name.eq_ignore_ascii_case("default")
}

/// Filename an environment injects into when only a directory is given:
/// `.env` for the root environment (see `is_root_environment`), otherwise
/// `.env.<name>` (lowercased).
pub fn environment_filename(name: &str) -> String {
    if is_root_environment(name) {
        ".env".to_string()
    } else {
        format!(".env.{}", name.to_ascii_lowercase())
    }
}

/// Placeholder-template counterpart of `environment_filename`:
/// `.env.example` for the root environment, `.env.example.<name>` otherwise.
pub fn environment_example_filename(name: &str) -> String {
    if is_root_environment(name) {
        ".env.example".to_string()
    } else {
        format!(".env.example.{}", name.to_ascii_lowercase())
    }
}

/// Strict allowlist: `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`, plus an explicit
/// rejection of a trailing `.` or `-`. Environment names are machine
/// identifiers, land in a filename (`.env.<name>`), and are the field with
/// the proven traversal exploit (issue #7) — the tight rule costs nothing
/// real for names like `production`, `local`, `staging-2`. The empty name is
/// the one exception: it denotes the unnamed root environment (`.env`).
pub fn validate_environment_name(name: &str) -> Result<(), String> {
    const RULE: &str =
        "must be empty (root .env) or 1-64 chars, start with a letter or digit, and contain only letters, digits, '.', '_' or '-'";
    if name.is_empty() {
        return Ok(());
    }
    if reject_filesystem_hostile(name).is_err() {
        return Err(format!("name: {RULE}"));
    }
    let starts_alnum = name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric());
    let charset_ok = name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    let valid = !name.is_empty()
        && name.len() <= 64
        && starts_alnum
        && charset_ok
        && !name.ends_with('.')
        && !name.ends_with('-');
    if valid {
        Ok(())
    } else {
        Err(format!("name: {RULE}"))
    }
}

/// Deny-list, deliberately laxer than the environment rule: rejects the
/// filesystem-hostile core above, plus a length cap and leading dot/
/// whitespace. Allows spaces and non-ASCII letters — project names are human
/// labels ("Mi Proyecto" is a perfectly normal edit to an existing project)
/// and an ASCII-only allowlist would reject real edits for no present
/// security gain (see plan §4/D2).
pub fn validate_project_name(name: &str) -> Result<(), String> {
    if let Err(reason) = reject_filesystem_hostile(name) {
        return Err(format!("name: {reason}"));
    }
    if name.chars().count() > 128 {
        return Err("name: must be 128 characters or fewer".to_string());
    }
    if name.starts_with('.') || name.starts_with(' ') {
        return Err("name: must not start with '.' or whitespace".to_string());
    }
    Ok(())
}

/// Creates (id = 0) or updates (id > 0) a project's metadata. A newly created
/// project always gets one 'default' environment so it's immediately usable.
pub async fn save_project(db: &VaultDb, input: ProjectInput) -> Result<i64, String> {
    validate_project_name(&input.name)?;
    let root = match input.root_path.as_deref().map(str::trim) {
        None => None,
        Some("") => Some(None),
        Some(r) => Some(Some(validate_root_path(r)?)),
    };
    let is_new = input.id == 0;
    let initial_env = input
        .initial_environment
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or("default")
        .to_ascii_lowercase();
    // Validate before any write so a bad name can't leave an env-less project.
    if is_new {
        validate_environment_name(&initial_env)?;
    }
    let project_id = db
        .upsert_project(input.id, &input.name, input.description.as_deref(), &input.template)
        .await?;
    if is_new {
        // A brand-new project has no environments yet, so this can never
        // actually collide — kept anyway so `save_environment` stays the
        // single choke point for the case-insensitivity rule rather than
        // special-casing the auto-created initial environment.
        ensure_no_case_collision(db, project_id, 0, &initial_env).await?;
        let env_id = db.upsert_environment(0, project_id, &initial_env, true).await?;
        // A project born with a root is injectable immediately: its initial
        // environment targets the conventional file at the root.
        if matches!(root, Some(Some(_))) {
            db.set_environment_paths(env_id, &[environment_filename(&initial_env)]).await?;
        }
    }
    if let Some(r) = &root {
        db.set_project_root(project_id, r.as_deref()).await?;
    }
    let category_ids = category_names_to_ids(db, &input.categories).await?;
    db.set_project_categories(project_id, &category_ids).await?;
    Ok(project_id)
}

/// A project root must be an absolute path as the vault host sees it (a
/// `\\wsl.localhost\…` UNC path counts on Windows). Trailing separators are
/// trimmed so the stored form is canonical for display and comparison.
pub fn validate_root_path(root: &str) -> Result<String, String> {
    if root.contains('\0') || root.chars().any(|c| c.is_control()) {
        return Err("rootPath: must not contain control characters".into());
    }
    if !Path::new(root).is_absolute() {
        return Err("rootPath: must be an absolute directory path".into());
    }
    let trimmed = root.trim_end_matches(['/', '\\']);
    Ok(if trimmed.is_empty() || trimmed.ends_with(':') { root.to_string() } else { trimmed.to_string() })
}

fn is_absolute_config_path(path: &str) -> bool {
    Path::new(path).is_absolute() || path.starts_with("\\\\")
}

/// Resolves one configured environment path to the path actually written.
/// Absolute paths are returned unchanged (legacy behavior). Relative paths
/// (`.env`, `apps/api/.env`, `./x`, either separator) are joined onto the
/// project `root`; they may not climb out of it (`..`) and require a root.
pub fn resolve_env_path(root: Option<&str>, path: &str) -> Result<String, String> {
    if is_absolute_config_path(path) {
        return Ok(path.to_string());
    }
    let root = root.ok_or_else(|| {
        format!("path '{path}' is relative but the project has no root directory — set one in the project settings")
    })?;
    let mut out = PathBuf::from(root);
    let mut pushed = 0usize;
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => continue,
            ".." => return Err(format!("path '{path}' must stay inside the project root")),
            p if p.contains(':') => return Err(format!("path '{path}' is not a valid relative path")),
            p => {
                out.push(p);
                pushed += 1;
            }
        }
    }
    if pushed == 0 {
        return Err(format!("path '{path}' does not name a file"));
    }
    Ok(out.to_string_lossy().into_owned())
}

pub async fn delete_project(db: &VaultDb, id: i64) -> Result<ProjectDeleteImpact, String> {
    db.delete_project(id).await
}

pub async fn project_delete_preview(db: &VaultDb, id: i64) -> Result<ProjectDeleteImpact, String> {
    db.preview_delete_project(id).await
}

/// Case-insensitive collision check against this environment's siblings in
/// the same project, skipping `exclude_id` (so renaming an environment to a
/// different case of its own name is allowed; pass `0` when creating new).
///
/// Unicode-aware (Rust `to_lowercase()`), unlike the DB's `NOCASE` index
/// (SQLite `NOCASE` folds ASCII `A-Z` only) — this is what actually catches a
/// non-ASCII collision like `PRODUCCIÓN` vs `producción`, which satisfies the
/// index but would otherwise reach `resolve_environment`'s ambiguity check
/// only after already being persisted. Racy on its own (TOCTOU between this
/// read and `db.upsert_environment`'s write) — the DB's unique index is the
/// authoritative backstop for the ASCII case; neither layer is redundant,
/// each covers the other's gap.
async fn ensure_no_case_collision(
    db: &VaultDb,
    project_id: i64,
    exclude_id: i64,
    name: &str,
) -> Result<(), String> {
    let siblings = db.list_environments(project_id).await?;
    let lower = name.to_lowercase();
    // `""` and legacy `"default"` both resolve to `.env`, so they collide too.
    let root = is_root_environment(name);
    let collides = siblings.iter().any(|env| {
        env.id != exclude_id && (env.name.to_lowercase() == lower || (root && is_root_environment(&env.name)))
    });
    if collides {
        return Err(crate::db::ENVIRONMENT_NAME_CONFLICT.to_string());
    }
    Ok(())
}

/// Persists an environment's vars, granting item ownership to `project_id` as
/// a side effect for any referenced item it doesn't already own — but only
/// when that item is global. A local item can never silently gain a second
/// owner through this path; the caller must mark it global first.
pub async fn save_environment(db: &VaultDb, input: EnvironmentInput) -> Result<i64, String> {
    // Canonical form is lowercase ASCII: the name becomes the `.env.<name>`
    // filename and frameworks look it up case-sensitively.
    let input = EnvironmentInput { name: input.name.to_ascii_lowercase(), ..input };
    // Order matters: reject a structurally invalid name (issue #7) before
    // spending a query on the collision check (issue #12).
    validate_environment_name(&input.name)?;
    ensure_no_case_collision(db, input.project_id, input.id, &input.name).await?;

    // Validate before opening the transaction so rejected input never writes.
    crate::db::validate_unique_keys(input.vars.iter().map(|v| v.key.as_str()))?;
    crate::db::validate_unique_paths(&input.paths)?;

    // Rename, paths, ownership grants and vars commit together or not at all.
    // Dropping `tx` on an early `?` return rolls everything back.
    let mut tx = db.begin().await?;

    let env_id =
        VaultDb::upsert_environment_tx(&mut tx, input.id, input.project_id, &input.name, input.is_default)
            .await?;

    VaultDb::set_environment_paths_tx(&mut tx, env_id, &input.paths).await?;

    for v in &input.vars {
        let owners = VaultDb::list_owning_projects_tx(&mut tx, v.item_id).await?;
        if owners.contains(&input.project_id) {
            continue;
        }
        if VaultDb::is_item_global_tx(&mut tx, v.item_id).await?.unwrap_or(false) {
            VaultDb::add_item_owner_tx(&mut tx, v.item_id, input.project_id).await?;
        } else {
            return Err(format!(
                "item {} is not global and not already owned by this project",
                v.item_id
            ));
        }
    }

    let db_vars: Vec<DbEnvironmentVar> = input
        .vars
        .into_iter()
        .map(|v| DbEnvironmentVar { id: 0, environment_id: env_id, key: v.key, item_id: Some(v.item_id), literal: None })
        .collect();

    VaultDb::set_environment_vars_tx(&mut tx, env_id, &db_vars).await?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(env_id)
}

pub async fn delete_environment(db: &VaultDb, id: i64) -> Result<(), String> {
    db.delete_environment(id).await
}

/// Stable prefix on the `Err` string `resolve_environment` returns when a
/// case-insensitive project/environment lookup matches more than one
/// candidate. `pub` so callers match on this constant instead of a bare
/// string literal — today that's the HTTP layer's `resolve_scope`, mapping
/// it to `409 AMBIGUOUS_SCOPE`.
pub const AMBIGUOUS_MATCH_PREFIX: &str = "ambiguous match";

/// Resolves the environment identified either by numeric `environment_id`,
/// or by a case-insensitive `project` name + `environment` name pair — the
/// same two lookup shapes CLI's `project inject --id` / `--project
/// --environment` already offers against `GET /projects`. Every HTTP
/// endpoint that needs to scope its work to a single project+environment
/// reuses this instead of inventing a second lookup convention.
///
/// Rejects ambiguity instead of guessing: if more than one project matches
/// `project`, or more than one environment within the resolved project
/// matches `environment`, this returns an `Err` starting with
/// `AMBIGUOUS_MATCH_PREFIX` and naming every colliding candidate, rather than
/// silently taking the first (`ORDER BY id ASC`) match. This is required, not
/// defence-in-depth — SQLite's `NOCASE` (used by `idx_projects_name_nocase` /
/// `idx_environments_name_nocase`) folds ASCII `A-Z` only, while the
/// `to_lowercase()` comparisons here fold full Unicode. So `PRODUCCIÓN` and
/// `producción` can both satisfy the index (SQLite sees two distinct names)
/// while still colliding here — the index alone does not close that case;
/// this check does.
pub async fn resolve_environment(
    db: &VaultDb,
    environment_id: Option<i64>,
    project: Option<&str>,
    environment: Option<&str>,
) -> Result<Environment, String> {
    if let Some(id) = environment_id {
        let env = get_environment_full(db, id).await?;
        return env.ok_or_else(|| "environment not found".to_string());
    }

    if let (Some(p), Some(e)) = (project, environment) {
        let p_lower = p.to_lowercase();
        let e_lower = e.to_lowercase();
        let projects = list_projects(db).await?;

        let matching_projects: Vec<Project> =
            projects.into_iter().filter(|proj| proj.name.to_lowercase() == p_lower).collect();

        if matching_projects.len() > 1 {
            let options: Vec<String> = matching_projects
                .iter()
                .map(|proj| format!("{} (id {})", proj.name, proj.id))
                .collect();
            return Err(format!(
                "{AMBIGUOUS_MATCH_PREFIX} for project '{p}': {}. Pass environment_id instead.",
                options.join(", ")
            ));
        }

        let proj = match matching_projects.into_iter().next() {
            Some(proj) => proj,
            None => return Err(format!("project/environment not found: {p} / {e}")),
        };

        // The root environment answers to both `""` and legacy `"default"`.
        let e_root = is_root_environment(e);
        let matching_envs: Vec<Environment> = proj
            .environments
            .into_iter()
            .filter(|env| env.name.to_lowercase() == e_lower || (e_root && is_root_environment(&env.name)))
            .collect();

        if matching_envs.len() > 1 {
            let options: Vec<String> =
                matching_envs.iter().map(|env| format!("{} (id {})", env.name, env.id)).collect();
            return Err(format!(
                "{AMBIGUOUS_MATCH_PREFIX} for environment '{e}': {}. Pass environment_id instead.",
                options.join(", ")
            ));
        }

        return matching_envs
            .into_iter()
            .next()
            .ok_or_else(|| format!("project/environment not found: {p} / {e}"));
    }

    Err("provide environment_id, or both project and environment".to_string())
}

async fn get_environment_full(db: &VaultDb, id: i64) -> Result<Option<Environment>, String> {
    let env = match db.get_environment(id).await? {
        Some(e) => e,
        None => return Ok(None),
    };
    let vars = db.get_environment_vars(id).await?;
    Ok(Some(Environment {
        id: env.id,
        project_id: env.project_id,
        name: env.name,
        is_default: env.is_default,
        paths: env.paths,
        vars: vars
            .into_iter()
            .filter_map(|v| v.item_id.map(|item_id| EnvironmentVar { id: v.id, key: v.key, item_id }))
            .collect(),
        created: env.created,
        updated: env.updated,
    }))
}

/// Resolves this environment's write targets (its configured `paths[]` plus
/// an optional `output_path`/`output_dir`-derived path) and classifies each
/// one via `envfile::inspect`, without decrypting anything or writing a
/// byte. Shared by `inject_environment` (phases 1-2 below) and
/// `inject_environment_preview` (which stops here).
///
/// `targets`, when given, restricts the configured paths to that subset
/// (matched against the stored strings, e.g. the GUI's inject-target
/// picker); naming a path that is not configured is an error. Relative
/// configured paths are resolved against the project root first.
async fn resolve_and_inspect(
    db: &VaultDb,
    environment_id: i64,
    output_path: Option<String>,
    output_dir: Option<String>,
    targets: Option<&[String]>,
) -> Result<(crate::db::DbEnvironment, Vec<(String, PathOrigin)>, HashMap<String, envfile::Target>), String> {
    let env = db
        .get_environment(environment_id)
        .await?
        .ok_or("environment not found")?;

    let configured: Vec<&String> = match targets {
        None => env.paths.iter().collect(),
        Some(t) => {
            if let Some(unknown) = t.iter().find(|p| !env.paths.contains(p)) {
                return Err(format!("target '{unknown}' is not a configured path of this environment"));
            }
            env.paths.iter().filter(|p| t.contains(p)).collect()
        }
    };
    let root = if configured.is_empty() { None } else { db.get_project_root(env.project_id).await? };
    let mut resolved: Vec<(String, PathOrigin)> = Vec::with_capacity(configured.len());
    for p in configured {
        let mut r = resolve_env_path(root.as_deref(), p)?;
        // A relative path must stay inside the root even through symlinked
        // directories. Absolute paths are the owner's explicit choice and
        // keep their location (the final component is still no-follow).
        if !is_absolute_config_path(p) {
            if let Some(root) = root.as_deref() {
                let contained = crate::fsguard::contain_relative(Path::new(root), p)
                    .map_err(|e| format!("path '{p}' must stay inside the project root ({e})"))?;
                r = contained.to_string_lossy().into_owned();
            }
        }
        if !resolved.iter().any(|(existing, _)| existing == &r) {
            resolved.push((r, PathOrigin::Configured));
        }
    }

    if let Some(p) = output_path {
        if !resolved.iter().any(|(existing, _)| existing == &p) {
            resolved.push((p, PathOrigin::CallerSupplied));
        }
    } else if resolved.is_empty() {
        if let Some(dir) = output_dir {
            // Contained resolution (issue #7): the environment name is
            // stored, untrusted data — it may only pick the filename inside
            // `dir`, never redirect the write elsewhere. This is the same
            // sink `/fill` and `/environments/:id/example` guard in
            // `api::mod`; issue #8 moved the resolution in here, so the
            // containment check has to live here too rather than at the
            // former call site.
            let target = crate::fsguard::resolve_within(dir.as_str(), &environment_filename(&env.name))
                .map_err(|e| format!("output_dir: {e}"))?;
            resolved.push((target.to_string_lossy().into_owned(), PathOrigin::CallerSupplied));
        }
    }

    let mut inspected: HashMap<String, envfile::Target> = HashMap::new();
    for (path, _) in &resolved {
        let target = envfile::inspect(Path::new(path)).map_err(|e| match e {
            // Bare message so the HTTP handler maps it to 409 NOT_REGULAR_FILE.
            envfile::EnvFileError::NotRegularFile(_) => e.to_string(),
            e => format!("inspect {path}: {e}"),
        })?;
        inspected.insert(path.clone(), target);
    }

    Ok((env, resolved, inspected))
}

/// Read-only dry run of `inject_environment`'s path resolution and gate,
/// for the GUI's pre-inject confirm dialog: lists every configured path
/// that is currently `Foreign` (would be reported in `unmanaged_paths` on
/// an actual inject) so the user can be asked before anything is touched.
pub async fn inject_environment_preview(
    db: &VaultDb,
    environment_id: i64,
    targets: Option<&[String]>,
) -> Result<InjectPreview, String> {
    let (_, resolved, inspected) = resolve_and_inspect(db, environment_id, None, None, targets).await?;

    let paths: Vec<String> = resolved.iter().map(|(p, _)| p.clone()).collect();
    let foreign: Vec<String> = resolved
        .iter()
        .filter(|(p, _)| matches!(inspected.get(p), Some(envfile::Target::Foreign)))
        .map(|(p, _)| p.clone())
        .collect();

    let symlinks: Vec<String> = resolved
        .iter()
        .filter(|(p, _)| matches!(inspected.get(p), Some(envfile::Target::Symlink)))
        .map(|(p, _)| p.clone())
        .collect();

    Ok(InjectPreview { paths, foreign, symlinks })
}

/// Decrypt referenced vault items and write KEY=VALUE pairs into every path
/// configured on this environment, plus `output_path` (if given — added to
/// the configured set, never replacing it) or, when the environment has no
/// configured paths and no `output_path` was given, a single default file
/// named `.env.<environment-name>` (or `.env` for the root environment)
/// inside `output_dir`. Merges with existing
/// file content (existing keys not in the environment are preserved).
/// `written` reports the union of keys touched across ALL paths, not just
/// the first one.
///
/// Gate strictly precedes decryption (issue #8): every resolved path is
/// classified before a single secret is decrypted or a byte is written. A
/// `output_path`/`output_dir`-derived path (`CallerSupplied`) that is
/// `Foreign` is hard-refused unless `overwrite` is `true`. A path the vault
/// owner saved into `environment.paths[]` (`Configured`) is never refused —
/// only reported in `unmanaged_paths` — since gating those would break
/// every pre-existing installation on upgrade (see §4.4 of the issue #8
/// plan); it is written through, taking a `.bak` of its prior contents on
/// this first touch, and self-heals (becomes `Managed`) from then on.
pub async fn inject_environment(
    db: &VaultDb,
    vault_key: &[u8; 32],
    environment_id: i64,
    output_path: Option<String>,
    output_dir: Option<String>,
    overwrite: bool,
    targets: Option<&[String]>,
) -> Result<InjectResult, String> {
    // Phase 1 — resolve, tracking provenance.
    let (env, resolved, mut cached) =
        resolve_and_inspect(db, environment_id, output_path, output_dir, targets).await?;

    if resolved.is_empty() {
        return Err("environment has no paths configured".into());
    }

    // Phase 2 — gate, strictly before any decryption or write.
    // A symlink target is refused outright (never overridable by
    // `overwrite`), so nothing is written for any path of this environment.
    if let Some((path, _)) = resolved
        .iter()
        .find(|(p, _)| matches!(cached.get(p), Some(envfile::Target::Symlink)))
    {
        return Err(envfile::symlink_message(Path::new(path)));
    }
    let mut refused: Vec<PathBuf> = Vec::new();
    let mut unmanaged: Vec<String> = Vec::new();
    for (path, origin) in &resolved {
        if matches!(cached.get(path), Some(envfile::Target::Foreign)) {
            match origin {
                PathOrigin::CallerSupplied => refused.push(PathBuf::from(path)),
                PathOrigin::Configured => unmanaged.push(path.clone()),
            }
        }
    }

    if !refused.is_empty() && !overwrite {
        return Err(envfile::refuse_message(&refused));
    }

    let paths: Vec<String> = resolved.iter().map(|(p, _)| p.clone()).collect();
    let vars = db.get_environment_vars(environment_id).await?;

    if vars.is_empty() {
        return Ok(InjectResult { paths, written: vec![], unmanaged_paths: unmanaged, backups: vec![], failed_keys: vec![] });
    }

    // Phase 3 — decrypt, only reachable past the gate above. Only the items
    // this environment references are decrypted, and values are held in
    // zeroizing containers. An item that can't be decrypted fails its own
    // key (reported by name), never the whole inject.
    let referenced: Vec<i64> = vars.iter().filter_map(|v| v.item_id).collect();
    let raw_items = db.get_items_by_ids(&referenced).await?;
    let mut item_values: HashMap<i64, Zeroizing<String>> = HashMap::new();
    let mut undecryptable: HashSet<i64> = HashSet::new();
    for (item_id, _, data, _, _) in &raw_items {
        match decrypt_item_value(vault_key, data) {
            Ok(value) => {
                item_values.insert(*item_id, value);
            }
            Err(_) => {
                undecryptable.insert(*item_id);
            }
        }
    }

    // Build key → value map for this environment
    let mut inject_map: HashMap<String, Zeroizing<String>> = HashMap::new();
    let mut failed: HashSet<String> = HashSet::new();
    for v in &vars {
        // A key crypt-env cannot write as a valid dotenv name is reported,
        // not written.
        if !envfile::is_valid_key(&v.key) {
            failed.insert(v.key.clone());
            continue;
        }
        let value = match v.item_id {
            Some(iid) if undecryptable.contains(&iid) => {
                failed.insert(v.key.clone());
                continue;
            }
            Some(iid) => item_values.get(&iid).cloned().unwrap_or_default(),
            None => Zeroizing::new(v.literal.clone().unwrap_or_default()),
        };
        inject_map.insert(v.key.clone(), value);
    }
    let mut failed_keys: Vec<String> = failed.into_iter().collect();
    failed_keys.sort();

    // Marker names the project + environment (informational only, never
    // parsed back — see `envfile::marker_line`).
    let project_name = db
        .get_project_name(env.project_id)
        .await?
        .unwrap_or_else(|| "unknown".to_string());
    let marker = envfile::marker_line(&project_name, &env.name);

    // Phase 4 — write. `written` reports the union of keys touched across
    // ALL paths, not just the first one.
    let mut written: HashSet<String> = HashSet::new();
    let mut backups: Vec<String> = Vec::new();
    let mut done: Vec<String> = Vec::new();

    for (path, origin) in &resolved {
        let existing = match cached.remove(path) {
            Some(envfile::Target::Managed(content)) => content,
            _ => String::new(),
        };
        let mut lines: Vec<String> = existing.lines().map(String::from).collect();
        let mut updated_keys: HashSet<String> = HashSet::new();

        for line in &mut lines {
            let trimmed = line.trim();
            if trimmed.starts_with('#') || !trimmed.contains('=') {
                continue;
            }
            let eq_pos = match trimmed.find('=') {
                Some(p) => p,
                None => continue,
            };
            let existing_key = trimmed[..eq_pos].trim().to_string();
            if let Some(new_val) = inject_map.get(&existing_key) {
                // Keys in `inject_map` are validated, so this always yields a line.
                if let Some(serialized) = envfile::serialize_line(&existing_key, new_val) {
                    *line = serialized;
                    updated_keys.insert(existing_key.clone());
                    written.insert(existing_key);
                }
            }
        }

        // Sorted so the written file is stable across injects.
        let mut pending: Vec<&String> = inject_map.keys().filter(|k| !updated_keys.contains(*k)).collect();
        pending.sort();
        for k in pending {
            if let Some(serialized) = inject_map.get(k).and_then(|v| envfile::serialize_line(k, v)) {
                lines.push(serialized);
                written.insert(k.clone());
            }
        }

        // Holds secret values; wiped when it goes out of scope.
        let content = Zeroizing::new(lines.join("\n") + "\n");

        // Configured paths are pre-consented by the vault owner (§4.4) and
        // are written through even if Foreign — never hard-gated. Caller-
        // supplied paths only reach here Absent or Managed, unless the
        // caller explicitly passed `overwrite: true` past the gate above.
        let effective_overwrite = overwrite || *origin == PathOrigin::Configured;
        let opts = envfile::WriteOptions { overwrite: effective_overwrite, mode: envfile::FileMode::Private0600 };

        match envfile::commit(Path::new(path), &content, &marker, &opts) {
            Ok(committed) => {
                if let Some(bak) = committed.backup {
                    backups.push(bak.to_string_lossy().into_owned());
                }
                done.push(path.clone());
            }
            Err(e @ envfile::EnvFileError::BackupExists(_))
            | Err(e @ envfile::EnvFileError::TargetExists(_))
            | Err(e @ envfile::EnvFileError::Symlink(_))
            | Err(e @ envfile::EnvFileError::NotRegularFile(_)) => {
                // Keep the bare message so `err.starts_with(BACKUP_EXISTS_PREFIX
                // | TARGET_EXISTS_PREFIX)` still matches in the HTTP handler
                // and this maps to 409, not the generic 500 below.
                // (`TargetExists` should be unreachable here in practice —
                // `effective_overwrite` is always `true` for every path that
                // reaches this loop — kept as a defensive fallback.)
                return Err(e.to_string());
            }
            Err(e) => {
                // Stop-and-report: cross-file atomicity across arbitrary
                // user-chosen locations is not achievable (§4.5) — name
                // the failing path and everything already written.
                let already = if done.is_empty() { "none".to_string() } else { done.join(", ") };
                return Err(format!("write .env ({path}): {e} (already written: {already})"));
            }
        }
    }

    let mut written_keys: Vec<String> = written.into_iter().collect();
    written_keys.sort();

    Ok(InjectResult { paths, written: written_keys, unmanaged_paths: unmanaged, backups, failed_keys })
}

// ─── Tauri commands ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn project_list(state: State<'_, SharedState>) -> Result<Vec<Project>, String> {
    let s = state.lock().await;
    list_projects(&s.db).await
}

#[tauri::command]
pub async fn project_save(state: State<'_, SharedState>, project: ProjectInput) -> Result<i64, String> {
    let s = state.lock().await;
    save_project(&s.db, project).await
}

#[tauri::command]
pub async fn project_delete(state: State<'_, SharedState>, id: i64) -> Result<ProjectDeleteImpact, String> {
    let s = state.lock().await;
    delete_project(&s.db, id).await
}

#[tauri::command]
pub async fn project_preview_delete(state: State<'_, SharedState>, id: i64) -> Result<ProjectDeleteImpact, String> {
    let s = state.lock().await;
    project_delete_preview(&s.db, id).await
}

#[tauri::command]
pub async fn environment_save(
    state: State<'_, SharedState>,
    environment: EnvironmentInput,
) -> Result<i64, String> {
    let s = state.lock().await;
    save_environment(&s.db, environment).await
}

#[tauri::command]
pub async fn environment_delete(state: State<'_, SharedState>, id: i64) -> Result<(), String> {
    let s = state.lock().await;
    delete_environment(&s.db, id).await
}

#[tauri::command]
pub async fn environment_inject(
    state: State<'_, SharedState>,
    id: i64,
    overwrite: bool,
    targets: Option<Vec<String>>,
) -> Result<InjectResult, String> {
    let s = state.lock().await;
    let key = s.key.as_ref().ok_or("vault is locked")?;
    let vault_key: [u8; 32] = **key;
    inject_environment(&s.db, &vault_key, id, None, None, overwrite, targets.as_deref()).await
}

/// Dry run for the GUI's pre-inject confirm dialog — see
/// `inject_environment_preview`. Only ever inspects `environment.paths[]`:
/// the GUI never supplies a caller `output_path`/`output_dir`, so there is
/// nothing to hard-gate here, only `Foreign` configured paths to surface.
#[tauri::command]
pub async fn environment_inject_preview(
    state: State<'_, SharedState>,
    id: i64,
    targets: Option<Vec<String>>,
) -> Result<InjectPreview, String> {
    let s = state.lock().await;
    inject_environment_preview(&s.db, id, targets.as_deref()).await
}

#[tauri::command]
pub async fn project_pick_env_path(start_dir: Option<String>) -> Result<Option<String>, String> {
    let result = tokio::task::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Select .env file")
            .add_filter("All files", &["*"])
            .add_filter("Env files", &["env"]);
        if let Some(dir) = start_dir {
            dialog = dialog.set_directory(dir);
        }
        dialog.pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;

    Ok(result.map(|p| p.to_string_lossy().into_owned()))
}

// ─── Project root & `.crypt-env.yaml` (gui-project-yaml-paths) ──────────────

#[tauri::command]
pub async fn project_pick_root_dir(start_dir: Option<String>) -> Result<Option<String>, String> {
    let result = tokio::task::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new().set_title("Select project root directory");
        if let Some(dir) = start_dir {
            dialog = dialog.set_directory(dir);
        }
        dialog.pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(result.map(|p| p.to_string_lossy().into_owned()))
}

/// Inline validation for the project form's root field.
#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RootCheck {
    /// `None` when usable; otherwise `notAbsolute` | `notFound` | `notDir` | `notWritable`.
    pub problem: Option<String>,
    pub has_manifest: bool,
}

pub fn check_root(root: &str) -> RootCheck {
    let problem = |p: &str| RootCheck { problem: Some(p.to_string()), has_manifest: false };
    if validate_root_path(root).is_err() {
        return problem("notAbsolute");
    }
    let dir = Path::new(root);
    match std::fs::metadata(dir) {
        Err(_) => return problem("notFound"),
        Ok(m) if !m.is_dir() => return problem("notDir"),
        Ok(_) => {}
    }
    // Probe writability with a throwaway file (a read-only flag on the
    // directory is not reliable across Windows/WSL shares).
    let probe = dir.join(format!(".crypt-env-write-test-{}", std::process::id()));
    if std::fs::write(&probe, b"").is_err() {
        return problem("notWritable");
    }
    let _ = std::fs::remove_file(&probe);
    RootCheck { problem: None, has_manifest: dir.join(manifest::FILE_NAME).is_file() }
}

#[tauri::command]
pub async fn project_check_root(root: String) -> Result<RootCheck, String> {
    tokio::task::spawn_blocking(move || check_root(root.trim())).await.map_err(|e| e.to_string())
}

/// Writes `.crypt-env.yaml` for `project_id` into its root from vault state
/// (same serializer as `crypt-env init`/`config`). Refuses to replace an
/// existing file unless `overwrite`; that refusal is the stable error
/// `"manifest exists"` so the GUI can offer overwrite / link.
///
/// Split in two so callers can drop the vault lock between the steps:
/// [`load_manifest_source`] reads vault state, [`write_manifest_at`] is pure
/// filesystem I/O (a `\\wsl.localhost` root can stall it for a long time).
pub async fn write_project_manifest(db: &VaultDb, project_id: i64, overwrite: bool) -> Result<String, String> {
    let (project, root) = load_manifest_source(db, project_id).await?;
    write_manifest_at(&root, &project, overwrite)
}

/// Vault-state half of [`write_project_manifest`]: the project and its root.
pub async fn load_manifest_source(db: &VaultDb, project_id: i64) -> Result<(Project, PathBuf), String> {
    let project = list_projects(db)
        .await?
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or("project not found")?;
    let root = project.root_path.clone().ok_or("project has no root directory")?;
    Ok((project, PathBuf::from(root)))
}

/// Filesystem half of [`write_project_manifest`]. Touches no vault state.
pub fn write_manifest_at(dir: &Path, project: &Project, overwrite: bool) -> Result<String, String> {
    if !dir.is_dir() {
        return Err("root directory does not exist".into());
    }
    if dir.join(manifest::FILE_NAME).exists() && !overwrite {
        return Err("manifest exists".into());
    }
    let m = manifest::from_project(project, &|p| p.to_string());
    let path = manifest::write_file(dir, &m)?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn project_write_yaml(
    state: State<'_, SharedState>,
    project_id: i64,
    overwrite: bool,
) -> Result<String, String> {
    // The vault lock covers only the DB read; the guard is dropped before any
    // filesystem call, which runs on a blocking thread.
    let (project, root) = {
        let s = state.lock().await;
        load_manifest_source(&s.db, project_id).await?
    };
    tokio::task::spawn_blocking(move || write_manifest_at(&root, &project, overwrite))
        .await
        .map_err(|e| e.to_string())?
}

// ─── Export / Import ──────────────────────────────────────────────────────────
// Exports a whole project (all its environments) as a reusable template. Paths
// are machine-specific and intentionally dropped; item references can't cross
// vaults either, so only literal values survive the round-trip.

#[derive(Serialize, Deserialize)]
pub struct ExportedEnvironmentVar {
    pub key: String,
    pub literal: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ExportedEnvironment {
    pub name: String,
    #[serde(rename = "isDefault")]
    pub is_default: bool,
    pub vars: Vec<ExportedEnvironmentVar>,
}

#[derive(Serialize, Deserialize)]
pub struct ExportedProject {
    pub version: u8,
    pub name: String,
    pub description: Option<String>,
    pub template: String,
    pub environments: Vec<ExportedEnvironment>,
}

#[tauri::command]
pub async fn project_export(project_id: i64, state: State<'_, SharedState>) -> Result<(), String> {
    export_project_with(project_id, &state, |default_name| {
        tokio::task::spawn_blocking(move || {
            rfd::FileDialog::new()
                .set_title("Save project template")
                .set_file_name(&default_name)
                .add_filter("CryptEnv Project", &["cryptenv-proj"])
                .save_file()
        })
    })
    .await
}

/// Body of `project_export`, with the save dialog injected so a test can
/// assert the vault lock is free while the dialog is open. The lock covers
/// only the DB read: the dialog and the file write run after the guard is
/// dropped, otherwise auto-lock (and every other interface) would wait for
/// the user to dismiss the dialog.
async fn export_project_with<P>(
    project_id: i64,
    state: &SharedState,
    pick_file: impl FnOnce(String) -> P,
) -> Result<(), String>
where
    P: std::future::Future<Output = Result<Option<std::path::PathBuf>, tokio::task::JoinError>>,
{
    let project = {
        let s = state.lock().await;
        list_projects(&s.db)
            .await?
            .into_iter()
            .find(|p| p.id == project_id)
            .ok_or("project not found")?
    };

    let exported = ExportedProject {
        version: 1,
        name: project.name.clone(),
        description: project.description.clone(),
        template: project.template.clone(),
        environments: project
            .environments
            .into_iter()
            .map(|e| ExportedEnvironment {
                name: e.name,
                is_default: e.is_default,
                // Every var is a real vault item now, and item values never
                // cross vaults — templates only ever carry KEY names, never
                // resolved values, to avoid leaking secrets into an exported file.
                vars: e
                    .vars
                    .into_iter()
                    .map(|v| ExportedEnvironmentVar { key: v.key, literal: None })
                    .collect(),
            })
            .collect(),
    };

    let json = serde_json::to_vec_pretty(&exported).map_err(|e| e.to_string())?;

    let safe_name = project.name.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_");
    let default_name = format!("{}.cryptenv-proj", safe_name);

    let path = pick_file(default_name)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "cancelled".to_string())?;

    tokio::task::spawn_blocking(move || std::fs::write(&path, &json))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn project_import() -> Result<ExportedProject, String> {
    let path = tokio::task::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("Load project template")
            .add_filter("CryptEnv Project", &["cryptenv-proj"])
            .pick_file()
    })
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "cancelled".to_string())?;

    let json = std::fs::read(&path).map_err(|e| e.to_string())?;
    let project: ExportedProject = serde_json::from_slice(&json).map_err(|e| format!("invalid file: {e}"))?;
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::VaultDb;
    use crate::vault::VaultItem;

    async fn test_db() -> (tempfile::TempDir, VaultDb) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        let db = VaultDb::open(path.to_str().unwrap()).await.unwrap();
        (dir, db)
    }

    fn new_project(name: &str, initial_environment: Option<&str>) -> ProjectInput {
        ProjectInput {
            id: 0,
            name: name.to_string(),
            description: None,
            template: "generic".to_string(),
            categories: vec![],
            initial_environment: initial_environment.map(str::to_string),
            root_path: None,
        }
    }

    #[tokio::test]
    async fn new_project_initial_environment_defaults_and_customises() {
        let (_dir, db) = test_db().await;
        for (name, initial, expected) in [
            ("p1", None, "default"),
            ("p2", Some("   "), "default"),
            ("p3", Some(" production "), "production"),
        ] {
            let id = save_project(&db, new_project(name, initial)).await.unwrap();
            let envs = db.list_environments(id).await.unwrap();
            assert_eq!(envs.len(), 1);
            assert_eq!(envs[0].name, expected);
            assert!(envs[0].is_default);
        }
    }

    #[tokio::test]
    async fn invalid_initial_environment_rejected_before_any_write() {
        let (_dir, db) = test_db().await;
        let err = save_project(&db, new_project("p", Some("../evil"))).await.unwrap_err();
        assert!(err.starts_with("name: "));
        assert!(db.list_projects().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn initial_environment_ignored_on_update() {
        let (_dir, db) = test_db().await;
        let id = save_project(&db, new_project("p", None)).await.unwrap();
        let mut update = new_project("p", Some("staging"));
        update.id = id;
        save_project(&db, update).await.unwrap();
        let envs = db.list_environments(id).await.unwrap();
        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].name, "default");
    }

    fn plain_secret(name: &str, value: &str) -> VaultItem {
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
            is_global: Some(false),
        }
    }

    // ─── resolve_environment ────────────────────────────────────────────

    #[tokio::test]
    async fn resolve_by_environment_id() {
        let (_dir, db) = test_db().await;
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let env_id = db.upsert_environment(0, project_id, "production", true).await.unwrap();

        let env = resolve_environment(&db, Some(env_id), None, None).await.unwrap();
        assert_eq!(env.id, env_id);
        assert_eq!(env.project_id, project_id);
        assert_eq!(env.name, "production");
    }

    #[tokio::test]
    async fn resolve_by_environment_id_unknown_errors() {
        let (_dir, db) = test_db().await;
        let err = resolve_environment(&db, Some(999_999), None, None).await.unwrap_err();
        assert!(err.contains("not found"));
    }

    #[tokio::test]
    async fn resolve_by_project_and_environment() {
        let (_dir, db) = test_db().await;
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let env_id = db.upsert_environment(0, project_id, "production", true).await.unwrap();

        let env = resolve_environment(&db, None, Some("demo"), Some("production")).await.unwrap();
        assert_eq!(env.id, env_id);
    }

    #[tokio::test]
    async fn resolve_project_name_is_case_insensitive() {
        let (_dir, db) = test_db().await;
        let project_id = db.upsert_project(0, "Demo", None, "generic").await.unwrap();
        let env_id = db.upsert_environment(0, project_id, "production", true).await.unwrap();

        let env = resolve_environment(&db, None, Some("DEMO"), Some("production")).await.unwrap();
        assert_eq!(env.id, env_id);
    }

    #[tokio::test]
    async fn resolve_environment_name_is_case_insensitive() {
        let (_dir, db) = test_db().await;
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let env_id = db.upsert_environment(0, project_id, "Production", true).await.unwrap();

        let env = resolve_environment(&db, None, Some("demo"), Some("PRODUCTION")).await.unwrap();
        assert_eq!(env.id, env_id);
    }

    #[tokio::test]
    async fn resolve_known_project_unknown_environment_errors() {
        let (_dir, db) = test_db().await;
        db.upsert_project(0, "demo", None, "generic").await.unwrap();

        let err = resolve_environment(&db, None, Some("demo"), Some("nope")).await.unwrap_err();
        assert!(err.contains("not found"));
    }

    #[tokio::test]
    async fn resolve_unknown_project_errors() {
        let (_dir, db) = test_db().await;
        let err = resolve_environment(&db, None, Some("ghost"), Some("production")).await.unwrap_err();
        assert!(err.contains("not found"));
    }

    #[tokio::test]
    async fn resolve_project_alone_without_environment_errors() {
        // Current behaviour: `resolve_environment` has only two accepted
        // shapes — `environment_id` alone, or `project` + `environment`
        // together. There is NO "project alone -> default environment"
        // fallback in the code today, even though earlier design notes
        // floated one. A project name with no environment name falls
        // through to the same "provide environment_id, or both..." error as
        // passing neither. Pinned here so adding that fallback later is a
        // deliberate, visible diff to this test rather than a silent change.
        let (_dir, db) = test_db().await;
        db.upsert_project(0, "demo", None, "generic").await.unwrap();

        let err = resolve_environment(&db, None, Some("demo"), None).await.unwrap_err();
        assert!(err.contains("provide environment_id"));
    }

    #[tokio::test]
    async fn resolve_with_no_scope_params_errors() {
        let (_dir, db) = test_db().await;
        let err = resolve_environment(&db, None, None, None).await.unwrap_err();
        assert!(err.contains("provide environment_id"));
    }

    #[tokio::test]
    async fn resolve_environment_id_takes_precedence_over_project_and_environment() {
        let (_dir, db) = test_db().await;
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let env_id = db.upsert_environment(0, project_id, "production", true).await.unwrap();

        // Mismatched project/environment names are ignored when environment_id is present.
        let env = resolve_environment(&db, Some(env_id), Some("does-not-exist"), Some("also-not-real"))
            .await
            .unwrap();
        assert_eq!(env.id, env_id);
    }

    // ─── project root / relative paths ──────────────────────────────────

    #[test]
    fn resolve_env_path_rules() {
        assert_eq!(resolve_env_path(None, "/abs/.env").unwrap(), "/abs/.env");
        assert_eq!(resolve_env_path(Some("/r"), ".env").unwrap(), "/r/.env");
        assert_eq!(resolve_env_path(Some("/r"), "./apps\\api/.env").unwrap(), "/r/apps/api/.env");
        assert!(resolve_env_path(None, ".env").is_err(), "relative needs a root");
        assert!(resolve_env_path(Some("/r"), "../x/.env").is_err(), "no escaping the root");
        assert!(resolve_env_path(Some("/r"), "./").is_err(), "must name a file");
    }

    #[test]
    fn validate_root_path_rules() {
        assert_eq!(validate_root_path("/a/b/").unwrap(), "/a/b");
        assert_eq!(validate_root_path("/").unwrap(), "/");
        assert!(validate_root_path("rel/dir").is_err());
    }

    #[tokio::test]
    async fn save_project_with_root_sets_root_and_initial_env_path() {
        let (_dir, db) = test_db().await;
        let mut input = new_project("rooted", None);
        input.root_path = Some("/work/rooted/".into());
        let id = save_project(&db, input).await.unwrap();
        let p = list_projects(&db).await.unwrap().into_iter().find(|p| p.id == id).unwrap();
        assert_eq!(p.root_path.as_deref(), Some("/work/rooted"));
        assert_eq!(p.environments[0].paths, vec![".env".to_string()]);

        // Omitted keeps, "" clears.
        let mut keep = new_project("rooted", None);
        keep.id = id;
        save_project(&db, keep).await.unwrap();
        assert_eq!(db.get_project_root(id).await.unwrap().as_deref(), Some("/work/rooted"));
        let mut clear = new_project("rooted", None);
        clear.id = id;
        clear.root_path = Some(String::new());
        save_project(&db, clear).await.unwrap();
        assert_eq!(db.get_project_root(id).await.unwrap(), None);

        let mut bad = new_project("bad-root", None);
        bad.root_path = Some("relative".into());
        assert!(save_project(&db, bad).await.is_err());
    }

    #[tokio::test]
    async fn inject_resolves_relative_paths_and_honors_targets() {
        let (dir, db) = test_db().await;
        let (_, _, key) = crate::crypto::init_vault_crypto(b"pw").unwrap();
        let env_id = seeded_env_with_item(&db, &key, "DB_HOST", "localhost").await;
        let project_id = db.get_environment(env_id).await.unwrap().unwrap().project_id;
        db.set_project_root(project_id, Some(dir.path().to_str().unwrap())).await.unwrap();
        std::fs::create_dir_all(dir.path().join("web")).unwrap();
        db.set_environment_paths(env_id, &["web/.env".to_string(), ".env".to_string()]).await.unwrap();

        let only = vec!["web/.env".to_string()];
        let result = inject_environment(&db, &key, env_id, None, None, false, Some(&only)).await.unwrap();
        assert_eq!(result.paths, vec![dir.path().join("web").join(".env").to_str().unwrap().to_string()]);
        assert!(dir.path().join("web/.env").exists());
        assert!(!dir.path().join(".env").exists(), "non-selected target untouched");

        let bogus = vec!["nope/.env".to_string()];
        assert!(inject_environment(&db, &key, env_id, None, None, false, Some(&bogus)).await.is_err());

        let all = inject_environment(&db, &key, env_id, None, None, false, None).await.unwrap();
        assert_eq!(all.paths.len(), 2);
        assert!(dir.path().join(".env").exists());
    }

    #[test]
    fn check_root_reports_problems() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        assert_eq!(check_root(root), RootCheck { problem: None, has_manifest: false });
        assert_eq!(check_root("relative").problem.as_deref(), Some("notAbsolute"));
        assert_eq!(check_root(&format!("{root}/missing")).problem.as_deref(), Some("notFound"));
        let file = dir.path().join("f");
        std::fs::write(&file, "").unwrap();
        assert_eq!(check_root(file.to_str().unwrap()).problem.as_deref(), Some("notDir"));
        std::fs::write(dir.path().join(manifest::FILE_NAME), "project:\n  name: x\n").unwrap();
        assert!(check_root(root).has_manifest);
    }

    #[tokio::test]
    async fn write_project_manifest_respects_overwrite() {
        let (dir, db) = test_db().await;
        let root = dir.path().join("ws");
        std::fs::create_dir_all(&root).unwrap();
        let mut input = new_project("gui-app", None);
        input.root_path = Some(root.to_str().unwrap().to_string());
        let id = save_project(&db, input).await.unwrap();

        let path = write_project_manifest(&db, id, false).await.unwrap();
        let m = manifest::read_file(Path::new(&path)).unwrap();
        assert_eq!(m.project.name, "gui-app");
        assert_eq!(m.project.environments[0].paths, vec![".env".to_string()]);

        assert_eq!(write_project_manifest(&db, id, false).await.unwrap_err(), "manifest exists");
        assert!(write_project_manifest(&db, id, true).await.is_ok());

        let no_root = save_project(&db, new_project("no-root", None)).await.unwrap();
        assert!(write_project_manifest(&db, no_root, false).await.is_err());
    }

    // ─── inject_environment ─────────────────────────────────────────────

    async fn seeded_env_with_item(db: &VaultDb, key: &[u8; 32], var_key: &str, value: &str) -> i64 {
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let env_id = db.upsert_environment(0, project_id, "production", true).await.unwrap();
        let item = plain_secret(var_key, value);
        let encrypted = crate::vault::encrypt_item(key, &item).unwrap();
        let item_id = db.upsert_item(0, "secret", &encrypted, &item.created, false).await.unwrap();
        db.add_item_owner(item_id, project_id).await.unwrap();
        db.upsert_environment_var(env_id, var_key, item_id).await.unwrap();
        env_id
    }

    #[tokio::test]
    async fn inject_writes_key_value_to_configured_path() {
        let (dir, db) = test_db().await;
        let (_, _, key) = crate::crypto::init_vault_crypto(b"pw").unwrap();
        let env_id = seeded_env_with_item(&db, &key, "DB_HOST", "localhost").await;
        let path = dir.path().join(".env");
        db.set_environment_paths(env_id, &[path.to_str().unwrap().to_string()]).await.unwrap();

        let result = inject_environment(&db, &key, env_id, None, None, false, None).await.unwrap();

        assert_eq!(result.written, vec!["DB_HOST".to_string()]);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("DB_HOST=localhost"));
        // The written file's parent must stay inside the tempdir — the
        // fixture issue #7's path-traversal assertions build on this.
        assert_eq!(path.parent().unwrap(), dir.path());
    }

    /// A pre-existing *configured* path with no crypt-env marker reads as
    /// `Foreign`. Issue #8 changed what happens next: such a file is no longer
    /// merged into (its content is not a trusted base), it is backed up to
    /// `.bak` and rewritten from the environment's own keys. Unrelated keys
    /// therefore survive in the backup, not in the live file. Configured
    /// paths are never hard-gated, so this succeeds with `overwrite: false`
    /// and self-heals: the marker written here makes the next inject a
    /// normal `Managed` merge.
    #[tokio::test]
    async fn inject_backs_up_unmarked_configured_file_then_self_heals() {
        let (dir, db) = test_db().await;
        let (_, _, key) = crate::crypto::init_vault_crypto(b"pw").unwrap();
        let env_id = seeded_env_with_item(&db, &key, "DB_HOST", "localhost").await;
        let path = dir.path().join(".env");
        std::fs::write(&path, "PORT=3000\nDB_HOST=old-value\n").unwrap();
        db.set_environment_paths(env_id, &[path.to_str().unwrap().to_string()]).await.unwrap();

        let result = inject_environment(&db, &key, env_id, None, None, false, None).await.unwrap();

        // Reported once as unmanaged, with the backup that preserves the
        // original contents.
        assert_eq!(result.unmanaged_paths, vec![path.to_str().unwrap().to_string()]);
        assert_eq!(result.backups.len(), 1, "a Foreign target must be backed up");

        let backup = std::fs::read_to_string(&result.backups[0]).unwrap();
        assert!(backup.contains("PORT=3000"), "unrelated key must survive in the backup");

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("DB_HOST=localhost"), "managed key must be updated");
        assert!(!content.contains("PORT=3000"), "unmarked content is not a trusted merge base");

        // Second inject: the file now carries the marker, so it is Managed —
        // no longer reported unmanaged, no second backup, and a real merge.
        let again = inject_environment(&db, &key, env_id, None, None, false, None).await.unwrap();
        assert!(again.unmanaged_paths.is_empty(), "marker must make it Managed");
        assert!(again.backups.is_empty(), "Managed targets are never backed up");
    }

    #[tokio::test]
    async fn inject_output_path_is_added_to_configured_paths() {
        let (dir, db) = test_db().await;
        let (_, _, key) = crate::crypto::init_vault_crypto(b"pw").unwrap();
        let env_id = seeded_env_with_item(&db, &key, "DB_HOST", "localhost").await;
        let configured = dir.path().join(".env");
        db.set_environment_paths(env_id, &[configured.to_str().unwrap().to_string()]).await.unwrap();
        let extra = dir.path().join(".env.extra");

        let result = inject_environment(&db, &key, env_id, Some(extra.to_str().unwrap().to_string()), None, false, None)
            .await
            .unwrap();

        assert_eq!(result.paths.len(), 2);
        assert!(extra.exists());
        assert!(configured.exists());
    }

    #[tokio::test]
    async fn inject_output_dir_used_only_when_no_paths_configured() {
        let (dir, db) = test_db().await;
        let (_, _, key) = crate::crypto::init_vault_crypto(b"pw").unwrap();
        let env_id = seeded_env_with_item(&db, &key, "DB_HOST", "localhost").await;
        // no configured paths, no output_path

        let result = inject_environment(&db, &key, env_id, None, Some(dir.path().to_str().unwrap().to_string()), false, None)
            .await
            .unwrap();

        let expected = dir.path().join(".env.production");
        assert_eq!(result.paths, vec![expected.to_str().unwrap().to_string()]);
        assert!(expected.exists());
    }

    #[tokio::test]
    async fn inject_unknown_environment_errors_not_found() {
        let (_dir, db) = test_db().await;
        let (_, _, key) = crate::crypto::init_vault_crypto(b"pw").unwrap();
        let err = inject_environment(&db, &key, 999_999, None, None, false, None).await.unwrap_err();
        assert_eq!(err, "environment not found");
    }

    #[tokio::test]
    async fn inject_no_paths_and_no_output_errors() {
        let (_dir, db) = test_db().await;
        let (_, _, key) = crate::crypto::init_vault_crypto(b"pw").unwrap();
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let env_id = db.upsert_environment(0, project_id, "production", true).await.unwrap();

        let err = inject_environment(&db, &key, env_id, None, None, false, None).await.unwrap_err();
        assert_eq!(err, "environment has no paths configured");
    }

    // ─── save_environment multi-owner guard ────────────────────────────

    #[tokio::test]
    async fn save_environment_links_item_already_owned_by_project() {
        let (_dir, db) = test_db().await;
        let (_, _, key) = crate::crypto::init_vault_crypto(b"pw").unwrap();
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let item = plain_secret("DB_HOST", "localhost");
        let encrypted = crate::vault::encrypt_item(&key, &item).unwrap();
        let item_id = db.upsert_item(0, "secret", &encrypted, &item.created, false).await.unwrap();
        db.add_item_owner(item_id, project_id).await.unwrap();

        let input = EnvironmentInput {
            id: 0,
            project_id,
            name: "production".to_string(),
            is_default: true,
            paths: vec![],
            vars: vec![EnvironmentVar { id: 0, key: "DB_HOST".to_string(), item_id }],
        };

        let env_id = save_environment(&db, input).await.unwrap();
        let vars = db.get_environment_vars(env_id).await.unwrap();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].item_id, Some(item_id));
    }

    #[tokio::test]
    async fn save_environment_grants_ownership_for_global_item() {
        let (_dir, db) = test_db().await;
        let (_, _, key) = crate::crypto::init_vault_crypto(b"pw").unwrap();
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let item = plain_secret("SHARED", "shared-value");
        let encrypted = crate::vault::encrypt_item(&key, &item).unwrap();
        let item_id = db.upsert_item(0, "secret", &encrypted, &item.created, true).await.unwrap();
        // Not yet owned by project_id, but is_global = true.

        let input = EnvironmentInput {
            id: 0,
            project_id,
            name: "production".to_string(),
            is_default: true,
            paths: vec![],
            vars: vec![EnvironmentVar { id: 0, key: "SHARED".to_string(), item_id }],
        };

        save_environment(&db, input).await.unwrap();
        assert!(db.list_owning_projects(item_id).await.unwrap().contains(&project_id));
    }

    #[tokio::test]
    async fn save_environment_rejects_unowned_non_global_item() {
        let (_dir, db) = test_db().await;
        let (_, _, key) = crate::crypto::init_vault_crypto(b"pw").unwrap();
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let other_project = db.upsert_project(0, "other", None, "generic").await.unwrap();
        let item = plain_secret("PRIVATE", "value");
        let encrypted = crate::vault::encrypt_item(&key, &item).unwrap();
        let item_id = db.upsert_item(0, "secret", &encrypted, &item.created, false).await.unwrap();
        db.add_item_owner(item_id, other_project).await.unwrap();

        let input = EnvironmentInput {
            id: 0,
            project_id,
            name: "production".to_string(),
            is_default: true,
            paths: vec![],
            vars: vec![EnvironmentVar { id: 0, key: "PRIVATE".to_string(), item_id }],
        };

        let err = save_environment(&db, input).await.unwrap_err();
        assert!(err.contains("not global"));
    }

    #[tokio::test]
    async fn save_environment_replaces_previous_var_set() {
        let (_dir, db) = test_db().await;
        let (_, _, key) = crate::crypto::init_vault_crypto(b"pw").unwrap();
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let item_a = plain_secret("A", "va");
        let item_b = plain_secret("B", "vb");
        let enc_a = crate::vault::encrypt_item(&key, &item_a).unwrap();
        let enc_b = crate::vault::encrypt_item(&key, &item_b).unwrap();
        let id_a = db.upsert_item(0, "secret", &enc_a, &item_a.created, false).await.unwrap();
        let id_b = db.upsert_item(0, "secret", &enc_b, &item_b.created, false).await.unwrap();
        db.add_item_owner(id_a, project_id).await.unwrap();
        db.add_item_owner(id_b, project_id).await.unwrap();

        let env_id = save_environment(&db, EnvironmentInput {
            id: 0, project_id, name: "production".to_string(), is_default: true,
            paths: vec![], vars: vec![EnvironmentVar { id: 0, key: "A".to_string(), item_id: id_a }],
        }).await.unwrap();

        save_environment(&db, EnvironmentInput {
            id: env_id, project_id, name: "production".to_string(), is_default: true,
            paths: vec![], vars: vec![EnvironmentVar { id: 0, key: "B".to_string(), item_id: id_b }],
        }).await.unwrap();

        let vars = db.get_environment_vars(env_id).await.unwrap();
        assert_eq!(vars.len(), 1, "second save must replace, not append to, the var set");
        assert_eq!(vars[0].key, "B");
    }

    #[tokio::test]
    async fn save_environment_stores_lowercase_name() {
        let (_dir, db) = test_db().await;
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let env_id = save_environment(&db, EnvironmentInput {
            id: 0, project_id, name: "Production".to_string(), is_default: true,
            paths: vec![], vars: vec![],
        }).await.unwrap();
        let env = db.get_environment(env_id).await.unwrap().unwrap();
        assert_eq!(env.name, "production");
    }

    // ─── unnamed root environment (`.env`) ─────────────────────────────

    #[test]
    fn empty_environment_name_is_valid_root() {
        assert!(validate_environment_name("").is_ok());
        assert!(validate_environment_name("   ").is_err());
        assert!(is_root_environment(""));
        assert!(is_root_environment("default"));
        assert!(!is_root_environment("production"));
    }

    #[test]
    fn root_environment_filename_has_no_extension() {
        assert_eq!(environment_filename(""), ".env");
        assert_eq!(environment_filename("default"), ".env");
        assert_eq!(environment_filename("Production"), ".env.production");
        assert_eq!(environment_example_filename(""), ".env.example");
        assert_eq!(environment_example_filename("staging"), ".env.example.staging");
    }

    #[tokio::test]
    async fn inject_output_dir_resolves_unnamed_environment_to_dot_env() {
        let (dir, db) = test_db().await;
        let (_, _, key) = crate::crypto::init_vault_crypto(b"pw").unwrap();
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let env_id = save_environment(&db, EnvironmentInput {
            id: 0, project_id, name: String::new(), is_default: true,
            paths: vec![], vars: vec![],
        }).await.unwrap();
        let item = plain_secret("DB_HOST", "localhost");
        let encrypted = crate::vault::encrypt_item(&key, &item).unwrap();
        let item_id = db.upsert_item(0, "secret", &encrypted, &item.created, false).await.unwrap();
        db.add_item_owner(item_id, project_id).await.unwrap();
        db.upsert_environment_var(env_id, "DB_HOST", item_id).await.unwrap();

        let result = inject_environment(&db, &key, env_id, None, Some(dir.path().to_str().unwrap().to_string()), false, None)
            .await
            .unwrap();

        let expected = dir.path().join(".env");
        assert_eq!(result.paths, vec![expected.to_str().unwrap().to_string()]);
        assert!(expected.exists());
    }

    #[tokio::test]
    async fn unnamed_and_legacy_default_environments_collide() {
        let (_dir, db) = test_db().await;
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        db.upsert_environment(0, project_id, "default", true).await.unwrap();
        let err = save_environment(&db, EnvironmentInput {
            id: 0, project_id, name: String::new(), is_default: false,
            paths: vec![], vars: vec![],
        }).await.unwrap_err();
        assert_eq!(err, crate::db::ENVIRONMENT_NAME_CONFLICT);
    }

    #[tokio::test]
    async fn resolve_root_environment_by_empty_or_default_name() {
        let (_dir, db) = test_db().await;
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let env_id = db.upsert_environment(0, project_id, "", true).await.unwrap();
        for name in ["", "default", "DEFAULT"] {
            let env = resolve_environment(&db, None, Some("demo"), Some(name)).await.unwrap();
            assert_eq!(env.id, env_id);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn inject_rejects_relative_path_through_symlinked_directory() {
        let (dir, db) = test_db().await;
        let (_, _, key) = crate::crypto::init_vault_crypto(b"pw").unwrap();
        let env_id = seeded_env_with_item(&db, &key, "DB_HOST", "localhost").await;
        let project_id = db.get_environment(env_id).await.unwrap().unwrap().project_id;
        let root = dir.path().join("repo");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("config")).unwrap();
        db.set_project_root(project_id, root.to_str()).await.unwrap();
        db.set_environment_paths(env_id, &["config/.env".to_string()]).await.unwrap();

        let err = inject_environment(&db, &key, env_id, None, None, false, None).await.unwrap_err();
        assert!(err.contains("must stay inside the project root"), "got: {err}");
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0, "nothing written outside the root");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn inject_refuses_symlink_target_and_writes_nothing() {
        let (dir, db) = test_db().await;
        let (_, _, key) = crate::crypto::init_vault_crypto(b"pw").unwrap();
        let env_id = seeded_env_with_item(&db, &key, "DB_HOST", "localhost").await;
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, "keep").unwrap();
        let link = dir.path().join(".env");
        std::os::unix::fs::symlink(&victim, &link).unwrap();
        let plain = dir.path().join("plain.env");
        db.set_environment_paths(
            env_id,
            &[plain.to_str().unwrap().to_string(), link.to_str().unwrap().to_string()],
        )
        .await
        .unwrap();

        let preview = inject_environment_preview(&db, env_id, None).await.unwrap();
        assert_eq!(preview.symlinks, vec![link.to_str().unwrap().to_string()]);

        for overwrite in [false, true] {
            let err = inject_environment(&db, &key, env_id, None, None, overwrite, None).await.unwrap_err();
            assert!(err.starts_with(envfile::SYMLINK_PREFIX), "got: {err}");
        }
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        assert!(!plain.exists(), "no path is written when any target is a symlink");
    }

    // ─── vault-db-transactional-integrity ─────────────────────────────

    #[tokio::test]
    async fn save_environment_rejects_duplicate_keys_without_writing() {
        let (_dir, db) = test_db().await;
        let (_, _, key) = crate::crypto::init_vault_crypto(b"pw").unwrap();
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let item = plain_secret("A", "va");
        let enc = crate::vault::encrypt_item(&key, &item).unwrap();
        let item_id = db.upsert_item(0, "secret", &enc, &item.created, false).await.unwrap();
        db.add_item_owner(item_id, project_id).await.unwrap();
        let env_id = save_environment(&db, EnvironmentInput {
            id: 0, project_id, name: "production".to_string(), is_default: true,
            paths: vec![], vars: vec![EnvironmentVar { id: 0, key: "A".to_string(), item_id }],
        }).await.unwrap();

        let err = save_environment(&db, EnvironmentInput {
            id: env_id, project_id, name: "renamed".to_string(), is_default: true,
            paths: vec![],
            vars: vec![
                EnvironmentVar { id: 0, key: "DUP".to_string(), item_id },
                EnvironmentVar { id: 0, key: "DUP".to_string(), item_id },
            ],
        }).await.unwrap_err();
        assert!(err.contains("DUP"));

        let env = db.get_environment(env_id).await.unwrap().unwrap();
        assert_eq!(env.name, "production");
        let vars = db.get_environment_vars(env_id).await.unwrap();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].key, "A");
    }

    #[tokio::test]
    async fn save_environment_is_atomic_when_var_insert_fails() {
        let (_dir, db) = test_db().await;
        let (_, _, key) = crate::crypto::init_vault_crypto(b"pw").unwrap();
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let item = plain_secret("SHARED", "v");
        let enc = crate::vault::encrypt_item(&key, &item).unwrap();
        let global_id = db.upsert_item(0, "secret", &enc, &item.created, true).await.unwrap();
        let env_id = save_environment(&db, EnvironmentInput {
            id: 0, project_id, name: "production".to_string(), is_default: true,
            paths: vec!["/a".to_string()], vars: vec![],
        }).await.unwrap();

        sqlx::query(
            "CREATE TRIGGER boom BEFORE INSERT ON environment_vars
             BEGIN SELECT RAISE(ABORT, 'injected'); END",
        )
        .execute(&db.pool)
        .await
        .unwrap();

        let res = save_environment(&db, EnvironmentInput {
            id: env_id, project_id, name: "staging".to_string(), is_default: true,
            paths: vec!["/b".to_string()],
            vars: vec![EnvironmentVar { id: 0, key: "SHARED".to_string(), item_id: global_id }],
        }).await;
        assert!(res.is_err());

        let env = db.get_environment(env_id).await.unwrap().unwrap();
        assert_eq!(env.name, "production", "rename must be rolled back");
        assert_eq!(db.get_environment_paths(env_id).await.unwrap(), vec!["/a".to_string()]);
        assert!(db.list_owning_projects(global_id).await.unwrap().is_empty(), "ownership grant must be rolled back");
    }

    // ── inject: serialization and targeted decrypt ──

    /// Inserts an item whose stored ciphertext cannot be decrypted.
    async fn seed_corrupt_item(v: &crate::test_support::TestVault) -> i64 {
        let state = v.state.lock().await;
        state
            .db
            .upsert_item(0, "secret", "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef", "2024-01-01", false)
            .await
            .unwrap()
    }

    async fn inject_to(v: &crate::test_support::TestVault, env_id: i64, path: &Path) -> Result<InjectResult, String> {
        let state = v.state.lock().await;
        let key = state.key.clone().unwrap();
        state.db.set_environment_paths(env_id, &[path.to_string_lossy().into_owned()]).await.unwrap();
        inject_environment(&state.db, &key, env_id, None, None, false, None).await
    }

    #[tokio::test]
    async fn inject_writes_serialized_values_and_blocks_value_injection() {
        use crate::test_support::*;
        let v = unlocked_vault_empty().await;
        let (_p, envs) = seed_project(&v, "p", &["dev"]).await;
        let pem = "-----BEGIN KEY-----\nabc$def\n-----END KEY-----\n";
        for (k, val) in [("PLAIN", "abc"), ("SPACED", "a b"), ("PEM", pem), ("EVIL", "x\nADMIN=1")] {
            let id = seed_item(&v, k, val, false).await;
            link_var(&v, envs[0], k, id).await;
        }
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(".env");
        let result = inject_to(&v, envs[0], &target).await.unwrap();
        assert!(result.failed_keys.is_empty());

        let content = std::fs::read_to_string(&target).unwrap();
        let parsed: HashMap<String, String> = envfile::parse_dotenv(&content).into_iter().collect();
        assert_eq!(parsed.len(), 4, "no extra variable may appear: {parsed:?}");
        assert_eq!(parsed["PLAIN"], "abc");
        assert_eq!(parsed["SPACED"], "a b");
        assert_eq!(parsed["PEM"], pem);
        assert_eq!(parsed["EVIL"], "x\nADMIN=1");
        assert!(!parsed.contains_key("ADMIN"));
    }

    #[tokio::test]
    async fn inject_updates_existing_managed_line_with_serializer() {
        use crate::test_support::*;
        let v = unlocked_vault_empty().await;
        let (_p, envs) = seed_project(&v, "p", &["dev"]).await;
        let id = seed_item(&v, "K", "new value", false).await;
        link_var(&v, envs[0], "K", id).await;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(".env");
        std::fs::write(&target, "# crypt-env: managed file (project: p, environment: dev)\nK=old\nOTHER=keep\n").unwrap();
        inject_to(&v, envs[0], &target).await.unwrap();
        let content = std::fs::read_to_string(&target).unwrap();
        assert!(content.contains("K='new value'\n"), "{content}");
        assert!(content.contains("OTHER=keep\n"));
    }

    #[tokio::test]
    async fn unrelated_corrupt_item_does_not_fail_inject() {
        use crate::test_support::*;
        let v = unlocked_vault_empty().await;
        let (_p, envs) = seed_project(&v, "p", &["dev"]).await;
        let good = seed_item(&v, "GOOD", "v1", false).await;
        link_var(&v, envs[0], "GOOD", good).await;
        seed_corrupt_item(&v).await;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(".env");
        let result = inject_to(&v, envs[0], &target).await.unwrap();
        assert!(result.failed_keys.is_empty());
        assert_eq!(result.written, vec!["GOOD".to_string()]);
    }

    #[tokio::test]
    async fn referenced_corrupt_item_is_listed_and_other_keys_written() {
        use crate::test_support::*;
        let v = unlocked_vault_empty().await;
        let (_p, envs) = seed_project(&v, "p", &["dev"]).await;
        let good = seed_item(&v, "GOOD", "v1", false).await;
        link_var(&v, envs[0], "GOOD", good).await;
        let bad = seed_corrupt_item(&v).await;
        link_var(&v, envs[0], "BAD", bad).await;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(".env");
        let result = inject_to(&v, envs[0], &target).await.unwrap();
        assert_eq!(result.failed_keys, vec!["BAD".to_string()]);
        assert_eq!(result.written, vec!["GOOD".to_string()]);
        let content = std::fs::read_to_string(&target).unwrap();
        assert!(content.contains("GOOD=v1"));
        assert!(!content.contains("BAD"));
    }

    #[tokio::test]
    async fn invalid_key_is_reported_not_written() {
        use crate::test_support::*;
        let v = unlocked_vault_empty().await;
        let (_p, envs) = seed_project(&v, "p", &["dev"]).await;
        let id = seed_item(&v, "X", "v", false).await;
        link_var(&v, envs[0], "BAD KEY\nINJ=1", id).await;
        link_var(&v, envs[0], "OK", id).await;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(".env");
        let result = inject_to(&v, envs[0], &target).await.unwrap();
        assert_eq!(result.failed_keys, vec!["BAD KEY\nINJ=1".to_string()]);
        assert_eq!(result.written, vec!["OK".to_string()]);
        assert!(!std::fs::read_to_string(&target).unwrap().contains("INJ"));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn inject_into_fifo_target_fails_with_not_regular_prefix() {
        use crate::test_support::*;
        let v = unlocked_vault_empty().await;
        let (_p, envs) = seed_project(&v, "p", &["dev"]).await;
        let id = seed_item(&v, "K", "v", false).await;
        link_var(&v, envs[0], "K", id).await;
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("pipe");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let err = inject_to(&v, envs[0], &fifo).await.unwrap_err();
        assert!(err.starts_with(envfile::NOT_REGULAR_PREFIX), "{err}");
    }

    #[tokio::test]
    async fn export_dialog_runs_without_the_vault_lock() {
        use crate::test_support::*;
        let v = unlocked_vault().await;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.cryptenv-proj");
        let state = v.state.clone();
        let target_for_dialog = target.clone();
        export_project_with(v.project_id, &v.state, move |_name| async move {
            // The "dialog" is open here: the vault lock must be free.
            let free = state.try_lock().is_ok();
            assert!(free, "vault lock held while the save dialog is open");
            Ok(Some(target_for_dialog))
        })
        .await
        .unwrap();
        assert!(target.exists());
    }
}
