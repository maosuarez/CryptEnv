//! `crypt-env config` — bidirectional sync between `.crypt-env.yaml` and the
//! vault project (cli-tui-parity design D2). Last modified wins: the file's
//! mtime against `max(project.updated, environments[*].updated)`.
//!
//! A vault project with a root is bound to that directory
//! (harden-cli-manifest-and-sessions D1): `config` refuses to run from any
//! other directory unless `--relink` is given, and any change to where
//! secrets get written (root or environment paths) needs a live session and
//! an explicit confirmation.

use clap::Args;
use std::io::IsTerminal;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use crate::client::{self, CliError, Project, VaultApi};
use crate::commands::scope::{self, manifest, Binding, Manifest};
use crate::paths;

#[derive(Args)]
pub struct ConfigArgs {
    /// Move the vault project to this directory. Changes where secrets are
    /// written, so it always asks for confirmation.
    #[arg(long)]
    pub relink: bool,

    /// Confirm secret-routing changes without prompting (required when
    /// stdin is not a terminal)
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
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

// ─── Secret-routing diff ──────────────────────────────────────────────────────

/// A path added to an environment.
#[derive(Debug, PartialEq, Eq)]
pub struct AddedPath {
    pub path: String,
    /// Absolute and not under the project root.
    pub outside_root: bool,
}

/// Path changes for one environment.
#[derive(Debug, PartialEq, Eq)]
pub struct EnvRouting {
    pub name: String,
    /// The environment does not exist in the vault yet.
    pub new_env: bool,
    pub added: Vec<AddedPath>,
    pub removed: Vec<String>,
}

/// Everything in a push that changes where decrypted secrets get written.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RoutingDiff {
    /// `(old root, new root)`; the old one is `None` for an unbound project.
    pub root: Option<(Option<String>, String)>,
    pub envs: Vec<EnvRouting>,
}

impl RoutingDiff {
    pub fn is_empty(&self) -> bool {
        self.root.is_none() && self.envs.is_empty()
    }

    /// Human-readable diff, one entry per line.
    pub fn render(&self, project: &str) -> Vec<String> {
        let mut lines = vec![format!("Secret-routing changes for project '{project}':")];
        if let Some((old, new)) = &self.root {
            lines.push(format!("  root: {} -> {new}", old.as_deref().unwrap_or("(none)")));
        }
        for e in &self.envs {
            let label = if e.name.is_empty() { "(root)" } else { e.name.as_str() };
            lines.push(format!("  environment '{label}'{}:", if e.new_env { " (new)" } else { "" }));
            for a in &e.added {
                let note = if a.outside_root { "   (absolute, outside the project root)" } else { "" };
                lines.push(format!("    + {}{note}", a.path));
            }
            for r in &e.removed {
                lines.push(format!("    - {r}"));
            }
        }
        lines
    }
}

/// Comparison key that makes the manifest's relative form and the vault's
/// absolute-inside-root form of the same file equal (see `from_project`).
fn path_key(root_host: &str, p: &str) -> String {
    if let Some(rel) = manifest::relativize(root_host, p) {
        return rel;
    }
    if manifest::is_absolute_any(p) {
        return p.to_string();
    }
    p.replace('\\', "/")
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect::<Vec<_>>()
        .join("/")
}

/// Manifest path in the form the vault stores: absolute ones translated to
/// the host, relative ones untouched.
fn host_form(p: &str) -> String {
    if manifest::is_absolute_any(p) {
        paths::to_host(Path::new(p))
    } else {
        p.to_string()
    }
}

