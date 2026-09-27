use std::collections::HashMap;

use clap::{Args, Subcommand};
use comfy_table::{presets::UTF8_FULL, ContentArrangement, Table};
use serde::Deserialize;

use crate::client::{authenticated_delete, authenticated_get, authenticated_post, CliError, ItemSummary};

// ─── CLI argument structs ─────────────────────────────────────────────────────

#[derive(Args)]
pub struct ProjectArgs {
    #[command(subcommand)]
    pub cmd: ProjectCmd,
}

#[derive(Subcommand)]
pub enum ProjectCmd {
    /// List all projects with their environments
    List,
    /// Inject an environment's vars into its configured .env path(s)
    Inject {
        /// Environment ID
        #[arg(long, conflicts_with_all = ["project", "environment"])]
        id: Option<i64>,
        /// Project name (case-insensitive, used together with --environment)
        #[arg(long, requires = "environment")]
        project: Option<String>,
        /// Environment name within the project (case-insensitive)
        #[arg(long, requires = "project")]
        environment: Option<String>,
    },
    /// Delete a project (and all its environments) by ID
    Delete {
        /// Project ID to delete
        #[arg(long)]
        id: i64,
    },
    /// Delete a single environment by ID
    DeleteEnv {
        /// Environment ID to delete
        #[arg(long)]
        id: i64,
    },
    /// Share a whole project (structure + values, selected environments) via internet relay
    Share {
        /// Project ID (numeric)
        #[arg(long, conflicts_with = "name")]
        id: Option<i64>,
        /// Project name (case-insensitive, used if --id is not given)
        #[arg(long)]
        name: Option<String>,
        /// Comma-separated environment names to include (default: the project's
        /// default environment only). Pass "all" to include every environment.
        #[arg(long)]
        envs: Option<String>,
        /// Skip the confirmation prompt
        #[arg(long)]
        yes: bool,
    },
    /// Receive a project shared via internet relay — always creates a new project
    Receive {
        /// Relay code (e.g. X7K2-M9P4)
        #[arg(long)]
        code: String,
        /// Passphrase provided by the sender
        #[arg(long)]
        passphrase: String,
        /// Project name to use instead of the sender's, if it collides with an existing one
        #[arg(long = "as")]
        as_name: Option<String>,
    },
}

// ─── Response types ───────────────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct EnvironmentVar {
    key: String,
    #[serde(rename = "itemId")]
    item_id: i64,
}

#[derive(Deserialize, Debug)]
struct EnvironmentSummary {
    id: i64,
    name: String,
    #[serde(default, rename = "isDefault")]
    is_default: bool,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    vars: Vec<EnvironmentVar>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct ProjectSummary {
    id: i64,
    name: String,
    template: String,
    #[serde(default)]
    environments: Vec<EnvironmentSummary>,
}

#[derive(Deserialize, Debug)]
struct InjectResult {
    paths: Vec<String>,
    written: Vec<String>,
}

// ─── Entry point ──────────────────────────────────────────────────────────────

pub fn run(args: ProjectArgs) -> Result<(), CliError> {
    match args.cmd {
        ProjectCmd::List => run_list(),
        ProjectCmd::Inject { id, project, environment } => run_inject(id, project, environment),
        ProjectCmd::Delete { id } => run_delete(id),
        ProjectCmd::DeleteEnv { id } => run_delete_env(id),
        ProjectCmd::Share { id, name, envs, yes } => run_share(id, name, envs, yes),
        ProjectCmd::Receive { code, passphrase, as_name } => run_receive(code, passphrase, as_name),
    }
}

// ─── List ─────────────────────────────────────────────────────────────────────

fn fetch_projects() -> Result<Vec<ProjectSummary>, CliError> {
    let resp = authenticated_get(&format!("{base}/projects", base = crate::client::api_base()))?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("list projects failed: {text}")));
    }

    resp.json().map_err(|e| CliError::Api(e.to_string()))
}

fn run_list() -> Result<(), CliError> {
    let projects = fetch_projects()?;

    if projects.is_empty() {
        println!("No projects found.");
        return Ok(());
    }

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec!["Project", "Template", "Environment", "Paths", "Vars"]);

    for p in &projects {
        if p.environments.is_empty() {
            table.add_row(vec![p.name.clone(), p.template.clone(), "(none)".into(), "(none)".into(), "0".into()]);
            continue;
        }
        for e in &p.environments {
            table.add_row(vec![
                p.name.clone(),
                p.template.clone(),
                e.name.clone(),
                if e.paths.is_empty() { "(none)".into() } else { e.paths.join(", ") },
                e.vars.len().to_string(),
            ]);
        }
    }

    println!("{table}");
    Ok(())
}

// ─── Inject ───────────────────────────────────────────────────────────────────

