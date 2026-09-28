//! `crypt-env config` — bidirectional sync between `.crypt-env.yaml` and the
//! vault project (cli-tui-parity design D2). Last modified wins: the file's
//! mtime against `max(project.updated, environments[*].updated)`.

use clap::Args;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use crate::client::{self, CliError, Project};
use crate::commands::scope::{self, manifest, Manifest};
use crate::paths;

#[derive(Args)]
pub struct ConfigArgs {}

#[derive(Debug, PartialEq, Eq)]
pub enum Direction {
    Push,
    Pull,
}

/// Local strictly newer → push; otherwise the vault wins.
pub fn decide(local_mtime: u64, remote_updated: u64) -> Direction {
    if local_mtime > remote_updated {
        Direction::Push
    } else {
        Direction::Pull
    }
}

/// The vault-side "last modified": environment edits don't bump the
/// project's own `updated`, so take the max. Unparseable stamps count as 0.
pub fn remote_updated(p: &Project) -> u64 {
    let parse = |s: &str| s.trim().parse::<u64>().unwrap_or(0);
    p.environments.iter().map(|e| parse(&e.updated)).chain([parse(&p.updated)]).max().unwrap_or(0)
}

pub enum Outcome {
    Pushed { created: bool, untracked_envs: Vec<String> },
    Pulled,
    InSync,
}

pub fn run(_args: ConfigArgs) -> Result<(), CliError> {
    let (ws, m) = scope::yaml_workspace()?;
    match execute(&ws.root, &ws.config_path, &m)? {
        Outcome::Pushed { created, untracked_envs } => {
            if created {
                eprintln!("Project '{}' did not exist in the vault — created it from {}", m.project.name, manifest::FILE_NAME);
            }
            eprintln!("Vault updated from {}", ws.config_path.display());
            if !untracked_envs.is_empty() {
                eprintln!(
                    "note: vault environment(s) not in the file were kept (they may hold secrets): {} — delete them from the GUI if intended",
                    untracked_envs.join(", ")
                );
            }
        }
        Outcome::Pulled => eprintln!("{} updated from the vault", ws.config_path.display()),
        Outcome::InSync => eprintln!("{} is already in sync with the vault", ws.config_path.display()),
    }
    Ok(())
}

/// Core of `config`, shared with the TUI.
pub fn execute(root: &Path, manifest_path: &Path, m: &Manifest) -> Result<Outcome, CliError> {
    let local = std::fs::metadata(manifest_path)?
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let Some(project) = client::find_project(&m.project.name)? else {
        push(root, m, None)?;
        return Ok(Outcome::Pushed { created: true, untracked_envs: vec![] });
    };
    let remote = remote_updated(&project);
    match decide(local, remote) {
        Direction::Push => {
            let untracked_envs = push(root, m, Some(&project))?;
            Ok(Outcome::Pushed { created: false, untracked_envs })
        }
        Direction::Pull => {
            // A project created in the GUI without a root adopts this one.
            if project.root_path.is_none() {
                client::save_project(&crate::commands::init::project_body(&project, &paths::to_host(root)))?;
            }
            let pulled = manifest::from_project(&project, &paths::to_local);
            if &pulled == m {
                return Ok(Outcome::InSync);
            }
            let path = manifest::write_file(root, &pulled).map_err(CliError::Config)?;
            // Stamp the file with the vault's time so the next run doesn't
            // mistake our own write for a local edit.
            let f = std::fs::OpenOptions::new().write(true).open(&path)?;
            f.set_modified(UNIX_EPOCH + Duration::from_secs(remote))?;
            Ok(Outcome::Pulled)
        }
    }
}

/// Pushes `m` into the vault (creating the project when `existing` is
/// `None`). Returns vault environments absent from the manifest.
fn push(root: &Path, m: &Manifest, existing: Option<&Project>) -> Result<Vec<String>, CliError> {
    let root_host = paths::to_host(root);
    client::ensure_categories(&m.project.categories)?;
    let project_id = client::save_project(&serde_json::json!({
        "id": existing.map(|p| p.id).unwrap_or(0),
        "name": m.project.name,
        "description": m.project.description,
        "template": existing.map(|p| p.template.as_str()).unwrap_or("generic"),
        "categories": m.project.categories,
        "rootPath": root_host,
        "initialEnvironment": m.default_environment().map(|e| e.name.as_str()),
    }))?;
    let project = client::fetch_projects()?
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| CliError::Api("project vanished after saving".into()))?;

    let default_name = m.default_environment().map(|e| e.name.to_lowercase());
    for env in &m.project.environments {
        let host_paths: Vec<String> = env
            .paths
            .iter()
            .map(|p| if manifest::is_absolute_any(p) { paths::to_host(Path::new(p)) } else { p.clone() })
            .collect();
        let is_default = default_name.as_deref() == Some(&env.name.to_lowercase());
        let current = project.environments.iter().find(|e| e.name.eq_ignore_ascii_case(&env.name));
        let body = match current {
            Some(cur) => client::environment_body(cur, is_default, &host_paths),
            None => serde_json::json!({
                "id": 0,
                "projectId": project_id,
                "name": env.name,
                "isDefault": is_default,
                "paths": host_paths,
                "vars": [],
            }),
        };
        client::save_environment(&body)?;
    }

    Ok(project
        .environments
        .iter()
        .filter(|e| !m.project.environments.iter().any(|me| me.name.eq_ignore_ascii_case(&e.name)))
        .map(|e| e.name.clone())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Environment;

    #[test]
    fn decide_last_modified_wins_vault_on_tie() {
        assert_eq!(decide(11, 10), Direction::Push);
        assert_eq!(decide(10, 10), Direction::Pull);
        assert_eq!(decide(9, 10), Direction::Pull);
    }

    #[test]
    fn remote_updated_takes_max_over_environments() {
        let env = |u: &str| Environment {
            id: 1,
            project_id: 1,
            name: "x".into(),
            is_default: false,
            paths: vec![],
            vars: vec![],
            created: "0".into(),
            updated: u.into(),
        };
        let p = Project {
            id: 1,
            name: "p".into(),
            description: None,
            template: "generic".into(),
            created: "0".into(),
            updated: "100".into(),
            environments: vec![env("50"), env("250"), env("garbage")],
            categories: vec![],
            root_path: None,
        };
        assert_eq!(remote_updated(&p), 250);
    }
}