/// Pure: what pushing `m` into `existing` would change about secret routing.
/// Metadata (name, description, categories) is deliberately ignored.
pub fn routing_diff(root_host: &str, m: &Manifest, existing: &Project) -> RoutingDiff {
    let mut diff = RoutingDiff::default();
    if scope::binding_of(root_host, existing.root_path.as_deref()) != Binding::Bound {
        let old = existing.root_path.clone().filter(|r| !r.trim().is_empty());
        diff.root = Some((old, root_host.to_string()));
    }
    for env in &m.project.environments {
        let wanted: Vec<String> = env.paths.iter().map(|p| host_form(p)).collect();
        let current = existing.environments.iter().find(|e| e.name.eq_ignore_ascii_case(&env.name));
        let current_paths: &[String] = current.map(|c| c.paths.as_slice()).unwrap_or(&[]);
        let current_keys: Vec<String> = current_paths.iter().map(|p| path_key(root_host, p)).collect();
        let wanted_keys: Vec<String> = wanted.iter().map(|p| path_key(root_host, p)).collect();

        let added: Vec<AddedPath> = wanted
            .iter()
            .zip(&wanted_keys)
            .filter(|(_, k)| !current_keys.contains(k))
            .map(|(p, _)| AddedPath {
                path: p.clone(),
                outside_root: manifest::is_absolute_any(p) && manifest::relativize(root_host, p).is_none(),
            })
            .collect();
        let removed: Vec<String> = current_paths
            .iter()
            .zip(&current_keys)
            .filter(|(_, k)| !wanted_keys.contains(k))
            .map(|(p, _)| p.clone())
            .collect();
        if !added.is_empty() || !removed.is_empty() {
            diff.envs.push(EnvRouting { name: env.name.clone(), new_env: current.is_none(), added, removed });
        }
    }
    diff
}

// ─── Plan / apply ─────────────────────────────────────────────────────────────

/// What `config` is about to do, decided without changing anything.
pub struct Prepared {
    pub direction: Direction,
    /// `None` when the project does not exist yet (it will be created).
    pub project: Option<Project>,
    pub diff: RoutingDiff,
    pub relink: bool,
    remote: u64,
}

impl Prepared {
    /// A secret-routing change on an existing project, or any relink.
    /// Creating a brand-new project holds no secrets and never needs consent.
    pub fn needs_consent(&self) -> bool {
        self.project.is_some() && (self.relink || !self.diff.is_empty())
    }

    pub fn diff_lines(&self, project_name: &str) -> Vec<String> {
        let mut lines = self.diff.render(project_name);
        if self.diff.is_empty() && self.relink {
            lines.push("  (no path changes; the project stays bound to this directory)".into());
        }
        lines
    }
}

fn local_mtime(manifest_path: &Path) -> Result<u64, CliError> {
    Ok(std::fs::metadata(manifest_path)?
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0))
}

/// Resolves direction, binding and the routing diff. Read-only.
pub fn prepare(
    api: &dyn VaultApi,
    root: &Path,
    manifest_path: &Path,
    m: &Manifest,
    relink: bool,
) -> Result<Prepared, CliError> {
    let local = local_mtime(manifest_path)?;
    let Some(project) = api.find_project(&m.project.name)? else {
        return Ok(Prepared {
            direction: Direction::Push,
            project: None,
            diff: RoutingDiff::default(),
            relink: false,
            remote: 0,
        });
    };
    let binding = scope::check_root_binding(root, &project)?;
    if let Binding::Mismatch(bound) = &binding {
        if !relink {
            return Err(scope::mismatch_error(&project.name, bound));
        }
    }
    let remote = remote_updated(&project);
    let root_host = paths::to_host(root);
    let direction = if relink { Direction::Push } else { decide(local, remote) };
    let diff = match direction {
        Direction::Push => routing_diff(&root_host, m, &project),
        // A pull writes only the local file, except that a project without a
        // root adopts this directory — which is itself a routing change.
        Direction::Pull if binding == Binding::Unbound => {
            RoutingDiff { root: Some((None, root_host)), envs: vec![] }
        }
        Direction::Pull => RoutingDiff::default(),
    };
    Ok(Prepared { direction, project: Some(project), diff, relink, remote })
}

