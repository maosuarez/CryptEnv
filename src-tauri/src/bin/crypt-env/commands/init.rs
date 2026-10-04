//! `crypt-env init [NAME] [--path <PATH>]` — registers (or links) the vault
//! project for the current directory, records this directory as its root,
//! and writes `.crypt-env.yaml` here.
//!
//! An existing project bound to another directory is never taken over
//! (use `config --relink`); an existing project without a root adopts this
//! directory only after the same consent flow `config` uses.

use clap::Args;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::client::{self, CliError, Project, VaultApi};
use crate::commands::config::RoutingDiff;
use crate::commands::scope::{self, manifest, Binding};
use crate::paths;

#[derive(Args)]
pub struct InitArgs {
    /// Project name (defaults to the legacy crypt-env.json project, else the
    /// current folder name)
    pub name: Option<String>,

    /// Injection target for the default environment: a directory (its `.env`
    /// is used) or a `.env*` file. Defaults to `.env` in this directory.
    #[arg(long)]
    pub path: Option<PathBuf>,
}

pub struct InitReport {
    pub project: String,
    pub created: bool,
    pub manifest_path: PathBuf,
    pub target: String,
}

pub fn run(args: InitArgs) -> Result<(), CliError> {
    let cwd = std::env::current_dir()?;
    if cwd.join(manifest::FILE_NAME).is_file() {
        eprintln!(
            "warning: {} already exists here — nothing was changed. Use `crypt-env config` to sync it.",
            manifest::FILE_NAME
        );
        return Ok(());
    }
    let api = &client::Live;
    let mut adopt_confirmed = false;
    if let Existing::Adopt { lines, .. } = check(api, &cwd, args.name.as_deref())? {
        for line in &lines {
            eprintln!("{line}");
        }
        if !std::io::stdin().is_terminal() {
            return Err(CliError::Config(
                "adopting this directory changes where secrets are written and needs an interactive confirmation".into(),
            ));
        }
        api.ensure_session()?;
        if !crate::prompts::confirm("Bind this project to the current directory?") {
            return Err(CliError::Config("cancelled — the vault was not changed".into()));
        }
        adopt_confirmed = true;
    }
    let r = execute(api, &cwd, args.name.as_deref(), args.path.as_deref(), adopt_confirmed)?;
    eprintln!(
        "{} project '{}' (root {}), default environment → {}",
        if r.created { "Created" } else { "Linked existing" },
        r.project,
        cwd.display(),
        r.target
    );
    eprintln!("Wrote {}", r.manifest_path.display());
    Ok(())
}

/// What `init` would do about a same-name vault project.
pub enum Existing {
    /// No such project: it is created with this directory as root.
    New,
    /// Already bound to this directory.
    Linked,
    /// Exists without a root; binding it here needs consent. `lines` is the
    /// diff to show.
    Adopt { project: String, lines: Vec<String> },
}

/// Read-only: classifies the same-name project. A project bound to another
/// directory is the root-binding error.
pub fn check(api: &dyn VaultApi, dir: &Path, name: Option<&str>) -> Result<Existing, CliError> {
    let name = resolve_name(dir, name)?;
    let Some(project) = api.find_project(&name)? else { return Ok(Existing::New) };
    match scope::ensure_bound(dir, &project)? {
        Binding::Bound => Ok(Existing::Linked),
        _ => {
            let diff = RoutingDiff { root: Some((None, paths::to_host(dir))), envs: vec![] };
            Ok(Existing::Adopt { project: project.name.clone(), lines: diff.render(&project.name) })
        }
    }
}

/// Explicit name, else the legacy `crypt-env.json` project, else the folder name.
fn resolve_name(dir: &Path, name: Option<&str>) -> Result<String, CliError> {
    let legacy = dir.join(scope::LEGACY_CONFIG_FILE_NAME);
    let legacy_name = if legacy.is_file() { scope::parse_legacy_config(&legacy).ok().map(|c| c.project) } else { None };
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => Ok(n.to_string()),
        None => match legacy_name {
            Some(n) => Ok(n),
            None => dir
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
                .ok_or_else(|| CliError::Config("cannot derive a project name from this directory".into())),
        },
    }
}

