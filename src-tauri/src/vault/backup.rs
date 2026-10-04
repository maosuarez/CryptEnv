//! Backup format (v1 read, v2 read/write) and the atomic restore.
//!
//! Restore never touches the live database until a complete, verified
//! replacement exists next to it (`vault.db.restore-tmp`); the swap is a pair
//! of same-directory renames with rollback. See
//! `openspec/changes/backup-restore-completeness/design.md`.

use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, Transaction};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use super::{epoch_to_iso8601, SharedState, SystemTime, UNIX_EPOCH, VaultState};
use crate::crypto::{self, CryptoKey};
use crate::db::{
    pre_restore_path, remove_db_files, sibling_with_suffix, DbCategory, DbEnvironmentVar, VaultDb,
};

const CURRENT_VERSION: u32 = 2;
/// Bound to the enrolled password via DPAPI; only valid for the vault it was
/// enrolled against, and intentionally never carried across vaults.
const BIOMETRIC_SETTING: &str = "biometric_blob";

// ─── Format ───────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
pub(super) struct BackupItem {
    pub id: i64,
    pub item_type: String,
    /// Raw AES-GCM encrypted blob from the DB (hex-encoded nonce || ciphertext).
    pub data: String,
    pub created: String,
    #[serde(default)]
    pub is_global: bool,
}

#[derive(Serialize, Deserialize)]
pub(super) struct BackupFile {
    pub version: u32,
    pub created_at: u64,
    /// Hex-encoded Argon2id salt used to derive the vault key.
    pub salt: String,
    /// AES-GCM verify token that proves the master password is correct.
    pub token: String,
    pub items: Vec<BackupItem>,
    /// v1 only (plaintext). v2 keeps categories inside `meta`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<serde_json::Value>,
    /// v2 only: hex(nonce || AES-GCM(vault_key, json(Meta))).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<String>,
}

