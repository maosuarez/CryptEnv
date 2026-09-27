use clap::Args;
use dialoguer::{Input, Select};
use regex::Regex;
use std::collections::HashSet;
use crate::client::{self, CliError};

#[derive(Args)]
pub struct MemoryArgs {
    /// The command string to save (use {{VAR}} for placeholders)
    pub command: String,
}

pub fn run(args: MemoryArgs) -> Result<(), CliError> {
    // Auto-detect {{VAR}} placeholders
    let ph_re = Regex::new(r"\{\{([A-Z0-9_]+)\}\}").unwrap();
    let placeholders: Vec<String> = ph_re
        .captures_iter(&args.command)
        .map(|c| c[1].to_string())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    if !placeholders.is_empty() {
        eprintln!("Detected placeholders: {}", placeholders.join(", "));
    }

    let name: String = Input::new()
        .with_prompt("Command name (unique key)")
        .interact_text()
        .map_err(|e| CliError::Api(e.to_string()))?;

    let description: String = Input::new()
        .with_prompt("Description (optional, press Enter to skip)")
        .allow_empty(true)
        .interact_text()
        .map_err(|e| CliError::Api(e.to_string()))?;

    let shell_options = vec!["bash", "pwsh", "sh", "zsh"];
    let shell_idx = Select::new()
        .with_prompt("Shell")
        .items(&shell_options)
        .default(0)
        .interact()
        .map_err(|e| CliError::Api(e.to_string()))?;
    let shell = shell_options[shell_idx];

    let body = serde_json::json!({
        "id": 0,
        "type": "command",
        "name": name,
        "description": if description.is_empty() { serde_json::Value::Null } else { description.clone().into() },
        "command": args.command,
        "shell": shell,
        "categories": [],
        "created": "",
    });

    let resp = client::authenticated_post(
        &format!("{}/items", client::api_base()),
        &body,
    )?;

    if !resp.status().is_success() {
        return Err(CliError::Api(format!(
            "Failed to save: HTTP {}",
            resp.status()
        )));
    }

    eprintln!("Saved command '{}'", name);
    if !placeholders.is_empty() {
        eprintln!("Placeholders: {}", placeholders.join(", "));
    }

    Ok(())
}
