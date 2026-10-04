//! `crypt-env add KEY=value | $VAR | <file.env>` — adds secrets to the
//! workspace project's environment. A key that already exists halts the
//! whole addition (nothing is saved); the colliding value can be inspected
//! only after re-entering the master password.

use clap::Args;
use std::io::IsTerminal;
use std::path::Path;

use crate::client::{self, CliError};
use crate::commands::scope;

#[derive(Args)]
pub struct AddArgs {
    /// KEY=value, $VARNAME (read from this shell's environment), or a path
    /// to a .env file
    pub input: String,

    /// Target environment (defaults to the project's default environment)
    #[arg(long = "env")]
    pub env: Option<String>,

    /// Mark the new item(s) as global (reusable across projects)
    #[arg(long)]
    pub global: bool,
}

/// Where a colliding key already lives.
#[derive(Debug, PartialEq, Eq)]
pub struct Collision {
    pub key: String,
    pub item_id: i64,
    pub scope: String,
}

pub fn run(args: AddArgs) -> Result<(), CliError> {
    let pairs = parse_input(&args.input)?;
    if pairs.is_empty() {
        eprintln!("No entries to add.");
        return Ok(());
    }
    let (_ws, project, env) = scope::resolve(args.env.as_deref())?;

    let mut existing: Vec<(String, i64, String)> = env
        .vars
        .iter()
        .map(|v| (v.key.clone(), v.item_id, format!("environment '{}'", env.name)))
        .collect();
    if args.global {
        for item in client::list_items(&project.name, &env.name, "only")? {
            if let Some(name) = item.name.or(item.title) {
                existing.push((name, item.id, "global scope".to_string()));
            }
        }
    }
    let collisions = find_collisions(&pairs, &existing);

    if !collisions.is_empty() {
        for c in &collisions {
            eprintln!("Error: Key '{}' already exists in {}. Addition aborted.", c.key, c.scope);
        }
        if std::io::stdin().is_terminal() {
            for c in &collisions {
                if crate::prompts::confirm(&format!("Do you want to inspect the colliding value of '{}'?", c.key)) {
                    client::ensure_session()?;
                    let value = client::reveal_item(c.item_id)?;
                    eprintln!("warning: the value below stays in your terminal scrollback — clear it when done.");
                    println!("{}={}", c.key, value.as_str());
                }
            }
        }
        return Err(CliError::Api(format!("{} key(s) already exist — nothing was added", collisions.len())));
    }

    let url = format!(
        "{}/items?project={}&environment={}&on_conflict=error",
        client::api_base(),
        client::urlencod(&project.name),
        client::urlencod(&env.name)
    );
    let scope_label = format!("{} / {}{}", project.name, env.name, if args.global { ", global" } else { "" });
    let outcome = add_all(&pairs, |key, value| {
        let body = serde_json::json!({
            "id": 0,
            "type": "secret",
            "name": key,
            "value": value,
            "categories": [],
            "created": "",
            "key": key,
        });
        let resp = client::authenticated_post(&url, &body)?;
        if !resp.status().is_success() {
            return Err(CliError::Api(format!("HTTP {}", resp.status())));
        }
        // Items are never created global server-side; flip it explicitly.
        if args.global {
            let id = resp
                .json::<serde_json::Value>()
                .ok()
                .and_then(|v| v.get("id").and_then(|i| i.as_i64()))
                .ok_or_else(|| CliError::Api("added but the response had no id".into()))?;
            client::set_item_global(id)?;
        }
        Ok(())
    });
    for key in &outcome.added {
        eprintln!("Added: {key} ({scope_label})");
    }
    for (key, why) in &outcome.failed {
        // Only the key name and the error — never the value.
        eprintln!("Failed to add '{key}': {why}");
    }
    outcome.into_result()
}

/// Per-key results of an `add`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AddOutcome {
    pub added: Vec<String>,
    pub failed: Vec<(String, String)>,
}

impl AddOutcome {
    /// `Ok` when every key was added; otherwise [`CliError::Partial`] (exit 2)
    /// with a summary that names keys only.
    pub fn into_result(self) -> Result<(), CliError> {
        if self.failed.is_empty() {
            return Ok(());
        }
        let names = |v: Vec<&str>| if v.is_empty() { "none".to_string() } else { v.join(", ") };
        Err(CliError::Partial(format!(
            "added {}: {}; failed {}: {}",
            self.added.len(),
            names(self.added.iter().map(String::as_str).collect()),
            self.failed.len(),
            names(self.failed.iter().map(|(k, _)| k.as_str()).collect()),
        )))
    }
}

