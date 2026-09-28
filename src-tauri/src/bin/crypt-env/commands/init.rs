//! `crypt-env init [NAME] [--path <PATH>]` — registers (or links) the vault
//! project for the current directory, records this directory as its root,
//! and writes `.crypt-env.yaml` here.

use clap::Args;
use std::path::{Path, PathBuf};

use crate::client::{self, CliError, Project};
use crate::commands::scope::{self, manifest};
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
    let r = execute(&cwd, args.name.as_deref(), args.path.as_deref())?;
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

/// Core of `init`, shared with the TUI. `dir` becomes the project root.
pub fn execute(dir: &Path, name: Option<&str>, path: Option<&Path>) -> Result<InitReport, CliError> {
    if dir.join(manifest::FILE_NAME).is_file() {
        return Err(CliError::Config(format!("{} already exists in {}", manifest::FILE_NAME, dir.display())));
    }
    let legacy = dir.join(scope::LEGACY_CONFIG_FILE_NAME);
    let legacy_name = if legacy.is_file() { scope::parse_legacy_config(&legacy).ok().map(|c| c.project) } else { None };
    let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_string(),
        None => match legacy_name {
            Some(n) => n,
            None => dir
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
                .ok_or_else(|| CliError::Config("cannot derive a project name from this directory".into()))?,
        },
    };
    let target = default_target(dir, path);
    let root_host = paths::to_host(dir);

    let (project_id, created) = match client::find_project(&name)? {
        Some(p) => {
            client::save_project(&project_body(&p, &root_host))?;
            (p.id, false)
        }
        None => {
            let id = client::save_project(&serde_json::json!({
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
    let project = fetch_by_id(project_id)?;
    if let Some(env) = project.environments.iter().find(|e| e.is_default).or(project.environments.first()) {
        if !env.paths.iter().any(|p| p == &target) {
            let mut paths = if created { Vec::new() } else { env.paths.clone() };
            paths.push(target.clone());
            client::save_environment(&client::environment_body(env, env.is_default, &paths))?;
        }
    }

    let project = fetch_by_id(project_id)?;
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

fn fetch_by_id(id: i64) -> Result<Project, CliError> {
    client::fetch_projects()?
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
}
