use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use std::time::Duration;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DbCategory {
    pub cid: String,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DbWorkspace {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub paths: Vec<String>,
    pub template: String,
    pub created: String,
    pub updated: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DbWorkspaceVar {
    pub id: i64,
    pub workspace_id: i64,
    pub key: String,
    pub item_id: Option<i64>,
    pub literal: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDeleteImpact {
    pub environments: i64,
    pub items_deleted: i64,
    pub items_orphaned: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DbProject {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub template: String,
    pub created: String,
    pub updated: String,
    /// Project root directory as the vault host sees it (the directory
    /// holding `.crypt-env.yaml`). Relative `environments.paths` resolve
    /// against it. `None` for projects created before it existed.
    #[serde(default)]
    pub root_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DbEnvironment {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub is_default: bool,
    pub paths: Vec<String>,
    pub created: String,
    pub updated: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DbEnvironmentVar {
    pub id: i64,
    pub environment_id: i64,
    pub key: String,
    pub item_id: Option<i64>,
    pub literal: Option<String>,
}

/// One entry in the `env_name_dedup_v1` settings report: a single
/// case-insensitive-collision rename performed by
/// `VaultDb::dedupe_project_names_nocase` / `dedupe_environment_names_nocase`
/// during `init_schema`. Contains names and ids only — never variable keys or
/// values (environment/project names are already exposed via `GET
/// /projects`; secret material is not and must never appear here).
#[derive(Debug, Serialize, Deserialize, Clone)]
struct RenameRecord {
    table: String,
    id: i64,
    #[serde(skip_serializing_if = "Option::is_none", rename = "projectId")]
    project_id: Option<i64>,
    from: String,
    to: String,
    at: String,
}

// ─── Issue #9: create-or-update on env-key collision ──────────────────────
//
// `db` knows only ids and counts here — no HTTP status codes (that mapping
// lives in `api`) and no ciphertext (crypto lives in `vault`). See
// `create_or_link_item` for the transactional write this classification
// feeds.

/// Read-only snapshot of the item currently linked to a `(environment_id,
/// key)` pair, returned by `inspect_env_key`. Never carries `items.data`.
#[derive(Debug, Clone)]
pub struct EnvKeyConflict {
    pub item_id: i64,
    pub created: String,
    pub is_global: bool,
    /// Rows in `environment_vars` pointing at `item_id` (including this one).
    pub link_count: i64,
    pub owner_ids: Vec<i64>,
}

/// How `create_or_link_item` should behave on a collision. Mirrors the
/// `on_conflict` query parameter accepted by `POST /items`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkMode {
    /// Re-encrypt onto the existing row when it is exclusively owned by the
    /// calling project and linked nowhere else; otherwise conflict.
    Update,
    /// Always create a new row and repoint the link; delete the superseded
    /// item only if it is now unreachable (unlinked and non-global).
    Replace,
    /// Any collision at all is rejected — strict create semantics.
    Error,
}

/// Outcome of `create_or_link_item`.
#[derive(Debug, Clone)]
pub enum LinkOutcome {
    Created { item_id: i64, is_global: bool },
    Updated { item_id: i64, is_global: bool },
    Conflict { item_id: i64, reason: ConflictReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictReason {
    /// The colliding item is linked elsewhere, multi-owned, or global —
    /// mutating it in place would change a value seen outside the caller's
    /// project+environment.
    Shared,
    /// `on_conflict=error` (or an `Error`-mode caller) rejects any collision.
    KeyExists,
    /// The transactional re-check found a different `item_id` linked than
    /// the one the caller's earlier `inspect_env_key` call saw.
    StateChanged,
}

// ─── Project-relay receive (issue #4) ─────────────────────────────────────────
// Plain-data input to `insert_received_project` — the `db` layer never sees a
// vault key or a `PlainItem`, only ciphertext strings the caller (the
// `project`/`share` layers) already produced. See CLAUDE.md's module rule:
// `db` does not know about encryption.

/// One item to insert, already encrypted by the caller. `name` is used only
/// to resolve `ReceivedVar::item_name` references within this same call —
/// it is never stored (the vault item's name lives inside its ciphertext).
pub struct ReceivedProjectItem {
    pub name: String,
    pub item_type: String,
    pub ciphertext: String,
    pub created: String,
}

pub struct ReceivedVar {
    pub key: String,
    pub item_name: String,
}

pub struct ReceivedEnvironment {
    pub name: String,
    pub is_default: bool,
    pub vars: Vec<ReceivedVar>,
}

#[derive(Debug, Serialize, Clone)]
pub struct InsertedProject {
    pub project_id: i64,
    pub environment_ids: Vec<i64>,
    pub item_ids: Vec<i64>,
}

pub struct VaultDb {
    pub(crate) pool: SqlitePool,
    path: String,
}

/// Stable sentinel returned (instead of a raw sqlx error string) when a
/// `projects` INSERT/UPDATE trips `idx_projects_name_nocase`. `db` owns the
/// schema and therefore the constraint's meaning; `project` propagates this
/// string unchanged; `api` matches the `"conflict:"` prefix to map it to
/// `409 CONFLICT`. Never substring-match sqlx's own error text — it's not a
/// stable API and would leak SQL identifiers to the HTTP client on any
/// mismatch.
///
/// `pub` (not `pub(crate)`): every `[[bin]]` target in this package (the
/// `crypt-env` CLI, the `crypt-env-mcp` server) is its own crate, separate
/// from this `crypt_env_lib` library crate, even though they share one
/// Cargo.toml — `pub(crate)` items here are invisible to code in `src/bin/`.
/// Any consumer of this conflict-detection contract (e.g. issue #4) needs
/// `pub` to reach it with `crypt_env_lib::db::PROJECT_NAME_CONFLICT`.
pub const PROJECT_NAME_CONFLICT: &str = "conflict: a project with this name already exists";

/// Same sentinel contract as `PROJECT_NAME_CONFLICT`, for
/// `idx_environments_name_nocase`. `pub` for the same cross-crate reason, and
/// also because `project::ensure_no_case_collision`'s app-level
/// (Unicode-aware) pre-check returns this exact same string on its own —
/// reusing the constant instead of a second string literal keeps the two
/// layers from drifting apart.
pub const ENVIRONMENT_NAME_CONFLICT: &str =
    "conflict: an environment with this name already exists in this project";

/// Detects a unique-constraint violation via `sqlx::Error::Database(_).
/// is_unique_violation()` (sqlx 0.8) rather than substring-matching the
/// driver's error text, and maps it to `sentinel`. Any other error keeps
/// `e.to_string()` — the API layer is responsible for not echoing that raw
/// text back to an HTTP client (see `api::handle_save_project` /
/// `handle_save_environment`).
fn map_conflict(e: sqlx::Error, sentinel: &str) -> String {
    if let sqlx::Error::Database(ref dbe) = e {
        if dbe.is_unique_violation() {
            return sentinel.to_string();
        }
    }
    e.to_string()
}

/// Connection options applied to EVERY pooled connection (including ones
/// opened after the pool recycles idle connections). `foreign_keys` and
/// `secure_delete` are per-connection pragmas: running them once as a query
/// only configures whichever connection happened to execute it.
///
/// `secure_delete`: SQLite zeroes freed page content on every
/// DELETE/UPDATE-that-frees instead of merely unlinking it. Covers
/// `delete_item`, `delete_project`'s cascades, the `Replace` conflict branch,
/// and the orphan prune path uniformly (see `wipe_and_reset` for the
/// file-level version of the same idea). No on-disk format impact.
pub(crate) fn connect_options(path: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .pragma("secure_delete", "ON")
        .busy_timeout(Duration::from_secs(5))
}

impl VaultDb {
    /// Starts a transaction on the pool. Callers that need several writes to
    /// succeed or fail together (e.g. `project::save_environment`) use this
    /// together with the `*_tx` helpers.
    pub async fn begin(&self) -> Result<Transaction<'static, Sqlite>, String> {
        self.pool.begin().await.map_err(|e| e.to_string())
    }

    pub async fn open(path: &str) -> Result<Self, String> {
        let pool = connect_pool(Path::new(path))
            .await
            .map_err(|e| format!("db open: {e}"))?;

        let db = VaultDb { pool, path: path.to_string() };
        db.init_schema().await?;
        Ok(db)
    }

    async fn init_schema(&self) -> Result<(), String> {
        let stmts = [
            // Pragmas (WAL, foreign_keys, secure_delete) live in `connect_options`
            // so they apply to every pooled connection, not just this one.
            "CREATE TABLE IF NOT EXISTS vault_meta (
                id            INTEGER PRIMARY KEY CHECK(id = 1),
                kdf_salt      TEXT NOT NULL,
                verify_token  TEXT NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS items (
                id        INTEGER PRIMARY KEY AUTOINCREMENT,
                item_type TEXT NOT NULL,
                data      TEXT NOT NULL,
                created   TEXT NOT NULL,
                updated   TEXT NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS categories (
                cid   TEXT PRIMARY KEY,
                name  TEXT NOT NULL,
                color TEXT NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS share_log (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                mode       TEXT NOT NULL,
                direction  TEXT NOT NULL,
                item_ids   TEXT NOT NULL,
                peer_fp    TEXT,
                timestamp  TEXT NOT NULL
            )",
        ];
        for stmt in &stmts {
            sqlx::query(stmt)
                .execute(&self.pool)
                .await
                .map_err(|e| format!("schema init: {e}"))?;
        }
        // Additive migrations
        let _ = sqlx::query("ALTER TABLE categories ADD COLUMN description TEXT")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("ALTER TABLE items ADD COLUMN is_global INTEGER NOT NULL DEFAULT 0")
            .execute(&self.pool)
            .await;
        let migrations = [
            "CREATE TABLE IF NOT EXISTS workspaces (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT NOT NULL,
                description TEXT,
                path        TEXT,
                template    TEXT NOT NULL DEFAULT 'generic',
                created     TEXT NOT NULL,
                updated     TEXT NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS workspace_vars (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                key          TEXT NOT NULL,
                item_id      INTEGER,
                literal      TEXT,
                UNIQUE(workspace_id, key)
            )",
            "CREATE TABLE IF NOT EXISTS workspace_paths (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                path         TEXT NOT NULL,
                UNIQUE(workspace_id, path)
            )",
            "INSERT OR IGNORE INTO workspace_paths (workspace_id, path)
                SELECT id, path FROM workspaces WHERE path IS NOT NULL",
            // Projects/environments: additive, coexists with the legacy workspace
            // tables above (still used by the whole-workspace relay share feature).
            "CREATE TABLE IF NOT EXISTS projects (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT NOT NULL,
                description TEXT,
                template    TEXT NOT NULL DEFAULT 'generic',
                created     TEXT NOT NULL,
                updated     TEXT NOT NULL
            )",
            "CREATE TABLE IF NOT EXISTS environments (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id  INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                name        TEXT NOT NULL,
                is_default  INTEGER NOT NULL DEFAULT 0,
                created     TEXT NOT NULL,
                updated     TEXT NOT NULL,
                UNIQUE(project_id, name)
            )",
            "CREATE TABLE IF NOT EXISTS environment_vars (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                environment_id INTEGER NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
                key            TEXT NOT NULL,
                item_id        INTEGER,
                literal        TEXT,
                UNIQUE(environment_id, key)
            )",
            "CREATE TABLE IF NOT EXISTS environment_paths (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                environment_id INTEGER NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
                path           TEXT NOT NULL,
                UNIQUE(environment_id, path)
            )",
            // Backfill: one project per existing workspace (idempotent — PK conflicts are skipped).
            "INSERT OR IGNORE INTO projects (id, name, description, template, created, updated)
                SELECT id, name, description, template, created, updated FROM workspaces",
            // Backfill: every project without an environment yet gets a 'default' one.
            "INSERT INTO environments (project_id, name, is_default, created, updated)
                SELECT p.id, 'default', 1, p.created, p.updated
                FROM projects p
                WHERE NOT EXISTS (SELECT 1 FROM environments e WHERE e.project_id = p.id)",
            // Backfill: workspace_vars/workspace_paths into each project's default environment.
            "INSERT OR IGNORE INTO environment_vars (environment_id, key, item_id, literal)
                SELECT e.id, wv.key, wv.item_id, wv.literal
                FROM workspace_vars wv
                JOIN environments e ON e.project_id = wv.workspace_id AND e.name = 'default'",
            "INSERT OR IGNORE INTO environment_paths (environment_id, path)
                SELECT e.id, wp.path
                FROM workspace_paths wp
                JOIN environments e ON e.project_id = wp.workspace_id AND e.name = 'default'",
            // Item ownership: many-to-many so a global item can belong to more
            // than one project at once (see project::mod for the fork-on-unglobal
            // and cascade-delete-if-orphaned logic that relies on this table).
            "CREATE TABLE IF NOT EXISTS item_projects (
                item_id    INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
                project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                PRIMARY KEY (item_id, project_id)
            )",
            "CREATE TABLE IF NOT EXISTS project_categories (
                project_id  INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                category_id TEXT NOT NULL REFERENCES categories(cid) ON DELETE CASCADE,
                PRIMARY KEY (project_id, category_id)
            )",
            // Backfill: items already referenced by an environment_var are owned
            // by that environment's project. `environment_vars.item_id` was never
            // FK-enforced, so a stale reference to an already-deleted item is
            // possible — the join against `items` skips those instead of failing
            // the new (enforced) item_projects FK.
            "INSERT OR IGNORE INTO item_projects (item_id, project_id)
                SELECT ev.item_id, e.project_id
                FROM environment_vars ev
                JOIN environments e ON e.id = ev.environment_id
                JOIN items i ON i.id = ev.item_id",
        ];
        for stmt in &migrations {
            sqlx::query(stmt).execute(&self.pool).await.map_err(|e| format!("migration: {e}"))?;
        }

        // Case-insensitive uniqueness on project and environment names.
        //
        // These two indexes have an imperative precondition — an install that
        // predates them may already hold a case-colliding pair (e.g. `MyApp` +
        // `myapp`), and creating the index directly would fail with a UNIQUE
        // constraint violation, bricking `VaultDb::open` for a condition the
        // user never caused and cannot fix without a SQL client. So each
        // dedup runs first (deterministic: lowest `id` keeps its name, losers
        // get `-2`, `-3`, ... suffixes) and only then is the index created —
        // that ordering, and the fact it's imperative code rather than a flat
        // SQL string, is why this block lives outside the `migrations` array
        // above instead of inside it.
        //
        // Prevents two concurrent CLI auto-creates (see scope::resolve) from
        // ever landing two rows for the same folder-derived name, and two
        // `POST /environments` racing on the same case-insensitive name within
        // one project — the second now fails with a UNIQUE constraint
        // violation (mapped to the `"conflict:"` sentinel by
        // `upsert_project`/`upsert_environment`) instead of silently creating
        // a duplicate.
        //
        // Never `let _ = ...`: if the index can't be created for some other
        // reason, `init_schema` must fail loudly rather than leave the vault
        // unprotected with no signal.
        //
        // Each dedup pass persists its own `RenameRecord`s to
        // `env_name_dedup_v1` *as it renames each row* (inside
        // `dedupe_project_names_nocase`/`dedupe_environment_names_nocase`
        // themselves), not batched up and written here after both indexes
        // succeed — the report is the sole reversal path for an otherwise
        // irreversible rename, so a rename must never be able to happen
        // without also being recorded, even if a later step in this function
        // (e.g. the second `CREATE UNIQUE INDEX`) fails or the process is
        // killed mid-migration.
        // Additive: project root directory (cli-tui-parity, design D8).
        let _ = sqlx::query("ALTER TABLE projects ADD COLUMN root_path TEXT")
            .execute(&self.pool)
            .await;
        // Additive: marks the auto-created `Restored <date>` / `Imported <date>`
        // projects that own restored or imported items, so those items are not
        // reported as orphans (backup-restore-completeness, design D4).
        let _ = sqlx::query("ALTER TABLE projects ADD COLUMN is_holding INTEGER NOT NULL DEFAULT 0")
            .execute(&self.pool)
            .await;

        self.dedupe_project_names_nocase().await?;
        sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS idx_projects_name_nocase ON projects(name COLLATE NOCASE)")
            .execute(&self.pool)
            .await
            .map_err(|e| format!("migration: could not enforce unique project names (idx_projects_name_nocase): {e}"))?;

        // Ordering: must run after the "INSERT INTO environments ... 'default'"
        // backfill above (in `migrations`), because that backfill can itself
        // introduce a `default` row into a project that already has a
        // differently-cased `Default` — this block runs after the whole
        // `migrations` loop, so that case is caught too.
        self.dedupe_environment_names_nocase().await?;
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_environments_name_nocase ON environments(project_id, name COLLATE NOCASE)",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| format!("migration: could not enforce unique environment names (idx_environments_name_nocase): {e}"))?;

        // One-time (not re-run every launch): pre-existing items that end up with
        // zero owners after the backfill above predate the whole project/ownership
        // model — promote them to global so they surface in Global Secrets instead
        // of silently disappearing from the UI. Gated so it never re-flips items
        // that legitimately become owner-less later via normal project deletion.
        if self.get_setting("backfilled_global_orphans_v1").await?.is_none() {
            sqlx::query(
                "UPDATE items SET is_global = 1
                 WHERE id NOT IN (SELECT item_id FROM item_projects)",
            )
            .execute(&self.pool)
            .await
            .map_err(|e| format!("migration: {e}"))?;
            self.set_setting("backfilled_global_orphans_v1", "true").await?;
        }

        Ok(())
    }

    /// Renames every project whose name collides case-insensitively (ASCII —
    /// `LOWER()` in SQLite, matching what the `NOCASE` collation folds) with
    /// another project, so the `idx_projects_name_nocase` unique index can be
    /// created safely afterwards. See `dedupe_environment_names_nocase` for
    /// the shared algorithm/rationale; this is the same thing without the
    /// `project_id` scoping column.
    async fn dedupe_project_names_nocase(&self) -> Result<Vec<RenameRecord>, String> {
        let groups = sqlx::query(
            "SELECT LOWER(name) AS k, COUNT(*) c FROM projects GROUP BY k HAVING c > 1",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("dedupe project names: {e}"))?;

        let mut renames = Vec::new();
        for group in groups {
            let key: String = group.get(0);

            let rows = sqlx::query("SELECT id, name FROM projects WHERE LOWER(name) = ?1 ORDER BY id ASC")
                .bind(&key)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| format!("dedupe project names: {e}"))?;

            // Lowest id wins and keeps its name unchanged — `list_projects` is
            // `ORDER BY id ASC` and every name-based lookup takes the first
            // match, so lowest id is what today's lookups already resolve to.
            //
            // The rename base is the lowercased collision `key`, not the
            // winner's original casing: seeding `MyApp` (winner) + `myapp`
            // must rename the loser to `myapp-2`, not `MyApp-2`.
            for row in rows.iter().skip(1) {
                let id: i64 = row.get(0);
                let original_name: String = row.get(1);

                let new_name = self.next_free_project_name(&key, id).await?;

                let now = now_ts();
                sqlx::query("UPDATE projects SET name = ?1, updated = ?2 WHERE id = ?3")
                    .bind(&new_name)
                    .bind(&now)
                    .bind(id)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| format!("dedupe project names: {e}"))?;

                let record = RenameRecord {
                    table: "projects".to_string(),
                    id,
                    project_id: None,
                    from: original_name,
                    to: new_name,
                    at: now_iso8601(),
                };
                // Persisted immediately, per rename — not batched up and
                // written once at the end of `init_schema` — so a rename can
                // never land in the DB without also being recorded in the
                // one report that makes it (by hand) reversible.
                self.persist_rename_report(vec![record.clone()]).await?;
                renames.push(record);
            }
        }
        Ok(renames)
    }

    /// Finds the first `<base>-<n>` (n starting at 2) that doesn't
    /// case-insensitively collide with any other project name, excluding
    /// `exclude_id` (the row being renamed itself).
    ///
    /// Compares via `LOWER(name) = LOWER(?)` — both sides folded by SQLite's
    /// own (ASCII-only) `LOWER()` — rather than folding the candidate in Rust
    /// with `to_lowercase()` (full Unicode) and binding that. Folding in Rust
    /// would, for a non-ASCII `base` (e.g. `producciÓn`, itself already
    /// SQLite-folded and so still carrying an unfolded `Ó`), fold *further*
    /// than SQLite's `LOWER(name)` ever would on the stored side — so a real
    /// collision (e.g. against a pre-existing `producciÓn-2`) would compare
    /// unequal and be missed, the candidate would be accepted as "free", and
    /// the following `UPDATE` would then trip the table's exact-match
    /// `UNIQUE(project_id, name)` (or here, `UNIQUE` on `name`) constraint —
    /// turning a routine dedup into a hard `init_schema` failure. See the
    /// non-ASCII regression test for the reproduction.
    async fn next_free_project_name(&self, base: &str, exclude_id: i64) -> Result<String, String> {
        let mut suffix = 2i64;
        loop {
            let candidate = format!("{base}-{suffix}");
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM projects WHERE LOWER(name) = LOWER(?1) AND id != ?2",
            )
            .bind(&candidate)
            .bind(exclude_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| format!("dedupe project names: {e}"))?;
            if exists == 0 {
                return Ok(candidate);
            }
            suffix += 1;
        }
    }

    /// Renames every environment whose name collides case-insensitively
    /// (ASCII — `LOWER()` in SQLite, matching what the `NOCASE` collation
    /// folds) with a sibling in the same project, so
    /// `idx_environments_name_nocase` can be created safely afterwards.
    ///
    /// The first row (lowest `id`) in each colliding group keeps its name.
    /// Every subsequent row is renamed to `<lowercased-key>-<n>`, incrementing
    /// `n` past any existing collision (including a pre-existing `-2`, or a
    /// sibling already renamed earlier in this same run) until the candidate
    /// is free. Only the `name` column changes — `id` is untouched, so
    /// `environment_vars`/`environment_paths` (FK on `environment_id`) and
    /// `item_projects` ownership are structurally unaffected: zero rows move,
    /// zero rows are dropped.
    async fn dedupe_environment_names_nocase(&self) -> Result<Vec<RenameRecord>, String> {
        let groups = sqlx::query(
            "SELECT project_id, LOWER(name) AS k, COUNT(*) c FROM environments GROUP BY project_id, k HAVING c > 1",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("dedupe environment names: {e}"))?;

        let mut renames = Vec::new();
        for group in groups {
            let project_id: i64 = group.get(0);
            let key: String = group.get(1);

            let rows = sqlx::query(
                "SELECT id, name FROM environments WHERE project_id = ?1 AND LOWER(name) = ?2 ORDER BY id ASC",
            )
            .bind(project_id)
            .bind(&key)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| format!("dedupe environment names: {e}"))?;

            // Rename base is the lowercased collision `key`, not the winner's
            // original casing (same reasoning as `dedupe_project_names_nocase`).
            for row in rows.iter().skip(1) {
                let id: i64 = row.get(0);
                let original_name: String = row.get(1);

                let new_name = self.next_free_environment_name(project_id, &key, id).await?;

                let now = now_ts();
                sqlx::query("UPDATE environments SET name = ?1, updated = ?2 WHERE id = ?3")
                    .bind(&new_name)
                    .bind(&now)
                    .bind(id)
                    .execute(&self.pool)
                    .await
                    .map_err(|e| format!("dedupe environment names: {e}"))?;

                let record = RenameRecord {
                    table: "environments".to_string(),
                    id,
                    project_id: Some(project_id),
                    from: original_name,
                    to: new_name,
                    at: now_iso8601(),
                };
                // Persisted immediately, per rename — see the matching
                // comment in `dedupe_project_names_nocase`.
                self.persist_rename_report(vec![record.clone()]).await?;
                renames.push(record);
            }
        }
        Ok(renames)
    }

    /// Finds the first `<base>-<n>` (n starting at 2) that doesn't
    /// case-insensitively collide with any other environment in `project_id`,
    /// excluding `exclude_id` (the row being renamed itself).
    ///
    /// See `next_free_project_name` for why this compares via
    /// `LOWER(name) = LOWER(?)` (both sides folded by SQLite, ASCII-only)
    /// instead of pre-folding the candidate in Rust with `to_lowercase()`
    /// (full Unicode) — the mismatch between the two folds is exactly what
    /// let a non-ASCII collision slip past this check and then trip the
    /// dedup `UPDATE` on the table's exact-match unique constraint.
    async fn next_free_environment_name(
        &self,
        project_id: i64,
        base: &str,
        exclude_id: i64,
    ) -> Result<String, String> {
        let mut suffix = 2i64;
        loop {
            let candidate = format!("{base}-{suffix}");
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM environments WHERE project_id = ?1 AND LOWER(name) = LOWER(?2) AND id != ?3",
            )
            .bind(project_id)
            .bind(&candidate)
            .bind(exclude_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| format!("dedupe environment names: {e}"))?;
            if exists == 0 {
                return Ok(candidate);
            }
            suffix += 1;
        }
    }

    /// Merges (appends) `renames` into the `env_name_dedup_v1` settings key —
    /// a machine-readable, human-reversible audit of every rename performed
    /// by the two dedup passes above. Never overwritten: on every
    /// `init_schema` run this is called with whatever (possibly empty) list
    /// the current run produced, and existing entries from prior opens are
    /// preserved. A no-op (does not touch the setting at all) when `renames`
    /// is empty, so a vault that never had a collision never gets the key.
    async fn persist_rename_report(&self, mut renames: Vec<RenameRecord>) -> Result<(), String> {
        if renames.is_empty() {
            return Ok(());
        }
        let existing = self.get_setting("env_name_dedup_v1").await?;
        let mut all: Vec<RenameRecord> = match existing {
            Some(json) => match serde_json::from_str(&json) {
                Ok(parsed) => parsed,
                Err(e) => {
                    // Never silently drop prior history on a parse failure
                    // (schema drift, a partial/corrupt write) — that history
                    // is the sole reversal path for an otherwise irreversible
                    // rename. Log it loudly and preserve the raw value
                    // verbatim under a side key before starting a fresh list,
                    // so nothing is lost even though it can't be merged
                    // structurally.
                    eprintln!(
                        "env_name_dedup_v1: existing report failed to parse ({e}) — \
                         preserving it verbatim under env_name_dedup_v1_corrupt \
                         and starting a fresh report"
                    );
                    self.set_setting("env_name_dedup_v1_corrupt", &json).await?;
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        all.append(&mut renames);
        let json = serde_json::to_string(&all).map_err(|e| format!("serialize dedup report: {e}"))?;
        self.set_setting("env_name_dedup_v1", &json).await
    }

    pub async fn is_initialized(&self) -> Result<bool, String> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM vault_meta")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(count > 0)
    }

    pub async fn init_vault(&self, kdf_salt: &str, verify_token: &str) -> Result<(), String> {
        sqlx::query(
            "INSERT INTO vault_meta (id, kdf_salt, verify_token) VALUES (1, ?1, ?2)",
        )
        .bind(kdf_salt)
        .bind(verify_token)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn get_meta(&self) -> Result<Option<(String, String)>, String> {
        let row = sqlx::query("SELECT kdf_salt, verify_token FROM vault_meta WHERE id = 1")
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(row.map(|r| (r.get::<String, _>(0), r.get::<String, _>(1))))
    }

    /// Returns (id, item_type, encrypted_data, created, is_global).
    pub async fn list_items(&self) -> Result<Vec<(i64, String, String, String, bool)>, String> {
        let rows =
            sqlx::query("SELECT id, item_type, data, created, is_global FROM items ORDER BY id ASC")
                .fetch_all(&self.pool)
                .await
                .map_err(|e| e.to_string())?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let is_global: i64 = r.get(4);
                (
                    r.get::<i64, _>(0),
                    r.get::<String, _>(1),
                    r.get::<String, _>(2),
                    r.get::<String, _>(3),
                    is_global != 0,
                )
            })
            .collect())
    }

    /// Same row shape as `list_items`, restricted to `ids` (unknown ids are
    /// simply absent). Lets callers decrypt only what they reference.
    pub async fn get_items_by_ids(
        &self,
        ids: &[i64],
    ) -> Result<Vec<(i64, String, String, String, bool)>, String> {
        let mut out = Vec::with_capacity(ids.len());
        // Chunked to stay far below SQLite's bound-parameter limit.
        for chunk in ids.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT id, item_type, data, created, is_global FROM items WHERE id IN ({placeholders}) ORDER BY id ASC"
            );
            let mut query = sqlx::query(&sql);
            for id in chunk {
                query = query.bind(*id);
            }
            let rows = query.fetch_all(&self.pool).await.map_err(|e| e.to_string())?;
            for r in rows {
                let is_global: i64 = r.get(4);
                out.push((
                    r.get::<i64, _>(0),
                    r.get::<String, _>(1),
                    r.get::<String, _>(2),
                    r.get::<String, _>(3),
                    is_global != 0,
                ));
            }
        }
        Ok(out)
    }

    /// id = 0 → INSERT (returns new id). id > 0 → UPDATE (returns same id).
    /// `is_global` is written on both insert and update — callers must pass the
    /// item's current/intended value (updates never silently reset it).
    pub async fn upsert_item(
        &self,
        id: i64,
        item_type: &str,
        data: &str,
        created: &str,
        is_global: bool,
    ) -> Result<i64, String> {
        let now = now_ts();
        let is_global_i: i64 = if is_global { 1 } else { 0 };
        if id == 0 {
            let res = sqlx::query(
                "INSERT INTO items (item_type, data, created, updated, is_global) VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(item_type)
            .bind(data)
            .bind(created)
            .bind(&now)
            .bind(is_global_i)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
            Ok(res.last_insert_rowid())
        } else {
            sqlx::query("UPDATE items SET data = ?1, updated = ?2, is_global = ?3 WHERE id = ?4")
                .bind(data)
                .bind(&now)
                .bind(is_global_i)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(|e| e.to_string())?;
            Ok(id)
        }
    }

    pub async fn delete_item(&self, id: i64) -> Result<(), String> {
        sqlx::query("DELETE FROM environment_vars WHERE item_id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM items WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Reads one item's (data, is_global) inside an open transaction, so a
    /// read-merge-write sequence sees and writes a consistent state.
    pub async fn get_item_tx(
        tx: &mut Transaction<'_, Sqlite>,
        id: i64,
    ) -> Result<Option<(String, bool)>, String> {
        let row = sqlx::query("SELECT data, is_global FROM items WHERE id = ?1")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        Ok(row.map(|r| (r.get::<String, _>(0), r.get::<i64, _>(1) != 0)))
    }

    /// In-transaction counterpart of `upsert_item` for an existing row.
    pub async fn update_item_tx(
        tx: &mut Transaction<'_, Sqlite>,
        id: i64,
        data: &str,
        is_global: bool,
    ) -> Result<(), String> {
        sqlx::query("UPDATE items SET data = ?1, updated = ?2, is_global = ?3 WHERE id = ?4")
            .bind(data)
            .bind(now_ts())
            .bind(if is_global { 1_i64 } else { 0_i64 })
            .bind(id)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Forks a multi-owner item into one independent copy per owning project
    /// and removes the original, all in one transaction. `copies` holds
    /// `(project_id, encrypted_data)`; returns the new item ids in order. A
    /// failure at any step leaves the original item and its links untouched.
    pub async fn fork_item_per_owner(
        &self,
        original_id: i64,
        item_type: &str,
        created: &str,
        copies: &[(i64, String)],
    ) -> Result<Vec<i64>, String> {
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        let now = now_ts();
        let mut new_ids = Vec::with_capacity(copies.len());
        for (project_id, data) in copies {
            let res = sqlx::query(
                "INSERT INTO items (item_type, data, created, updated, is_global) VALUES (?1, ?2, ?3, ?4, 0)",
            )
            .bind(item_type)
            .bind(data)
            .bind(created)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
            let new_id = res.last_insert_rowid();
            Self::add_item_owner_tx(&mut tx, new_id, *project_id).await?;
            sqlx::query(
                "UPDATE environment_vars SET item_id = ?1
                 WHERE item_id = ?2 AND environment_id IN
                     (SELECT id FROM environments WHERE project_id = ?3)",
            )
            .bind(new_id)
            .bind(original_id)
            .bind(project_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
            new_ids.push(new_id);
        }
        sqlx::query("DELETE FROM environment_vars WHERE item_id = ?1")
            .bind(original_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("DELETE FROM items WHERE id = ?1")
            .bind(original_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(new_ids)
    }

    /// Applies the legacy literal-var migration atomically: each tuple is
    /// `(environment_var_id, project_id, encrypted_item_data, created)`. Creates
    /// the item, grants ownership, repoints the var, and finally records the
    /// `migrated_literals_v1` flag. Either everything lands or nothing does.
    pub async fn apply_literal_migration(
        &self,
        entries: &[(i64, i64, String, String)],
    ) -> Result<(), String> {
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        let now = now_ts();
        for (env_var_id, project_id, data, created) in entries {
            let res = sqlx::query(
                "INSERT INTO items (item_type, data, created, updated, is_global) VALUES ('secret', ?1, ?2, ?3, 0)",
            )
            .bind(data)
            .bind(created)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
            let new_id = res.last_insert_rowid();
            Self::add_item_owner_tx(&mut tx, new_id, *project_id).await?;
            sqlx::query("UPDATE environment_vars SET item_id = ?1, literal = NULL WHERE id = ?2")
                .bind(new_id)
                .bind(env_var_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        }
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES ('migrated_literals_v1', 'true')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        tx.commit().await.map_err(|e| e.to_string())
    }

    pub async fn set_item_global(&self, id: i64, is_global: bool) -> Result<(), String> {
        let is_global_i: i64 = if is_global { 1 } else { 0 };
        sqlx::query("UPDATE items SET is_global = ?1 WHERE id = ?2")
            .bind(is_global_i)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn is_item_global(&self, id: i64) -> Result<Option<bool>, String> {
        let val: Option<i64> = sqlx::query_scalar("SELECT is_global FROM items WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(val.map(|v| v != 0))
    }

    pub async fn item_owner_count(&self, item_id: i64) -> Result<i64, String> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM item_projects WHERE item_id = ?1")
            .bind(item_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(count)
    }

    pub async fn list_owning_projects(&self, item_id: i64) -> Result<Vec<i64>, String> {
        let rows = sqlx::query("SELECT project_id FROM item_projects WHERE item_id = ?1")
            .bind(item_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(|r| r.get(0)).collect())
    }

    pub async fn add_item_owner(&self, item_id: i64, project_id: i64) -> Result<(), String> {
        sqlx::query("INSERT OR IGNORE INTO item_projects (item_id, project_id) VALUES (?1, ?2)")
            .bind(item_id)
            .bind(project_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn list_owning_projects_tx(
        tx: &mut Transaction<'_, Sqlite>,
        item_id: i64,
    ) -> Result<Vec<i64>, String> {
        let rows = sqlx::query("SELECT project_id FROM item_projects WHERE item_id = ?1")
            .bind(item_id)
            .fetch_all(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(|r| r.get(0)).collect())
    }

    pub async fn is_item_global_tx(
        tx: &mut Transaction<'_, Sqlite>,
        id: i64,
    ) -> Result<Option<bool>, String> {
        let val: Option<i64> = sqlx::query_scalar("SELECT is_global FROM items WHERE id = ?1")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        Ok(val.map(|v| v != 0))
    }

    pub async fn add_item_owner_tx(
        tx: &mut Transaction<'_, Sqlite>,
        item_id: i64,
        project_id: i64,
    ) -> Result<(), String> {
        sqlx::query("INSERT OR IGNORE INTO item_projects (item_id, project_id) VALUES (?1, ?2)")
            .bind(item_id)
            .bind(project_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn list_owned_item_ids(&self, project_id: i64) -> Result<Vec<i64>, String> {
        let rows = sqlx::query("SELECT item_id FROM item_projects WHERE project_id = ?1")
            .bind(project_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(|r| r.get(0)).collect())
    }

    pub async fn delete_environment_vars_by_item(&self, item_id: i64) -> Result<(), String> {
        sqlx::query("DELETE FROM environment_vars WHERE item_id = ?1")
            .bind(item_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Legacy literal-only vars (predating the "every var is a real item"
    /// model) — returns (environment_var_id, key, literal, owning project_id).
    pub async fn list_unmigrated_literal_vars(&self) -> Result<Vec<(i64, String, String, i64)>, String> {
        let rows = sqlx::query(
            "SELECT ev.id, ev.key, ev.literal, e.project_id
             FROM environment_vars ev
             JOIN environments e ON e.id = ev.environment_id
             WHERE ev.item_id IS NULL AND ev.literal IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(rows
            .into_iter()
            .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
            .collect())
    }

    pub async fn set_environment_var_item(&self, env_var_id: i64, item_id: i64) -> Result<(), String> {
        sqlx::query("UPDATE environment_vars SET item_id = ?1, literal = NULL WHERE id = ?2")
            .bind(item_id)
            .bind(env_var_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Repoints every environment_var in `project_id`'s environments that
    /// referenced `old_item_id` to `new_item_id` — used when forking a
    /// multi-owner item back into independent per-project copies.
    pub async fn repoint_env_var_item(
        &self,
        project_id: i64,
        old_item_id: i64,
        new_item_id: i64,
    ) -> Result<(), String> {
        sqlx::query(
            "UPDATE environment_vars SET item_id = ?1
             WHERE item_id = ?2 AND environment_id IN
                 (SELECT id FROM environments WHERE project_id = ?3)",
        )
        .bind(new_item_id)
        .bind(old_item_id)
        .bind(project_id)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn list_categories(&self) -> Result<Vec<DbCategory>, String> {
        let rows = sqlx::query("SELECT cid, name, color, description FROM categories ORDER BY rowid ASC")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(rows
            .into_iter()
            .map(|r| DbCategory {
                cid: r.get(0),
                name: r.get(1),
                color: r.get(2),
                description: r.get(3),
            })
            .collect())
    }

    /// Diff + upsert in one transaction: categories absent from `cats` are
    /// deleted (their `project_categories` links cascade, which is correct),
    /// the rest are inserted or updated in place so links to categories that
    /// still exist survive. Any failure rolls the whole save back.
    pub async fn save_categories(&self, cats: &[DbCategory]) -> Result<(), String> {
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;

        let existing: Vec<String> = sqlx::query_scalar("SELECT cid FROM categories")
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        let keep: std::collections::HashSet<&str> = cats.iter().map(|c| c.cid.as_str()).collect();
        for cid in existing.iter().filter(|cid| !keep.contains(cid.as_str())) {
            sqlx::query("DELETE FROM categories WHERE cid = ?1")
                .bind(cid)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        }

        for cat in cats {
            sqlx::query(
                "INSERT INTO categories (cid, name, color, description) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(cid) DO UPDATE SET
                     name = excluded.name,
                     color = excluded.color,
                     description = excluded.description",
            )
            .bind(&cat.cid)
            .bind(&cat.name)
            .bind(&cat.color)
            .bind(&cat.description)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        }

        tx.commit().await.map_err(|e| e.to_string())
    }

    pub async fn insert_category(&self, cat: &DbCategory) -> Result<(), String> {
        sqlx::query(
            "INSERT INTO categories (cid, name, color, description) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(&cat.cid)
        .bind(&cat.name)
        .bind(&cat.color)
        .bind(&cat.description)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn update_category(&self, cat: &DbCategory) -> Result<bool, String> {
        let res =
            sqlx::query("UPDATE categories SET name = ?1, color = ?2, description = ?3 WHERE cid = ?4")
                .bind(&cat.name)
                .bind(&cat.color)
                .bind(&cat.description)
                .bind(&cat.cid)
                .execute(&self.pool)
                .await
                .map_err(|e| e.to_string())?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn delete_category(&self, cid: &str) -> Result<bool, String> {
        let res = sqlx::query("DELETE FROM categories WHERE cid = ?1")
            .bind(cid)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn get_setting(&self, key: &str) -> Result<Option<String>, String> {
        let val: Option<String> =
            sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
                .bind(key)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| e.to_string())?;
        Ok(val)
    }

    /// Record a share event to the audit log.
    pub async fn log_share(
        &self,
        mode: &str,
        direction: &str,
        item_ids: &[i64],
        peer_fp: Option<&str>,
    ) -> Result<(), String> {
        let ids_str: String = item_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let ts = now_ts();
        sqlx::query(
            "INSERT INTO share_log (mode, direction, item_ids, peer_fp, timestamp) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(mode)
        .bind(direction)
        .bind(ids_str)
        .bind(peer_fp)
        .bind(ts)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)")
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Re-key: atomically replaces vault_meta and re-encrypts all item blobs.
    pub async fn rekey(
        &self,
        new_salt: &str,
        new_token: &str,
        items: Vec<(i64, String)>,
    ) -> Result<(), String> {
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        sqlx::query("UPDATE vault_meta SET kdf_salt = ?1, verify_token = ?2 WHERE id = 1")
            .bind(new_salt)
            .bind(new_token)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        let now = now_ts();
        for (id, data) in &items {
            sqlx::query("UPDATE items SET data = ?1, updated = ?2 WHERE id = ?3")
                .bind(data.as_str())
                .bind(&now)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        }
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn list_workspaces(&self) -> Result<Vec<DbWorkspace>, String> {
        let rows = sqlx::query(
            "SELECT id, name, description, template, created, updated FROM workspaces ORDER BY id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        let path_rows = sqlx::query(
            "SELECT workspace_id, path FROM workspace_paths ORDER BY workspace_id, id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        let mut paths_map: HashMap<i64, Vec<String>> = HashMap::new();
        for row in path_rows {
            let ws_id: i64 = row.get(0);
            let path: String = row.get(1);
            paths_map.entry(ws_id).or_default().push(path);
        }

        Ok(rows
            .into_iter()
            .map(|r| {
                let id: i64 = r.get(0);
                DbWorkspace {
                    id,
                    name: r.get(1),
                    description: r.get(2),
                    paths: paths_map.remove(&id).unwrap_or_default(),
                    template: r.get(3),
                    created: r.get(4),
                    updated: r.get(5),
                }
            })
            .collect())
    }

    /// id = 0 → INSERT, returns new id. id > 0 → UPDATE, returns same id.
    pub async fn upsert_workspace(
        &self,
        id: i64,
        name: &str,
        description: Option<&str>,
        template: &str,
    ) -> Result<i64, String> {
        let now = now_ts();
        if id == 0 {
            let res = sqlx::query(
                "INSERT INTO workspaces (name, description, template, created, updated) VALUES (?1,?2,?3,?4,?5)",
            )
            .bind(name)
            .bind(description)
            .bind(template)
            .bind(&now)
            .bind(&now)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
            Ok(res.last_insert_rowid())
        } else {
            sqlx::query(
                "UPDATE workspaces SET name=?1, description=?2, template=?3, updated=?4 WHERE id=?5",
            )
            .bind(name)
            .bind(description)
            .bind(template)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
            Ok(id)
        }
    }

    pub async fn get_workspace_vars(&self, workspace_id: i64) -> Result<Vec<DbWorkspaceVar>, String> {
        let rows = sqlx::query(
            "SELECT id, workspace_id, key, item_id, literal FROM workspace_vars WHERE workspace_id = ?1 ORDER BY id ASC",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(rows
            .into_iter()
            .map(|r| DbWorkspaceVar {
                id: r.get(0),
                workspace_id: r.get(1),
                key: r.get(2),
                item_id: r.get(3),
                literal: r.get(4),
            })
            .collect())
    }

    pub async fn set_workspace_vars(
        &self,
        workspace_id: i64,
        vars: &[DbWorkspaceVar],
    ) -> Result<(), String> {
        sqlx::query("DELETE FROM workspace_vars WHERE workspace_id = ?1")
            .bind(workspace_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        for v in vars {
            sqlx::query(
                "INSERT INTO workspace_vars (workspace_id, key, item_id, literal) VALUES (?1,?2,?3,?4)",
            )
            .bind(workspace_id)
            .bind(&v.key)
            .bind(v.item_id)
            .bind(&v.literal)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Looks up a single project's name by id — used to build the
    /// `envfile` marker line (project + environment names, informational
    /// only) without loading the full project list.
    pub async fn get_project_name(&self, project_id: i64) -> Result<Option<String>, String> {
        sqlx::query_scalar("SELECT name FROM projects WHERE id = ?1")
            .bind(project_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn list_projects(&self) -> Result<Vec<DbProject>, String> {
        let rows = sqlx::query(
            "SELECT id, name, description, template, created, updated, root_path FROM projects ORDER BY id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(rows
            .into_iter()
            .map(|r| DbProject {
                id: r.get(0),
                name: r.get(1),
                description: r.get(2),
                template: r.get(3),
                created: r.get(4),
                updated: r.get(5),
                root_path: r.get(6),
            })
            .collect())
    }

    /// Sets (or clears, with `None`) a project's root directory and bumps
    /// its `updated` timestamp.
    pub async fn set_project_root(&self, project_id: i64, root_path: Option<&str>) -> Result<(), String> {
        sqlx::query("UPDATE projects SET root_path = ?1, updated = ?2 WHERE id = ?3")
            .bind(root_path)
            .bind(now_ts())
            .bind(project_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Root directory of one project (`None` when unset or no such project).
    pub async fn get_project_root(&self, project_id: i64) -> Result<Option<String>, String> {
        let row = sqlx::query("SELECT root_path FROM projects WHERE id = ?1")
            .bind(project_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(row.and_then(|r| r.get::<Option<String>, _>(0)))
    }

    /// id = 0 → INSERT, returns new id. id > 0 → UPDATE, returns same id.
    pub async fn upsert_project(
        &self,
        id: i64,
        name: &str,
        description: Option<&str>,
        template: &str,
    ) -> Result<i64, String> {
        let now = now_ts();
        if id == 0 {
            let res = sqlx::query(
                "INSERT INTO projects (name, description, template, created, updated) VALUES (?1,?2,?3,?4,?5)",
            )
            .bind(name)
            .bind(description)
            .bind(template)
            .bind(&now)
            .bind(&now)
            .execute(&self.pool)
            .await
            .map_err(|e| map_conflict(e, PROJECT_NAME_CONFLICT))?;
            Ok(res.last_insert_rowid())
        } else {
            sqlx::query(
                "UPDATE projects SET name=?1, description=?2, template=?3, updated=?4 WHERE id=?5",
            )
            .bind(name)
            .bind(description)
            .bind(template)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| map_conflict(e, PROJECT_NAME_CONFLICT))?;
            Ok(id)
        }
    }

    /// Read-only dry run of `delete_project`'s bookkeeping, for the
    /// typed-confirmation modal's blast-radius summary.
    pub async fn preview_delete_project(&self, project_id: i64) -> Result<ProjectDeleteImpact, String> {
        let environments: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM environments WHERE project_id = ?1")
                .bind(project_id)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| e.to_string())?;

        let owned = self.list_owned_item_ids(project_id).await?;
        let mut items_deleted = 0i64;
        let mut items_orphaned = 0i64;
        for item_id in owned {
            if self.item_owner_count(item_id).await? <= 1 {
                if self.is_item_global(item_id).await?.unwrap_or(false) {
                    items_orphaned += 1;
                } else {
                    items_deleted += 1;
                }
            }
        }
        Ok(ProjectDeleteImpact { environments, items_deleted, items_orphaned })
    }

    /// Deletes a project and everything it exclusively owns, in one
    /// transaction. Environments/environment_vars/environment_paths and this
    /// project's `item_projects` ownership links cascade automatically via
    /// FK. Afterwards, for every item this project used to own: if it still
    /// has another owner, nothing to do; if it has none left, delete it
    /// (local secret with nowhere else to live) unless it's global, in which
    /// case it survives as an unowned orphan, still visible in Global Secrets.
    pub async fn delete_project(&self, id: i64) -> Result<ProjectDeleteImpact, String> {
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;

        let environments: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM environments WHERE project_id = ?1")
                .bind(id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;

        let owned_rows = sqlx::query("SELECT item_id FROM item_projects WHERE project_id = ?1")
            .bind(id)
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        let owned: Vec<i64> = owned_rows.into_iter().map(|r| r.get(0)).collect();

        sqlx::query("DELETE FROM projects WHERE id = ?1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        let mut items_deleted = 0i64;
        let mut items_orphaned = 0i64;
        for item_id in owned {
            let owner_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM item_projects WHERE item_id = ?1")
                    .bind(item_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|e| e.to_string())?;
            if owner_count > 0 {
                continue;
            }
            let is_global: i64 = sqlx::query_scalar("SELECT is_global FROM items WHERE id = ?1")
                .bind(item_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
            if is_global != 0 {
                items_orphaned += 1;
            } else {
                // Defensive: any stray environment_var reference (there shouldn't
                // be any left outside this project, since owner_count is 0) is
                // cleaned up before the item itself goes.
                sqlx::query("DELETE FROM environment_vars WHERE item_id = ?1")
                    .bind(item_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| e.to_string())?;
                sqlx::query("DELETE FROM items WHERE id = ?1")
                    .bind(item_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| e.to_string())?;
                items_deleted += 1;
            }
        }

        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(ProjectDeleteImpact { environments, items_deleted, items_orphaned })
    }

    pub async fn set_project_categories(&self, project_id: i64, category_ids: &[String]) -> Result<(), String> {
        sqlx::query("DELETE FROM project_categories WHERE project_id = ?1")
            .bind(project_id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        for cid in category_ids {
            sqlx::query(
                "INSERT OR IGNORE INTO project_categories (project_id, category_id) VALUES (?1, ?2)",
            )
            .bind(project_id)
            .bind(cid)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Returns category NAMES (not ids) — same convention as `VaultItem.categories`.
    pub async fn list_project_categories(&self, project_id: i64) -> Result<Vec<String>, String> {
        let rows = sqlx::query(
            "SELECT c.name FROM project_categories pc
             JOIN categories c ON c.cid = pc.category_id
             WHERE pc.project_id = ?1",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(|r| r.get(0)).collect())
    }

    pub async fn list_environments(&self, project_id: i64) -> Result<Vec<DbEnvironment>, String> {
        let rows = sqlx::query(
            "SELECT id, project_id, name, is_default, created, updated FROM environments WHERE project_id = ?1 ORDER BY id ASC",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        let path_rows = sqlx::query(
            "SELECT ep.environment_id, ep.path FROM environment_paths ep
             JOIN environments e ON e.id = ep.environment_id
             WHERE e.project_id = ?1 ORDER BY ep.environment_id, ep.id ASC",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        let mut paths_map: HashMap<i64, Vec<String>> = HashMap::new();
        for row in path_rows {
            let env_id: i64 = row.get(0);
            let path: String = row.get(1);
            paths_map.entry(env_id).or_default().push(path);
        }

        Ok(rows
            .into_iter()
            .map(|r| {
                let id: i64 = r.get(0);
                let is_default_i: i64 = r.get(3);
                DbEnvironment {
                    id,
                    project_id: r.get(1),
                    name: r.get(2),
                    is_default: is_default_i != 0,
                    paths: paths_map.remove(&id).unwrap_or_default(),
                    created: r.get(4),
                    updated: r.get(5),
                }
            })
            .collect())
    }

    pub async fn get_environment(&self, id: i64) -> Result<Option<DbEnvironment>, String> {
        let row = sqlx::query(
            "SELECT id, project_id, name, is_default, created, updated FROM environments WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };

        let is_default_i: i64 = row.get(3);
        let paths = self.get_environment_paths(id).await?;

        Ok(Some(DbEnvironment {
            id: row.get(0),
            project_id: row.get(1),
            name: row.get(2),
            is_default: is_default_i != 0,
            paths,
            created: row.get(4),
            updated: row.get(5),
        }))
    }

    pub async fn upsert_environment_tx(
        tx: &mut Transaction<'_, Sqlite>,
        id: i64,
        project_id: i64,
        name: &str,
        is_default: bool,
    ) -> Result<i64, String> {
        let now = now_ts();
        let is_default_i: i64 = if is_default { 1 } else { 0 };
        if id == 0 {
            let res = sqlx::query(
                "INSERT INTO environments (project_id, name, is_default, created, updated) VALUES (?1,?2,?3,?4,?5)",
            )
            .bind(project_id)
            .bind(name)
            .bind(is_default_i)
            .bind(&now)
            .bind(&now)
            .execute(&mut **tx)
            .await
            .map_err(|e| map_conflict(e, ENVIRONMENT_NAME_CONFLICT))?;
            Ok(res.last_insert_rowid())
        } else {
            sqlx::query(
                "UPDATE environments SET name=?1, is_default=?2, updated=?3 WHERE id=?4",
            )
            .bind(name)
            .bind(is_default_i)
            .bind(&now)
            .bind(id)
            .execute(&mut **tx)
            .await
            .map_err(|e| map_conflict(e, ENVIRONMENT_NAME_CONFLICT))?;
            Ok(id)
        }
    }

    /// id = 0 → INSERT, returns new id. id > 0 → UPDATE, returns same id.
    pub async fn upsert_environment(
        &self,
        id: i64,
        project_id: i64,
        name: &str,
        is_default: bool,
    ) -> Result<i64, String> {
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        let env_id = Self::upsert_environment_tx(&mut tx, id, project_id, name, is_default).await?;
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(env_id)
    }

    pub async fn delete_environment(&self, id: i64) -> Result<(), String> {
        sqlx::query("DELETE FROM environments WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn get_environment_paths(&self, environment_id: i64) -> Result<Vec<String>, String> {
        let rows = sqlx::query(
            "SELECT path FROM environment_paths WHERE environment_id = ?1 ORDER BY id ASC",
        )
        .bind(environment_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(|r| r.get(0)).collect())
    }

    pub async fn set_environment_paths_tx(
        tx: &mut Transaction<'_, Sqlite>,
        environment_id: i64,
        paths: &[String],
    ) -> Result<(), String> {
        sqlx::query("DELETE FROM environment_paths WHERE environment_id = ?1")
            .bind(environment_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        for path in paths {
            sqlx::query(
                "INSERT INTO environment_paths (environment_id, path) VALUES (?1, ?2)",
            )
            .bind(environment_id)
            .bind(path)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Rejects duplicate paths before anything is written.
    pub async fn set_environment_paths(
        &self,
        environment_id: i64,
        paths: &[String],
    ) -> Result<(), String> {
        validate_unique_paths(paths)?;
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        Self::set_environment_paths_tx(&mut tx, environment_id, paths).await?;
        tx.commit().await.map_err(|e| e.to_string())
    }

    pub async fn get_environment_vars(&self, environment_id: i64) -> Result<Vec<DbEnvironmentVar>, String> {
        let rows = sqlx::query(
            "SELECT id, environment_id, key, item_id, literal FROM environment_vars WHERE environment_id = ?1 ORDER BY id ASC",
        )
        .bind(environment_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(rows
            .into_iter()
            .map(|r| DbEnvironmentVar {
                id: r.get(0),
                environment_id: r.get(1),
                key: r.get(2),
                item_id: r.get(3),
                literal: r.get(4),
            })
            .collect())
    }

    pub async fn set_environment_vars_tx(
        tx: &mut Transaction<'_, Sqlite>,
        environment_id: i64,
        vars: &[DbEnvironmentVar],
    ) -> Result<(), String> {
        sqlx::query("DELETE FROM environment_vars WHERE environment_id = ?1")
            .bind(environment_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        for v in vars {
            sqlx::query(
                "INSERT INTO environment_vars (environment_id, key, item_id, literal) VALUES (?1,?2,?3,?4)",
            )
            .bind(environment_id)
            .bind(&v.key)
            .bind(v.item_id)
            .bind(&v.literal)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Rejects duplicate keys (case-sensitive) before anything is written.
    /// The error names the key only, never a value.
    pub async fn set_environment_vars(
        &self,
        environment_id: i64,
        vars: &[DbEnvironmentVar],
    ) -> Result<(), String> {
        validate_unique_keys(vars.iter().map(|v| v.key.as_str()))?;
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        Self::set_environment_vars_tx(&mut tx, environment_id, vars).await?;
        tx.commit().await.map_err(|e| e.to_string())
    }

    /// Links a single item into an environment under `key`, without touching
    /// any other vars already set on that environment (unlike
    /// `set_environment_vars`, which replaces the whole set). If `key` is
    /// already used in this environment, it's repointed to `item_id`.
    /// Returns the environment_vars row id.
    pub async fn upsert_environment_var(
        &self,
        environment_id: i64,
        key: &str,
        item_id: i64,
    ) -> Result<i64, String> {
        sqlx::query(
            "INSERT INTO environment_vars (environment_id, key, item_id, literal) VALUES (?1, ?2, ?3, NULL)
             ON CONFLICT(environment_id, key) DO UPDATE SET item_id = excluded.item_id, literal = NULL",
        )
        .bind(environment_id)
        .bind(key)
        .bind(item_id)
        .execute(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        sqlx::query_scalar("SELECT id FROM environment_vars WHERE environment_id = ?1 AND key = ?2")
            .bind(environment_id)
            .bind(key)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| e.to_string())
    }

    // ─── Issue #9: create-or-update on env-key collision ──────────────────

    /// Read-only snapshot of the item currently linked to `key` in
    /// `environment_id`, plus enough shape (link/owner counts) for the caller
    /// to classify it as exclusive vs shared without decrypting anything.
    /// Never selects `items.data` — no ciphertext leaves this call.
    pub async fn inspect_env_key(
        &self,
        environment_id: i64,
        key: &str,
    ) -> Result<Option<EnvKeyConflict>, String> {
        let row = sqlx::query(
            "SELECT i.id, i.created, i.is_global
             FROM environment_vars ev
             JOIN items i ON i.id = ev.item_id
             WHERE ev.environment_id = ?1 AND ev.key = ?2",
        )
        .bind(environment_id)
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| e.to_string())?;

        // No row at all, or a legacy `literal`-only var (`item_id IS NULL`, so
        // the JOIN drops it) → nothing to update in place. The create path
        // runs and `upsert_environment_var`'s repoint replaces the literal,
        // matching `migrate_literal_vars_to_items`'s upgrade semantics.
        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };

        let item_id: i64 = row.get(0);
        let created: String = row.get(1);
        let is_global_i: i64 = row.get(2);

        let link_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM environment_vars WHERE item_id = ?1")
                .bind(item_id)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| e.to_string())?;

        let owner_ids = self.list_owning_projects(item_id).await?;

        Ok(Some(EnvKeyConflict {
            item_id,
            created,
            is_global: is_global_i != 0,
            link_count,
            owner_ids,
        }))
    }

    /// The single transactional mutation behind `POST /items`'s on-conflict
    /// handling. Receives an already-encrypted blob (crypto lives in `vault`,
    /// never here) and performs the whole read-classify-write inside one
    /// `sqlx::Transaction`, so there is no interleaving that leaves an item
    /// owned but unlinked (M3) or a `Replace` delete racing its own repoint.
    ///
    /// `expected` is the `item_id` (if any) the caller saw during its earlier
    /// (non-transactional) `inspect_env_key` call. The inspection is re-run
    /// here, inside the transaction; if the current state disagrees, the
    /// write is rejected as `Conflict { StateChanged }` rather than acting on
    /// a classification that may no longer hold. In practice all writers
    /// serialize on the process-wide `SharedState` mutex, so this is defence
    /// in depth, not the primary mechanism.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_or_link_item(
        &self,
        environment_id: i64,
        project_id: i64,
        key: &str,
        item_type: &str,
        encrypted: &str,
        created: &str,
        mode: LinkMode,
        expected: Option<i64>,
    ) -> Result<LinkOutcome, String> {
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;

        let current = sqlx::query(
            "SELECT i.id, i.is_global
             FROM environment_vars ev
             JOIN items i ON i.id = ev.item_id
             WHERE ev.environment_id = ?1 AND ev.key = ?2",
        )
        .bind(environment_id)
        .bind(key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        // Carry `is_global` alongside the id so the occupied-key branch below
        // reads it straight off the matched row instead of re-unwrapping
        // `current` (bare `unwrap()` is forbidden in production code).
        let current_row = current
            .as_ref()
            .map(|r| (r.get::<i64, _>(0), r.get::<i64, _>(1) != 0));
        let current_item_id = current_row.map(|(id, _)| id);

        if current_item_id != expected {
            tx.rollback().await.map_err(|e| e.to_string())?;
            // `item_id` in the response is best-effort: whichever id the
            // caller already knew about, for the error message.
            return Ok(LinkOutcome::Conflict {
                item_id: expected.or(current_item_id).unwrap_or(0),
                reason: ConflictReason::StateChanged,
            });
        }

        let now = now_ts();

        match current_row {
            None => {
                // Free key → create, own, link.
                let res = sqlx::query(
                    "INSERT INTO items (item_type, data, created, updated, is_global) VALUES (?1, ?2, ?3, ?4, 0)",
                )
                .bind(item_type)
                .bind(encrypted)
                .bind(created)
                .bind(&now)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
                let new_id = res.last_insert_rowid();

                sqlx::query("INSERT OR IGNORE INTO item_projects (item_id, project_id) VALUES (?1, ?2)")
                    .bind(new_id)
                    .bind(project_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| e.to_string())?;

                sqlx::query(
                    "INSERT INTO environment_vars (environment_id, key, item_id, literal) VALUES (?1, ?2, ?3, NULL)
                     ON CONFLICT(environment_id, key) DO UPDATE SET item_id = excluded.item_id, literal = NULL",
                )
                .bind(environment_id)
                .bind(key)
                .bind(new_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;

                tx.commit().await.map_err(|e| e.to_string())?;
                Ok(LinkOutcome::Created { item_id: new_id, is_global: false })
            }
            Some((item_id, is_global)) => {

                match mode {
                    LinkMode::Error => {
                        tx.rollback().await.map_err(|e| e.to_string())?;
                        Ok(LinkOutcome::Conflict { item_id, reason: ConflictReason::KeyExists })
                    }
                    LinkMode::Update => {
                        let link_count: i64 =
                            sqlx::query_scalar("SELECT COUNT(*) FROM environment_vars WHERE item_id = ?1")
                                .bind(item_id)
                                .fetch_one(&mut *tx)
                                .await
                                .map_err(|e| e.to_string())?;
                        let owner_rows = sqlx::query("SELECT project_id FROM item_projects WHERE item_id = ?1")
                            .bind(item_id)
                            .fetch_all(&mut *tx)
                            .await
                            .map_err(|e| e.to_string())?;
                        let owner_ids: Vec<i64> = owner_rows.into_iter().map(|r| r.get(0)).collect();

                        let exclusive = !is_global && link_count == 1 && owner_ids == [project_id];

                        if !exclusive {
                            tx.rollback().await.map_err(|e| e.to_string())?;
                            return Ok(LinkOutcome::Conflict { item_id, reason: ConflictReason::Shared });
                        }

                        sqlx::query("UPDATE items SET data = ?1, updated = ?2 WHERE id = ?3")
                            .bind(encrypted)
                            .bind(&now)
                            .bind(item_id)
                            .execute(&mut *tx)
                            .await
                            .map_err(|e| e.to_string())?;

                        tx.commit().await.map_err(|e| e.to_string())?;
                        Ok(LinkOutcome::Updated { item_id, is_global: false })
                    }
                    LinkMode::Replace => {
                        let res = sqlx::query(
                            "INSERT INTO items (item_type, data, created, updated, is_global) VALUES (?1, ?2, ?3, ?4, 0)",
                        )
                        .bind(item_type)
                        .bind(encrypted)
                        .bind(created)
                        .bind(&now)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| e.to_string())?;
                        let new_id = res.last_insert_rowid();

                        sqlx::query("INSERT OR IGNORE INTO item_projects (item_id, project_id) VALUES (?1, ?2)")
                            .bind(new_id)
                            .bind(project_id)
                            .execute(&mut *tx)
                            .await
                            .map_err(|e| e.to_string())?;

                        sqlx::query("UPDATE environment_vars SET item_id = ?1, literal = NULL WHERE environment_id = ?2 AND key = ?3")
                            .bind(new_id)
                            .bind(environment_id)
                            .bind(key)
                            .execute(&mut *tx)
                            .await
                            .map_err(|e| e.to_string())?;

                        // Delete only if now unreachable — a property of the
                        // statement (NOT EXISTS), not of application logic
                        // that could drift. Safe no-op when `item_id` is still
                        // linked elsewhere or is global.
                        sqlx::query(
                            "DELETE FROM items WHERE id = ?1 AND is_global = 0
                             AND NOT EXISTS (SELECT 1 FROM environment_vars WHERE item_id = ?1)",
                        )
                        .bind(item_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| e.to_string())?;

                        sqlx::query(
                            "DELETE FROM item_projects WHERE item_id = ?1
                             AND NOT EXISTS (SELECT 1 FROM items WHERE id = ?1)",
                        )
                        .bind(item_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| e.to_string())?;

                        tx.commit().await.map_err(|e| e.to_string())?;
                        Ok(LinkOutcome::Created { item_id: new_id, is_global: false })
                    }
                }
            }
        }
    }

    /// The issue's reproduction query, verbatim: ids of items with zero
    /// `environment_vars` references and `is_global = 0`. Global items have a
    /// reachable surface (Global Secrets) even when unlinked; non-global
    /// unlinked items have none — that asymmetry is why the predicate omits
    /// `is_global = 1` rows. See plan §4.6 for the condition under which this
    /// predicate would need to gain a `item_projects` clause (it does not
    /// today — re-verified against `main` immediately before this shipped).
    pub async fn list_orphan_item_ids(&self) -> Result<Vec<i64>, String> {
        let rows = sqlx::query(
            "SELECT i.id FROM items i
             LEFT JOIN environment_vars ev ON ev.item_id = i.id
             WHERE ev.id IS NULL AND i.is_global = 0
               AND NOT EXISTS (
                   SELECT 1 FROM item_projects ip
                   JOIN projects p ON p.id = ip.project_id
                   WHERE ip.item_id = i.id AND p.is_holding = 1
               )",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(|r| r.get(0)).collect())
    }

    /// Deletes `item_projects` then `items` for the given ids, in one
    /// transaction, re-checking the orphan predicate per id inside the
    /// transaction so a concurrently re-linked item is skipped rather than
    /// destroyed underneath its new link.
    pub async fn delete_items_cascade(&self, ids: &[i64]) -> Result<(), String> {
        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        for id in ids {
            // Re-check the full orphan predicate (unlinked AND non-global) per
            // id, inside the transaction: a concurrently re-linked item, or
            // one promoted to global, is skipped rather than destroyed
            // underneath its new reachability.
            sqlx::query(
                "DELETE FROM item_projects WHERE item_id = ?1
                 AND EXISTS (SELECT 1 FROM items WHERE id = ?1 AND is_global = 0)
                 AND NOT EXISTS (SELECT 1 FROM environment_vars WHERE item_id = ?1)
                 AND NOT EXISTS (
                     SELECT 1 FROM item_projects ip JOIN projects p ON p.id = ip.project_id
                     WHERE ip.item_id = ?1 AND p.is_holding = 1)",
            )
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

            sqlx::query(
                "DELETE FROM items WHERE id = ?1 AND is_global = 0
                 AND NOT EXISTS (SELECT 1 FROM environment_vars WHERE item_id = ?1)
                 AND NOT EXISTS (
                     SELECT 1 FROM item_projects ip JOIN projects p ON p.id = ip.project_id
                     WHERE ip.item_id = ?1 AND p.is_holding = 1)",
            )
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        }
        tx.commit().await.map_err(|e| e.to_string())
    }

    /// Inserts an entire received project (from a relay `ProjectBundle`) in
    /// one transaction: project row, items (already-encrypted ciphertext,
    /// never decrypted here), `item_projects` ownership, environments and
    /// `environment_vars` — all-or-nothing (issue #4 D7). A failure partway
    /// through must not leave orphaned rows, since the relay payload has
    /// already been burned by the time this runs and cannot be re-fetched.
    ///
    /// Checks for a case-insensitive project-name collision **before**
    /// writing anything, returning a `"conflict: ..."`-prefixed error the
    /// caller maps to HTTP 409 — this is an application-level, Unicode-aware
    /// pre-check (`str::to_lowercase`), not a reliance on the DB's ASCII-only
    /// `NOCASE` unique index, so it (a) never surfaces a raw SQLite
    /// constraint-violation string and (b) also catches non-ASCII collisions
    /// the index would miss. TODO(#12): once `idx_projects_name_nocase`'s
    /// shared collision helper / `PROJECT_NAME_CONFLICT` constant lands,
    /// swap this loop for it instead of duplicating the convention.
    pub async fn insert_received_project(
        &self,
        name: &str,
        description: Option<&str>,
        template: &str,
        items: &[ReceivedProjectItem],
        environments: &[ReceivedEnvironment],
    ) -> Result<InsertedProject, String> {
        let existing_names: Vec<String> = sqlx::query_scalar("SELECT name FROM projects")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        let name_lower = name.to_lowercase();
        if existing_names.iter().any(|n| n.to_lowercase() == name_lower) {
            // Named error (D5): the caller (Tauri command / HTTP handler) has
            // no other way to learn which name collided, since the bundle
            // that carried it has already been decrypted server-side and
            // will be burned from the relay before any retry — so the name
            // is echoed back here rather than only asserting a generic
            // conflict, letting the GUI pre-fill a rename suggestion.
            return Err(format!("conflict: a project named '{name}' already exists"));
        }

        let mut tx = self.pool.begin().await.map_err(|e| e.to_string())?;
        let now = now_ts();

        let project_id = sqlx::query(
            "INSERT INTO projects (name, description, template, created, updated) VALUES (?1,?2,?3,?4,?5)",
        )
        .bind(name)
        .bind(description)
        .bind(template)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?
        .last_insert_rowid();

        // Items — ciphertext only, is_global = false (D7: provenance is one
        // project; the receiver opts in to global explicitly afterwards).
        let mut item_id_by_name: HashMap<String, i64> = HashMap::new();
        let mut item_ids = Vec::with_capacity(items.len());
        for it in items {
            let item_id = sqlx::query(
                "INSERT INTO items (item_type, data, created, updated, is_global) VALUES (?1,?2,?3,?4,0)",
            )
            .bind(&it.item_type)
            .bind(&it.ciphertext)
            .bind(&it.created)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?
            .last_insert_rowid();

            sqlx::query("INSERT INTO item_projects (item_id, project_id) VALUES (?1, ?2)")
                .bind(item_id)
                .bind(project_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;

            item_id_by_name.insert(it.name.clone(), item_id);
            item_ids.push(item_id);
        }

        // Environments + vars, resolving each var's item_name against the
        // items just inserted above.
        let mut environment_ids = Vec::with_capacity(environments.len());
        for env in environments {
            let is_default_i: i64 = if env.is_default { 1 } else { 0 };
            let env_id = sqlx::query(
                "INSERT INTO environments (project_id, name, is_default, created, updated) VALUES (?1,?2,?3,?4,?5)",
            )
            .bind(project_id)
            .bind(&env.name)
            .bind(is_default_i)
            .bind(&now)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?
            .last_insert_rowid();

            for v in &env.vars {
                // Defensive only — the bundle builder never emits a var
                // whose item_name isn't also in `items` (see
                // `project::relay::build_project_bundle`), so this should
                // never actually skip anything in practice.
                let item_id = match item_id_by_name.get(&v.item_name) {
                    Some(id) => *id,
                    None => continue,
                };
                sqlx::query(
                    "INSERT INTO environment_vars (environment_id, key, item_id, literal) VALUES (?1,?2,?3,NULL)",
                )
                .bind(env_id)
                .bind(&v.key)
                .bind(item_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
            }

            environment_ids.push(env_id);
        }

        tx.commit().await.map_err(|e| e.to_string())?;

        Ok(InsertedProject { project_id, environment_ids, item_ids })    }

    // ─── Backup / restore support (backup-restore-completeness) ───────────────

    pub fn path(&self) -> &str {
        &self.path
    }

    /// Opens a brand-new database at `path` (schema + pragmas), discarding any
    /// stale file or `-wal`/`-shm` leftovers from an earlier aborted restore.
    pub async fn create_at(path: &Path) -> Result<Self, String> {
        remove_db_files(path)?;
        let path_str = path.to_str().ok_or_else(|| "invalid database path".to_string())?;
        Self::open(path_str).await
    }

    /// Flushes the WAL into the main file and closes every pooled connection,
    /// so the file can be renamed (required on Windows) and carries all data.
    pub async fn close(&self) -> Result<(), String> {
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await
            .map_err(|e| format!("db checkpoint: {e}"))?;
        self.pool.close().await;
        Ok(())
    }

    /// Rewrites the database file and truncates the WAL, so a value that was
    /// just overwritten or deleted does not linger in free pages or old WAL
    /// frames.
    pub async fn compact(&self) -> Result<(), String> {
        sqlx::query("VACUUM")
            .execute(&self.pool)
            .await
            .map_err(|e| format!("db vacuum: {e}"))?;
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await
            .map_err(|e| format!("db checkpoint: {e}"))?;
        Ok(())
    }

    /// Writes a consistent copy of this database to `dest` (must not exist).
    pub async fn vacuum_into(&self, dest: &Path) -> Result<(), String> {
        let dest = dest.to_str().ok_or_else(|| "invalid database path".to_string())?;
        sqlx::query("VACUUM INTO ?1")
            .bind(dest)
            .execute(&self.pool)
            .await
            .map_err(|e| format!("db copy: {e}"))?;
        Ok(())
    }

    /// `PRAGMA integrity_check`; `Err` unless SQLite reports exactly `ok`.
    pub async fn integrity_check(&self) -> Result<(), String> {
        let rows: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| format!("integrity check: {e}"))?;
        if rows.len() == 1 && rows[0] == "ok" {
            Ok(())
        } else {
            Err("integrity check failed on the restored database".to_string())
        }
    }

    pub async fn list_settings(&self) -> Result<Vec<(String, String)>, String> {
        let rows = sqlx::query("SELECT key, value FROM settings")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(|r| (r.get(0), r.get(1))).collect())
    }

    /// Best-effort removal of the previous database kept by a restore.
    /// Called when the vault is unlocked.
    pub fn discard_pre_restore(&self) {
        let _ = remove_db_files(&pre_restore_path(Path::new(&self.path)));
    }

    pub async fn list_item_projects(&self) -> Result<Vec<(i64, i64)>, String> {
        let rows = sqlx::query("SELECT item_id, project_id FROM item_projects")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(|r| (r.get(0), r.get(1))).collect())
    }

    /// Raw (project_id, category_id) pairs. `list_project_categories` returns
    /// names; a backup needs the ids.
    pub async fn list_project_category_links(&self) -> Result<Vec<(i64, String)>, String> {
        let rows = sqlx::query("SELECT project_id, category_id FROM project_categories")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(|r| (r.get(0), r.get(1))).collect())
    }

    pub async fn list_holding_project_ids(&self) -> Result<Vec<i64>, String> {
        sqlx::query_scalar("SELECT id FROM projects WHERE is_holding = 1")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn insert_item_tx(
        tx: &mut Transaction<'_, Sqlite>,
        item_type: &str,
        data: &str,
        created: &str,
        is_global: bool,
    ) -> Result<i64, String> {
        let res = sqlx::query(
            "INSERT INTO items (item_type, data, created, updated, is_global) VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(item_type)
        .bind(data)
        .bind(created)
        .bind(now_ts())
        .bind(if is_global { 1i64 } else { 0i64 })
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
        Ok(res.last_insert_rowid())
    }

    /// Inserts a category unless one with the same id already exists.
    pub async fn insert_category_if_absent_tx(
        tx: &mut Transaction<'_, Sqlite>,
        cat: &DbCategory,
    ) -> Result<(), String> {
        sqlx::query(
            "INSERT OR IGNORE INTO categories (cid, name, color, description) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(&cat.cid)
        .bind(&cat.name)
        .bind(&cat.color)
        .bind(&cat.description)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Case-insensitive project lookup, matching `idx_projects_name_nocase`.
    pub async fn find_project_by_name_tx(
        tx: &mut Transaction<'_, Sqlite>,
        name: &str,
    ) -> Result<Option<i64>, String> {
        sqlx::query_scalar("SELECT id FROM projects WHERE name = ?1 COLLATE NOCASE")
            .bind(name)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn insert_project_tx(
        tx: &mut Transaction<'_, Sqlite>,
        name: &str,
        description: Option<&str>,
        template: &str,
        root_path: Option<&str>,
        is_holding: bool,
    ) -> Result<i64, String> {
        let now = now_ts();
        let res = sqlx::query(
            "INSERT INTO projects (name, description, template, created, updated, root_path, is_holding)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(name)
        .bind(description)
        .bind(template)
        .bind(&now)
        .bind(&now)
        .bind(root_path)
        .bind(if is_holding { 1i64 } else { 0i64 })
        .execute(&mut **tx)
        .await
        .map_err(|e| map_conflict(e, PROJECT_NAME_CONFLICT))?;
        Ok(res.last_insert_rowid())
    }

    /// Returns the id of the project named `name`, creating it (template
    /// `generic`, no root, flagged as a holding project) when missing. Used to
    /// give restored legacy items and imported items an owner.
    pub async fn ensure_holding_project_tx(
        tx: &mut Transaction<'_, Sqlite>,
        name: &str,
    ) -> Result<i64, String> {
        if let Some(id) = Self::find_project_by_name_tx(tx, name).await? {
            return Ok(id);
        }
        Self::insert_project_tx(tx, name, None, "generic", None, true).await
    }

    pub async fn add_project_category_tx(
        tx: &mut Transaction<'_, Sqlite>,
        project_id: i64,
        category_id: &str,
    ) -> Result<(), String> {
        sqlx::query("INSERT OR IGNORE INTO project_categories (project_id, category_id) VALUES (?1, ?2)")
            .bind(project_id)
            .bind(category_id)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Id of the environment named `name` in `project_id` (case-insensitive).
    pub async fn find_environment_tx(
        tx: &mut Transaction<'_, Sqlite>,
        project_id: i64,
        name: &str,
    ) -> Result<Option<i64>, String> {
        sqlx::query_scalar(
            "SELECT id FROM environments WHERE project_id = ?1 AND name = ?2 COLLATE NOCASE",
        )
        .bind(project_id)
        .bind(name)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| e.to_string())
    }

    pub async fn project_has_default_environment_tx(
        tx: &mut Transaction<'_, Sqlite>,
        project_id: i64,
    ) -> Result<bool, String> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM environments WHERE project_id = ?1 AND is_default = 1",
        )
        .bind(project_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
        Ok(n > 0)
    }

    /// Replaces the database with an empty one. The current files are renamed
    /// aside first and only overwritten and deleted once the fresh database
    /// is open, so a rename that fails (a scanner holding the file on
    /// Windows) leaves the current database open and usable. Anything that
    /// could not be deleted is retried by [`sweep_pending_wipes`] at startup.
    pub async fn wipe_and_reset(&mut self) -> Result<(), String> {
        let target = wipe_target_path(Path::new(&self.path));
        self.wipe_and_reset_to(&target).await
    }

    async fn wipe_and_reset_to(&mut self, target: &Path) -> Result<(), String> {
        let live = std::path::PathBuf::from(&self.path);

        // Checkpoint so the main file carries everything and the WAL is empty,
        // then close so the files can be renamed (required on Windows). If the
        // checkpoint fails the pool is still open.
        self.close().await?;

        if let Err(e) = rename_db_files(&live, target) {
            return match connect_pool(&live).await {
                Ok(pool) => {
                    self.pool = pool;
                    Err(e)
                }
                Err(e2) => Err(format!("{e}; reopening the vault also failed ({e2}); restart the app")),
            };
        }

        if let Err(e) = self.open_fresh(&live).await {
            // Put the old database back so the vault stays usable.
            self.pool.close().await;
            let _ = remove_db_files(&live);
            let _ = rename_db_files(target, &live);
            return match connect_pool(&live).await {
                Ok(pool) => {
                    self.pool = pool;
                    Err(e)
                }
                Err(e2) => Err(format!("{e}; reopening the vault also failed ({e2}); restart the app")),
            };
        }

        let target = target.to_path_buf();
        let _ = tokio::task::spawn_blocking(move || scrub_db_files(&target)).await;
        Ok(())
    }

    async fn open_fresh(&mut self, live: &Path) -> Result<(), String> {
        self.pool = connect_pool(live).await.map_err(|e| format!("db reopen: {e}"))?;
        self.init_schema().await
    }
}

/// Pool for the vault file with the shared per-connection options.
async fn connect_pool(path: &Path) -> Result<SqlitePool, sqlx::Error> {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(connect_options(path))
        .await
}

/// Where `wipe_and_reset` parks the old files: `<db>.wipe-<nanos>`.
fn wipe_target_path(db_path: &Path) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    sibling_with_suffix(db_path, &format!(".wipe-{stamp}"))
}

const WIPE_MARKER: &str = ".wipe-";

/// Renames a SQLite database and its `-wal`/`-shm` companions from `from` to
/// `to` (missing companions are skipped). Refuses to overwrite and undoes the
/// files already moved if one rename fails.
fn rename_db_files(from: &Path, to: &Path) -> Result<(), String> {
    let mut moved: Vec<&str> = Vec::new();
    for suffix in ["", "-wal", "-shm"] {
        let src = sibling_with_suffix(from, suffix);
        if !src.exists() {
            continue;
        }
        let dst = sibling_with_suffix(to, suffix);
        let result = if dst.exists() {
            Err(format!("{} already exists", dst.display()))
        } else {
            std::fs::rename(&src, &dst).map_err(|e| e.to_string())
        };
        if let Err(e) = result {
            for done in moved.iter().rev() {
                let _ = std::fs::rename(sibling_with_suffix(to, done), sibling_with_suffix(from, done));
            }
            return Err(format!("move {} aside: {e}", src.display()));
        }
        moved.push(suffix);
    }
    Ok(())
}

/// Overwrites `path` with zeros, flushes, and deletes it. The overwrite is
/// best effort (the delete is attempted regardless); a failed delete is the
/// error.
fn scrub_file(path: &Path) -> Result<(), String> {
    use std::io::Write;
    if let Ok(meta) = std::fs::metadata(path) {
        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(path) {
            let zeros = [0u8; 64 * 1024];
            let mut left = meta.len();
            while left > 0 {
                let n = left.min(zeros.len() as u64) as usize;
                if f.write_all(&zeros[..n]).is_err() {
                    break;
                }
                left -= n as u64;
            }
            let _ = f.sync_all();
        }
    }
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("remove {}: {e}", path.display())),
    }
}

/// Scrubs a database file and its `-wal`/`-shm` companions. Failures are
/// left for [`sweep_pending_wipes`].
fn scrub_db_files(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = scrub_file(&sibling_with_suffix(path, suffix));
    }
}

/// Retries the deletion of files a previous `wipe_and_reset` could not
/// remove (`<db>.wipe-*`). Called at startup; best effort.
pub fn sweep_pending_wipes(db_path: &Path) {
    let (Some(dir), Some(name)) = (db_path.parent(), db_path.file_name()) else {
        return;
    };
    let prefix = format!("{}{WIPE_MARKER}", name.to_string_lossy());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = scrub_file(&entry.path());
        }
    }
}

/// Startup recovery: renames an unopenable database (and its WAL/SHM files)
/// to `<db>.corrupt-<stamp>`, never deleting it. Returns the new main path.
pub fn move_db_aside(db_path: &Path, stamp: &str) -> Result<std::path::PathBuf, String> {
    let mut target = sibling_with_suffix(db_path, &format!(".corrupt-{stamp}"));
    let mut n = 1u32;
    while target.exists() {
        target = sibling_with_suffix(db_path, &format!(".corrupt-{stamp}-{n}"));
        n += 1;
    }
    rename_db_files(db_path, &target)?;
    Ok(target)
}

/// Duplicate-key validation shared by the environment write paths. Runs
/// before any statement so a rejected save never touches the database.
pub(crate) fn validate_unique_keys<'a>(keys: impl Iterator<Item = &'a str>) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for k in keys {
        if !seen.insert(k) {
            return Err(format!("duplicate variable key: {k}"));
        }
    }
    Ok(())
}

pub(crate) fn validate_unique_paths(paths: &[String]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for p in paths {
        if !seen.insert(p.as_str()) {
            return Err(format!("duplicate path: {p}"));
        }
    }
    Ok(())
}

fn now_ts() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

/// Current UTC time as an ISO-8601 string ("2006-01-02T15:04:05Z"), for the
/// `env_name_dedup_v1` rename report. Duplicated in miniature from
/// `vault::epoch_to_iso8601` rather than reused, to keep `db` from depending
/// on `vault` (see CLAUDE.md: `db` does not know about other modules).
fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let sec = secs % 60;
    let min = (secs / 60) % 60;
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
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simulates a pre-migration install: only the legacy `workspaces` /
    /// `workspace_vars` / `workspace_paths` tables exist, with real data.
    /// Opening `VaultDb` against this file must run the additive migration
    /// and backfill a 'default' environment per project without touching
    /// the legacy rows.
    #[tokio::test]
    async fn migrates_legacy_workspace_into_project_default_environment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.db");
        let path_str = path.to_str().unwrap().to_string();

        {
            let opts = SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new().max_connections(1).connect_with(opts).await.unwrap();

            sqlx::query(
                "CREATE TABLE workspaces (
                    id          INTEGER PRIMARY KEY AUTOINCREMENT,
                    name        TEXT NOT NULL,
                    description TEXT,
                    path        TEXT,
                    template    TEXT NOT NULL DEFAULT 'generic',
                    created     TEXT NOT NULL,
                    updated     TEXT NOT NULL
                )",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "CREATE TABLE workspace_vars (
                    id           INTEGER PRIMARY KEY AUTOINCREMENT,
                    workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                    key          TEXT NOT NULL,
                    item_id      INTEGER,
                    literal      TEXT,
                    UNIQUE(workspace_id, key)
                )",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "CREATE TABLE workspace_paths (
                    id           INTEGER PRIMARY KEY AUTOINCREMENT,
                    workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
                    path         TEXT NOT NULL,
                    UNIQUE(workspace_id, path)
                )",
            )
            .execute(&pool)
            .await
            .unwrap();

            sqlx::query(
                "INSERT INTO workspaces (id, name, description, template, created, updated)
                 VALUES (1, 'api-backend', 'legacy workspace', 'node', '1000', '1000')",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO workspace_vars (workspace_id, key, item_id, literal) VALUES
                 (1, 'DB_HOST', 42, NULL),
                 (1, 'DB_PASSWORD', NULL, 'literal-secret')",
            )
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO workspace_paths (workspace_id, path) VALUES
                 (1, '/srv/api-backend/.env'),
                 (1, '/srv/api-backend/.env.local')",
            )
            .execute(&pool)
            .await
            .unwrap();

            pool.close().await;
        }

        let db = VaultDb::open(&path_str).await.expect("open should run the migration");

        let projects = db.list_projects().await.unwrap();
        assert_eq!(projects.len(), 1, "one project should be backfilled from the legacy workspace");
        let project = &projects[0];
        assert_eq!(project.id, 1);
        assert_eq!(project.name, "api-backend");
        assert_eq!(project.description.as_deref(), Some("legacy workspace"));
        assert_eq!(project.template, "node");

        let environments = db.list_environments(project.id).await.unwrap();
        assert_eq!(environments.len(), 1, "exactly one 'default' environment per migrated project");
        let env = &environments[0];
        assert_eq!(env.name, "default");
        assert!(env.is_default);

        let mut paths = env.paths.clone();
        paths.sort();
        assert_eq!(paths, vec!["/srv/api-backend/.env", "/srv/api-backend/.env.local"]);

        let mut vars = db.get_environment_vars(env.id).await.unwrap();
        vars.sort_by(|a, b| a.key.cmp(&b.key));
        assert_eq!(vars.len(), 2);
        assert_eq!(vars[0].key, "DB_HOST");
        assert_eq!(vars[0].item_id, Some(42));
        assert_eq!(vars[1].key, "DB_PASSWORD");
        assert_eq!(vars[1].literal.as_deref(), Some("literal-secret"));

        // Legacy rows must survive untouched — the relay "share whole workspace"
        // feature still reads them directly until it moves to ProjectBundle.
        let legacy = db.list_workspaces().await.unwrap();
        assert_eq!(legacy.len(), 1);
        assert_eq!(legacy[0].name, "api-backend");

        // Re-opening (idempotent init_schema) must not create duplicate projects/environments.
        drop(db);
        let db2 = VaultDb::open(&path_str).await.unwrap();
        assert_eq!(db2.list_projects().await.unwrap().len(), 1);
        assert_eq!(db2.list_environments(1).await.unwrap().len(), 1);
    }

    // ─── upsert_environment_var (R2: ON CONFLICT repoint, not insert) ─────

    async fn fresh_env() -> (tempfile::TempDir, VaultDb, i64) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        let db = VaultDb::open(path.to_str().unwrap()).await.unwrap();
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let env_id = db.upsert_environment(0, project_id, "production", true).await.unwrap();
        (dir, db, env_id)
    }

    #[tokio::test]
    async fn upsert_environment_var_first_insert_creates_a_row() {
        let (_dir, db, env_id) = fresh_env().await;
        let row_id = db.upsert_environment_var(env_id, "DB_HOST", 42).await.unwrap();

        let vars = db.get_environment_vars(env_id).await.unwrap();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].id, row_id);
        assert_eq!(vars[0].item_id, Some(42));
    }

    #[tokio::test]
    async fn upsert_environment_var_same_key_twice_repoints_not_duplicates() {
        let (_dir, db, env_id) = fresh_env().await;
        let first_id = db.upsert_environment_var(env_id, "DB_HOST", 42).await.unwrap();
        let second_id = db.upsert_environment_var(env_id, "DB_HOST", 99).await.unwrap();

        assert_eq!(first_id, second_id, "same key must repoint the same row, not insert a new one");
        let vars = db.get_environment_vars(env_id).await.unwrap();
        assert_eq!(vars.len(), 1, "ON CONFLICT must not leave two rows for the same (environment_id, key)");
        assert_eq!(vars[0].item_id, Some(99));
    }

    #[tokio::test]
    async fn upsert_environment_var_two_different_keys_yield_two_rows() {
        let (_dir, db, env_id) = fresh_env().await;
        db.upsert_environment_var(env_id, "DB_HOST", 1).await.unwrap();
        db.upsert_environment_var(env_id, "DB_PASSWORD", 2).await.unwrap();

        let vars = db.get_environment_vars(env_id).await.unwrap();
        assert_eq!(vars.len(), 2);
    }

    #[tokio::test]
    async fn upsert_environment_var_same_key_in_different_environments_are_independent() {
        let (dir, db, env_a) = fresh_env().await;
        let _ = &dir;
        let project_id = db.upsert_project(0, "other", None, "generic").await.unwrap();
        let env_b = db.upsert_environment(0, project_id, "staging", true).await.unwrap();

        db.upsert_environment_var(env_a, "SHARED_KEY", 1).await.unwrap();
        db.upsert_environment_var(env_b, "SHARED_KEY", 2).await.unwrap();

        let vars_a = db.get_environment_vars(env_a).await.unwrap();
        let vars_b = db.get_environment_vars(env_b).await.unwrap();
        assert_eq!(vars_a.len(), 1);
        assert_eq!(vars_b.len(), 1);
        assert_eq!(vars_a[0].item_id, Some(1));
        assert_eq!(vars_b[0].item_id, Some(2));
    }

    #[tokio::test]
    async fn upsert_environment_var_updating_one_key_leaves_others_untouched() {
        let (_dir, db, env_id) = fresh_env().await;
        db.upsert_environment_var(env_id, "DB_HOST", 1).await.unwrap();
        db.upsert_environment_var(env_id, "DB_PASSWORD", 2).await.unwrap();

        db.upsert_environment_var(env_id, "DB_HOST", 100).await.unwrap();

        let mut vars = db.get_environment_vars(env_id).await.unwrap();
        vars.sort_by(|a, b| a.key.cmp(&b.key));
        assert_eq!(vars.len(), 2);
        assert_eq!(vars[0].key, "DB_HOST");
        assert_eq!(vars[0].item_id, Some(100));
        assert_eq!(vars[1].key, "DB_PASSWORD");
        assert_eq!(vars[1].item_id, Some(2), "unrelated key must be untouched by repointing DB_HOST");
    }

    #[tokio::test]
    async fn upsert_environment_var_returned_id_matches_subsequent_lookup() {
        let (_dir, db, env_id) = fresh_env().await;
        let row_id = db.upsert_environment_var(env_id, "DB_HOST", 1).await.unwrap();

        let vars = db.get_environment_vars(env_id).await.unwrap();
        let found = vars.iter().find(|v| v.key == "DB_HOST").unwrap();
        assert_eq!(found.id, row_id);
    }

    // ─── vault-db-transactional-integrity ─────────────────────────────

    async fn open_test_db() -> (tempfile::TempDir, VaultDb) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        let db = VaultDb::open(path.to_str().unwrap()).await.unwrap();
        (dir, db)
    }

    fn cat(cid: &str, name: &str) -> DbCategory {
        DbCategory { cid: cid.to_string(), name: name.to_string(), color: "#fff".to_string(), description: None }
    }

    #[tokio::test]
    async fn every_pooled_connection_has_secure_delete_and_foreign_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pragmas.db");
        let pool = SqlitePoolOptions::new()
            .max_connections(3)
            .connect_with(connect_options(&path))
            .await
            .unwrap();

        // Hold all three at once so the pool is forced to open distinct connections.
        let mut c1 = pool.acquire().await.unwrap();
        let mut c2 = pool.acquire().await.unwrap();
        let mut c3 = pool.acquire().await.unwrap();
        for conn in [&mut c1, &mut c2, &mut c3] {
            let secure: i64 = sqlx::query_scalar("PRAGMA secure_delete").fetch_one(&mut **conn).await.unwrap();
            let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys").fetch_one(&mut **conn).await.unwrap();
            assert_eq!(secure, 1, "secure_delete must be ON on every connection");
            assert_eq!(fk, 1, "foreign_keys must be ON on every connection");
        }
    }

    #[tokio::test]
    async fn save_categories_rename_preserves_project_links() {
        let (_dir, db) = open_test_db().await;
        db.save_categories(&[cat("a", "Backend"), cat("b", "Frontend")]).await.unwrap();
        let p = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        db.set_project_categories(p, &["a".to_string(), "b".to_string()]).await.unwrap();

        db.save_categories(&[cat("a", "API"), cat("b", "Frontend")]).await.unwrap();

        let mut names = db.list_project_categories(p).await.unwrap();
        names.sort();
        assert_eq!(names, vec!["API".to_string(), "Frontend".to_string()]);
    }

    #[tokio::test]
    async fn save_categories_delete_removes_only_that_categorys_links() {
        let (_dir, db) = open_test_db().await;
        db.save_categories(&[cat("a", "Backend"), cat("b", "Frontend")]).await.unwrap();
        let p = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        db.set_project_categories(p, &["a".to_string(), "b".to_string()]).await.unwrap();

        db.save_categories(&[cat("b", "Frontend")]).await.unwrap();

        assert_eq!(db.list_project_categories(p).await.unwrap(), vec!["Frontend".to_string()]);
        assert_eq!(db.list_categories().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn save_categories_failure_changes_nothing() {
        let (_dir, db) = open_test_db().await;
        db.save_categories(&[cat("a", "Backend"), cat("b", "Frontend")]).await.unwrap();
        let p = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        db.set_project_categories(p, &["a".to_string(), "b".to_string()]).await.unwrap();

        sqlx::query(
            "CREATE TRIGGER boom BEFORE INSERT ON categories WHEN NEW.name = 'boom'
             BEGIN SELECT RAISE(ABORT, 'injected'); END",
        )
        .execute(&db.pool)
        .await
        .unwrap();

        // Drops `a` (delete step) and then fails on the insert of `c`.
        let err = db.save_categories(&[cat("b", "Frontend"), cat("c", "boom")]).await;
        assert!(err.is_err());

        let cats = db.list_categories().await.unwrap();
        assert_eq!(cats.len(), 2, "rolled-back save must restore the deleted category");
        let mut names = db.list_project_categories(p).await.unwrap();
        names.sort();
        assert_eq!(names, vec!["Backend".to_string(), "Frontend".to_string()]);
    }

    #[tokio::test]
    async fn set_environment_vars_rejects_duplicate_keys_before_writing() {
        let (_dir, db) = open_test_db().await;
        let p = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let env = db.upsert_environment(0, p, "production", true).await.unwrap();
        let var = |key: &str| DbEnvironmentVar {
            id: 0, environment_id: env, key: key.to_string(), item_id: None, literal: Some("x".to_string()),
        };
        db.set_environment_vars(env, &[var("KEEP")]).await.unwrap();

        let err = db.set_environment_vars(env, &[var("API_KEY"), var("API_KEY")]).await.unwrap_err();
        assert!(err.contains("API_KEY"));
        let vars = db.get_environment_vars(env).await.unwrap();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].key, "KEEP");
    }

    fn wipe_leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(WIPE_MARKER))
            .collect()
    }

    #[tokio::test]
    async fn wipe_replaces_the_database_and_leaves_no_wipe_files() {
        let (dir, mut db) = open_test_db().await;
        db.set_setting("k", "v").await.unwrap();
        db.wipe_and_reset().await.unwrap();
        assert_eq!(db.get_setting("k").await.unwrap(), None);
        db.set_setting("after", "ok").await.unwrap();
        sweep_pending_wipes(Path::new(db.path()));
        assert!(wipe_leftovers(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn failed_rename_keeps_the_database_usable() {
        let (dir, mut db) = open_test_db().await;
        db.set_setting("k", "v").await.unwrap();
        // The rename target is occupied, so the old files cannot be moved aside.
        let blocked = dir.path().join("occupied.wipe-1");
        std::fs::write(&blocked, b"x").unwrap();
        assert!(db.wipe_and_reset_to(&blocked).await.is_err());

        assert_eq!(db.get_setting("k").await.unwrap().as_deref(), Some("v"));
        db.set_setting("k2", "v2").await.unwrap();
        // The next launch can open the same file.
        let again = VaultDb::open(db.path()).await.unwrap();
        assert_eq!(again.get_setting("k2").await.unwrap().as_deref(), Some("v2"));
    }

    #[tokio::test]
    async fn sweep_removes_leftover_wipe_files_only() {
        let (dir, db) = open_test_db().await;
        let live = Path::new(db.path());
        let leftover = sibling_with_suffix(live, ".wipe-42");
        let leftover_wal = sibling_with_suffix(live, ".wipe-42-wal");
        std::fs::write(&leftover, vec![7u8; 100_000]).unwrap();
        std::fs::write(&leftover_wal, b"wal").unwrap();
        let unrelated = dir.path().join("vault.db.pre-restore");
        std::fs::write(&unrelated, b"keep").unwrap();

        sweep_pending_wipes(live);

        assert!(wipe_leftovers(dir.path()).is_empty());
        assert!(unrelated.exists());
        assert!(live.exists());
    }

    #[test]
    fn move_aside_renames_with_timestamp_and_never_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("vault.db");
        std::fs::write(&live, b"corrupt").unwrap();
        std::fs::write(dir.path().join("vault.db-wal"), b"wal").unwrap();

        let moved = move_db_aside(&live, "1700000000").unwrap();
        assert_eq!(moved, dir.path().join("vault.db.corrupt-1700000000"));
        assert_eq!(std::fs::read(&moved).unwrap(), b"corrupt");
        assert!(dir.path().join("vault.db.corrupt-1700000000-wal").exists());
        assert!(!live.exists());

        // A second move in the same second does not overwrite the first.
        std::fs::write(&live, b"corrupt2").unwrap();
        let moved2 = move_db_aside(&live, "1700000000").unwrap();
        assert_ne!(moved, moved2);
        assert_eq!(std::fs::read(&moved).unwrap(), b"corrupt");
    }

    #[tokio::test]
    async fn set_environment_paths_rejects_duplicate_paths_before_writing() {
        let (_dir, db) = open_test_db().await;
        let p = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let env = db.upsert_environment(0, p, "production", true).await.unwrap();
        db.set_environment_paths(env, &["/a".to_string()]).await.unwrap();

        let err = db.set_environment_paths(env, &["/b".to_string(), "/b".to_string()]).await;
        assert!(err.is_err());
        assert_eq!(db.get_environment_paths(env).await.unwrap(), vec!["/a".to_string()]);
    }
}

/// Path of the previous database retained by a restore (`vault.db.pre-restore`).
pub fn pre_restore_path(db_path: &Path) -> std::path::PathBuf {
    sibling_with_suffix(db_path, ".pre-restore")
}

/// `<db_path><suffix>` in the same directory (same filesystem, so renames
/// between the two are atomic).
pub fn sibling_with_suffix(db_path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut name = db_path.as_os_str().to_owned();
    name.push(suffix);
    std::path::PathBuf::from(name)
}

/// Removes a SQLite database file together with its `-wal` and `-shm`
/// companions. Missing files are not an error.
pub fn remove_db_files(path: &Path) -> Result<(), String> {
    for suffix in ["", "-wal", "-shm"] {
        let p = sibling_with_suffix(path, suffix);
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("remove {}: {e}", p.display())),
        }
    }
    Ok(())
}
