use clap::Args;
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::client::{self, CliError};
use crate::commands::scope::{self, ResolvedScope};

#[derive(Args)]
pub struct FillArgs {
    /// Path to .env or .env.example. Defaults to searching the current
    /// directory, then falling back to generating a fresh .env from the
    /// resolved environment's vars (the inverse of `add`).
    pub path: Option<PathBuf>,

    /// Project name (defaults to crypt-env.json or the cwd folder name)
    #[arg(long)]
    pub project: Option<String>,

    /// Environment name (defaults to crypt-env.json or the project's default environment)
    #[arg(long = "env")]
    pub env: Option<String>,

    /// Overwrite the output file even if it already exists and was not
    /// created by crypt-env. The prior contents are copied to `<path>.bak`
    /// first. Without this flag, an existing unmanaged file causes the
    /// command to fail loudly (HTTP 409) rather than silently truncate it.
    /// No interactive prompt: `fill` must behave identically under a
    /// terminal or a pipe/CI runner.
    #[arg(short = 'f', long = "force")]
    pub force: bool,
}

#[derive(Deserialize)]
struct FillResponse {
    #[serde(default)]
    path: Option<String>,
    injected: usize,
    not_found: usize,
    #[serde(default)]
    missing_keys: Vec<String>,
}

pub fn run(args: FillArgs) -> Result<(), CliError> {
    let resolved_scope = scope::resolve(args.project.as_deref(), args.env.as_deref(), false)?;

    let (template, output_path) = resolve_template_and_output(args.path.as_deref(), &resolved_scope)?;

    let body = serde_json::json!({
        "template": template,
        "output_path": output_path.to_string_lossy(),
        "overwrite": args.force,
    });

    let url = resolved_scope.append_query(&format!("{base}/fill", base = crate::client::api_base()));
    let resp = client::authenticated_post(&url, &body)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if resp.status() == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("scope error: {text}")));
    }
    if resp.status() == reqwest::StatusCode::CONFLICT {
        // Existing file not created by crypt-env: print the server's
        // message (names the path and the `--force` remedy) as-is.
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(text));
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("fill failed: {text}")));
    }

    let result: FillResponse = resp.json().map_err(|e| CliError::Api(e.to_string()))?;

    for key in &result.missing_keys {
        eprintln!("warning: '{}' not found in {} / {}", key, resolved_scope.project, resolved_scope.environment);
    }

    let path_display = result.path.unwrap_or_else(|| output_path.display().to_string());
    eprintln!(
        "OK: {} ({} secrets injected, {} not found)",
        path_display, result.injected, result.not_found
    );
    eprintln!("Warning: '{}' contains secrets in plaintext. Keep permissions restrictive.", path_display);

    Ok(())
}

/// Determines the template content and output path, mirroring `add`'s
/// defaulting in reverse:
///   - explicit `path` arg: fill that file (`.env.example` → sibling `.env`,
///     otherwise fill in place).
///   - no arg: look for `.env.example` then `.env` in the current directory.
///   - neither exists: generate a fresh `.env` template from the resolved
///     environment's own variable keys (the "inverse of add" default).
fn resolve_template_and_output(
    arg: Option<&Path>,
    resolved_scope: &ResolvedScope,
) -> Result<(String, PathBuf), CliError> {
    if let Some(p) = arg {
        let is_example = p
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".example"))
            .unwrap_or(false);
        let content = std::fs::read_to_string(p)?;
        let output = if is_example {
            p.parent().unwrap_or(Path::new(".")).join(".env")
        } else {
            p.to_path_buf()
        };
        return Ok((content, output));
    }

    let cwd = std::env::current_dir()?;
    let example = cwd.join(".env.example");
    if example.exists() {
        let content = std::fs::read_to_string(&example)?;
        return Ok((content, cwd.join(".env")));
    }

    let plain = cwd.join(".env");
    if plain.exists() {
        let content = std::fs::read_to_string(&plain)?;
        return Ok((content, plain));
    }

    // Neither exists — generate a fresh .env template from the environment's
    // own variable keys, then let the server fill real values into it.
    let keys = environment_var_keys(resolved_scope)?;
    let mut template = keys.into_iter().map(|k| format!("{k}=")).collect::<Vec<_>>().join("\n");
    if !template.is_empty() {
        template.push('\n');
    }
    Ok((template, cwd.join(".env")))
}

#[derive(Deserialize)]
struct EnvVarLookup {
    key: String,
}

#[derive(Deserialize)]
struct EnvLookup {
    name: String,
    vars: Vec<EnvVarLookup>,
}

#[derive(Deserialize)]
struct ProjectLookup {
    name: String,
    environments: Vec<EnvLookup>,
}

/// Reads the resolved environment's variable keys straight from `GET
/// /projects` — used only to build the "nothing specified" default template.
fn environment_var_keys(resolved_scope: &ResolvedScope) -> Result<Vec<String>, CliError> {
    let resp = client::authenticated_get(&format!("{base}/projects", base = crate::client::api_base()))?;
    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        return Err(CliError::Api(format!("list projects failed: HTTP {}", resp.status())));
    }
    let projects: Vec<ProjectLookup> = resp.json().map_err(|e| CliError::Api(e.to_string()))?;

    let project_lower = resolved_scope.project.to_lowercase();
    let env_lower = resolved_scope.environment.to_lowercase();

    let project = projects
        .into_iter()
        .find(|p| p.name.to_lowercase() == project_lower)
        .ok_or_else(|| CliError::NotFound(resolved_scope.project.clone()))?;
    let environment = project
        .environments
        .into_iter()
        .find(|e| e.name.to_lowercase() == env_lower)
        .ok_or_else(|| CliError::NotFound(resolved_scope.environment.clone()))?;

    let mut keys: Vec<String> = environment.vars.into_iter().map(|v| v.key).collect();
    keys.sort();
    Ok(keys)
}
