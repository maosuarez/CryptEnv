use clap::Args;
use regex::Regex;
use crate::client::{self, CliError, CommandDetail};
use crate::commands::scope;
use crate::shell::{detect_shell, Shell};

#[derive(Args)]
pub struct ExecArgs {
    /// Name of the saved command
    pub name: String,

    /// Placeholder values: --VAR=value
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub vars: Vec<String>,

    /// Force shell for execution (pwsh/bash)
    #[arg(long)]
    pub shell: Option<String>,

    /// Project name (defaults to crypt-env.json or the cwd folder name)
    #[arg(long)]
    pub project: Option<String>,

    /// Environment name (defaults to crypt-env.json or the project's default environment)
    #[arg(long = "env")]
    pub env: Option<String>,
}

pub fn run(args: ExecArgs) -> Result<(), CliError> {
    let resolved_scope = scope::resolve(args.project.as_deref(), args.env.as_deref(), false)?;
    let url = resolved_scope.append_query(&format!("{}/commands", client::api_base()));
    let resp = client::authenticated_get(&url)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        return Err(CliError::Api(format!("HTTP {}", resp.status())));
    }

    let commands: Vec<CommandDetail> = resp.json().map_err(|e| CliError::Api(e.to_string()))?;

    let cmd = client::resolve_command_by_name(commands, &args.name)
        .ok_or_else(|| CliError::NotFound(args.name.clone()))?;

    let mut template = cmd.command.unwrap_or_default();

    // Resolve {{VAR}} placeholders from --KEY=value args
    let replacements = client::parse_vars(&args.vars);

    // Verify all required placeholders are provided before running
    let ph_re = Regex::new(r"\{\{([A-Z0-9_]+)\}\}").unwrap();
    let required: Vec<String> = ph_re
        .captures_iter(&template)
        .map(|c| c[1].to_string())
        .collect();

    for req in &required {
        if !replacements.contains_key(req) {
            return Err(CliError::Api(format!(
                "Missing placeholder: --{}=<value>",
                req
            )));
        }
    }

    for (key, val) in &replacements {
        template = template.replace(&format!("{{{{{}}}}}", key), val);
    }

    let forced_shell = match args.shell.as_deref() {
        Some("pwsh") | Some("powershell") => Some(Shell::PowerShell),
        Some("bash") => Some(Shell::Bash),
        _ => None,
    };

    // Prefer direct argv execution via shlex to avoid shell injection.
    // Fall back to shell only when forced or when direct spawn fails.
    let status = if let Some(sh) = forced_shell {
        match sh {
            Shell::PowerShell => std::process::Command::new("powershell")
                .args(["-Command", &template])
                .status(),
            _ => std::process::Command::new("bash")
                .args(["-c", &template])
                .status(),
        }
    } else {
        // Tokenize with shlex and spawn without a shell wrapper
        match shlex::split(&template) {
            Some(tokens) if !tokens.is_empty() => std::process::Command::new(&tokens[0])
                .args(&tokens[1..])
                .status(),
            _ => {
                return Err(CliError::Api(
                    "Failed to tokenize command — use --shell to force shell execution"
                        .to_string(),
                ))
            }
        }
    };

    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(CliError::Api(format!("Command exited with status: {}", s))),
        Err(e) => {
            // Direct spawn failed — retry via detected shell
            eprintln!("Direct spawn failed ({}), retrying via shell...", e);
            let shell = detect_shell();
            let status2 = match shell {
                Shell::PowerShell => std::process::Command::new("powershell")
                    .args(["-Command", &template])
                    .status(),
                _ => std::process::Command::new("bash")
                    .args(["-c", &template])
                    .status(),
            };
            status2.map(|_| ()).map_err(CliError::Io)
        }
    }
}
