use clap::Args;
use crate::client::{self, CliError, CommandDetail};
use crate::commands::scope::{self, ResolvedScope};

#[derive(Args)]
pub struct CmdArgs {
    /// Name of the command (omit to use --list)
    pub name: Option<String>,
    /// List all saved commands
    #[arg(long)]
    pub list: bool,
    /// Show command details (name, description, placeholders)
    #[arg(long)]
    pub info: bool,
    /// Placeholder values: --VAR=value
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub vars: Vec<String>,

    /// Project name (defaults to crypt-env.json or the cwd folder name)
    #[arg(long)]
    pub project: Option<String>,

    /// Environment name (defaults to crypt-env.json or the project's default environment)
    #[arg(long = "env")]
    pub env: Option<String>,
}

pub fn run(args: CmdArgs) -> Result<(), CliError> {
    let resolved_scope = scope::resolve(args.project.as_deref(), args.env.as_deref(), false)?;
    match (args.name.as_deref(), args.list, args.info) {
        (None, true, _) => list_commands(&resolved_scope),
        (Some(name), _, true) => command_info(name, &resolved_scope),
        (Some(name), _, false) => run_command(name, &args.vars, &resolved_scope),
        (None, false, _) => {
            eprintln!("Use --list to list commands or provide a name.");
            std::process::exit(1);
        }
    }
}

fn list_commands(resolved_scope: &ResolvedScope) -> Result<(), CliError> {
    let url = resolved_scope.append_query(&format!("{}/commands", client::api_base()));
    let resp = client::authenticated_get(&url)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        let code = resp.status();
        return Err(CliError::Api(format!("HTTP error {code}")));
    }

    let commands: Vec<CommandDetail> = resp.json().map_err(|e| CliError::Api(e.to_string()))?;

    if commands.is_empty() {
        eprintln!("No saved commands in vault.");
        return Ok(());
    }

    println!("{:<24} {:<40} SHELL", "NAME", "DESCRIPTION");
    println!("{}", "-".repeat(80));
    for cmd in &commands {
        println!(
            "{:<24} {:<40} {}",
            cmd.name,
            cmd.description.as_deref().unwrap_or(""),
            cmd.shell.as_deref().unwrap_or("")
        );
    }

    Ok(())
}

fn command_info(name: &str, resolved_scope: &ResolvedScope) -> Result<(), CliError> {
    let url = resolved_scope.append_query(&format!("{}/commands", client::api_base()));
    let resp = client::authenticated_get(&url)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        let code = resp.status();
        return Err(CliError::Api(format!("HTTP error {code}")));
    }

    let commands: Vec<CommandDetail> = resp.json().map_err(|e| CliError::Api(e.to_string()))?;

    let found = client::resolve_command_by_name(commands, name);

    let cmd_id = match found {
        Some(c) => c.id,
        None => return Err(CliError::NotFound(name.to_string())),
    };

    let url2 = format!("{}/commands/{}", client::api_base(), cmd_id);
    let resp2 = client::authenticated_get(&url2)?;

    if resp2.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(CliError::NotFound(name.to_string()));
    }
    if !resp2.status().is_success() {
        let code = resp2.status();
        return Err(CliError::Api(format!("HTTP error {code}")));
    }

    let detail: CommandDetail = resp2.json().map_err(|e| CliError::Api(e.to_string()))?;
    let command_template = detail.command.as_deref().unwrap_or("");

    println!("Name:         {}", detail.name);
    println!("Description:  {}", detail.description.as_deref().unwrap_or(""));
    println!("Shell:        {}", detail.shell.as_deref().unwrap_or(""));
    println!("Command:      {}", command_template);
    if !detail.placeholders.is_empty() {
        println!("Placeholders: {}", detail.placeholders.join(", "));
    }

    Ok(())
}

fn run_command(name: &str, vars: &[String], resolved_scope: &ResolvedScope) -> Result<(), CliError> {
    let url = resolved_scope.append_query(&format!("{}/commands", client::api_base()));
    let resp = client::authenticated_get(&url)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        let code = resp.status();
        return Err(CliError::Api(format!("HTTP error {code}")));
    }

    let commands: Vec<CommandDetail> = resp.json().map_err(|e| CliError::Api(e.to_string()))?;

    let cmd = client::resolve_command_by_name(commands, name)
        .ok_or_else(|| CliError::NotFound(name.to_string()))?;

    let mut template = cmd.command.unwrap_or_default();

    if !vars.is_empty() {
        let replacements = client::parse_vars(vars);
        for (key, val) in &replacements {
            let placeholder = format!("{{{{{}}}}}", key);
            template = template.replace(&placeholder, val);
        }
    }

    println!("{}", template);
    Ok(())
}
