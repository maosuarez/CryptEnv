// Integration tests for issue #12 (case-insensitive uniqueness for
// `environments.name`, retrofitted the same for `projects.name`, and the
// ambiguous-scope rejection in `project::resolve_environment`).
//
// Same pattern as `vault_integration.rs`: `#[tokio::test]` + `tempfile::tempdir()`
// + `VaultDb::open`. Cases T1-T8 and T11 from the plan live here (T9/T10 are
// HTTP-level and belong to issue #11's harness).
//
// A few tests (T3-T7, and the projects-side collision test) need to seed a DB
// that *already* contains a case-colliding name pair — impossible through the
// public API once the NOCASE unique indexes exist. They do this with a raw
// sqlx connection (hence `sqlx` in `[dev-dependencies]`) against a schema
// subset mirroring `db::VaultDb::init_schema`, written *before* `VaultDb`
// ever opens the file. Because the NOCASE indexes are created imperatively
// (after the dedup pass) rather than declared on the tables themselves, a
// fresh raw-seeded file with colliding names has no index yet to violate —
// `VaultDb::open`'s dedup-then-index sequence is exactly what's under test.

use crypt_env_lib::db::VaultDb;
use crypt_env_lib::project::{self, EnvironmentInput, EnvironmentVar, ProjectInput};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};
use tempfile::tempdir;

/// Minimal schema subset needed to seed pre-collision rows, mirroring the
/// relevant `CREATE TABLE` statements in `db::VaultDb::init_schema`. Deliberately
/// omits the `idx_*_name_nocase` indexes — those are what `VaultDb::open` must
/// create (after deduping) when it opens this file for the first time.
const MINIMAL_SCHEMA: &[&str] = &[
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
    "CREATE TABLE IF NOT EXISTS items (
        id        INTEGER PRIMARY KEY AUTOINCREMENT,
        item_type TEXT NOT NULL,
        data      TEXT NOT NULL,
        created   TEXT NOT NULL,
        updated   TEXT NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS item_projects (
        item_id    INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
        project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        PRIMARY KEY (item_id, project_id)
    )",
];

async fn raw_pool(path: &std::path::Path) -> SqlitePool {
    let opts = SqliteConnectOptions::new().filename(path).create_if_missing(true);
    SqlitePoolOptions::new().max_connections(1).connect_with(opts).await.unwrap()
}

async fn seed_minimal_schema(pool: &SqlitePool) {
    for stmt in MINIMAL_SCHEMA {
        sqlx::query(stmt).execute(pool).await.unwrap();
    }
}

async fn seed_project(pool: &SqlitePool, id: i64, name: &str) {
    sqlx::query(
        "INSERT INTO projects (id, name, description, template, created, updated)
         VALUES (?1, ?2, NULL, 'generic', '1000', '1000')",
    )
    .bind(id)
    .bind(name)
    .execute(pool)
    .await
    .unwrap();
}

async fn seed_environment(pool: &SqlitePool, id: i64, project_id: i64, name: &str) {
    sqlx::query(
        "INSERT INTO environments (id, project_id, name, is_default, created, updated)
         VALUES (?1, ?2, ?3, 0, '1000', '1000')",
    )
    .bind(id)
    .bind(project_id)
    .bind(name)
    .execute(pool)
    .await
    .unwrap();
}

fn db_path(dir: &tempfile::TempDir, name: &str) -> (std::path::PathBuf, String) {
    let path = dir.path().join(name);
    let path_str = path.to_str().unwrap().to_string();
    (path, path_str)
}

// T1: fresh DB, upsert "production" then "Production" in the same project.
#[tokio::test]
async fn t1_case_insensitive_collision_rejected_on_insert() {
    let dir = tempdir().unwrap();
    let (_path, path_str) = db_path(&dir, "t1.db");
    let db = VaultDb::open(&path_str).await.unwrap();

    let project_id = db.upsert_project(0, "acme", None, "generic").await.unwrap();
    db.upsert_environment(0, project_id, "production", false).await.unwrap();
    let err = db.upsert_environment(0, project_id, "Production", false).await.unwrap_err();

    assert!(err.starts_with("conflict:"), "expected conflict sentinel, got: {err}");
}

