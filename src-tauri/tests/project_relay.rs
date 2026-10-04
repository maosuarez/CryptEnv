// Integration tests for the whole-project relay feature (issue #4), exercised
// directly against the `db`/`project::relay` layers (no HTTP harness): issue
// #11's `test_support` fixture (`TestVault`, `seed_project`, `seed_item`,
// `link_var`) is not available in this worktree yet, so these tests build
// their own minimal local fixtures on top of `VaultDb` — the same pattern
// used by `tests/vault_integration.rs`. Once #11's harness lands, these can
// be rebased onto it; the assertions themselves (row counts, dedup, no
// leaked paths) are the ones the plan's cases 5-11 call for.
//
// Transport (relay upload/download) is not exercised here — the bundle is
// built and handed directly to the receive-side function, per the plan's
// "transport stubbed" instruction for this test file.

use crypt_env_lib::crypto::VaultKey;
use crypt_env_lib::db::VaultDb;
use crypt_env_lib::project::relay as project_relay;
use crypt_env_lib::share::package::PlainItem;
use crypt_env_lib::share::relay::{EnvironmentBundle, ProjectBundle, ProjectBundleVar};
use crypt_env_lib::vault::VaultItem;
use tempfile::tempdir;

fn sender_key() -> VaultKey {
    VaultKey::from_slice(&[7u8; 32]).unwrap()
}
fn receiver_key() -> VaultKey {
    VaultKey::from_slice(&[9u8; 32]).unwrap()
}

async fn open_db(dir: &tempfile::TempDir) -> VaultDb {
    let path = dir.path().join("vault.db").to_str().unwrap().to_string();
    VaultDb::open(&path).await.unwrap()
}

/// Encrypts and inserts a "secret" item with `name`/`value`, returning its id.
async fn seed_item(db: &VaultDb, key: &VaultKey, name: &str, value: &str) -> i64 {
    let item = VaultItem {
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
        created: "1000".to_string(),
        is_global: Some(false),
    };
    let json = serde_json::to_vec(&item).unwrap();
    let encrypted = crypt_env_lib::crypto::encrypt(key, &json).unwrap();
    db.upsert_item(0, "secret", &encrypted, "1000", false).await.unwrap()
}

/// Grants `project_id` ownership of `item_id` and links it into
/// `environment_id` under `key` — mirrors what `save_environment` /
/// `import_plain_items_into_vault` do together in production code.
async fn link_var(db: &VaultDb, environment_id: i64, project_id: i64, item_id: i64, key: &str) {
    db.add_item_owner(item_id, project_id).await.unwrap();
    db.upsert_environment_var(environment_id, key, item_id).await.unwrap();
}

/// Decrypts every item in `db` and returns (item_id, name, is_global).
async fn decrypt_all(db: &VaultDb, key: &VaultKey) -> Vec<(i64, String, bool)> {
    let raw = db.list_items().await.unwrap();
    raw.into_iter()
        .map(|(id, _item_type, data, _created, is_global)| {
            let plaintext = crypt_env_lib::crypto::decrypt(key, &data).unwrap();
            let item: VaultItem = serde_json::from_slice(&plaintext).unwrap();
            (id, item.name.unwrap_or_default(), is_global)
        })
        .collect()
}