/// Performs the planned push or pull. Consent, if needed, was already given.
pub fn apply(api: &dyn VaultApi, root: &Path, m: &Manifest, prepared: &Prepared) -> Result<Outcome, CliError> {
    let Some(project) = &prepared.project else {
        push(api, root, m, None)?;
        return Ok(Outcome::Pushed { created: true, untracked_envs: vec![] });
    };
    match prepared.direction {
        Direction::Push => {
            let untracked_envs = push(api, root, m, Some(project))?;
            Ok(Outcome::Pushed { created: false, untracked_envs })
        }
        Direction::Pull => {
            if project.root_path.as_deref().map(str::trim).unwrap_or("").is_empty() {
                api.save_project(&crate::commands::init::project_body(project, &paths::to_host(root)))?;
            }
            let pulled = manifest::from_project(project, &paths::to_local);
            if &pulled == m {
                return Ok(Outcome::InSync);
            }
            let path = manifest::write_file(root, &pulled).map_err(CliError::Config)?;
            // Stamp the file with the vault's time so the next run doesn't
            // mistake our own write for a local edit.
            let f = std::fs::OpenOptions::new().write(true).open(&path)?;
            f.set_modified(UNIX_EPOCH + Duration::from_secs(prepared.remote))?;
            Ok(Outcome::Pulled)
        }
    }
}

/// `prepare` + `apply` for a caller that has already obtained consent (the
/// TUI's confirm modal).
pub fn execute_confirmed(
    api: &dyn VaultApi,
    root: &Path,
    manifest_path: &Path,
    m: &Manifest,
) -> Result<Outcome, CliError> {
    let prepared = prepare(api, root, manifest_path, m, false)?;
    apply(api, root, m, &prepared)
}

// ─── CLI entry ────────────────────────────────────────────────────────────────

/// How `config` may obtain consent for a secret-routing change.
pub struct ConsentOptions {
    pub relink: bool,
    pub yes: bool,
    /// stdin is a terminal, so a `y/N` prompt is possible.
    pub interactive: bool,
}

