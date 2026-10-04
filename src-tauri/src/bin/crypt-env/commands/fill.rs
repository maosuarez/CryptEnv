//! `crypt-env fill [--env <NAME>]` — after master-password verification,
//! materializes every environment (or one) into its configured target files
//! (written by the vault host, relative targets resolved against the project
//! root) and writes a sanitized `.env.example` next to each target.
//!
//! Refuses to run from a directory other than the project's bound root, and
//! derives the `.env.example` directories from the paths the vault actually
//! wrote, so the example always sits next to a real `.env`.

use clap::Args;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::client::{self, CliError, Environment, VaultApi};
use crate::commands::scope;
use crate::paths;
use crypt_env_lib::envfile;

#[derive(Args)]
pub struct FillArgs {
    /// Only this environment (defaults to all environments of the project)
    #[arg(long = "env")]
    pub env: Option<String>,
}

pub struct FillReport {
    pub lines: Vec<String>,
    pub examples: Vec<PathBuf>,
}

pub fn run(args: FillArgs) -> Result<(), CliError> {
    let ws = scope::workspace()?;
    client::ensure_session()?;
    let report = execute(&client::Live, &ws, args.env.as_deref())?;
    for l in &report.lines {
        eprintln!("{l}");
    }
    for e in &report.examples {
        eprintln!("Wrote {}", e.display());
    }
    Ok(())
}

/// Core of `fill`, shared with the TUI. The caller has already verified the
/// master password.
pub fn execute(api: &dyn VaultApi, ws: &scope::Workspace, env_flag: Option<&str>) -> Result<FillReport, CliError> {
    let project = api.find_project(&ws.project)?.ok_or_else(|| {
        CliError::Config(format!(
            "project '{}' does not exist in the vault — run `crypt-env config` (or `crypt-env init`) to register it",
            ws.project
        ))
    })?;
    // Before any write: a second checkout must not receive (or overwrite)
    // the secrets of the checkout the project is bound to.
    scope::ensure_bound(&ws.root, &project)?;
    let envs: Vec<&Environment> = match env_flag {
        Some(name) => vec![scope::pick_environment(&project, None, Some(name))?],
        None => project.environments.iter().collect(),
    };

    let mut lines = Vec::new();
    // directory → keys, for the `.env.example` files.
    let mut examples: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    for env in envs {
        let label = if env.name.is_empty() { "(root)" } else { env.name.as_str() };
        if env.paths.is_empty() {
            lines.push(format!(
                "skip {label}: no target paths — add some under this environment in {} and run `crypt-env config`",
                scope::manifest::FILE_NAME
            ));
            continue;
        }
        let result = api.inject_environment(env.id)?;
        lines.push(format!("{label}: {} key(s) → {}", result.written.len(), result.paths.join(", ")));
        for b in &result.backups {
            lines.push(format!("  backup of a pre-existing unmanaged file: {b}"));
        }
        if !result.failed_keys.is_empty() {
            lines.push(format!("  not written (item could not be decrypted or invalid key name): {}", result.failed_keys.join(", ")));
        }
        let keys: BTreeSet<String> = env.vars.iter().map(|v| v.key.clone()).collect();
        for dir in example_dirs(&result.paths) {
            examples.entry(dir).or_default().extend(keys.iter().cloned());
        }
    }

    let mut written = Vec::new();
    for (dir, keys) in examples {
        if keys.is_empty() || !dir.is_dir() {
            continue;
        }
        written.push(write_example(&dir, &keys)?);
    }
    if !written.is_empty() || lines.iter().any(|l| l.contains(" key(s) → ")) {
        lines.push("Warning: the .env files contain secrets in plaintext — keep them out of version control.".into());
    }
    Ok(FillReport { lines, examples: written })
}

/// Directories (local form) of the files the vault reported writing.
pub fn example_dirs(written_host_paths: &[String]) -> BTreeSet<PathBuf> {
    written_host_paths
        .iter()
        .filter_map(|p| PathBuf::from(paths::to_local(p)).parent().map(Path::to_path_buf))
        .collect()
}

