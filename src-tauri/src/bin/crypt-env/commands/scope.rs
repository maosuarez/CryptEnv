//! Workspace discovery and project/environment scope resolution shared by
//! every project-aware CLI command.
//!
//! A workspace is the directory holding `.crypt-env.yaml` (the project root,
//! cli-tui-parity design D1/D8), found by searching from the current
//! directory upward (same convention as `.git`). A legacy `crypt-env.json`
//! is still honored when no YAML is found, with a one-line migration notice.
//!
//! Environment resolution: `--env` flag → the manifest's default environment
//! (if it exists in the vault) → the vault project's `isDefault` environment.

use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::client::{self, CliError, Environment, Project};

pub use crypt_env_lib::project::manifest::{self, Manifest};

pub const LEGACY_CONFIG_FILE_NAME: &str = "crypt-env.json";

/// Schema of the legacy `crypt-env.json`.
#[derive(Deserialize, Debug, Clone)]
pub struct LegacyConfig {
    pub project: String,
    #[serde(default)]
    pub environment: Option<String>,
}

pub struct Workspace {
    /// Project root: the directory holding the config file.
    pub root: PathBuf,
    pub config_path: PathBuf,
    pub project: String,
    pub default_env: Option<String>,
    /// `None` for a legacy `crypt-env.json` workspace.
    pub manifest: Option<Manifest>,
}

/// Nearest `.crypt-env.yaml` (preferred) or legacy `crypt-env.json` from
/// `start` upward. At the same level the YAML wins.
pub fn find_config(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start.to_path_buf());
    while let Some(d) = dir {
        for name in [manifest::FILE_NAME, LEGACY_CONFIG_FILE_NAME] {
            let candidate = d.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    None
}

pub fn parse_legacy_config(path: &Path) -> Result<LegacyConfig, CliError> {
    let content = std::fs::read_to_string(path)?;
    let config: LegacyConfig = serde_json::from_str(&content)
        .map_err(|e| CliError::Config(format!("{}: invalid JSON — {e}", path.display())))?;
    if config.project.trim().is_empty() {
        return Err(CliError::Config(format!("{}: 'project' is required and must not be empty", path.display())));
    }
    Ok(config)
}

/// Parses a config file found by [`find_config`] into a [`Workspace`].
pub fn load_config(path: &Path) -> Result<Workspace, CliError> {
    let root = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    if path.file_name().and_then(|n| n.to_str()) == Some(LEGACY_CONFIG_FILE_NAME) {
        let legacy = parse_legacy_config(path)?;
        return Ok(Workspace {
            root,
            config_path: path.to_path_buf(),
            project: legacy.project,
            default_env: legacy.environment,
            manifest: None,
        });
    }
    let m = manifest::read_file(path).map_err(CliError::Config)?;
    Ok(Workspace {
        root,
        config_path: path.to_path_buf(),
        project: m.project.name.clone(),
        default_env: m.default_environment().map(|e| e.name.clone()),
        manifest: Some(m),
    })
}

/// The workspace for the current directory, or a clear error suggesting
/// `crypt-env init`. Prints the migration notice for a legacy config.
pub fn workspace() -> Result<Workspace, CliError> {
    let cwd = std::env::current_dir()?;
    let path = find_config(&cwd).ok_or_else(|| {
        CliError::Config(format!(
            "no {} found in this directory or any parent — run `crypt-env init` first",
            manifest::FILE_NAME
        ))
    })?;
    let ws = load_config(&path)?;
    if ws.manifest.is_none() {
        eprintln!(
            "notice: using legacy {} — run `crypt-env init` in {} to migrate to {}",
            LEGACY_CONFIG_FILE_NAME,
            ws.root.display(),
            manifest::FILE_NAME
        );
    }
    Ok(ws)
}

/// Like [`workspace`] but requires a `.crypt-env.yaml` (for `config`).
pub fn yaml_workspace() -> Result<(Workspace, Manifest), CliError> {
    let ws = workspace()?;
    match ws.manifest.clone() {
        Some(m) => Ok((ws, m)),
        None => Err(CliError::Config(format!(
            "{} is not supported here — run `crypt-env init` to create {}",
            LEGACY_CONFIG_FILE_NAME,
            manifest::FILE_NAME
        ))),
    }
}

/// The vault project for `ws`, or an error suggesting `init`/`config`.
pub fn vault_project(ws: &Workspace) -> Result<Project, CliError> {
    client::find_project(&ws.project)?.ok_or_else(|| {
        CliError::Config(format!(
            "project '{}' does not exist in the vault — run `crypt-env config` (or `crypt-env init`) to register it",
            ws.project
        ))
    })
}

/// Picks the environment per the module-level resolution order.
pub fn pick_environment<'a>(
    project: &'a Project,
    ws_default: Option<&str>,
    flag: Option<&str>,
) -> Result<&'a Environment, CliError> {
    let by_name = |n: &str| {
        let lower = n.to_lowercase();
        project.environments.iter().find(|e| e.name.to_lowercase() == lower)
    };
    if let Some(name) = flag {
        return by_name(name).ok_or_else(|| {
            CliError::Config(format!("environment '{name}' does not exist in project '{}'", project.name))
        });
    }
    if let Some(env) = ws_default.and_then(by_name) {
        return Ok(env);
    }
    project
        .environments
        .iter()
        .find(|e| e.is_default)
        .or_else(|| project.environments.first())
        .ok_or_else(|| CliError::Config(format!("project '{}' has no environments", project.name)))
}

