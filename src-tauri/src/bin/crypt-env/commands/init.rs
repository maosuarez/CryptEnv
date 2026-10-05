//! `crypt-env init [NAME] [--path <PATH>] [--yes]` — registers (or links) the vault
//! project for the current directory, records this directory as its root,
//! and writes `.crypt-env.yaml` here.
//!
//! Inside a Git repository the manifest is added to that directory's
//! `.gitignore` (additive only).
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
    /// is used) or a `.env*` file. Defaults to `./` (this directory).
    #[arg(long)]
    pub path: Option<PathBuf>,

    /// Confirm binding an existing, rootless project to this directory
    /// without prompting (required when stdin is not a terminal)
    #[arg(long)]
    pub yes: bool,
}

pub struct InitReport {
    pub project: String,
    pub created: bool,
    pub manifest_path: PathBuf,
    pub target: String,
    pub gitignore: GitignoreUpdate,
}

/// What `init` did about keeping the manifest out of Git.
#[derive(Debug, PartialEq, Eq)]
pub enum GitignoreUpdate {
    /// Not inside a Git repository: nothing to do.
    NotAGitRepo,
    /// `.gitignore` already lists the manifest.
    AlreadyIgnored,
    /// The manifest entry was appended (the file was created if missing).
    Added,
    /// `.gitignore` could not be updated; the reason is shown as a warning.
    Skipped(String),
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
    let adopt_confirmed = adoption_consent(
        api,
        &cwd,
        args.name.as_deref(),
        args.yes,
        std::io::stdin().is_terminal(),
        &mut || Ok(crate::prompts::confirm("Bind this project to the current directory?")),
    )?;
    let r = execute(api, &cwd, args.name.as_deref(), args.path.as_deref(), adopt_confirmed)?;
    eprintln!(
        "{} project '{}' (root {}), default environment → {}",
        if r.created { "Created" } else { "Linked existing" },
        r.project,
        cwd.display(),
        r.target
    );
    eprintln!("Wrote {}", r.manifest_path.display());
    match &r.gitignore {
        GitignoreUpdate::Added => eprintln!("Added {} to .gitignore", manifest::FILE_NAME),
        GitignoreUpdate::Skipped(why) => {
            eprintln!("warning: could not add {} to .gitignore: {why}", manifest::FILE_NAME)
        }
        GitignoreUpdate::NotAGitRepo | GitignoreUpdate::AlreadyIgnored => {}
    }
    Ok(())
}

/// Consent flow for adopting a rootless project: show the diff, require a
/// non-interactive `--yes` or a TTY, require a live session, then ask `y/N`
/// unless `--yes`. Returns whether adoption was confirmed (false when no
/// adoption is involved). `--yes` only answers the routing confirmation;
/// the session is still required.
fn adoption_consent(
    api: &dyn VaultApi,
    dir: &Path,
    name: Option<&str>,
    yes: bool,
    interactive: bool,
    confirm: &mut dyn FnMut() -> Result<bool, CliError>,
) -> Result<bool, CliError> {
    let Existing::Adopt { lines, .. } = check(api, dir, name)? else { return Ok(false) };
    for line in &lines {
        eprintln!("{line}");
    }
    if !yes && !interactive {
        return Err(CliError::Config(
            "adopting this directory changes where secrets are written and needs confirmation — re-run interactively or pass --yes"
                .into(),
        ));
    }
    api.ensure_session()?;
    if !yes && !confirm()? {
        return Err(CliError::Config("cancelled — the vault was not changed".into()));
    }
    Ok(true)
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
        let already = env.paths.iter().any(|p| p == &target || same_file(&env.name, p, &target));
        if created || !already {
            let mut paths = if created { Vec::new() } else { env.paths.clone() };
            paths.push(target.clone());
            api.save_environment(&client::environment_body(env, env.is_default, &paths))?;
        }
    }

    let project = fetch_by_id(api, project_id)?;
    let m = manifest::from_project(&project, &paths::to_local);
    let manifest_path = manifest::write_file(dir, &m).map_err(CliError::Config)?;
    let gitignore = ensure_gitignored(dir);
    Ok(InitReport { project: project.name, created, manifest_path, target, gitignore })
}

/// True when `dir` or any ancestor holds `.git` (a directory, or the file a
/// worktree/submodule uses).
fn inside_git_repo(dir: &Path) -> bool {
    dir.ancestors().any(|d| d.join(".git").exists())
}