fn run_inject(id: Option<i64>, project: Option<String>, environment: Option<String>) -> Result<(), CliError> {
    // Resolve environment ID: use --id directly, or look up by --project/--environment
    let environment_id = match (id, project, environment) {
        (Some(i), _, _) => i,
        (None, Some(p), Some(e)) => resolve_environment_id(&p, &e)?,
        _ => {
            return Err(CliError::Api(
                "provide --id, or both --project and --environment to identify the environment".into(),
            ));
        }
    };

    let resp = authenticated_post(
        &format!("{base}/environments/{environment_id}/inject", base = crate::client::api_base()),
        &serde_json::json!({}),
    )?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(CliError::NotFound(format!("environment {environment_id}")));
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("inject failed: {text}")));
    }

    let result: InjectResult = resp.json().map_err(|e| CliError::Api(e.to_string()))?;

    for path in &result.paths {
        println!("Injected {} var(s) into: {}", result.written.len(), path);
    }
    for key in &result.written {
        println!("  + {key}");
    }

    Ok(())
}

/// Find the environment ID matching a project name + environment name (both case-insensitive).
fn resolve_environment_id(project: &str, environment: &str) -> Result<i64, CliError> {
    let projects = fetch_projects()?;
    let project_lower = project.to_lowercase();
    let env_lower = environment.to_lowercase();

    projects
        .into_iter()
        .find(|p| p.name.to_lowercase() == project_lower)
        .and_then(|p| p.environments.into_iter().find(|e| e.name.to_lowercase() == env_lower))
        .map(|e| e.id)
        .ok_or_else(|| CliError::NotFound(format!("{project} / {environment}")))
}

// ─── Delete ───────────────────────────────────────────────────────────────────

fn run_delete(id: i64) -> Result<(), CliError> {
    let resp = authenticated_delete(&format!("{base}/projects/{id}", base = crate::client::api_base()))?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(CliError::NotFound(format!("project {id}")));
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("delete project failed: {text}")));
    }

    println!("Project {id} deleted.");
    Ok(())
}

fn run_delete_env(id: i64) -> Result<(), CliError> {
    let resp = authenticated_delete(&format!("{base}/environments/{id}", base = crate::client::api_base()))?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(CliError::NotFound(format!("environment {id}")));
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("delete environment failed: {text}")));
    }

    println!("Environment {id} deleted.");
    Ok(())
}

// ─── Share ────────────────────────────────────────────────────────────────────
// Any environment whose name looks production-like gets a distinct warning in
// the manifest before the confirmation prompt (D4) — a heuristic, not a hard
// gate: it only ever adds friction, never removes it.
const PROD_NAME_PATTERN: &[&str] = &["prod", "production", "live", "release"];

fn looks_production(env_name: &str) -> bool {
    let lower = env_name.to_lowercase();
    PROD_NAME_PATTERN.iter().any(|p| lower.contains(p))
}

fn resolve_project(id: Option<i64>, name: Option<&str>) -> Result<ProjectSummary, CliError> {
    let projects = fetch_projects()?;
    if let Some(id) = id {
        return projects.into_iter().find(|p| p.id == id).ok_or_else(|| CliError::NotFound(format!("project {id}")));
    }
    if let Some(name) = name {
        let name_lower = name.to_lowercase();
        return projects
            .into_iter()
            .find(|p| p.name.to_lowercase() == name_lower)
            .ok_or_else(|| CliError::NotFound(format!("project '{name}'")));
    }
    Err(CliError::Api("provide --id or --name to identify the project".into()))
}

/// Selects which environments to include, given `--envs` (comma-separated
/// names, or "all"). Omitted defaults to the project's default environment
/// only — the same safety default as the GUI's checklist (D4): under-sharing
/// costs one extra round trip, over-sharing is unrecoverable.
fn select_environments<'a>(project: &'a ProjectSummary, envs: Option<&str>) -> Result<Vec<&'a EnvironmentSummary>, CliError> {
    if project.environments.is_empty() {
        return Err(CliError::Api(format!("project '{}' has no environments", project.name)));
    }

    match envs {
        None => {
            let default_env = project
                .environments
                .iter()
                .find(|e| e.is_default)
                .unwrap_or(&project.environments[0]);
            Ok(vec![default_env])
        }
        Some("all") => Ok(project.environments.iter().collect()),
        Some(list) => {
            let mut selected = Vec::new();
            for want in list.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                let want_lower = want.to_lowercase();
                let env = project
                    .environments
                    .iter()
                    .find(|e| e.name.to_lowercase() == want_lower)
                    .ok_or_else(|| CliError::NotFound(format!("environment '{want}' in project '{}'", project.name)))?;
                selected.push(env);
            }
            if selected.is_empty() {
                return Err(CliError::Api("--envs resolved to no environments".into()));
            }
            Ok(selected)
        }
    }
}

