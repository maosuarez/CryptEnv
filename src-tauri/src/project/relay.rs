//! Whole-project sharing via the encrypted relay (issue #4). Two pure
//! orchestrators shared by the HTTP handlers (`api::mod`) and the Tauri
//! commands (`project::relay_commands`), following the same
//! decoupling CLAUDE.md requires elsewhere: this module encrypts/decrypts
//! with the vault key and hands `db` only plain data (ciphertext strings,
//! never the key) — see `db::insert_received_project`.

use std::collections::HashMap;

use crate::crypto::VaultKey;
use crate::db::{ReceivedEnvironment, ReceivedProjectItem, ReceivedVar, VaultDb};
use crate::share::package::PlainItem;
use crate::share::relay::{EnvironmentBundle, ProjectBundle, ProjectBundleVar};
use crate::vault::VaultItem;

use super::list_projects;

/// Refuse to build a bundle whose pre-encryption JSON exceeds this size
/// (D8). The relay's actual request-size limit isn't verifiable from this
/// repo, so this is a deterministic local guard rather than a guess at a
/// third party's limit — a 200-item project lands around 100 KiB, so 1 MiB
/// is comfortable headroom for realistic projects.
const MAX_BUNDLE_BYTES: usize = 1024 * 1024;

fn now_ts() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

/// Builds a `ProjectBundle` for `project_id`, restricted to `environment_ids`.
/// Items referenced by more than one selected environment are deduped by
/// name and hoisted to the bundle root (D1); an `environment_vars` row whose
/// `item_id` no longer resolves to a real item is skipped rather than
/// failing the whole send (mirrors the pre-existing item-relay behaviour).
pub async fn build_project_bundle(
    db: &VaultDb,
    vault_key: &VaultKey,
    project_id: i64,
    environment_ids: &[i64],
) -> Result<ProjectBundle, String> {
    let project = list_projects(db)
        .await?
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| "project not found".to_string())?;

    let selected: Vec<_> = project
        .environments
        .into_iter()
        .filter(|e| environment_ids.contains(&e.id))
        .collect();

    if selected.is_empty() {
        return Err("no environments selected".to_string());
    }

    // Decrypt every vault item once, keyed by id (mirrors the existing
    // item-relay send handler's pattern).
    let raw = db.list_items().await?;
    let mut items_by_id: HashMap<i64, VaultItem> = HashMap::new();
    for (item_id, _, data, _, _) in &raw {
        if let Ok(json) = crate::crypto::decrypt(vault_key, data) {
            if let Ok(item) = serde_json::from_slice::<VaultItem>(&json) {
                items_by_id.insert(*item_id, item);
            }
        }
    }

    let mut bundled: HashMap<String, PlainItem> = HashMap::new();
    let mut environments = Vec::with_capacity(selected.len());

    for env in &selected {
        let mut vars = Vec::with_capacity(env.vars.len());
        for v in &env.vars {
            let item = match items_by_id.get(&v.item_id) {
                Some(it) => it,
                // Dangling reference (item deleted after linking) — skip
                // cleanly rather than fail the whole send.
                None => continue,
            };
            let item_name = item.name.clone().unwrap_or_default();
            bundled.entry(item_name.clone()).or_insert_with(|| PlainItem {
                item_type: item.item_type.clone(),
                name: item_name.clone(),
                value: item.value.clone(),
                username: item.username.clone(),
                password: item.password.clone(),
                url: item.url.clone(),
                notes: item.notes.clone(),
                category: item.categories.clone().and_then(|c| c.into_iter().next()),
                command: item.command.clone(),
            });
            vars.push(ProjectBundleVar { key: v.key.clone(), item_name });
        }
        environments.push(EnvironmentBundle {
            name: env.name.clone(),
            is_default: env.is_default,
            vars,
        });
    }

    let bundle = ProjectBundle {
        kind: ProjectBundle::KIND.to_string(),
        version: ProjectBundle::VERSION,
        name: project.name.clone(),
        description: project.description.clone(),
        template: project.template.clone(),
        environments,
        items: bundled.into_values().collect(),
    };

    let json_len = zeroize::Zeroizing::new(serde_json::to_vec(&bundle).map_err(|e| e.to_string())?).len();
    if json_len > MAX_BUNDLE_BYTES {
        return Err(format!(
            "project bundle too large ({} KiB, max {} KiB) — share fewer environments",
            json_len / 1024,
            MAX_BUNDLE_BYTES / 1024
        ));
    }

    Ok(bundle)
}

/// Result of a successful project-relay receive.
#[derive(Debug)]
pub struct ReceivedProject {
    pub project_id: i64,
    pub project_name: String,
    pub environment_names: Vec<String>,
    pub item_count: usize,
}

/// Recreates `bundle` as a brand-new project in the receiver's vault, in one
/// all-or-nothing transaction (D7). `name_override` lets the caller rename
/// on a collision retry (D5) instead of guessing a name here.
///
/// Default-environment promotion: if the sender deselected the default
/// environment, none of the bundled environments has `is_default = true` —
/// the first one in the bundle is promoted so the receiver never ends up
/// with a project that has no default environment.
pub async fn receive_project_bundle(
    db: &VaultDb,
    vault_key: &VaultKey,
    bundle: ProjectBundle,
    name_override: Option<String>,
) -> Result<ReceivedProject, String> {
    let project_name = name_override.unwrap_or_else(|| bundle.name.clone());

    // Issue #7's name validation is enforced in `save_project` /
    // `save_environment`, which this path deliberately bypasses (it writes
    // the whole project in one transaction via `insert_received_project`).
    // Re-assert it here: every name in the bundle is remote, attacker-
    // controllable input, and this is the only layer-1 check on the receive
    // path. `fsguard::resolve_within` remains the independent layer 2 at the
    // write sites, so a name slipping past here still cannot escape an
    // output directory — but an unvalidated name must not reach the DB in
    // the first place.
    crate::project::validate_project_name(&project_name)?;
    for env in &bundle.environments {
        crate::project::validate_environment_name(&env.name)?;
    }

    let created = now_ts();

    // Encrypt each unique item with the RECEIVER's key — items never cross
    // vaults as ciphertext, only as the decrypted `PlainItem`s already
    // carried inside the bundle.
    let mut items = Vec::with_capacity(bundle.items.len());
    for plain in &bundle.items {
        let (item_type, ciphertext) =
            crate::share::build_encrypted_item(plain, vault_key, &created).map_err(|e| e.to_string())?;
        items.push(ReceivedProjectItem {
            name: plain.name.clone(),
            item_type,
            ciphertext,
            created: created.clone(),
        });
    }

    let mut environments: Vec<ReceivedEnvironment> = bundle
        .environments
        .iter()
        .map(|e| ReceivedEnvironment {
            name: e.name.clone(),
            is_default: e.is_default,
            vars: e
                .vars
                .iter()
                .map(|v| ReceivedVar { key: v.key.clone(), item_name: v.item_name.clone() })
                .collect(),
        })
        .collect();

    if !environments.iter().any(|e| e.is_default) {
        if let Some(first) = environments.first_mut() {
            first.is_default = true;
        }
    }

    let environment_names: Vec<String> = environments.iter().map(|e| e.name.clone()).collect();
    let item_count = items.len();

    let inserted = db
        .insert_received_project(&project_name, bundle.description.as_deref(), &bundle.template, &items, &environments)
        .await?;

    Ok(ReceivedProject {
        project_id: inserted.project_id,
        project_name,
        environment_names,
        item_count,
    })
}