// T2: "production" in project A and project B — proves project_id is in the index.
#[tokio::test]
async fn t2_same_name_different_projects_allowed() {
    let dir = tempdir().unwrap();
    let (_path, path_str) = db_path(&dir, "t2.db");
    let db = VaultDb::open(&path_str).await.unwrap();

    let project_a = db.upsert_project(0, "project-a", None, "generic").await.unwrap();
    let project_b = db.upsert_project(0, "project-b", None, "generic").await.unwrap();

    db.upsert_environment(0, project_a, "production", false).await.unwrap();
    db.upsert_environment(0, project_b, "production", false).await.unwrap();

    assert_eq!(db.list_environments(project_a).await.unwrap().len(), 1);
    assert_eq!(db.list_environments(project_b).await.unwrap().len(), 1);
}

// T3: seed a pre-existing colliding pair, then open with VaultDb — must not
// brick, and must deterministically rename the loser (lowest id keeps its name).
#[tokio::test]
async fn t3_legacy_collision_deduped_on_open() {
    let dir = tempdir().unwrap();
    let (path, path_str) = db_path(&dir, "t3.db");

    {
        let pool = raw_pool(&path).await;
        seed_minimal_schema(&pool).await;
        seed_project(&pool, 1, "acme").await;
        seed_environment(&pool, 1, 1, "production").await;
        seed_environment(&pool, 2, 1, "Production").await;
        pool.close().await;
    }

    let db = VaultDb::open(&path_str).await.expect("must not brick on a legacy case collision");

    let envs = db.list_environments(1).await.unwrap();
    let by_id: HashMap<i64, String> = envs.into_iter().map(|e| (e.id, e.name)).collect();
    assert_eq!(by_id.get(&1).unwrap(), "production", "lowest id keeps its name unchanged");
    assert_eq!(by_id.get(&2).unwrap(), "production-2", "loser gets a -2 suffix");
}

