//! `crypt-env sync [--global]` — after master-password verification, reads
//! `.env.example` and provisions every key missing from the environment as a
//! vault item with the placeholder value `change-me`. With `--global`, keys
//! matching an existing global secret are linked to it instead, and the
//! environment is then materialized into its `.env` target(s).

use clap::Args;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::client::{self, CliError};
use crate::commands::scope;

pub const PLACEHOLDER: &str = "change-me";

#[derive(Args)]
pub struct SyncArgs {
    /// Link keys that match global vault secrets and write the resulting .env
    #[arg(long)]
    pub global: bool,

    /// Target environment (defaults to the project's default environment)
    #[arg(long = "env")]
    pub env: Option<String>,

    /// Template to read (defaults to ./.env.example, then the project root's)
    #[arg(long)]
    pub example: Option<PathBuf>,
}

#[derive(Default)]
pub struct SyncReport {
    pub example: PathBuf,
    pub created: Vec<String>,
    pub linked: Vec<String>,
    pub written: Vec<String>,
}

pub fn run(args: SyncArgs) -> Result<(), CliError> {
    let ws = scope::workspace()?;
    let example = locate_example(args.example.as_deref(), &ws.root)?;
    client::ensure_session()?;
    let r = execute(&ws, &example, args.env.as_deref(), args.global)?;
    eprintln!("Read {}", r.example.display());
    if r.created.is_empty() && r.linked.is_empty() {
        eprintln!("No missing keys — the environment already has every key.");
    }
    if !r.created.is_empty() {
        eprintln!("Created with placeholder '{PLACEHOLDER}': {}", r.created.join(", "));
    }
    if !r.linked.is_empty() {
        eprintln!("Linked to global secrets: {}", r.linked.join(", "));
    }
    if !r.written.is_empty() {
        eprintln!("Wrote {}", r.written.join(", "));
        eprintln!("Warning: these files contain secrets in plaintext — keep them out of version control.");
    }
    Ok(())
}

/// `--example`, else `./.env.example`, else `<root>/.env.example`.
pub fn locate_example(arg: Option<&Path>, root: &Path) -> Result<PathBuf, CliError> {
    if let Some(p) = arg {
        return if p.is_file() {
            Ok(p.to_path_buf())
        } else {
            Err(CliError::NotFound(format!("{} (template)", p.display())))
        };
    }
    let cwd = std::env::current_dir()?;
    for dir in [cwd.as_path(), root] {
        let c = dir.join(".env.example");
        if c.is_file() {
            return Ok(c);
        }
    }
    Err(CliError::Config(".env.example was not found in this directory or the project root".into()))
}

/// Keys of a dotenv template, in file order, deduplicated.
pub fn template_keys(content: &str) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let t = t.strip_prefix("export ").unwrap_or(t);
        if let Some((k, _)) = t.split_once('=') {
            let k = k.trim();
            if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !keys.iter().any(|x| x == k) {
                keys.push(k.to_string());
            }
        }
    }
    keys
}

/// Core of `sync`, shared with the TUI. The caller has already verified the
/// master password.
pub fn execute(ws: &scope::Workspace, example: &Path, env_flag: Option<&str>, global: bool) -> Result<SyncReport, CliError> {
    let project = scope::vault_project(ws)?;
    let env = scope::pick_environment(&project, ws.default_env.as_deref(), env_flag)?.clone();
    let keys = template_keys(&std::fs::read_to_string(example)?);
    let missing: Vec<String> = keys.into_iter().filter(|k| !env.vars.iter().any(|v| &v.key == k)).collect();

    let mut report = SyncReport { example: example.to_path_buf(), ..Default::default() };
    let mut to_create = missing.clone();

    if global {
        let globals: HashMap<String, i64> = client::list_items(&project.name, &env.name, "only")?
            .into_iter()
            .filter_map(|i| i.name.map(|n| (n, i.id)))
            .collect();
        let mut vars: Vec<serde_json::Value> =
            env.vars.iter().map(|v| serde_json::json!({"key": v.key, "itemId": v.item_id})).collect();
        to_create.clear();
        for k in &missing {
            match globals.get(k) {
                Some(id) => {
                    vars.push(serde_json::json!({"key": k, "itemId": id}));
                    report.linked.push(k.clone());
                }
                None => to_create.push(k.clone()),
            }
        }
        if !report.linked.is_empty() {
            let mut body = client::environment_body(&env, env.is_default, &env.paths);
            body["vars"] = serde_json::Value::Array(vars);
            client::save_environment(&body)?;
        }
    }

    let url = format!(
        "{}/items?project={}&environment={}&on_conflict=error",
        client::api_base(),
        client::urlencod(&project.name),
        client::urlencod(&env.name)
    );
    for k in &to_create {
        let body = serde_json::json!({
            "id": 0, "type": "secret", "name": k, "value": PLACEHOLDER,
            "categories": [], "created": "", "key": k,
        });
        let resp = client::authenticated_post(&url, &body)?;
        if !resp.status().is_success() {
            return Err(CliError::Api(format!("creating '{k}' failed: HTTP {}", resp.status())));
        }
        report.created.push(k.clone());
    }

    if global {
        // Materialize: configured targets, or a `.env` next to the template.
        let output = if env.paths.is_empty() {
            let dir = example.parent().unwrap_or(Path::new("."));
            let abs = std::fs::canonicalize(dir)?.join(crypt_env_lib::project::environment_filename(&env.name));
            Some(crate::paths::to_host(&abs))
        } else {
            None
        };
        let result = client::inject_environment(env.id, output.as_deref())?;
        report.written = result.paths;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_keys_parses_and_dedupes() {
        let keys = template_keys("# c\nA=1\nexport B=\n\nA=2\nbad key=x\nC_D=\n");
        assert_eq!(keys, vec!["A", "B", "C_D"]);
    }

    #[test]
    fn locate_example_prefers_explicit_and_errors_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(locate_example(Some(&dir.path().join("nope")), dir.path()).is_err());
        let f = dir.path().join(".env.example");
        std::fs::write(&f, "A=\n").unwrap();
        assert_eq!(locate_example(Some(&f), dir.path()).unwrap(), f);
        assert_eq!(locate_example(None, dir.path()).unwrap(), f);
    }
}