/// Fetches item metadata (name, never value) linked into `environment_id`, so
/// the manifest can show `KEY -> item name` instead of raw item ids — reuses
/// the existing `GET /items` endpoint rather than adding a new one (D4).
fn fetch_environment_item_names(environment_id: i64) -> Result<HashMap<i64, String>, CliError> {
    let resp = authenticated_get(&format!("{base}/items?environment_id={environment_id}", base = crate::client::api_base()))?;
    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("list items failed: {text}")));
    }
    let items: Vec<ItemSummary> = resp.json().map_err(|e| CliError::Api(e.to_string()))?;
    Ok(items
        .into_iter()
        .map(|i| (i.id, i.name.or(i.title).unwrap_or_else(|| format!("#{}", i.id))))
        .collect())
}

fn run_share(id: Option<i64>, name: Option<String>, envs: Option<String>, yes: bool) -> Result<(), CliError> {
    let project = resolve_project(id, name.as_deref())?;
    let selected = select_environments(&project, envs.as_deref())?;

    println!(
        "About to share project '{}' ({} environment{}):",
        project.name,
        selected.len(),
        if selected.len() != 1 { "s" } else { "" }
    );
    for env in &selected {
        let warn = if looks_production(&env.name) { "  /!\\ PRODUCTION-LIKE NAME" } else { "" };
        println!("  {}{}", env.name, warn);
        if env.vars.is_empty() {
            println!("    (no variables)");
            continue;
        }
        let names = fetch_environment_item_names(env.id)?;
        for v in &env.vars {
            let item_name = names.get(&v.item_id).cloned().unwrap_or_else(|| format!("#{}", v.item_id));
            println!("    {} -> {}", v.key, item_name);
        }
    }
    println!();
    println!("Values leave this machine via the encrypted relay. Never shown here.");

    if !yes {
        print!("Proceed? [y/N]: ");
        if !prompt_confirm()? {
            println!("Cancelled.");
            return Ok(());
        }
    }

    let environment_ids: Vec<i64> = selected.iter().map(|e| e.id).collect();
    let body = serde_json::json!({ "environment_ids": environment_ids });
    let resp = authenticated_post(&format!("{}/projects/{}/relay/send", crate::client::api_base(), project.id), &body)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(CliError::NotFound(format!("project {}", project.id)));
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("project share failed: {text}")));
    }

    #[derive(Deserialize)]
    struct ShareResult {
        code: String,
        passphrase: String,
        environment_count: usize,
        item_count: usize,
    }
    let result: ShareResult = resp.json().map_err(|e| CliError::Api(e.to_string()))?;

    println!();
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║  RELAY SHARE CODE AND PASSPHRASE — show to recipient now ║");
    println!("╠══════════════════════════════════════════════════════════╣");
    println!("║  Code:       {:<46} ║", result.code);
    println!("║  Passphrase: {:<46} ║", result.passphrase);
    println!("╚══════════════════════════════════════════════════════════╝");
    println!();
    println!(
        "{} environment(s), {} item(s). The passphrase is shown once. Provide both to the recipient.",
        result.environment_count, result.item_count
    );

    Ok(())
}

// ─── Receive ──────────────────────────────────────────────────────────────────

fn run_receive(code: String, passphrase: String, as_name: Option<String>) -> Result<(), CliError> {
    let mut body = serde_json::json!({ "code": code, "passphrase": passphrase });
    if let Some(name) = &as_name {
        body["project_name_override"] = serde_json::json!(name);
    }

    let resp = authenticated_post(&format!("{base}/projects/relay/receive", base = crate::client::api_base()), &body)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if resp.status() == reqwest::StatusCode::CONFLICT {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("{text} — retry with --as NAME to choose a different project name")));
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("project receive failed: {text}")));
    }

    #[derive(Deserialize)]
    struct ReceiveResult {
        project: String,
        environments: Vec<String>,
        item_count: usize,
    }
    let result: ReceiveResult = resp.json().map_err(|e| CliError::Api(e.to_string()))?;

    println!(
        "Received project '{}' — {} environment(s), {} item(s):",
        result.project,
        result.environments.len(),
        result.item_count
    );
    for name in &result.environments {
        println!("  + {name}");
    }
    println!("Run `crypt-env project list` to see it — set paths on each environment before injecting.");

    Ok(())
}

// ─── Prompt helper ────────────────────────────────────────────────────────────

fn prompt_confirm() -> Result<bool, CliError> {
    use std::io::{self, Write};
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input).map_err(CliError::Io)?;
    Ok(matches!(input.trim().to_lowercase().as_str(), "y" | "yes"))
}
