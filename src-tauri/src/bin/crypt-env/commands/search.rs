//! `crypt-env search [PATTERN] [--global]` — after master-password
//! verification, lists variables (keys and metadata, never values) of the
//! workspace project across its environments, or global vault items.
//! `%TEXT` / `TEXT%` / `%TEXT%` are substring matches; anything else is a
//! case-insensitive regex (falling back to substring if it doesn't compile).

use clap::Args;
use std::collections::HashMap;

use crate::client::{self, CliError};
use crate::commands::scope;

#[derive(Args)]
pub struct SearchArgs {
    /// Filter on variable names (omit to list everything)
    pub pattern: Option<String>,

    /// Search global vault items instead of the project's variables
    #[arg(long)]
    pub global: bool,
}

pub struct Row {
    pub key: String,
    pub environment: String,
    pub item_type: String,
    pub scope: &'static str,
}

pub fn run(args: SearchArgs) -> Result<(), CliError> {
    let matcher = Matcher::new(args.pattern.as_deref());
    // Scope carrier: the workspace project if any (required unless --global).
    let ws = match scope::workspace() {
        Ok(ws) => Some(ws),
        Err(e) if !args.global => return Err(e),
        Err(_) => None,
    };
    client::ensure_session()?;
    let project = match &ws {
        Some(ws) => Some(scope::vault_project(ws)?),
        None => None,
    };
    let rows = execute(project.as_ref(), args.global, &matcher)?;
    if rows.is_empty() {
        eprintln!("No matching variables.");
        return Ok(());
    }
    println!("{:<32} {:<16} {:<12} SCOPE", "KEY", "ENVIRONMENT", "TYPE");
    println!("{}", "-".repeat(72));
    for r in &rows {
        println!("{:<32} {:<16} {:<12} {}", r.key, r.environment, r.item_type, r.scope);
    }
    Ok(())
}

/// Core of `search`, shared with the TUI. Values are never fetched.
pub fn execute(project: Option<&client::Project>, global: bool, matcher: &Matcher) -> Result<Vec<Row>, CliError> {
    let mut rows = Vec::new();
    if global {
        // `/items` needs some project/environment scope; any one will do
        // for `include_global=only`.
        let carrier = match project {
            Some(p) => Some(p.clone()),
            None => client::fetch_projects()?.into_iter().find(|p| !p.environments.is_empty()),
        };
        let Some(p) = carrier else { return Ok(rows) };
        let env = p.environments.iter().find(|e| e.is_default).unwrap_or(&p.environments[0]);
        for item in client::list_items(&p.name, &env.name, "only")? {
            let name = item.name.or(item.title).unwrap_or_default();
            if matcher.is_match(&name) {
                rows.push(Row { key: name, environment: "-".into(), item_type: item.item_type, scope: "global" });
            }
        }
    } else if let Some(p) = project {
        for env in &p.environments {
            let meta: HashMap<i64, client::ItemSummary> = client::list_items(&p.name, &env.name, "with")?
                .into_iter()
                .map(|i| (i.id, i))
                .collect();
            for v in env.vars.iter().filter(|v| matcher.is_match(&v.key)) {
                let m = meta.get(&v.item_id);
                rows.push(Row {
                    key: v.key.clone(),
                    environment: if env.name.is_empty() { "(root)".into() } else { env.name.clone() },
                    item_type: m.map(|m| m.item_type.clone()).unwrap_or_else(|| "secret".into()),
                    scope: if m.map(|m| m.is_global).unwrap_or(false) { "global+linked" } else { "project" },
                });
            }
        }
    }
    rows.sort_by(|a, b| a.key.cmp(&b.key).then(a.environment.cmp(&b.environment)));
    Ok(rows)
}

pub enum Matcher {
    All,
    Substring(String),
    Regex(regex::Regex),
}

impl Matcher {
    pub fn new(pattern: Option<&str>) -> Self {
        let Some(p) = pattern.map(str::trim).filter(|p| !p.is_empty()) else { return Matcher::All };
        if p.starts_with('%') || p.ends_with('%') {
            return Matcher::Substring(p.trim_matches('%').to_lowercase());
        }
        match regex::RegexBuilder::new(p).case_insensitive(true).build() {
            Ok(r) => Matcher::Regex(r),
            Err(_) => Matcher::Substring(p.to_lowercase()),
        }
    }

    pub fn is_match(&self, s: &str) -> bool {
        match self {
            Matcher::All => true,
            Matcher::Substring(sub) => s.to_lowercase().contains(sub),
            Matcher::Regex(r) => r.is_match(s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matcher_modes() {
        assert!(Matcher::new(None).is_match("X"));
        let sub = Matcher::new(Some("%TOKEN"));
        assert!(sub.is_match("GITHUB_TOKEN_RO") && !sub.is_match("SECRET"));
        let re = Matcher::new(Some("^db_"));
        assert!(re.is_match("DB_HOST") && !re.is_match("MY_DB_HOST"));
        let broken = Matcher::new(Some("a(b"));
        assert!(broken.is_match("xA(Bx"));
    }
}
