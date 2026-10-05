// Integration tests for VaultDb using a real SQLite database in a temp directory.
// Run with: cargo test --test vault_integration

use crypt_env_lib::crypto::VaultKey;
use crypt_env_lib::db::{LinkMode, VaultDb};
use crypt_env_lib::project::{self, EnvironmentInput, EnvironmentVar, ProjectInput};
use crypt_env_lib::vault::{self, VaultItem};
use tempfile::tempdir;

/// Shared setup for the `inject_environment` read-guard tests below: a fresh
/// db, one project with its auto-created default environment, and one vault
/// item wired into that environment as `DB_PASSWORD`. Returns
/// `(db, vault_key, environment_id)`.
async fn setup_project_with_one_var(db: &VaultDb) -> (VaultKey, i64) {
    let key = VaultKey::from_slice(&[0x11u8; 32]).unwrap();

    let project_id = project::save_project(db, ProjectInput {
        id: 0,
        name: "Demo".into(),
        description: None,
        template: "generic".into(),
        categories: vec![],
        initial_environment: None,
        root_path: None,
        seed_baseline: false,
    }).await.unwrap();

    let projects = project::list_projects(db).await.unwrap();
    let default_env_id = projects
        .iter()
        .find(|p| p.id == project_id)
        .unwrap()
        .environments
        .iter()
        .find(|e| e.is_default)
        .unwrap()
        .id;

    let item = VaultItem {
        id: 0,
        item_type: "secret".into(),
        name: Some("DB_PASSWORD".into()),
        value: Some("hunter2".into()),
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
        created: "2026-01-01T00:00:00Z".into(),
        is_global: None,
    };
    let item_id = vault::create_project_item(db, &key, &item, project_id).await.unwrap();

    project::save_environment(db, EnvironmentInput {
        id: default_env_id,
        project_id,
        name: "default".into(),
        is_default: true,
        paths: vec![],
        vars: vec![EnvironmentVar { id: 0, key: "DB_PASSWORD".into(), item_id }],
    }).await.unwrap();

    (key, default_env_id)
}

// Issue #3, plan §3.6: `inject_environment`'s read side must distinguish
// `ErrorKind::NotFound` (legitimate: create a new file) from every other
// error kind, which must abort the write rather than proceed as if the
// target were empty. This is the gate the WSL bridge plan declared but
// deferred to #8 — landed here as the minimal `match e.kind()` guard.
#[tokio::test]
async fn test_inject_environment_aborts_on_non_not_found_read_error() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();
    let (key, env_id) = setup_project_with_one_var(&db).await;

    // A directory, not a file: std::fs::read_to_string fails with a
    // non-NotFound error kind. Before the guard, `unwrap_or_default()`
    // swallowed this and silently overwrote the "empty" content with only
    // this environment's keys — exactly the data-loss path the gate closes.
    let existing_dir = dir.path().join("not_a_file");
    std::fs::create_dir(&existing_dir).unwrap();
    let bad_path = existing_dir.to_str().unwrap().to_string();

    let result = project::inject_environment(&db, &key, env_id, Some(bad_path), None, false, None).await;
    assert!(
        result.is_err(),
        "a non-NotFound read error must abort the write, not be treated as an empty file"
    );
}

#[tokio::test]
async fn test_inject_environment_creates_new_file_on_not_found() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();
    let (key, env_id) = setup_project_with_one_var(&db).await;

    // Parent directory exists, target file does not — read must fail with
    // NotFound and be treated as "start from empty", not propagate an error.
    let target = dir.path().join(".env.new");
    let good_path = target.to_str().unwrap().to_string();

    let result = project::inject_environment(&db, &key, env_id, Some(good_path.clone()), None, false, None).await;
    assert!(result.is_ok(), "NotFound must still be treated as an empty starting file: {:?}", result.err());

    let content = std::fs::read_to_string(&good_path).unwrap();
    assert!(content.contains("DB_PASSWORD=hunter2"), "written content: {content}");
}

#[tokio::test]
async fn test_db_settings_roundtrip() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();

    db.set_setting("test_key", "hello").await.unwrap();
    let val = db.get_setting("test_key").await.unwrap();
    assert_eq!(val, Some("hello".to_string()));

    let missing = db.get_setting("nonexistent").await.unwrap();
    assert_eq!(missing, None);
}

#[tokio::test]
async fn test_db_settings_overwrite() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();

    db.set_setting("key", "first").await.unwrap();
    db.set_setting("key", "second").await.unwrap();
    let val = db.get_setting("key").await.unwrap();
    assert_eq!(val, Some("second".to_string()), "set_setting must overwrite existing value");
}