/// Adds every pair through `add_one`, continuing past failures so the caller
/// can report all of them. `add_one` errors never carry the value.
pub fn add_all(
    pairs: &[(String, String)],
    mut add_one: impl FnMut(&str, &str) -> Result<(), CliError>,
) -> AddOutcome {
    let mut outcome = AddOutcome::default();
    for (key, value) in pairs {
        match add_one(key, value) {
            Ok(()) => outcome.added.push(key.clone()),
            Err(e) => outcome.failed.push((key.clone(), e.to_string())),
        }
    }
    outcome
}

/// `$VAR` → that variable; `KEY=value` → literal; anything else → a dotenv
/// file path.
pub fn parse_input(input: &str) -> Result<Vec<(String, String)>, CliError> {
    if let Some(var) = input.strip_prefix('$') {
        let value = std::env::var(var).map_err(|_| CliError::NotFound(format!("environment variable ${var}")))?;
        return Ok(vec![(var.to_string(), value)]);
    }
    if !Path::new(input).is_file() {
        if let Some((k, v)) = input.split_once('=') {
            let k = k.trim();
            if k.is_empty() {
                return Err(CliError::Config("KEY=value needs a non-empty KEY".into()));
            }
            return Ok(vec![(k.to_string(), v.to_string())]);
        }
        return Err(CliError::Config(format!(
            "'{input}' is not KEY=value, $VARNAME, or an existing .env file"
        )));
    }
    // Same grammar crypt-env writes (`envfile::serialize_value`), so a file
    // it produced reads back exactly.
    let content = std::fs::read_to_string(input).map_err(|e| CliError::Config(format!("{input}: {}", e.kind())))?;
    Ok(crypt_env_lib::envfile::parse_dotenv(&content))
}

/// Keys of `pairs` already present in `existing` (case-sensitive, like the
/// env files they materialize into).
pub fn find_collisions(pairs: &[(String, String)], existing: &[(String, i64, String)]) -> Vec<Collision> {
    pairs
        .iter()
        .filter_map(|(k, _)| {
            existing
                .iter()
                .find(|(name, ..)| name == k)
                .map(|(_, id, scope)| Collision { key: k.clone(), item_id: *id, scope: scope.clone() })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_literal_env_and_file() {
        assert_eq!(parse_input("A=b=c").unwrap(), vec![("A".into(), "b=c".into())]);
        assert!(parse_input("=x").is_err());
        assert!(parse_input("nothing").is_err());
        std::env::set_var("CRYPTENV_ADD_TEST", "v");
        assert_eq!(parse_input("$CRYPTENV_ADD_TEST").unwrap(), vec![("CRYPTENV_ADD_TEST".into(), "v".into())]);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join(".env");
        std::fs::write(&f, "# c\nX=1\nY=\"two\"\n").unwrap();
        assert_eq!(
            parse_input(f.to_str().unwrap()).unwrap(),
            vec![("X".into(), "1".into()), ("Y".into(), "two".into())]
        );
    }

    #[test]
    fn partial_failure_lists_added_and_failed_keys_and_is_an_error() {
        let pairs: Vec<(String, String)> =
            ["A", "B", "C", "D"].iter().map(|k| (k.to_string(), "s3cret".to_string())).collect();
        let outcome = add_all(&pairs, |k, _| {
            if k == "C" { Err(CliError::Api("HTTP 500".into())) } else { Ok(()) }
        });
        assert_eq!(outcome.added, vec!["A", "B", "D"]);
        assert_eq!(outcome.failed.len(), 1);
        match outcome.into_result() {
            Err(CliError::Partial(msg)) => {
                assert!(msg.contains("added 3: A, B, D") && msg.contains("failed 1: C"), "{msg}");
                assert!(!msg.contains("s3cret"));
            }
            other => panic!("expected Partial, got {other:?}"),
        }
        assert!(add_all(&pairs, |_, _| Ok(())).into_result().is_ok());
    }

    #[test]
    fn collisions_are_reported_with_scope() {
        let pairs = vec![("A".to_string(), "1".to_string()), ("B".to_string(), "2".to_string())];
        let existing = vec![("B".to_string(), 7, "environment 'default'".to_string())];
        assert_eq!(
            find_collisions(&pairs, &existing),
            vec![Collision { key: "B".into(), item_id: 7, scope: "environment 'default'".into() }]
        );
        assert!(find_collisions(&pairs, &[("a".into(), 1, "x".into())]).is_empty());
    }
}