// T4: vars/paths/ownership on both colliding rows survive the rename untouched —
// only the `name` column changes, `id` is stable, nothing moves or is dropped.
#[tokio::test]
async fn t4_dedup_preserves_vars_paths_and_ownership() {
    let dir = tempdir().unwrap();
    let (path, path_str) = db_path(&dir, "t4.db");

    {
        let pool = raw_pool(&path).await;
        seed_minimal_schema(&pool).await;
        seed_project(&pool, 1, "acme").await;
        seed_environment(&pool, 1, 1, "production").await;
        seed_environment(&pool, 2, 1, "Production").await;

        sqlx::query(
            "INSERT INTO items (id, item_type, data, created, updated) VALUES
             (1, 'secret', 'enc1', '1000', '1000'),
             (2, 'secret', 'enc2', '1000', '1000')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO item_projects (item_id, project_id) VALUES (1, 1), (2, 1)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO environment_vars (environment_id, key, item_id) VALUES (1, 'DB_HOST', 1)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO environment_vars (environment_id, key, item_id) VALUES (2, 'DB_HOST', 2)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO environment_paths (environment_id, path) VALUES (1, '/srv/.env')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO environment_paths (environment_id, path) VALUES (2, '/srv/.env.local')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    let db = VaultDb::open(&path_str).await.unwrap();

    let vars1 = db.get_environment_vars(1).await.unwrap();
    let vars2 = db.get_environment_vars(2).await.unwrap();
    assert_eq!(vars1.len(), 1);
    assert_eq!(vars2.len(), 1);
    assert_eq!(vars1[0].item_id, Some(1));
    assert_eq!(vars2[0].item_id, Some(2));

    assert_eq!(db.get_environment_paths(1).await.unwrap(), vec!["/srv/.env".to_string()]);
    assert_eq!(db.get_environment_paths(2).await.unwrap(), vec!["/srv/.env.local".to_string()]);

    assert_eq!(db.list_owning_projects(1).await.unwrap(), vec![1]);
    assert_eq!(db.list_owning_projects(2).await.unwrap(), vec![1]);

    let envs = db.list_environments(1).await.unwrap();
    let names: HashSet<String> = envs.into_iter().map(|e| e.name).collect();
    assert!(names.contains("production"));
    assert!(names.contains("production-2"));
}

// T5: reopening an already-deduped vault is a no-op — no further renames, and
// the persisted report is not duplicated.
#[tokio::test]
async fn t5_reopen_is_idempotent_and_report_not_duplicated() {
    let dir = tempdir().unwrap();
    let (path, path_str) = db_path(&dir, "t5.db");

    {
        let pool = raw_pool(&path).await;
        seed_minimal_schema(&pool).await;
        seed_project(&pool, 1, "acme").await;
        seed_environment(&pool, 1, 1, "production").await;
        seed_environment(&pool, 2, 1, "Production").await;
        pool.close().await;
    }

    let db = VaultDb::open(&path_str).await.unwrap();
    let report_json = db.get_setting("env_name_dedup_v1").await.unwrap().expect("report must be persisted");
    let report: serde_json::Value = serde_json::from_str(&report_json).unwrap();
    assert_eq!(report.as_array().unwrap().len(), 1, "exactly one rename recorded");
    drop(db);

    let db2 = VaultDb::open(&path_str).await.unwrap();
    let envs = db2.list_environments(1).await.unwrap();
    let names: HashSet<String> = envs.into_iter().map(|e| e.name).collect();
    assert!(names.contains("production"));
    assert!(names.contains("production-2"));

    let report_json2 = db2.get_setting("env_name_dedup_v1").await.unwrap().unwrap();
    let report2: serde_json::Value = serde_json::from_str(&report_json2).unwrap();
    assert_eq!(report2.as_array().unwrap().len(), 1, "no duplicate entries on reopen");
}

// T6: three-way collision gets sequential suffixes off the winner's name.
#[tokio::test]
async fn t6_three_way_collision_gets_sequential_suffixes() {
    let dir = tempdir().unwrap();
    let (path, path_str) = db_path(&dir, "t6.db");

    {
        let pool = raw_pool(&path).await;
        seed_minimal_schema(&pool).await;
        seed_project(&pool, 1, "acme").await;
        seed_environment(&pool, 1, 1, "prod").await;
        seed_environment(&pool, 2, 1, "Prod").await;
        seed_environment(&pool, 3, 1, "PROD").await;
        pool.close().await;
    }

    let db = VaultDb::open(&path_str).await.unwrap();
    let mut envs = db.list_environments(1).await.unwrap();
    envs.sort_by_key(|e| e.id);
    assert_eq!(envs[0].name, "prod");
    assert_eq!(envs[1].name, "prod-2");
    assert_eq!(envs[2].name, "prod-3");
}

// T7: a pre-existing "prod-2" is skipped over (not touched, not collided into)
// when the suffix search runs for the actual loser.
#[tokio::test]
async fn t7_existing_suffixed_name_is_skipped_over() {
    let dir = tempdir().unwrap();
    let (path, path_str) = db_path(&dir, "t7.db");

    {
        let pool = raw_pool(&path).await;
        seed_minimal_schema(&pool).await;
        seed_project(&pool, 1, "acme").await;
        seed_environment(&pool, 1, 1, "prod").await;
        seed_environment(&pool, 2, 1, "Prod").await;
        seed_environment(&pool, 3, 1, "prod-2").await;
        pool.close().await;
    }

    let db = VaultDb::open(&path_str).await.unwrap();
    let mut envs = db.list_environments(1).await.unwrap();
    envs.sort_by_key(|e| e.id);
    assert_eq!(envs[0].name, "prod");
    assert_eq!(envs[1].name, "prod-3", "loser skips the taken prod-2 and lands on prod-3");
    assert_eq!(envs[2].name, "prod-2", "pre-existing prod-2 must be untouched");
}

// T8: non-ASCII case collisions survive the NOCASE index (SQLite NOCASE folds
// ASCII only) but are rejected by resolve_environment's Unicode-aware ambiguity check.
#[tokio::test]
async fn t8_non_ascii_collision_survives_index_but_rejected_by_resolver() {
    let dir = tempdir().unwrap();
    let (_path, path_str) = db_path(&dir, "t8.db");
    let db = VaultDb::open(&path_str).await.unwrap();

    let project_id = db.upsert_project(0, "acme", None, "generic").await.unwrap();
    db.upsert_environment(0, project_id, "producci\u{f3}n", false).await.unwrap();
    db.upsert_environment(0, project_id, "PRODUCCI\u{d3}N", false).await.unwrap();

    let envs = db.list_environments(project_id).await.unwrap();
    assert_eq!(envs.len(), 2, "non-ASCII case variants are not caught by the NOCASE index");

    // `Environment` doesn't derive `Debug`, so `.unwrap_err()` (which requires
    // `T: Debug`) can't be used here — match instead.
    let err = match project::resolve_environment(&db, None, Some("acme"), Some("PRODUCCI\u{d3}N")).await {
        Err(e) => e,
        Ok(_) => panic!("expected an ambiguous match error"),
    };
    assert!(err.contains(project::AMBIGUOUS_MATCH_PREFIX), "got: {err}");
    for env in &envs {
        assert!(err.contains(&env.id.to_string()), "error must name candidate id {}: {err}", env.id);
    }
}

// Regression: dedup's candidate-search must fold exactly like SQLite's own
// LOWER() (ASCII-only), not Rust's `to_lowercase()` (full Unicode). A prior
// version bound a Rust-folded candidate against `LOWER(name)` (SQLite-folded)
// — for a non-ASCII base, the two folds disagree, so a real collision against
// a pre-existing suffixed sibling was missed, the "free" candidate was
// accepted, and the following UPDATE then hit the table's exact-match unique
// constraint: `init_schema` returned `Err` and the vault could not be opened
// at all. This seeds exactly that shape (two colliding non-ASCII names plus a
// pre-existing `-2`) and asserts `VaultDb::open` succeeds.
#[tokio::test]
async fn non_ascii_collision_with_pre_existing_suffix_does_not_brick_open() {
    let dir = tempdir().unwrap();
    let (path, path_str) = db_path(&dir, "non_ascii_suffix.db");

    {
        let pool = raw_pool(&path).await;
        seed_minimal_schema(&pool).await;
        seed_project(&pool, 1, "acme").await;
        seed_environment(&pool, 1, 1, "PRODUCCI\u{d3}N").await;
        seed_environment(&pool, 2, 1, "prODUCCI\u{d3}N").await;
        seed_environment(&pool, 3, 1, "producci\u{d3}n-2").await;
        pool.close().await;
    }

    let db = VaultDb::open(&path_str)
        .await
        .expect("must not brick on a non-ASCII collision with a pre-existing suffixed sibling");

    let envs = db.list_environments(1).await.unwrap();
    let by_id: HashMap<i64, String> = envs.into_iter().map(|e| (e.id, e.name)).collect();
    assert_eq!(by_id.get(&1).unwrap(), "PRODUCCI\u{d3}N", "lowest id keeps its name unchanged");
    assert_eq!(
        by_id.get(&2).unwrap(),
        "producci\u{d3}n-3",
        "loser must skip the taken -2 suffix (correctly detected via SQLite's own LOWER(), not Rust's to_lowercase())"
    );
    assert_eq!(by_id.get(&3).unwrap(), "producci\u{d3}n-2", "pre-existing suffixed sibling must be untouched");
}

// T11: save_project auto-creates "default"; saving a sibling environment named
// "Default" must be rejected by the app-level pre-check with the conflict sentinel.
#[tokio::test]
async fn t11_save_project_default_env_collision_rejected() {
    let dir = tempdir().unwrap();
    let (_path, path_str) = db_path(&dir, "t11.db");
    let db = VaultDb::open(&path_str).await.unwrap();

    let project_id = project::save_project(
        &db,
        ProjectInput { id: 0, name: "acme".to_string(), description: None, template: "generic".to_string(), categories: vec![], initial_environment: None , root_path: None, seed_baseline: false },
    )
    .await
    .unwrap();

    let err = project::save_environment(
        &db,
        EnvironmentInput {
            id: 0,
            project_id,
            name: "Default".to_string(),
            is_default: false,
            paths: vec![],
            vars: Vec::<EnvironmentVar>::new(),
        },
    )
    .await
    .unwrap_err();

    assert!(err.starts_with("conflict:"), "got: {err}");
}

// Projects-side retrofit (step 0): a legacy install already holding `MyApp` +
// `myapp` must still open (previously this would fail `init_schema` -> `Err`
// -> `VaultDb::open` -> `Err`, bricking the vault), with the loser renamed.
#[tokio::test]
async fn projects_legacy_collision_deduped_on_open() {
    let dir = tempdir().unwrap();
    let (path, path_str) = db_path(&dir, "projects_dup.db");

    {
        let pool = raw_pool(&path).await;
        seed_minimal_schema(&pool).await;
        seed_project(&pool, 1, "MyApp").await;
        seed_project(&pool, 2, "myapp").await;
        pool.close().await;
    }

    let db = VaultDb::open(&path_str).await.expect("must not brick on a legacy project name collision");

    let projects = db.list_projects().await.unwrap();
    let by_id: HashMap<i64, String> = projects.into_iter().map(|p| (p.id, p.name)).collect();
    assert_eq!(by_id.get(&1).unwrap(), "MyApp", "lowest id keeps its name unchanged");
    assert_eq!(by_id.get(&2).unwrap(), "myapp-2", "loser gets a -2 suffix");
}