/// A 3-environment project (local default, staging, production) with 5
/// distinct items, 2 of which (S1, S2) are linked into all 3 environments —
/// the fixture named in the plan's case 5. Returns (db, project_id, [local,
/// staging, production]).
async fn seed_three_env_project(dir: &tempfile::TempDir) -> (VaultDb, i64, [i64; 3]) {
    let db = open_db(dir).await;
    let project_id = db.upsert_project(0, "MyApp", None, "generic").await.unwrap();
    let local = db.upsert_environment(0, project_id, "local", true).await.unwrap();
    let staging = db.upsert_environment(0, project_id, "staging", false).await.unwrap();
    let production = db.upsert_environment(0, project_id, "production", false).await.unwrap();

    let s1 = seed_item(&db, &sender_key(), "S1", "shared-1").await;
    let s2 = seed_item(&db, &sender_key(), "S2", "shared-2").await;
    let it3 = seed_item(&db, &sender_key(), "IT3", "v3").await;
    let it4 = seed_item(&db, &sender_key(), "IT4", "v4").await;
    let it5 = seed_item(&db, &sender_key(), "IT5", "v5").await;

    // local: S1, S2, IT3, IT4  (4)
    link_var(&db, local, project_id, s1, "S1").await;
    link_var(&db, local, project_id, s2, "S2").await;
    link_var(&db, local, project_id, it3, "IT3").await;
    link_var(&db, local, project_id, it4, "IT4").await;

    // staging: S1, S2, IT3, IT5 (4)
    link_var(&db, staging, project_id, s1, "S1").await;
    link_var(&db, staging, project_id, s2, "S2").await;
    link_var(&db, staging, project_id, it3, "IT3").await;
    link_var(&db, staging, project_id, it5, "IT5").await;

    // production: S1, S2, IT5 (3)
    link_var(&db, production, project_id, s1, "S1").await;
    link_var(&db, production, project_id, s2, "S2").await;
    link_var(&db, production, project_id, it5, "IT5").await;

    // total vars: 4 + 4 + 3 = 11, distinct items: S1, S2, IT3, IT4, IT5 = 5
    (db, project_id, [local, staging, production])
}

#[tokio::test]
async fn share_three_env_project_dedups_reused_items() {
    let sender_dir = tempdir().unwrap();
    let (db, project_id, envs) = seed_three_env_project(&sender_dir).await;

    let bundle = project_relay::build_project_bundle(&db, &sender_key(), project_id, &envs)
        .await
        .expect("build_project_bundle should succeed");

    assert_eq!(bundle.items.len(), 5, "items must be deduped across environments, not 11");
    let total_vars: usize = bundle.environments.iter().map(|e| e.vars.len()).sum();
    assert_eq!(total_vars, 11);

    let receiver_dir = tempdir().unwrap();
    let receiver_db = open_db(&receiver_dir).await;
    let result = project_relay::receive_project_bundle(&receiver_db, &receiver_key(), bundle, None)
        .await
        .expect("receive_project_bundle should succeed into a fresh vault");

    assert_eq!(result.environment_names.len(), 3);
    assert_eq!(result.item_count, 5);

    let items = receiver_db.list_items().await.unwrap();
    assert_eq!(items.len(), 5, "exactly 5 new items rows");

    let projects = receiver_db.list_projects().await.unwrap();
    assert_eq!(projects.len(), 1);
    let new_project_id = projects[0].id;

    let received_envs = receiver_db.list_environments(new_project_id).await.unwrap();
    assert_eq!(received_envs.len(), 3, "3 environments rows");

    let mut total_env_var_rows = 0;
    for e in &received_envs {
        total_env_var_rows += receiver_db.get_environment_vars(e.id).await.unwrap().len();
    }
    assert_eq!(total_env_var_rows, 11, "11 environment_vars rows");

    let owned = receiver_db.list_owned_item_ids(new_project_id).await.unwrap();
    assert_eq!(owned.len(), 5, "5 item_projects rows, all pointing at the new project");

    // The two shared items appear as one row each -- 0 duplicated ciphertext rows.
    let decrypted = decrypt_all(&receiver_db, &receiver_key()).await;
    let count_named = |n: &str| decrypted.iter().filter(|(_, name, _)| name == n).count();
    assert_eq!(count_named("S1"), 1);
    assert_eq!(count_named("S2"), 1);
}