#[tokio::test]
async fn test_db_item_upsert_and_list() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();

    let created = "2026-01-01T00:00:00Z";
    let encrypted_data = "deadbeef0102030405";

    let inserted_id = db.upsert_item(0, "env", encrypted_data, created, false).await.unwrap();
    assert!(inserted_id > 0, "inserted id must be positive");

    let items = db.list_items().await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].0, inserted_id);
    assert_eq!(items[0].1, "env");
    assert_eq!(items[0].2, encrypted_data);
    assert_eq!(items[0].3, created);
}

#[tokio::test]
async fn test_db_item_update() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();

    let id = db.upsert_item(0, "env", "original_data", "2026-01-01", false).await.unwrap();
    db.upsert_item(id, "env", "updated_data", "2026-01-01", false).await.unwrap();

    let items = db.list_items().await.unwrap();
    assert_eq!(items.len(), 1, "update must not create a second row");
    assert_eq!(items[0].2, "updated_data");
}

#[tokio::test]
async fn test_db_item_delete() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();

    let id = db.upsert_item(0, "env", "some_data", "2026-01-01", false).await.unwrap();
    db.delete_item(id).await.unwrap();

    let items = db.list_items().await.unwrap();
    assert!(items.is_empty(), "list must be empty after deleting the only item");
}

#[tokio::test]
async fn test_db_is_initialized_false_on_fresh_db() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();

    let initialized = db.is_initialized().await.unwrap();
    assert!(!initialized, "fresh database must not be initialized");
}

#[tokio::test]
async fn test_db_init_vault_and_get_meta() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();

    db.init_vault("deadbeef_salt", "deadbeef_token").await.unwrap();

    let initialized = db.is_initialized().await.unwrap();
    assert!(initialized, "db must report initialized after init_vault");

    let meta = db.get_meta().await.unwrap();
    assert!(meta.is_some(), "get_meta must return Some after init_vault");
    let (salt, token) = meta.unwrap();
    assert_eq!(salt, "deadbeef_salt");
    assert_eq!(token, "deadbeef_token");
}

// ─── Issue #9: db-layer tests (12-15 of the plan's test matrix) ───────────
//
// Tests 1-11 need an HTTP-level harness (bound router + token + unlocked
// temp vault) that issue #11 owns and had not landed in this worktree at the
// time of writing — see the plan's §3.9 and this repo's parallel-worktree
// setup. Consuming that harness for the `api`-level create-or-update-on-
// collision assertions (200/201/409 status codes, `SHARED_ITEM_CONFLICT` /
// `KEY_EXISTS` / `CONFLICT_RETRY` bodies) is a documented follow-up once
// `crate::test_support` / `TestVault` land here. These four are pure
// `VaultDb` tests and need no HTTP surface at all.

#[tokio::test]
async fn test_link_item_rolls_back_when_link_fails() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();

    let project_id = db.upsert_project(0, "acme", None, "generic").await.unwrap();
    let bogus_environment_id = 999_999; // no such environment exists

    let result = db
        .create_or_link_item(
            bogus_environment_id,
            project_id,
            "API_KEY",
            "secret",
            "ciphertext-blob",
            "2026-01-01T00:00:00Z",
            LinkMode::Update,
            None, // caller's inspect_env_key saw a free key
        )
        .await;

    assert!(
        result.is_err(),
        "linking into a nonexistent environment must fail (FK violation on environment_vars)"
    );

    let items = db.list_items().await.unwrap();
    assert!(items.is_empty(), "the item insert must be rolled back with the rest of the transaction");

    let owned = db.list_owned_item_ids(project_id).await.unwrap();
    assert!(owned.is_empty(), "the item_projects insert must be rolled back too — no orphaned ownership row");
}

#[tokio::test]
async fn test_list_orphan_items_finds_unlinked_nonglobal() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();

    let id = db.upsert_item(0, "secret", "ciphertext", "2026-01-01", false).await.unwrap();
    // No environment_vars row created — this item is unreferenced.

    let orphans = db.list_orphan_item_ids().await.unwrap();
    assert_eq!(orphans, vec![id], "an unlinked, non-global item must be reported as an orphan");
}

