//! `crypt-env inject <KEY>...` — after master-password verification, prints
//! shell assignments for `eval` on stdout only (e.g.
//! `eval "$(crypt-env inject DATABASE_URL)"`). The password prompt and hints
//! go to the terminal/stderr, never the value.

use clap::Args;

use crate::client::{self, CliError};
use crate::commands::scope;
use crate::shell::{detect_shell, format_assignment, Shell};

#[derive(Args)]
pub struct InjectArgs {
    /// Variable name(s) to inject
    #[arg(required = true)]
    pub keys: Vec<String>,

    /// Environment to read from (defaults to the project's default environment)
    #[arg(long = "env")]
    pub env: Option<String>,

    /// Force shell format: pwsh, bash, zsh, sh
    #[arg(long)]
    pub shell: Option<String>,
}

pub fn run(args: InjectArgs) -> Result<(), CliError> {
    let shell = match args.shell.as_deref() {
        Some("pwsh") | Some("powershell") => Shell::PowerShell,
        Some("bash") => Shell::Bash,
        Some("zsh") => Shell::Zsh,
        Some("sh") => Shell::Sh,
        Some(other) => return Err(CliError::Config(format!("unknown --shell '{other}' (pwsh, bash, zsh, sh)"))),
        None => detect_shell(),
    };
    for k in &args.keys {
        if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(CliError::Config(format!("'{k}' is not a valid variable name")));
        }
    }
    let ws = scope::workspace()?;
    client::ensure_session()?;
    let project = scope::vault_project(&ws)?;
    let env = scope::pick_environment(&project, ws.default_env.as_deref(), args.env.as_deref())?.clone();

    // Resolve every key before printing anything: a partial output would be
    // half-applied by `eval`.
    let globals = client::list_items(&project.name, &env.name, "only")?;
    let mut ids = Vec::with_capacity(args.keys.len());
    for k in &args.keys {
        let id = env
            .vars
            .iter()
            .find(|v| &v.key == k)
            .map(|v| v.item_id)
            .or_else(|| globals.iter().find(|g| g.name.as_deref() == Some(k.as_str())).map(|g| g.id))
            .ok_or_else(|| CliError::NotFound(format!("{k} in {} / {}", project.name, env.name)))?;
        ids.push(id);
    }
    let mut out = Vec::with_capacity(ids.len());
    for (k, id) in args.keys.iter().zip(ids) {
        let value = client::reveal_item(id)?;
        out.push(zeroize::Zeroizing::new(format_assignment(&shell, k, &value)));
    }
    for line in &out {
        println!("{}", line.as_str());
    }
    Ok(())
}