fn minimal_bundle(name: &str) -> ProjectBundle {
    ProjectBundle {
        kind: ProjectBundle::KIND.to_string(),
        version: ProjectBundle::VERSION,
        name: name.to_string(),
        description: None,
        template: "generic".to_string(),
        environments: vec![EnvironmentBundle {
            name: "default".to_string(),
            is_default: true,
            vars: vec![ProjectBundleVar { key: "K".to_string(), item_name: "itemA".to_string() }],
        }],
        items: vec![PlainItem {
            item_type: "secret".to_string(),
            name: "itemA".to_string(),
            value: Some("v".to_string()),
            username: None,
            password: None,
            url: None,
            notes: None,
            category: None,
            command: None,
        }],
    }
}

#[tokio::test]
async fn receive_refuses_case_insensitive_project_name_collision() {
    let dir = tempdir().unwrap();
    let db = open_db(&dir).await;

    // Receiving vault already has a project named "myapp" (lowercase).
    let existing_id = db.upsert_project(0, "myapp", None, "generic").await.unwrap();
    db.upsert_environment(0, existing_id, "default", true).await.unwrap();

    let before_items = db.list_items().await.unwrap().len();
    let before_envs = db.list_environments(existing_id).await.unwrap().len();
    let before_owned = db.list_owned_item_ids(existing_id).await.unwrap().len();
    let before_projects = db.list_projects().await.unwrap().len();

    // Incoming bundle uses "MyApp" -- different case, same name.
    let bundle = minimal_bundle("MyApp");
    let err = project_relay::receive_project_bundle(&db, &receiver_key(), bundle, None)
        .await
        .expect_err("case-insensitive name collision must be a hard error");

    assert!(err.starts_with("conflict:"), "expected a 'conflict:'-prefixed error, got: {err}");

    // Vault must be entirely unchanged -- transaction rolled back.
    assert_eq!(db.list_items().await.unwrap().len(), before_items, "no new items rows");
    assert_eq!(db.list_environments(existing_id).await.unwrap().len(), before_envs, "no new environments rows");
    assert_eq!(db.list_owned_item_ids(existing_id).await.unwrap().len(), before_owned, "no new item_projects rows");
    assert_eq!(db.list_projects().await.unwrap().len(), before_projects, "no new project row");
}

#[tokio::test]
async fn receive_with_name_override_succeeds_after_collision() {
    let sender_dir = tempdir().unwrap();
    let (sender_db, project_id, envs) = seed_three_env_project(&sender_dir).await;
    let bundle = project_relay::build_project_bundle(&sender_db, &sender_key(), project_id, &envs)
        .await
        .unwrap();
    assert_eq!(bundle.name, "MyApp");

    let dir = tempdir().unwrap();
    let db = open_db(&dir).await;
    // A colliding project already exists in the receiving vault.
    db.upsert_project(0, "MyApp", None, "generic").await.unwrap();

    let result = project_relay::receive_project_bundle(
        &db,
        &receiver_key(),
        bundle,
        Some("MyApp-received".to_string()),
    )
    .await
    .expect("receive with an override name must succeed despite the collision");

    assert_eq!(result.project_name, "MyApp-received");
    assert_eq!(result.item_count, 5);
    assert_eq!(result.environment_names.len(), 3);

    let projects = db.list_projects().await.unwrap();
    assert_eq!(projects.len(), 2, "original + newly received project");
    assert!(projects.iter().any(|p| p.name == "MyApp-received"));
}