#[tokio::test]
async fn test_list_orphan_items_excludes_global_and_linked() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();

    // Unlinked but global: has a reachable surface (Global Secrets) even
    // without a link, so it must NOT be reported as an orphan.
    let global_id = db.upsert_item(0, "secret", "global-ciphertext", "2026-01-01", true).await.unwrap();

    // Non-global but linked: reachable via its environment's scoped list,
    // so it must NOT be reported as an orphan either.
    let linked_id = db.upsert_item(0, "secret", "linked-ciphertext", "2026-01-01", false).await.unwrap();
    let project_id = db.upsert_project(0, "acme", None, "generic").await.unwrap();
    let environment_id = db.upsert_environment(0, project_id, "production", true).await.unwrap();
    db.upsert_environment_var(environment_id, "DB_PASSWORD", linked_id).await.unwrap();

    let orphans = db.list_orphan_item_ids().await.unwrap();
    assert!(!orphans.contains(&global_id), "a global unlinked item must not be reported as an orphan");
    assert!(!orphans.contains(&linked_id), "a linked non-global item must not be reported as an orphan");
    assert!(orphans.is_empty(), "no other rows were created in this test");
}

#[tokio::test]
async fn test_prune_orphan_items_clears_item_projects() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();

    let project_id = db.upsert_project(0, "acme", None, "generic").await.unwrap();

    // The orphan to be pruned: owned, but never linked into any environment.
    let orphan_id = db.upsert_item(0, "secret", "orphan-ciphertext", "2026-01-01", false).await.unwrap();
    db.add_item_owner(orphan_id, project_id).await.unwrap();

    // A second, unrelated item that must survive the prune untouched.
    let kept_id = db.upsert_item(0, "secret", "kept-ciphertext", "2026-01-01", false).await.unwrap();
    db.add_item_owner(kept_id, project_id).await.unwrap();
    let environment_id = db.upsert_environment(0, project_id, "production", true).await.unwrap();
    db.upsert_environment_var(environment_id, "DB_PASSWORD", kept_id).await.unwrap();

    vault::prune_orphan_items(&db, &[orphan_id]).await.unwrap();

    let items = db.list_items().await.unwrap();
    assert!(
        items.iter().all(|(id, ..)| *id != orphan_id),
        "the orphan's items row must be deleted"
    );
    assert!(
        db.list_owned_item_ids(project_id).await.unwrap().iter().all(|id| *id != orphan_id),
        "the orphan's item_projects row must be deleted too"
    );

    assert!(items.iter().any(|(id, ..)| *id == kept_id), "an item not in the prune list must survive");
    assert!(
        db.list_owned_item_ids(project_id).await.unwrap().contains(&kept_id),
        "the surviving item's ownership link must be untouched"
    );
}

// ─── Issue #13: global-item scoped visibility — data-path guards ─────────────
//
// The API's scope filter (src-tauri/src/api/mod.rs, `scope_items`) now reads
// `is_global` off every row returned by `list_items()` to decide whether an
// unlinked item should be unioned into a discovery response. These two tests
// guard the data path that decision depends on: the DB layer must round-trip
// `is_global` faithfully through both insert/update (`upsert_item`) and the
// dedicated toggle (`set_item_global`).

#[tokio::test]
async fn test_db_list_items_preserves_is_global_flag() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();

    let global_id = db.upsert_item(0, "secret", "global_data", "2026-01-01", true).await.unwrap();
    let non_global_id = db.upsert_item(0, "secret", "scoped_data", "2026-01-01", false).await.unwrap();

    let items = db.list_items().await.unwrap();
    assert_eq!(items.len(), 2);

    let global_row = items.iter().find(|r| r.0 == global_id).expect("global item must be present");
    assert!(global_row.4, "tuple index 4 (is_global) must be true for the item created global");

    let scoped_row = items.iter().find(|r| r.0 == non_global_id).expect("non-global item must be present");
    assert!(!scoped_row.4, "tuple index 4 (is_global) must be false for the item created non-global");
}

#[tokio::test]
async fn test_db_set_item_global_roundtrip() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_str().unwrap().to_string();
    let db = VaultDb::open(&db_path).await.unwrap();

    let id = db.upsert_item(0, "secret", "data", "2026-01-01", false).await.unwrap();

    db.set_item_global(id, true).await.unwrap();
    let items = db.list_items().await.unwrap();
    let row = items.iter().find(|r| r.0 == id).unwrap();
    assert!(row.4, "set_item_global(true) must flip is_global to true in list_items");

    db.set_item_global(id, false).await.unwrap();
    let items = db.list_items().await.unwrap();
    let row = items.iter().find(|r| r.0 == id).unwrap();
    assert!(!row.4, "set_item_global(false) must flip is_global back to false in list_items");
}