/// Writes/extends `<dir>/.env.example` with `KEY=` lines. Existing lines are
/// kept verbatim; only missing keys are appended. Never writes a value.
pub fn write_example(dir: &Path, keys: &BTreeSet<String>) -> Result<PathBuf, CliError> {
    let path = dir.join(".env.example");
    // Never read or write through a link: a planted `.env.example` symlink
    // must neither leak its target's lines nor be written into.
    if std::fs::symlink_metadata(&path).map(|m| m.file_type().is_symlink()).unwrap_or(false) {
        return Err(CliError::Config(envfile::symlink_message(&path)));
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let present: BTreeSet<String> = existing
        .lines()
        .filter_map(|l| l.trim().split_once('=').map(|(k, _)| k.trim().trim_start_matches("export ").to_string()))
        .collect();
    let mut out = existing.clone();
    if out.is_empty() {
        out.push_str("# Generated by crypt-env fill — keys only, no values.\n");
    } else if !out.ends_with('\n') {
        out.push('\n');
    }
    let mut added = false;
    for k in keys.iter().filter(|k| !present.contains(*k)) {
        out.push_str(k);
        out.push_str("=\n");
        added = true;
    }
    if added || existing.is_empty() {
        use std::io::Write as _;
        let mut file = envfile::open_nofollow(&path, envfile::FileMode::Inherit)
            .map_err(|e| CliError::Config(e.to_string()))?;
        file.write_all(out.as_bytes())?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_example_appends_missing_keys_only() {
        let dir = tempfile::tempdir().unwrap();
        let keys: BTreeSet<String> = ["A", "B"].iter().map(|s| s.to_string()).collect();
        let p = write_example(dir.path(), &keys).unwrap();
        let first = std::fs::read_to_string(&p).unwrap();
        assert!(first.contains("A=\n") && first.contains("B=\n"));

        std::fs::write(&p, "A=keep-me\n").unwrap();
        write_example(dir.path(), &keys).unwrap();
        let second = std::fs::read_to_string(&p).unwrap();
        assert_eq!(second, "A=keep-me\nB=\n");
    }

    use crate::testing::{env, project, FakeVault};

    fn workspace(root: &Path) -> scope::Workspace {
        scope::Workspace {
            root: root.to_path_buf(),
            config_path: root.join(scope::manifest::FILE_NAME),
            project: "app".into(),
            default_env: None,
            manifest: None,
        }
    }

    #[test]
    fn mismatch_is_an_error_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let vault = FakeVault::new(vec![project("app", Some("/home/u/checkout-a"), vec![env(1, "default", true, &[".env"])])]);
        let err = execute(&vault, &workspace(dir.path()), None).err().unwrap().to_string();
        assert!(err.contains("/home/u/checkout-a") && err.contains("--relink"), "{err}");
        assert!(vault.mutations().is_empty(), "no inject call: {:?}", vault.calls.borrow());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn example_directories_are_the_parents_of_the_written_paths() {
        let dir = tempfile::tempdir().unwrap();
        let web = dir.path().join("apps/web");
        std::fs::create_dir_all(&web).unwrap();
        let vault = FakeVault::new(vec![project(
            "app",
            Some(&paths::to_host(dir.path())),
            vec![env(1, "default", true, &[".env", "apps/web/.env"])],
        )]);
        *vault.inject_paths.borrow_mut() = vec![
            paths::to_host(&dir.path().join(".env")),
            paths::to_host(&web.join(".env")),
        ];
        // The fake env has no vars, so give it one to produce example keys.
        vault.projects.borrow_mut()[0].environments[0].vars =
            vec![crypt_env_lib::project::EnvironmentVar { id: 1, key: "A".into(), item_id: 1 }];

        let report = execute(&vault, &workspace(dir.path()), None).unwrap();

        let mut got: Vec<PathBuf> = report.examples.iter().map(|p| p.parent().unwrap().to_path_buf()).collect();
        got.sort();
        assert_eq!(got, vec![dir.path().to_path_buf(), web.clone()]);
        assert!(web.join(".env.example").is_file());
    }

    #[test]
    fn failed_keys_are_reported_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let vault = FakeVault::new(vec![project(
            "app",
            Some(&paths::to_host(dir.path())),
            vec![env(1, "default", true, &[".env"])],
        )]);
        *vault.inject_paths.borrow_mut() = vec![paths::to_host(&dir.path().join(".env"))];
        *vault.inject_failed_keys.borrow_mut() = vec!["BAD_ONE".into(), "BAD_TWO".into()];

        let report = execute(&vault, &workspace(dir.path()), None).unwrap();

        assert!(
            report.lines.iter().any(|l| l.contains("not written") && l.contains("BAD_ONE, BAD_TWO")),
            "{:?}",
            report.lines
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_example_is_refused_and_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, "keep\n").unwrap();
        std::os::unix::fs::symlink(&victim, dir.path().join(".env.example")).unwrap();
        let keys: BTreeSet<String> = ["A"].iter().map(|s| s.to_string()).collect();
        assert!(write_example(dir.path(), &keys).is_err());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep\n");
    }
}