#[tokio::test]
async fn send_excludes_unselected_environments() {
    let dir = tempdir().unwrap();
    let db = open_db(&dir).await;
    let project_id = db.upsert_project(0, "MyApp", None, "generic").await.unwrap();
    let local = db.upsert_environment(0, project_id, "local", true).await.unwrap();
    let production = db.upsert_environment(0, project_id, "production", false).await.unwrap();

    let shared = seed_item(&db, &sender_key(), "SHARED", "shared-value").await;
    let prod_only = seed_item(&db, &sender_key(), "PRODONLY", "prod-value").await;

    link_var(&db, local, project_id, shared, "SHARED").await;
    link_var(&db, production, project_id, shared, "SHARED").await;
    link_var(&db, production, project_id, prod_only, "PRODONLY").await;

    // Select only the default (local) environment.
    let bundle = project_relay::build_project_bundle(&db, &sender_key(), project_id, &[local])
        .await
        .unwrap();

    assert_eq!(bundle.environments.len(), 1);
    assert_eq!(bundle.environments[0].name, "local");
    assert!(
        bundle.items.iter().all(|i| i.name != "PRODONLY"),
        "an item used only by the unselected 'production' environment must be absent from bundle.items"
    );
    assert!(bundle.items.iter().any(|i| i.name == "SHARED"));
}

#[tokio::test]
async fn send_skips_dangling_item_reference() {
    let dir = tempdir().unwrap();
    let db = open_db(&dir).await;
    let project_id = db.upsert_project(0, "MyApp", None, "generic").await.unwrap();
    let local = db.upsert_environment(0, project_id, "local", true).await.unwrap();

    let real_item = seed_item(&db, &sender_key(), "REAL", "value").await;
    link_var(&db, local, project_id, real_item, "REAL").await;

    // Link a var whose item_id does not (and never will) resolve to a real
    // item -- simulates an item deleted after linking.
    let dangling_item_id = 999_999;
    db.upsert_environment_var(local, "GHOST", dangling_item_id).await.unwrap();

    let bundle = project_relay::build_project_bundle(&db, &sender_key(), project_id, &[local])
        .await
        .expect("a dangling item_id must not fail the whole send");

    let total_vars: usize = bundle.environments.iter().map(|e| e.vars.len()).sum();
    assert_eq!(total_vars, 1, "the dangling var is skipped, only the real one remains");
    assert_eq!(bundle.items.len(), 1);
    assert_eq!(bundle.items[0].name, "REAL");
}

#[tokio::test]
async fn received_items_are_project_owned_not_global() {
    let sender_dir = tempdir().unwrap();
    let (sender_db, project_id, envs) = seed_three_env_project(&sender_dir).await;
    let bundle = project_relay::build_project_bundle(&sender_db, &sender_key(), project_id, &envs)
        .await
        .unwrap();

    let dir = tempdir().unwrap();
    let db = open_db(&dir).await;
    let result = project_relay::receive_project_bundle(&db, &receiver_key(), bundle, None)
        .await
        .unwrap();

    let owned = db.list_owned_item_ids(result.project_id).await.unwrap();
    assert_eq!(owned.len(), 5);
    for item_id in owned {
        assert_eq!(db.is_item_global(item_id).await.unwrap(), Some(false), "received items must not be global");
        assert_eq!(db.item_owner_count(item_id).await.unwrap(), 1, "exactly one item_projects row per item");
    }
}

#[tokio::test]
async fn bundle_never_contains_paths() {
    let dir = tempdir().unwrap();
    let db = open_db(&dir).await;
    let project_id = db.upsert_project(0, "MyApp", None, "generic").await.unwrap();
    let local = db.upsert_environment(0, project_id, "local", true).await.unwrap();

    let leaky_path = "C:\\Users\\maosuarez\\dev\\myapp\\.env.production";
    db.set_environment_paths(local, &[leaky_path.to_string()]).await.unwrap();

    let item = seed_item(&db, &sender_key(), "SECRET", "value").await;
    link_var(&db, local, project_id, item, "SECRET").await;

    let bundle = project_relay::build_project_bundle(&db, &sender_key(), project_id, &[local])
        .await
        .unwrap();

    let json = serde_json::to_string(&bundle).unwrap();
    assert!(!json.contains("maosuarez"), "bundle JSON must not contain any path substring");
    assert!(!json.contains(".env.production"));
    assert!(!json.contains("C:\\Users"));
}
