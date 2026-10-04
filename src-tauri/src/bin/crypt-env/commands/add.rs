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
    for (key, value) in &pairs {
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
        if resp.status().is_success() {
            // Items are never created global server-side; flip it explicitly.
            if args.global {
                let id = resp
                    .json::<serde_json::Value>()
                    .ok()
                    .and_then(|v| v.get("id").and_then(|i| i.as_i64()))
                    .ok_or_else(|| CliError::Api(format!("added '{key}' but the response had no id")))?;
                client::set_item_global(id)?;
            }
            eprintln!(
                "Added: {key} ({} / {}{})",
                project.name,
                env.name,
                if args.global { ", global" } else { "" }
            );
        } else {
            // Only the key name — never the value.
            eprintln!("Failed to add '{key}': HTTP {}", resp.status());
        }
    }
    Ok(())
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
