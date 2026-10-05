//! `crypt-env add KEY=value | VARNAME | $VARNAME | <file.env>` — adds secrets to the
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
    /// KEY=value, VARNAME or $VARNAME (read from this shell's environment —
    /// prefer the bare name or quote it, or the shell expands it first), or
    /// a path to a .env file
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

/// `$VAR` / `\$VAR` → that variable; an existing file → dotenv entries;
/// `KEY=value` → literal; a bare `VAR` naming a set environment variable →
/// that variable. Anything else is an error that never echoes the argument
/// (it may be a secret value the shell expanded from `$VAR`).
pub fn parse_input(input: &str) -> Result<Vec<(String, String)>, CliError> {
    if let Some(var) = input.strip_prefix("\\$").or_else(|| input.strip_prefix('$')) {
        return from_environment(var);
    }
    if !Path::new(input).is_file() {
        if let Some((k, v)) = input.split_once('=') {
            let k = k.trim();
            if k.is_empty() {
                return Err(CliError::Config("KEY=value needs a non-empty KEY".into()));
            }
            return Ok(vec![(k.to_string(), v.to_string())]);
        }
        if crypt_env_lib::envfile::is_valid_key(input) && std::env::var_os(input).is_some() {
            return from_environment(input);
        }
        return Err(CliError::Config(unrecognized_input_message(&names_holding_value(
            input,
            std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))),
        ))));
    }
    // Same grammar crypt-env writes (`envfile::serialize_value`), so a file
    // it produced reads back exactly.
    let content = std::fs::read_to_string(input).map_err(|e| CliError::Config(format!("{input}: {}", e.kind())))?;
    Ok(crypt_env_lib::envfile::parse_dotenv(&content))
}

/// `NAME` → `(NAME, value)` from this process's environment.
fn from_environment(name: &str) -> Result<Vec<(String, String)>, CliError> {
    if !crypt_env_lib::envfile::is_valid_key(name) {
        return Err(CliError::Config(format!("'{name}' is not a valid environment variable name")));
    }
    let value = std::env::var(name)
        .map_err(|_| CliError::Config(format!("environment variable {name} is not set in this shell")))?;
    Ok(vec![(name.to_string(), value)])
}

/// Names (never values) of the variables whose value is exactly `token`.
/// A hit means the shell already replaced `$NAME` by its value before
/// `crypt-env` started.
fn names_holding_value(token: &str, vars: impl Iterator<Item = (String, String)>) -> Vec<String> {
    if token.is_empty() {
        return Vec::new();
    }
    let mut names: Vec<String> = vars.filter(|(_, v)| v == token).map(|(k, _)| k).collect();
    names.sort();
    names
}

/// Error text for an argument that is none of the accepted forms. Mentions
/// variable names only.
fn unrecognized_input_message(expanded_from: &[String]) -> String {
    const FORMATS: &str = "expected KEY=value, VARNAME (read from this shell's environment), or a .env file path";
    match expanded_from.split_first() {
        None => format!(
            "the argument is not recognized — {FORMATS}. If you meant an environment variable, pass its name \
             without `$` (a bare `$VAR` is expanded by the shell before crypt-env runs)"
        ),
        Some((first, rest)) => {
            let others = if rest.is_empty() { String::new() } else { format!(" (or {} other variable(s))", rest.len()) };
            format!(
                "the argument equals the value of environment variable {first}{others} — your shell expanded `$` \
                 before crypt-env ran. Run `crypt-env add {first}` without the `$` (or quote it: '${first}'); {FORMATS}"
            )
        }
    }
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
        assert_eq!(parse_input("CRYPTENV_ADD_TEST").unwrap(), vec![("CRYPTENV_ADD_TEST".into(), "v".into())]);
        assert_eq!(parse_input("\\$CRYPTENV_ADD_TEST").unwrap(), vec![("CRYPTENV_ADD_TEST".into(), "v".into())]);
        assert!(parse_input("$CRYPTENV_ADD_UNSET_VAR").is_err());
        assert!(parse_input("$not-a-name").is_err());
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
    fn unrecognized_input_error_never_echoes_the_argument() {
        std::env::set_var("CRYPTENV_ADD_EXPANDED", "HolaSecreto-9f3");
        let Err(CliError::Config(msg)) = parse_input("HolaSecreto-9f3") else { panic!("expected a config error") };
        assert!(msg.contains("CRYPTENV_ADD_EXPANDED") && msg.contains("without the `$`"), "{msg}");
        assert!(!msg.contains("HolaSecreto-9f3"), "{msg}");
        let Err(CliError::Config(msg)) = parse_input("no-such-value-anywhere-0d41") else { panic!("expected a config error") };
        assert!(msg.contains("KEY=value") && msg.contains("without `$`"), "{msg}");
        assert!(!msg.contains("no-such-value-anywhere-0d41"), "{msg}");
    }

    #[test]
    fn names_holding_value_lists_names_only() {
        let vars = vec![("B".to_string(), "x".to_string()), ("A".to_string(), "x".to_string()), ("C".to_string(), "y".to_string())];
        assert_eq!(names_holding_value("x", vars.clone().into_iter()), vec!["A", "B"]);
        assert!(names_holding_value("", vars.into_iter()).is_empty());
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