/// Workspace + vault project + selected environment, in one call.
pub fn resolve(env_flag: Option<&str>) -> Result<(Workspace, Project, Environment), CliError> {
    let ws = workspace()?;
    let project = vault_project(&ws)?;
    let env = pick_environment(&project, ws.default_env.as_deref(), env_flag)?.clone();
    Ok((ws, project, env))
}

/// Local filesystem path of a configured environment target: relative paths
/// are under the workspace root, absolute ones are translated from the host
/// form (see `paths`).
pub fn local_target(root: &Path, configured: &str) -> PathBuf {
    if manifest::is_absolute_any(configured) {
        PathBuf::from(crate::paths::to_local(configured))
    } else {
        let mut p = root.to_path_buf();
        for part in configured.split(['/', '\\']).filter(|c| !c.is_empty() && *c != ".") {
            p.push(part);
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(name: &str, default: bool) -> Environment {
        Environment {
            id: 1,
            project_id: 1,
            name: name.into(),
            is_default: default,
            paths: vec![],
            vars: vec![],
            created: "0".into(),
            updated: "0".into(),
        }
    }

    fn project() -> Project {
        Project {
            id: 1,
            name: "svc".into(),
            description: None,
            template: "generic".into(),
            created: "0".into(),
            updated: "0".into(),
            environments: vec![env("dev", false), env("default", true)],
            categories: vec![],
            root_path: None,
        }
    }

    #[test]
    fn find_config_prefers_yaml_and_walks_up() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(LEGACY_CONFIG_FILE_NAME), r#"{"project":"old"}"#).unwrap();
        std::fs::write(dir.path().join(manifest::FILE_NAME), "project:\n  name: new\n").unwrap();
        let nested = dir.path().join("a/b");
        std::fs::create_dir_all(&nested).unwrap();
        let found = find_config(&nested).unwrap();
        assert!(found.ends_with(manifest::FILE_NAME));
        let ws = load_config(&found).unwrap();
        assert_eq!(ws.project, "new");
        assert_eq!(ws.root, dir.path());
    }

    #[test]
    fn legacy_config_is_loaded_without_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(LEGACY_CONFIG_FILE_NAME);
        std::fs::write(&p, r#"{"project":"old","environment":"prod"}"#).unwrap();
        let ws = load_config(&p).unwrap();
        assert_eq!(ws.project, "old");
        assert_eq!(ws.default_env.as_deref(), Some("prod"));
        assert!(ws.manifest.is_none());
        std::fs::write(&p, r#"{"project":""}"#).unwrap();
        assert!(load_config(&p).is_err());
    }

    #[test]
    fn invalid_yaml_reports_path() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(manifest::FILE_NAME);
        std::fs::write(&p, "project: [").unwrap();
        let err = load_config(&p).err().unwrap().to_string();
        assert!(err.contains(".crypt-env.yaml"), "{err}");
    }

    #[test]
    fn pick_environment_order() {
        let p = project();
        assert_eq!(pick_environment(&p, None, Some("DEV")).unwrap().name, "dev");
        assert_eq!(pick_environment(&p, Some("dev"), None).unwrap().name, "dev");
        assert_eq!(pick_environment(&p, Some("missing"), None).unwrap().name, "default");
        assert_eq!(pick_environment(&p, None, None).unwrap().name, "default");
        assert!(pick_environment(&p, None, Some("nope")).is_err());
    }

    #[test]
    fn local_target_joins_relative_paths() {
        let root = Path::new("/r");
        assert_eq!(local_target(root, "./apps\\api/.env"), PathBuf::from("/r/apps/api/.env"));
        assert_eq!(local_target(root, "/abs/.env"), PathBuf::from("/abs/.env"));
    }
}