pub fn run(args: ConfigArgs) -> Result<(), CliError> {
    let (ws, m) = scope::yaml_workspace()?;
    let opts = ConsentOptions { relink: args.relink, yes: args.yes, interactive: std::io::stdin().is_terminal() };
    let outcome = execute(&client::Live, &ws.root, &ws.config_path, &m, &opts, &mut || {
        Ok(crate::prompts::confirm("Apply these changes to the vault?"))
    })?;
    match outcome {
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

/// Core of the CLI `config`: plan, run the consent flow when the plan is a
/// secret-routing change, then apply. `confirm` is the interactive `y/N`.
pub fn execute(
    api: &dyn VaultApi,
    root: &Path,
    manifest_path: &Path,
    m: &Manifest,
    opts: &ConsentOptions,
    confirm: &mut dyn FnMut() -> Result<bool, CliError>,
) -> Result<Outcome, CliError> {
    let prepared = prepare(api, root, manifest_path, m, opts.relink)?;
    if prepared.needs_consent() {
        for line in prepared.diff_lines(&m.project.name) {
            eprintln!("{line}");
        }
        if !opts.yes && !opts.interactive {
            return Err(CliError::Config(
                "this change alters where secrets are written and needs confirmation — re-run interactively or pass --yes"
                    .into(),
            ));
        }
        api.ensure_session()?;
        if !opts.yes && !confirm()? {
            return Err(CliError::Config("cancelled — the vault was not changed".into()));
        }
    }
    apply(api, root, m, &prepared)
}

/// Pushes `m` into the vault (creating the project when `existing` is
/// `None`). Returns vault environments absent from the manifest.
fn push(api: &dyn VaultApi, root: &Path, m: &Manifest, existing: Option<&Project>) -> Result<Vec<String>, CliError> {
    let root_host = paths::to_host(root);
    api.ensure_categories(&m.project.categories)?;
    let project_id = api.save_project(&serde_json::json!({
        "id": existing.map(|p| p.id).unwrap_or(0),
        "name": m.project.name,
        "description": m.project.description,
        "template": existing.map(|p| p.template.as_str()).unwrap_or("generic"),
        "categories": m.project.categories,
        "rootPath": root_host,
        "initialEnvironment": m.default_environment().map(|e| e.name.as_str()),
    }))?;
    let project = api
        .fetch_projects()?
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| CliError::Api("project vanished after saving".into()))?;

    let default_name = m.default_environment().map(|e| e.name.to_lowercase());
    for env in &m.project.environments {
        let host_paths: Vec<String> = env.paths.iter().map(|p| host_form(p)).collect();
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
        api.save_environment(&body)?;
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
    use crate::testing::{env, project, FakeVault};

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

    // ─── RoutingDiff ────────────────────────────────────────────────────

    const ROOT: &str = "/home/u/app";

    fn manifest_of(yaml: &str) -> Manifest {
        manifest::parse(yaml).unwrap()
    }

    fn vault_project() -> Project {
        project("app", Some(ROOT), vec![env(1, "default", true, &[".env"])])
    }

    #[test]
    fn metadata_only_change_has_an_empty_diff() {
        let m = manifest_of(
            "project:\n  name: app\n  description: changed\n  tags: [x]\n  environments:\n    - {name: default, isDefault: true, paths: ['.env']}\n",
        );
        assert!(routing_diff(ROOT, &m, &vault_project()).is_empty());
    }

    #[test]
    fn vault_absolute_path_inside_root_equals_manifest_relative_form() {
        let mut p = vault_project();
        p.environments[0].paths = vec![format!("{ROOT}/apps/web/.env")];
        let m = manifest_of(
            "project:\n  name: app\n  environments:\n    - {name: default, isDefault: true, paths: ['apps/web/.env']}\n",
        );
        assert!(routing_diff(ROOT, &m, &p).is_empty(), "round-tripped manifest must not look like a change");
    }

    #[test]
    fn added_and_removed_paths_are_reported() {
        let m = manifest_of(
            "project:\n  name: app\n  environments:\n    - {name: default, isDefault: true, paths: ['apps/web/.env', '/etc/elsewhere/.env']}\n",
        );
        let d = routing_diff(ROOT, &m, &vault_project());
        assert!(d.root.is_none());
        assert_eq!(d.envs.len(), 1);
        let e = &d.envs[0];
        assert!(!e.new_env);
        assert_eq!(e.removed, vec![".env".to_string()]);
        assert_eq!(e.added.len(), 2);
        assert!(!e.added[0].outside_root);
        assert!(e.added[1].outside_root, "absolute path outside the root is flagged");
    }

    #[test]
    fn new_environment_with_paths_is_a_change_without_paths_is_not() {
        let with_paths = manifest_of(
            "project:\n  name: app\n  environments:\n    - {name: default, isDefault: true, paths: ['.env']}\n    - {name: prod, paths: ['prod/.env']}\n",
        );
        let d = routing_diff(ROOT, &with_paths, &vault_project());
        assert_eq!(d.envs.len(), 1);
        assert!(d.envs[0].new_env);

        let without = manifest_of(
            "project:\n  name: app\n  environments:\n    - {name: default, isDefault: true, paths: ['.env']}\n    - {name: prod}\n",
        );
        assert!(routing_diff(ROOT, &without, &vault_project()).is_empty());
    }

    #[test]
    fn unbound_or_moved_root_is_a_change() {
        let m = manifest_of("project:\n  name: app\n  environments:\n    - {name: default, isDefault: true, paths: ['.env']}\n");
        let mut p = vault_project();
        p.root_path = None;
        assert_eq!(routing_diff(ROOT, &m, &p).root, Some((None, ROOT.to_string())));
        p.root_path = Some("/other".into());
        assert_eq!(routing_diff(ROOT, &m, &p).root, Some((Some("/other".to_string()), ROOT.to_string())));
    }

    // ─── execute: consent flow ──────────────────────────────────────────

    struct Fixture {
        dir: tempfile::TempDir,
        manifest_path: std::path::PathBuf,
        m: Manifest,
        host_root: String,
    }

    /// A workspace whose manifest adds `apps/web/.env` to `default`. The file
    /// is newer than the fake vault (`updated: "0"`), so the direction is push.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let yaml = "project:\n  name: app\n  environments:\n    - {name: default, isDefault: true, paths: ['.env', 'apps/web/.env']}\n";
        let manifest_path = dir.path().join(manifest::FILE_NAME);
        std::fs::write(&manifest_path, yaml).unwrap();
        let host_root = paths::to_host(dir.path());
        Fixture { dir, manifest_path, m: manifest_of(yaml), host_root }
    }

    fn opts(relink: bool, yes: bool, interactive: bool) -> ConsentOptions {
        ConsentOptions { relink, yes, interactive }
    }

    #[test]
    fn declined_confirmation_makes_no_save_calls() {
        let f = fixture();
        let vault = FakeVault::new(vec![project("app", Some(&f.host_root), vec![env(1, "default", true, &[".env"])])]);
        let err = execute(&vault, f.dir.path(), &f.manifest_path, &f.m, &opts(false, false, true), &mut || Ok(false))
            .err()
            .unwrap();
        assert!(err.to_string().contains("cancelled"), "{err}");
        assert!(vault.mutations().is_empty(), "{:?}", vault.calls.borrow());
        assert!(vault.calls.borrow().contains(&"ensure_session".to_string()), "session is required before asking");
    }

    #[test]
    fn confirmed_path_change_is_applied() {
        let f = fixture();
        let vault = FakeVault::new(vec![project("app", Some(&f.host_root), vec![env(1, "default", true, &[".env"])])]);
        let out = execute(&vault, f.dir.path(), &f.manifest_path, &f.m, &opts(false, false, true), &mut || Ok(true));
        assert!(matches!(out, Ok(Outcome::Pushed { created: false, .. })));
        assert!(vault.mutations().contains(&"save_environment".to_string()));
    }

    #[test]
    fn non_interactive_path_change_needs_yes() {
        let f = fixture();
        let vault = FakeVault::new(vec![project("app", Some(&f.host_root), vec![env(1, "default", true, &[".env"])])]);
        let err = execute(&vault, f.dir.path(), &f.manifest_path, &f.m, &opts(false, false, false), &mut || Ok(true))
            .err()
            .unwrap();
        assert!(err.to_string().contains("--yes"), "{err}");
        assert!(vault.mutations().is_empty());

        let ok = execute(&vault, f.dir.path(), &f.manifest_path, &f.m, &opts(false, true, false), &mut || Ok(false));
        assert!(ok.is_ok(), "--yes applies without prompting");
    }

    #[test]
    fn mismatch_without_relink_is_an_error_with_no_calls() {
        let f = fixture();
        let vault = FakeVault::new(vec![project("app", Some("/home/u/real-project"), vec![env(1, "default", true, &[".env"])])]);
        let err = execute(&vault, f.dir.path(), &f.manifest_path, &f.m, &opts(false, true, true), &mut || Ok(true))
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("/home/u/real-project") && err.contains("--relink"), "{err}");
        assert!(vault.mutations().is_empty());
        assert!(!vault.calls.borrow().contains(&"ensure_session".to_string()));
    }

    #[test]
    fn relink_declined_changes_nothing_and_accepted_rebinds() {
        let f = fixture();
        let vault = FakeVault::new(vec![project("app", Some("/home/u/real-project"), vec![env(1, "default", true, &[".env"])])]);
        let declined = execute(&vault, f.dir.path(), &f.manifest_path, &f.m, &opts(true, false, true), &mut || Ok(false));
        assert!(declined.is_err());
        assert!(vault.mutations().is_empty());

        let accepted = execute(&vault, f.dir.path(), &f.manifest_path, &f.m, &opts(true, false, true), &mut || Ok(true));
        assert!(accepted.is_ok());
        assert!(vault.mutations().contains(&"save_project".to_string()));
    }

    #[test]
    fn metadata_only_push_needs_no_consent() {
        let f = fixture();
        let vault = FakeVault::new(vec![project(
            "app",
            Some(&f.host_root),
            vec![env(1, "default", true, &[".env", "apps/web/.env"])],
        )]);
        let out = execute(&vault, f.dir.path(), &f.manifest_path, &f.m, &opts(false, false, false), &mut || Ok(false));
        assert!(out.is_ok());
        assert!(!vault.calls.borrow().contains(&"ensure_session".to_string()));
    }

    #[test]
    fn brand_new_project_needs_no_consent() {
        let f = fixture();
        let vault = FakeVault::new(vec![]);
        let out = execute(&vault, f.dir.path(), &f.manifest_path, &f.m, &opts(false, false, false), &mut || Ok(false));
        assert!(matches!(out, Ok(Outcome::Pushed { created: true, .. })));
    }
}