/// Organisational metadata of a v2 backup. Ids are backup-local; restore
/// remaps them. Encrypted as a whole, so names and paths are not readable
/// without the master password.
#[derive(Serialize, Deserialize, Default)]
pub(super) struct Meta {
    #[serde(default)]
    pub categories: Vec<MetaCategory>,
    #[serde(default)]
    pub projects: Vec<MetaProject>,
    #[serde(default)]
    pub environments: Vec<MetaEnvironment>,
    #[serde(default)]
    pub environment_vars: Vec<MetaVar>,
    #[serde(default)]
    pub item_projects: Vec<MetaItemProject>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct MetaCategory {
    pub id: String,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct MetaProject {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub template: String,
    pub root_path: Option<String>,
    #[serde(default)]
    pub is_holding: bool,
    /// Category ids.
    #[serde(default)]
    pub categories: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct MetaEnvironment {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub is_default: bool,
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct MetaVar {
    pub environment_id: i64,
    pub key: String,
    pub item_id: Option<i64>,
    #[serde(default)]
    pub literal: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct MetaItemProject {
    pub item_id: i64,
    pub project_id: i64,
}

pub(super) fn encode_meta(key: &CryptoKey, meta: &Meta) -> Result<String, String> {
    let json = serde_json::to_vec(meta).map_err(|e| format!("serialize backup metadata: {e}"))?;
    crypto::encrypt(key, &json)
}

pub(super) fn decode_meta(key: &CryptoKey, hex: &str) -> Result<Meta, String> {
    let json = crypto::decrypt(key, hex).map_err(|_| "backup metadata is corrupt".to_string())?;
    serde_json::from_slice(&json).map_err(|_| "backup metadata is corrupt".to_string())
}

/// Parses a `.cenvbak` and accepts versions 1 and 2.
pub(super) fn parse_backup(json: &str) -> Result<BackupFile, String> {
    let backup: BackupFile =
        serde_json::from_str(json).map_err(|e| format!("parse backup: {e}"))?;
    match backup.version {
        1 => Ok(backup),
        2 if backup.meta.is_some() => Ok(backup),
        2 => Err("backup is missing its metadata section".to_string()),
        v => Err(format!("unsupported backup version: {v}")),
    }
}

/// The backup's metadata: decrypted for v2, rebuilt from the plaintext
/// category list for v1 (which carried no projects).
fn load_meta(backup: &BackupFile, key: &CryptoKey) -> Result<Meta, String> {
    match &backup.meta {
        Some(hex) => decode_meta(key, hex),
        None => Ok(Meta {
            categories: backup
                .categories
                .iter()
                .filter_map(|v| {
                    Some(MetaCategory {
                        id: v.get("id")?.as_str()?.to_string(),
                        name: v.get("name")?.as_str()?.to_string(),
                        color: v.get("color")?.as_str()?.to_string(),
                        description: v
                            .get("description")
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string()),
                    })
                })
                .collect(),
            ..Meta::default()
        }),
    }
}

// ─── Export ───────────────────────────────────────────────────────────────────

/// Builds a v2 backup of the whole vault. Returns the file and its item count.
pub(super) async fn build_backup(
    db: &VaultDb,
    key: &CryptoKey,
) -> Result<(BackupFile, usize), String> {
    let (salt, token) = db.get_meta().await?.ok_or("vault_meta missing")?;

    let items: Vec<BackupItem> = db
        .list_items()
        .await?
        .into_iter()
        .map(|(id, item_type, data, created, is_global)| BackupItem {
            id,
            item_type,
            data,
            created,
            is_global,
        })
        .collect();
    let item_count = items.len();

    let holding = db.list_holding_project_ids().await?;
    let mut links: HashMap<i64, Vec<String>> = HashMap::new();
    for (project_id, category_id) in db.list_project_category_links().await? {
        links.entry(project_id).or_default().push(category_id);
    }

    let mut meta = Meta {
        categories: db
            .list_categories()
            .await?
            .into_iter()
            .map(|c| MetaCategory { id: c.cid, name: c.name, color: c.color, description: c.description })
            .collect(),
        ..Meta::default()
    };
    for p in db.list_projects().await? {
        for env in db.list_environments(p.id).await? {
            for v in db.get_environment_vars(env.id).await? {
                meta.environment_vars.push(MetaVar {
                    environment_id: env.id,
                    key: v.key,
                    item_id: v.item_id,
                    literal: v.literal,
                });
            }
            meta.environments.push(MetaEnvironment {
                id: env.id,
                project_id: p.id,
                name: env.name,
                is_default: env.is_default,
                paths: env.paths,
            });
        }
        meta.projects.push(MetaProject {
            id: p.id,
            is_holding: holding.contains(&p.id),
            categories: links.remove(&p.id).unwrap_or_default(),
            name: p.name,
            description: p.description,
            template: p.template,
            root_path: p.root_path,
        });
    }
    for (item_id, project_id) in db.list_item_projects().await? {
        meta.item_projects.push(MetaItemProject { item_id, project_id });
    }

    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let file = BackupFile {
        version: CURRENT_VERSION,
        created_at,
        salt,
        token,
        items,
        categories: Vec::new(),
        meta: Some(encode_meta(key, &meta)?),
    };
    Ok((file, item_count))
}

// ─── Restore ──────────────────────────────────────────────────────────────────

/// What a restore did, for the GUI summary.
#[derive(Serialize, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RestoreSummary {
    pub items: usize,
    pub categories: usize,
    pub projects: usize,
    pub environments: usize,
    pub backup_version: u32,
    /// False for v1 backups, which never carried projects.
    pub projects_included: bool,
    /// Name of the `Restored <date>` project that received legacy items.
    pub holding_project: Option<String>,
}

/// Applies a decoded backup inside `tx`, remapping every backup-local id.
/// `reencrypt = Some((backup_key, current_key))` is the merge case; `None`
/// keeps the stored ciphertext (replace: the vault key becomes the backup's).
/// Existing projects (by name) and environments are kept as they are, and
/// missing ones are added next to them.
async fn apply_backup(
    tx: &mut Transaction<'_, Sqlite>,
    backup: &BackupFile,
    meta: &Meta,
    reencrypt: Option<(&CryptoKey, &CryptoKey)>,
    holding_name: &str,
) -> Result<RestoreSummary, String> {
    let mut summary = RestoreSummary {
        backup_version: backup.version,
        projects_included: backup.version >= 2,
        ..RestoreSummary::default()
    };

    for c in &meta.categories {
        let cat = DbCategory {
            cid: c.id.clone(),
            name: c.name.clone(),
            color: c.color.clone(),
            description: c.description.clone(),
        };
        VaultDb::insert_category_if_absent_tx(tx, &cat).await?;
    }
    summary.categories = meta.categories.len();

    let mut item_map: HashMap<i64, i64> = HashMap::new();
    let mut holding_id: Option<i64> = None;
    for item in &backup.items {
        let data = match reencrypt {
            Some((from, to)) => {
                let plain = crypto::decrypt(from, &item.data)
                    .map_err(|e| format!("decrypt backup item {}: {e}", item.id))?;
                crypto::encrypt(to, &plain)
                    .map_err(|e| format!("re-encrypt item {}: {e}", item.id))?
            }
            None => item.data.clone(),
        };
        let new_id =
            VaultDb::insert_item_tx(tx, &item.item_type, &data, &item.created, item.is_global).await?;
        item_map.insert(item.id, new_id);

        // v1 backups have no ownership data: give non-global items an owner so
        // they are never reported as orphans.
        if backup.version < 2 && !item.is_global {
            let pid = match holding_id {
                Some(id) => id,
                None => {
                    let id = VaultDb::ensure_holding_project_tx(tx, holding_name).await?;
                    holding_id = Some(id);
                    id
                }
            };
            VaultDb::add_item_owner_tx(tx, new_id, pid).await?;
        }
    }
    summary.items = backup.items.len();
    if holding_id.is_some() {
        summary.holding_project = Some(holding_name.to_string());
    }

    // old project id -> (new id, whether it already existed)
    let mut project_map: HashMap<i64, (i64, bool)> = HashMap::new();
    for p in &meta.projects {
        let mapped = match VaultDb::find_project_by_name_tx(tx, &p.name).await? {
            Some(id) => (id, true),
            None => (
                VaultDb::insert_project_tx(
                    tx,
                    &p.name,
                    p.description.as_deref(),
                    &p.template,
                    p.root_path.as_deref(),
                    p.is_holding,
                )
                .await?,
                false,
            ),
        };
        for cid in &p.categories {
            VaultDb::add_project_category_tx(tx, mapped.0, cid).await?;
        }
        project_map.insert(p.id, mapped);
    }
    summary.projects = meta.projects.len();

    for env in &meta.environments {
        let (project_id, existed) = *project_map
            .get(&env.project_id)
            .ok_or("backup is inconsistent: environment without project")?;
        if VaultDb::find_environment_tx(tx, project_id, &env.name).await?.is_some() {
            continue;
        }
        let is_default = env.is_default
            && !(existed && VaultDb::project_has_default_environment_tx(tx, project_id).await?);
        let env_id =
            VaultDb::upsert_environment_tx(tx, 0, project_id, &env.name, is_default).await?;
        VaultDb::set_environment_paths_tx(tx, env_id, &env.paths).await?;

        let mut vars = Vec::new();
        for v in meta.environment_vars.iter().filter(|v| v.environment_id == env.id) {
            let item_id = match v.item_id {
                Some(old) => Some(
                    *item_map
                        .get(&old)
                        .ok_or("backup is inconsistent: variable references a missing item")?,
                ),
                None => None,
            };
            vars.push(DbEnvironmentVar {
                id: 0,
                environment_id: env_id,
                key: v.key.clone(),
                item_id,
                literal: v.literal.clone(),
            });
        }
        VaultDb::set_environment_vars_tx(tx, env_id, &vars).await?;
        summary.environments += 1;
    }

    for ip in &meta.item_projects {
        let item = item_map
            .get(&ip.item_id)
            .ok_or("backup is inconsistent: ownership references a missing item")?;
        let (project, _) = project_map
            .get(&ip.project_id)
            .ok_or("backup is inconsistent: ownership references a missing project")?;
        VaultDb::add_item_owner_tx(tx, *item, *project).await?;
    }

    Ok(summary)
}

/// Everything a restore needs besides the live and staging databases.
struct Plan<'a> {
    backup: &'a BackupFile,
    meta: &'a Meta,
    backup_key: &'a CryptoKey,
    current_key: &'a CryptoKey,
    merge: bool,
    holding_name: &'a str,
}

/// Builds and verifies the replacement database at `staging`, then closes it.
/// Any failure leaves the live database untouched; the caller removes the
/// staging files.
async fn build_staging(
    live: &VaultDb,
    staging: &Path,
    plan: &Plan<'_>,
) -> Result<RestoreSummary, String> {
    let merge = plan.merge;
    remove_db_files(staging)?;
    let staging_db = if merge {
        live.vacuum_into(staging).await?;
        let path = staging.to_str().ok_or("invalid database path")?;
        VaultDb::open(path).await?
    } else {
        VaultDb::create_at(staging).await?
    };

    let result = populate(live, &staging_db, plan).await;
    match result {
        Ok(summary) => {
            staging_db.close().await?;
            Ok(summary)
        }
        Err(e) => {
            let _ = staging_db.close().await;
            Err(e)
        }
    }
}

async fn populate(
    live: &VaultDb,
    staging_db: &VaultDb,
    plan: &Plan<'_>,
) -> Result<RestoreSummary, String> {
    let Plan { backup, meta, backup_key, current_key, merge, holding_name } = *plan;
    if !merge {
        staging_db.init_vault(&backup.salt, &backup.token).await?;
        // Device-level settings (MCP token, hotkey, relay config, ...) are not
        // part of a backup and survive a replace. The biometric blob only
        // stays valid while the vault key is unchanged.
        let same_vault = live
            .get_meta()
            .await?
            .map(|(_, token)| token == backup.token)
            .unwrap_or(false);
        for (key, value) in live.list_settings().await? {
            if key == BIOMETRIC_SETTING && !same_vault {
                continue;
            }
            staging_db.set_setting(&key, &value).await?;
        }
    }

    let mut tx = staging_db.begin().await?;
    let reencrypt = if merge { Some((backup_key, current_key)) } else { None };
    let summary = apply_backup(&mut tx, backup, meta, reencrypt, holding_name).await?;
    tx.commit().await.map_err(|e| e.to_string())?;

    staging_db.integrity_check().await?;
    let expected_key = if merge { current_key } else { backup_key };
    let (_, token) = staging_db.get_meta().await?.ok_or("restored vault is missing its key material")?;
    crypto::decrypt(expected_key, &token).map_err(|_| "restored vault failed verification".to_string())?;
    Ok(summary)
}

/// Moves the live database aside and the staged one into its place. On any
/// failure the original is back at `live` (or, if even that fails, the error
/// names where it is).
fn swap_files(live: &Path, staging: &Path, pre_restore: &Path) -> Result<(), String> {
    remove_db_files(pre_restore)?;
    std::fs::rename(live, pre_restore).map_err(|e| format!("swap: set aside current vault: {e}"))?;
    // A stale WAL of the old database must never attach to the new file.
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(sibling_with_suffix(live, suffix));
    }
    if let Err(e) = std::fs::rename(staging, live) {
        return match std::fs::rename(pre_restore, live) {
            Ok(()) => Err(format!("swap: install restored vault: {e}; the current vault was kept")),
            Err(e2) => Err(format!(
                "swap: install restored vault: {e}; rollback failed: {e2}; the previous vault is at {}",
                pre_restore.display()
            )),
        };
    }
    Ok(())
}

/// Puts the previous database back at `live` after the new one could not be
/// opened.
fn rollback_swap(live: &Path, pre_restore: &Path) -> Result<(), String> {
    remove_db_files(live)?;
    std::fs::rename(pre_restore, live).map_err(|e| format!("rollback: {e}"))
}

async fn reopen_original(s: &mut VaultState, live: &Path, cause: String) -> String {
    let path = live.to_string_lossy().into_owned();
    match VaultDb::open(&path).await {
        Ok(db) => {
            s.db = db;
            cause
        }
        Err(e) => format!("{cause}; reopening the vault also failed ({e}); restart the app"),
    }
}

/// Restores `json` into the vault behind `shared`. The password derivations
/// (the current master password for a replace, the backup's) run off the
/// vault lock; the restore itself re-checks the lock epoch and key before it
/// swaps anything. See `restore` for the rules.
pub(super) async fn restore_shared(
    shared: &SharedState,
    json: &str,
    backup_password: &str,
    current_password: Option<&str>,
    merge: bool,
) -> Result<RestoreSummary, String> {
    // Phase 1: snapshot under the lock.
    let (current_key, current_meta, epoch) = {
        let s = shared.lock().await;
        let key = s.key.as_ref().ok_or("vault is locked")?.clone();
        let meta = if merge { None } else { Some(s.db.get_meta().await?.ok_or("vault_meta missing")?) };
        (key, meta, s.epoch)
    };
    if !merge && current_password.filter(|p| !p.is_empty()).is_none() {
        return Err("the current master password is required to replace the vault".to_string());
    }
    let backup = parse_backup(json)?;

    // Phase 2: both derivations, off the runtime and off the lock.
    let current_pw = current_password.map(|p| Zeroizing::new(p.as_bytes().to_vec()));
    let backup_pw = Zeroizing::new(backup_password.as_bytes().to_vec());
    let (salt, token) = (backup.salt.clone(), backup.token.clone());
    let expected = current_key.clone();
    let backup_key = tokio::task::spawn_blocking(move || {
        if let (Some(pw), Some((salt, token))) = (current_pw, current_meta) {
            let derived = Zeroizing::new(
                crypto::unlock_vault_crypto(&pw, &salt, &token)
                    .map_err(|_| "incorrect current master password".to_string())?,
            );
            if !bool::from(derived.as_slice().ct_eq(expected.as_slice())) {
                return Err("incorrect current master password".to_string());
            }
        }
        crypto::unlock_vault_crypto(&backup_pw, &salt, &token)
            .map(Zeroizing::new)
            .map_err(|_| "incorrect master password for this backup".to_string())
    })
    .await
    .map_err(|_| "key derivation task failed".to_string())??;

    // Phase 3: the swap needs the lock for its whole duration.
    let mut s = shared.lock().await;
    if s.epoch != epoch {
        return Err("the vault changed while the passwords were being verified, try again".to_string());
    }
    restore_verified(&mut s, &backup, backup_key, current_key, merge).await
}

/// Restores `json` into the vault held by `s`, deriving under the caller's
/// lock. Kept for the unit tests of the restore rules; production goes
/// through `restore_shared`.
///
/// - Both modes require an unlocked vault and the backup's master password.
/// - `merge = false` additionally requires `current_password` (the vault's own
///   master password) and replaces the vault's contents; the vault key becomes
///   the backup's.
/// - `merge = true` adds the backup's content, re-encrypted with the current
///   vault key.
#[cfg(test)]
pub(super) async fn restore(
    s: &mut VaultState,
    json: &str,
    backup_password: &str,
    current_password: Option<&str>,
    merge: bool,
) -> Result<RestoreSummary, String> {
    let current_key = s.key.as_ref().ok_or("vault is locked")?.clone();

    if !merge {
        let pw = current_password
            .filter(|p| !p.is_empty())
            .ok_or("the current master password is required to replace the vault")?;
        let (salt, token) = s.db.get_meta().await?.ok_or("vault_meta missing")?;
        let derived = Zeroizing::new(
            crypto::unlock_vault_crypto(pw.as_bytes(), &salt, &token)
                .map_err(|_| "incorrect current master password".to_string())?,
        );
        if !bool::from(derived.as_slice().ct_eq(current_key.as_slice())) {
            return Err("incorrect current master password".to_string());
        }
    }

    let backup = parse_backup(json)?;
    let backup_key = Zeroizing::new(
        crypto::unlock_vault_crypto(backup_password.as_bytes(), &backup.salt, &backup.token)
            .map_err(|_| "incorrect master password for this backup".to_string())?,
    );
    restore_verified(s, &backup, backup_key, current_key, merge).await
}

/// The restore proper, once the passwords have been verified and both keys
/// are known. Runs under the vault lock.
async fn restore_verified(
    s: &mut VaultState,
    backup: &BackupFile,
    backup_key: Zeroizing<[u8; 32]>,
    current_key: Zeroizing<[u8; 32]>,
    merge: bool,
) -> Result<RestoreSummary, String> {
    let meta = load_meta(backup, &backup_key)?;

    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let date = epoch_to_iso8601(now)[..10].to_string();
    let holding_name = format!("Restored {date}");

    let live = PathBuf::from(s.db.path());
    let staging = sibling_with_suffix(&live, ".restore-tmp");
    let pre_restore = pre_restore_path(&live);

    let plan = Plan {
        backup,
        meta: &meta,
        backup_key: &backup_key,
        current_key: &current_key,
        merge,
        holding_name: &holding_name,
    };
    let summary = match build_staging(&s.db, &staging, &plan).await {
        Ok(summary) => summary,
        Err(e) => {
            let _ = remove_db_files(&staging);
            return Err(e);
        }
    };

    if let Err(e) = s.db.close().await {
        let _ = remove_db_files(&staging);
        return Err(e);
    }
    if let Err(e) = swap_files(&live, &staging, &pre_restore) {
        let _ = remove_db_files(&staging);
        return Err(reopen_original(s, &live, e).await);
    }
    let new_db = match VaultDb::open(&live.to_string_lossy()).await {
        Ok(db) => db,
        Err(e) => {
            let mut cause = format!("open restored vault: {e}");
            if let Err(e2) = rollback_swap(&live, &pre_restore) {
                cause = format!("{cause}; {e2}");
            }
            return Err(reopen_original(s, &live, cause).await);
        }
    };

    s.db = new_db;
    if !merge {
        s.set_key(Some(backup_key));
    }
    s.touch();
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{unlocked_vault, unlocked_vault_empty, TestVault};
    use crate::vault::decrypt_item;

    async fn export_json(v: &TestVault) -> String {
        let s = v.state.lock().await;
        let key = s.key.as_ref().unwrap().clone();
        let (file, _) = build_backup(&s.db, &key).await.unwrap();
        serde_json::to_string(&file).unwrap()
    }

    async fn item_names(v: &TestVault) -> Vec<String> {
        let s = v.state.lock().await;
        let key = s.key.as_ref().unwrap().clone();
        let mut names: Vec<String> = s
            .db
            .list_items()
            .await
            .unwrap()
            .into_iter()
            .filter_map(|(id, _, data, _, g)| decrypt_item(&key, id, &data, g).ok()?.name)
            .collect();
        names.sort();
        names
    }

    async fn restore_into(
        v: &TestVault,
        json: &str,
        current: Option<&str>,
        merge: bool,
    ) -> Result<RestoreSummary, String> {
        let mut s = v.state.lock().await;
        restore(&mut s, json, &v.master_password, current, merge).await
    }

    #[tokio::test]
    async fn v2_export_hides_metadata_and_round_trips() {
        let v = unlocked_vault().await;
        let json = export_json(&v).await;
        for secret in ["demo", "production", "local", "DB_HOST"] {
            assert!(!json.contains(secret), "{secret} leaked into the backup file");
        }
        let backup = parse_backup(&json).unwrap();
        assert_eq!(backup.version, 2);
        let key = v.state.lock().await.key.as_ref().unwrap().clone();
        let meta = decode_meta(&key, backup.meta.as_ref().unwrap()).unwrap();
        assert_eq!(meta.projects.len(), 1);
        assert_eq!(meta.projects[0].name, "demo");
        assert_eq!(meta.environments.len(), 2);
        assert_eq!(meta.environment_vars.len(), 3);
        assert_eq!(meta.item_projects.len(), 3);
    }

    #[test]
    fn parse_accepts_v1_v2_and_rejects_others() {
        let base = |version: u32, extra: &str| {
            format!(
                r#"{{"version":{version},"created_at":0,"salt":"00","token":"00","items":[]{extra}}}"#
            )
        };
        assert!(parse_backup(&base(1, r#","categories":[]"#)).is_ok());
        assert!(parse_backup(&base(2, r#","meta":"00""#)).is_ok());
        assert!(parse_backup(&base(2, "")).is_err());
        assert!(parse_backup(&base(3, "")).is_err());
    }

    #[tokio::test]
    async fn replace_round_trip_across_vaults() {
        let a = unlocked_vault().await;
        let b = unlocked_vault_empty().await;
        let json = export_json(&a).await;
        let a_key = a.state.lock().await.key.as_ref().unwrap().clone();

        let summary = restore_into(&b, &json, Some(&b.master_password), false).await.unwrap();
        assert_eq!(summary.items, 4);
        assert_eq!(summary.projects, 1);
        assert_eq!(summary.environments, 2);
        assert!(summary.projects_included);

        assert_eq!(item_names(&b).await, item_names(&a).await);
        let s = b.state.lock().await;
        assert_eq!(s.key.as_ref().unwrap().as_slice(), a_key.as_slice());
        let projects = s.db.list_projects().await.unwrap();
        assert_eq!(projects.len(), 1);
        let envs = s.db.list_environments(projects[0].id).await.unwrap();
        assert_eq!(envs.len(), 2);
        let prod = envs.iter().find(|e| e.name == "production").unwrap();
        assert!(prod.is_default);
        let vars = s.db.get_environment_vars(prod.id).await.unwrap();
        assert_eq!(vars.len(), 3);
        // every binding points at a restored, decryptable item
        for var in vars {
            let (_, _, data, _, g) = s
                .db
                .list_items()
                .await
                .unwrap()
                .into_iter()
                .find(|i| Some(i.0) == var.item_id)
                .unwrap();
            let item = decrypt_item(&a_key, var.item_id.unwrap(), &data, g).unwrap();
            assert_eq!(item.name.as_deref(), Some(var.key.as_str()));
        }
        // device settings survive a replace
        assert!(s.db.get_setting("mcp_token").await.unwrap().is_some());
        assert!(pre_restore_path(Path::new(s.db.path())).exists());
        assert!(!sibling_with_suffix(Path::new(s.db.path()), ".restore-tmp").exists());
    }

    #[tokio::test]
    async fn pre_restore_is_discarded_on_next_unlock() {
        let a = unlocked_vault().await;
        let b = unlocked_vault_empty().await;
        let json = export_json(&a).await;
        restore_into(&b, &json, Some(&b.master_password), false).await.unwrap();

        let mut s = b.state.lock().await;
        let pre = pre_restore_path(Path::new(s.db.path()));
        assert!(pre.exists());
        let key = s.key.as_ref().unwrap().clone();
        s.set_key(None);
        assert!(pre.exists(), "locking must not discard it");
        s.set_key(Some(key));
        assert!(!pre.exists());
    }

    #[tokio::test]
    async fn restore_authorization() {
        let a = unlocked_vault().await;
        let json = export_json(&a).await;

        let locked = crate::test_support::locked_vault().await;
        let count_before = locked.state.lock().await.db.list_items().await.unwrap().len();
        assert!(restore_into(&locked, &json, Some(&locked.master_password), false).await.is_err());
        assert!(restore_into(&locked, &json, None, true).await.is_err());

        let b = unlocked_vault().await;
        assert!(restore_into(&b, &json, None, false).await.is_err());
        assert!(restore_into(&b, &json, Some("wrong-password"), false).await.is_err());
        assert!(restore_into(&b, &json, Some(""), false).await.is_err());
        // wrong backup password
        {
            let mut s = b.state.lock().await;
            let err = restore(&mut s, &json, "nope", Some(&b.master_password), false).await.unwrap_err();
            assert!(err.contains("incorrect master password"));
        }
        assert_eq!(item_names(&b).await, item_names(&a).await);
        // locked vault unchanged
        assert_eq!(locked.state.lock().await.db.list_items().await.unwrap().len(), count_before);
    }

    #[tokio::test]
    async fn v1_restore_gives_items_an_owner() {
        let a = unlocked_vault().await;
        let b = unlocked_vault_empty().await;
        // Hand-built v1 file: items + plaintext categories, no meta.
        let (salt, token, items) = {
            let s = a.state.lock().await;
            let (salt, token) = s.db.get_meta().await.unwrap().unwrap();
            let items: Vec<serde_json::Value> = s
                .db
                .list_items()
                .await
                .unwrap()
                .into_iter()
                .map(|(id, t, d, c, g)| {
                    serde_json::json!({"id": id, "item_type": t, "data": d, "created": c, "is_global": g})
                })
                .collect();
            (salt, token, items)
        };
        let v1 = serde_json::json!({
            "version": 1, "created_at": 0, "salt": salt, "token": token,
            "items": items,
            "categories": [{"id": "c1", "name": "Infra", "color": "#fff", "description": null}],
        })
        .to_string();

        let summary = restore_into(&b, &v1, Some(&b.master_password), false).await.unwrap();
        assert_eq!(summary.backup_version, 1);
        assert!(!summary.projects_included);
        assert_eq!(summary.items, 4);
        let name = summary.holding_project.clone().unwrap();
        assert!(name.starts_with("Restored "));

        let s = b.state.lock().await;
        assert!(s.db.list_orphan_item_ids().await.unwrap().is_empty());
        let projects = s.db.list_projects().await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, name);
        assert_eq!(s.db.list_owned_item_ids(projects[0].id).await.unwrap().len(), 3);
        assert_eq!(s.db.list_categories().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn merge_adds_projects_with_current_key() {
        let a = unlocked_vault().await;
        let b = unlocked_vault_empty().await;
        let json = export_json(&a).await;
        let b_key = b.state.lock().await.key.as_ref().unwrap().clone();

        let summary = restore_into(&b, &json, None, true).await.unwrap();
        assert_eq!(summary.items, 4);
        let s = b.state.lock().await;
        assert_eq!(s.key.as_ref().unwrap().as_slice(), b_key.as_slice());
        let projects = s.db.list_projects().await.unwrap();
        assert_eq!(projects.len(), 1);
        let envs = s.db.list_environments(projects[0].id).await.unwrap();
        assert_eq!(envs.len(), 2);
        for (id, _, data, _, g) in s.db.list_items().await.unwrap() {
            assert!(decrypt_item(&b_key, id, &data, g).is_ok());
        }
        // settings and key material kept (merge copies the live DB)
        assert!(s.db.get_setting("mcp_token").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn merge_into_existing_project_keeps_existing_environments() {
        let a = unlocked_vault().await;
        let b = unlocked_vault().await; // also has project "demo"
        let json = export_json(&a).await;
        restore_into(&b, &json, None, true).await.unwrap();
        let s = b.state.lock().await;
        let projects = s.db.list_projects().await.unwrap();
        assert_eq!(projects.len(), 1);
        let envs = s.db.list_environments(projects[0].id).await.unwrap();
        assert_eq!(envs.len(), 2);
        assert_eq!(envs.iter().filter(|e| e.is_default).count(), 1);
    }

    #[tokio::test]
    async fn failure_before_swap_leaves_vault_untouched() {
        let a = unlocked_vault().await;
        let b = unlocked_vault().await;
        let before = item_names(&b).await;

        // Valid, authentic backup whose metadata references a missing item.
        let mut file: BackupFile = serde_json::from_str(&export_json(&a).await).unwrap();
        let key = a.state.lock().await.key.as_ref().unwrap().clone();
        let mut meta = decode_meta(&key, file.meta.as_ref().unwrap()).unwrap();
        meta.environment_vars[0].item_id = Some(9_999);
        file.meta = Some(encode_meta(&key, &meta).unwrap());
        let bad = serde_json::to_string(&file).unwrap();

        let err = restore_into(&b, &bad, Some(&b.master_password), false).await.unwrap_err();
        assert!(err.contains("inconsistent"), "{err}");
        assert_eq!(item_names(&b).await, before);
        let s = b.state.lock().await;
        let live = Path::new(s.db.path());
        assert!(!sibling_with_suffix(live, ".restore-tmp").exists());
        assert!(!pre_restore_path(live).exists());
        assert!(s.db.list_projects().await.is_ok());
    }

    #[tokio::test]
    async fn failed_second_rename_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("vault.db");
        let pre = pre_restore_path(&live);
        let db = VaultDb::open(live.to_str().unwrap()).await.unwrap();
        db.set_setting("marker", "original").await.unwrap();
        db.close().await.unwrap();

        // The staging file does not exist, so the second rename fails.
        let missing = sibling_with_suffix(&live, ".restore-tmp");
        let err = swap_files(&live, &missing, &pre).unwrap_err();
        assert!(err.contains("current vault was kept"), "{err}");
        assert!(!pre.exists());

        let reopened = VaultDb::open(live.to_str().unwrap()).await.unwrap();
        assert_eq!(reopened.get_setting("marker").await.unwrap().as_deref(), Some("original"));
    }
}