/// True when `.gitignore` content already has a rule for exactly the manifest.
fn lists_manifest(gitignore: &str) -> bool {
    gitignore.lines().map(str::trim).any(|l| {
        matches!(l.strip_prefix("**/").or_else(|| l.strip_prefix('/')).unwrap_or(l), manifest::FILE_NAME)
    })
}

/// Inside a Git repository, makes sure `dir/.gitignore` ignores the manifest:
/// creates the file or appends one line, never rewriting or removing rules.
/// Best effort — the manifest is already written, so a failure is reported
/// rather than raised. A symlinked or non-regular `.gitignore` is left alone
/// (the append must not be redirected elsewhere).
pub fn ensure_gitignored(dir: &Path) -> GitignoreUpdate {
    if !inside_git_repo(dir) {
        return GitignoreUpdate::NotAGitRepo;
    }
    let path = dir.join(".gitignore");
    let existing = match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_file() => match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(e) => return GitignoreUpdate::Skipped(e.kind().to_string()),
        },
        Ok(_) => return GitignoreUpdate::Skipped(".gitignore is not a regular file".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return GitignoreUpdate::Skipped(e.kind().to_string()),
    };
    if lists_manifest(&existing) {
        return GitignoreUpdate::AlreadyIgnored;
    }
    let separator = if existing.is_empty() || existing.ends_with('\n') { "" } else { "\n" };
    match append_no_follow(&path, &format!("{separator}{}\n", manifest::FILE_NAME)) {
        Ok(()) => GitignoreUpdate::Added,
        Err(e) => GitignoreUpdate::Skipped(e.kind().to_string()),
    }
}

/// Appends `text` to `path` (creating it), refusing a symlink at the final
/// component on Unix via `O_NOFOLLOW`.
fn append_no_follow(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)?.write_all(text.as_bytes())
}