/// Core of `init`, shared with the TUI. `dir` becomes the project root.
/// `adopt_confirmed` is the user's consent to bind an existing, rootless
/// project to `dir` (see [`check`]).
pub fn execute(
    api: &dyn VaultApi,
    dir: &Path,
    name: Option<&str>,
    path: Option<&Path>,
    adopt_confirmed: bool,
) -> Result<InitReport, CliError> {
    if dir.join(manifest::FILE_NAME).is_file() {
        return Err(CliError::Config(format!("{} already exists in {}", manifest::FILE_NAME, dir.display())));
    }
    if let Existing::Adopt { project, .. } = check(api, dir, name)? {
        if !adopt_confirmed {
            return Err(CliError::Config(format!(
                "project '{project}' has no root yet; binding it to this directory needs confirmation"
            )));
        }
    }
    let name = resolve_name(dir, name)?;
    let target = default_target(dir, path);
    let root_host = paths::to_host(dir);

    let (project_id, created) = match api.find_project(&name)? {
        Some(p) => {
            api.save_project(&project_body(&p, &root_host))?;
            (p.id, false)
        }
        None => {
            let id = api.save_project(&serde_json::json!({
                "id": 0,
                "name": name,
                "template": "generic",
                "categories": [],
                "rootPath": root_host,
            }))?;
            (id, true)
        }
    };

    // Ensure the default environment targets `target`.
    let project = fetch_by_id(api, project_id)?;
    if let Some(env) = project.environments.iter().find(|e| e.is_default).or(project.environments.first()) {
        if !env.paths.iter().any(|p| p == &target) {
            let mut paths = if created { Vec::new() } else { env.paths.clone() };
            paths.push(target.clone());
            api.save_environment(&client::environment_body(env, env.is_default, &paths))?;
        }
    }

    let project = fetch_by_id(api, project_id)?;
    let m = manifest::from_project(&project, &paths::to_local);
    let manifest_path = manifest::write_file(dir, &m).map_err(CliError::Config)?;
    Ok(InitReport { project: project.name, created, manifest_path, target })
}

/// Body that updates `p` in place, only (re)setting its root.
pub fn project_body(p: &Project, root_host: &str) -> serde_json::Value {
    serde_json::json!({
        "id": p.id,
        "name": p.name,
        "description": p.description,
        "template": p.template,
        "categories": p.categories,
        "rootPath": root_host,
    })
}

fn fetch_by_id(api: &dyn VaultApi, id: i64) -> Result<Project, CliError> {
    api.fetch_projects()?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| CliError::Api(format!("project {id} vanished after saving")))
}

/// `--path` → the stored target: relative to `root` with `/` separators when
/// inside it (a directory gets `/.env` appended), else a host absolute path.
pub fn default_target(root: &Path, path: Option<&Path>) -> String {
    let Some(p) = path else { return ".env".to_string() };
    let abs = if p.is_absolute() { p.to_path_buf() } else { root.join(p) };
    let is_file = abs
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with(".env") && !abs.is_dir())
        .unwrap_or(false);
    let file = if is_file { abs } else { abs.join(".env") };
    let root_s = root.to_string_lossy();
    let normalized = normalize(&file);
    match manifest::relativize(&root_s, &normalized.to_string_lossy()) {
        Some(rel) => rel,
        None => paths::to_host(&normalized),
    }
}

/// Lexically removes `.` and resolves `..` components.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_target_variants() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("app")).unwrap();
        assert_eq!(default_target(root, None), ".env");
        assert_eq!(default_target(root, Some(Path::new("./app"))), "app/.env");
        assert_eq!(default_target(root, Some(Path::new("app/.env.local"))), "app/.env.local");
        assert_eq!(default_target(root, Some(Path::new("new-dir"))), "new-dir/.env");
        let outside = default_target(root, Some(Path::new("../elsewhere")));
        assert!(outside.ends_with(".env") && outside != "elsewhere/.env", "{outside}");
    }

    use crate::testing::{env, project, FakeVault};

    fn run_init(vault: &FakeVault, dir: &Path, adopt: bool) -> Result<InitReport, CliError> {
        execute(vault, dir, Some("backend"), None, adopt)
    }

    #[test]
    fn existing_project_bound_elsewhere_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let vault = FakeVault::new(vec![project("backend", Some("/home/u/backend"), vec![env(1, "default", true, &[".env"])])]);
        let err = run_init(&vault, dir.path(), true).err().unwrap().to_string();
        assert!(err.contains("/home/u/backend") && err.contains("--relink"), "{err}");
        assert!(vault.mutations().is_empty(), "{:?}", vault.calls.borrow());
        assert!(!dir.path().join(manifest::FILE_NAME).exists());
    }

    #[test]
    fn unbound_project_needs_confirmation_before_adopting() {
        let dir = tempfile::tempdir().unwrap();
        let vault = FakeVault::new(vec![project("backend", None, vec![env(1, "default", true, &[".env"])])]);
        assert!(matches!(check(&vault, dir.path(), Some("backend")), Ok(Existing::Adopt { .. })));
        assert!(run_init(&vault, dir.path(), false).is_err());
        assert!(vault.mutations().is_empty());
        assert!(run_init(&vault, dir.path(), true).is_ok());
        assert!(vault.mutations().contains(&"save_project".to_string()));
    }

    #[test]
    fn bound_here_links_and_new_project_is_created_without_consent() {
        let dir = tempfile::tempdir().unwrap();
        let host = paths::to_host(dir.path());
        let vault = FakeVault::new(vec![project("backend", Some(&host), vec![env(1, "default", true, &[".env"])])]);
        assert!(matches!(check(&vault, dir.path(), Some("backend")), Ok(Existing::Linked)));

        let dir2 = tempfile::tempdir().unwrap();
        let empty = FakeVault::new(vec![]);
        assert!(matches!(check(&empty, dir2.path(), Some("fresh")), Ok(Existing::New)));
        let r = execute(&empty, dir2.path(), Some("fresh"), None, false).unwrap();
        assert!(r.created);
    }
}