/// True when two stored relative targets inject into the same file for
/// `env_name` (`.env` and `./` for the root environment, for example).
fn same_file(env_name: &str, a: &str, b: &str) -> bool {
    let file = |p: &str| {
        let expanded = crypt_env_lib::project::expand_relative_target(None, p, env_name);
        expanded.trim_start_matches("./").to_string()
    };
    file(a) == file(b)
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
/// Without `--path` the target is the project root folder, `./`.
pub fn default_target(root: &Path, path: Option<&Path>) -> String {
    let Some(p) = path else { return "./".to_string() };
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
        assert_eq!(default_target(root, None), "./");
        assert_eq!(default_target(root, Some(Path::new("./app"))), "app/.env");
        assert_eq!(default_target(root, Some(Path::new("app/.env.local"))), "app/.env.local");
        assert_eq!(default_target(root, Some(Path::new("new-dir"))), "new-dir/.env");
        let outside = default_target(root, Some(Path::new("../elsewhere")));
        assert!(outside.ends_with(".env") && outside != "elsewhere/.env", "{outside}");
    }

    use crate::testing::{env, project, FakeVault};

    #[test]
    fn same_file_treats_root_folder_and_env_file_as_equal_for_the_root_environment() {
        assert!(same_file("default", ".env", "./"));
        assert!(same_file("default", "./.env", "./"));
        assert!(!same_file("production", ".env", "./"), "`./` is .env.production there");
        assert!(same_file("production", ".env.production", "./"));
        assert!(!same_file("default", "app/.env", "./"));
    }

    fn git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        dir
    }

    #[test]
    fn gitignore_is_created_and_appended_once_inside_a_repo() {
        let dir = git_repo();
        let gi = dir.path().join(".gitignore");
        assert_eq!(ensure_gitignored(dir.path()), GitignoreUpdate::Added);
        assert_eq!(std::fs::read_to_string(&gi).unwrap(), ".crypt-env.yaml\n");
        assert_eq!(ensure_gitignored(dir.path()), GitignoreUpdate::AlreadyIgnored);
        assert_eq!(std::fs::read_to_string(&gi).unwrap(), ".crypt-env.yaml\n");
    }

    #[test]
    fn gitignore_append_keeps_existing_rules_and_adds_a_missing_newline() {
        let dir = git_repo();
        let gi = dir.path().join(".gitignore");
        std::fs::write(&gi, "target/\n*.log").unwrap();
        assert_eq!(ensure_gitignored(dir.path()), GitignoreUpdate::Added);
        assert_eq!(std::fs::read_to_string(&gi).unwrap(), "target/\n*.log\n.crypt-env.yaml\n");
    }

    #[test]
    fn gitignore_recognizes_anchored_and_glob_forms() {
        for rule in [".crypt-env.yaml", "/.crypt-env.yaml", "**/.crypt-env.yaml", "  .crypt-env.yaml  "] {
            let dir = git_repo();
            std::fs::write(dir.path().join(".gitignore"), format!("a\n{rule}\n")).unwrap();
            assert_eq!(ensure_gitignored(dir.path()), GitignoreUpdate::AlreadyIgnored, "{rule}");
        }
        let dir = git_repo();
        std::fs::write(dir.path().join(".gitignore"), "#.crypt-env.yaml\n.crypt-env.yaml.bak\n").unwrap();
        assert_eq!(ensure_gitignored(dir.path()), GitignoreUpdate::Added);
    }

    #[test]
    fn gitignore_untouched_outside_a_repo_and_for_worktree_git_files() {
        let plain = tempfile::tempdir().unwrap();
        assert_eq!(ensure_gitignored(plain.path()), GitignoreUpdate::NotAGitRepo);
        assert!(!plain.path().join(".gitignore").exists());

        let worktree = tempfile::tempdir().unwrap();
        std::fs::write(worktree.path().join(".git"), "gitdir: /elsewhere\n").unwrap();
        assert_eq!(ensure_gitignored(worktree.path()), GitignoreUpdate::Added);
    }

    #[test]
    fn gitignore_found_from_a_subdirectory_of_the_repo() {
        let repo = git_repo();
        let sub = repo.path().join("apps/api");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(ensure_gitignored(&sub), GitignoreUpdate::Added);
        assert!(sub.join(".gitignore").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_gitignore_is_never_written_through() {
        let dir = git_repo();
        let victim = dir.path().join("victim");
        std::fs::write(&victim, "keep\n").unwrap();
        std::os::unix::fs::symlink(&victim, dir.path().join(".gitignore")).unwrap();
        assert!(matches!(ensure_gitignored(dir.path()), GitignoreUpdate::Skipped(_)));
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep\n");
    }

    #[test]
    fn init_in_a_repo_writes_manifest_and_gitignore() {
        let dir = git_repo();
        let vault = FakeVault::new(vec![]);
        let r = execute(&vault, dir.path(), Some("backend"), None, false).unwrap();
        assert_eq!(r.gitignore, GitignoreUpdate::Added);
        assert!(dir.path().join(manifest::FILE_NAME).is_file());
    }

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

    fn rootless_vault() -> FakeVault {
        FakeVault::new(vec![project("backend", None, vec![env(1, "default", true, &[".env"])])])
    }

    #[test]
    fn yes_adopts_rootless_project_non_interactively() {
        let dir = tempfile::tempdir().unwrap();
        let vault = rootless_vault();
        let mut no_prompt = || -> Result<bool, CliError> { Err(CliError::Config("must not prompt".into())) };
        let confirmed = adoption_consent(&vault, dir.path(), Some("backend"), true, false, &mut no_prompt).unwrap();
        assert!(confirmed);
        assert!(vault.calls.borrow().contains(&"ensure_session".to_string()), "--yes does not skip authentication");
        assert!(vault.mutations().is_empty(), "consent alone changes nothing");
        assert!(run_init(&vault, dir.path(), confirmed).is_ok());
        assert!(vault.mutations().contains(&"save_project".to_string()));
    }

    #[test]
    fn non_tty_without_yes_fails_mentioning_flag_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let vault = rootless_vault();
        let err = adoption_consent(&vault, dir.path(), Some("backend"), false, false, &mut || Ok(true))
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("--yes"), "{err}");
        assert!(vault.mutations().is_empty());
        assert!(!vault.calls.borrow().contains(&"ensure_session".to_string()));
    }

    #[test]
    fn interactive_decline_cancels_and_non_adoption_needs_no_consent() {
        let dir = tempfile::tempdir().unwrap();
        let vault = rootless_vault();
        assert!(adoption_consent(&vault, dir.path(), Some("backend"), false, true, &mut || Ok(false)).is_err());
        assert!(vault.mutations().is_empty());
        let empty = FakeVault::new(vec![]);
        assert!(!adoption_consent(&empty, dir.path(), Some("fresh"), false, false, &mut || Ok(true)).unwrap());
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
