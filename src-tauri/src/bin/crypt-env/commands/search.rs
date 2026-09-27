use clap::Args;
use crate::client::{self, CliError, ItemSummary};
use crate::commands::scope;

#[derive(Args)]
pub struct SearchArgs {
    /// Text to search for
    pub query: String,

    /// Project name (defaults to crypt-env.json or the cwd folder name)
    #[arg(long)]
    pub project: Option<String>,

    /// Environment name (defaults to crypt-env.json or the project's default environment)
    #[arg(long = "env")]
    pub env: Option<String>,

    /// Whether to include reusable global items not linked into this
    /// environment: `with` (default) unions them in, `without` matches
    /// pre-issue-13 behaviour (linked items only), `only` returns globals
    /// regardless of linkage.
    #[arg(long = "scope-globals", default_value = "with")]
    pub scope_globals: String,
}

pub fn run(args: SearchArgs) -> Result<(), CliError> {
    let resolved_scope = scope::resolve(args.project.as_deref(), args.env.as_deref(), false)?;
    let url = resolved_scope.append_query(&format!(
        "{}/items?search={}&include_global={}",
        client::api_base(),
        client::urlencod(&args.query),
        client::urlencod(&args.scope_globals)
    ));
    let resp = client::authenticated_get(&url)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if resp.status() == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("invalid --scope-globals value: {text}")));
    }
    if !resp.status().is_success() {
        let code = resp.status();
        return Err(CliError::Api(format!("HTTP error {code}")));
    }

    let items: Vec<ItemSummary> = resp.json().map_err(|e| CliError::Api(e.to_string()))?;

    if items.is_empty() {
        eprintln!("No results found for '{}'.", args.query);
        return Ok(());
    }

    println!("{:<6} {:<16} {:<32} {:<16} CATEGORIES", "ID", "TYPE", "NAME/TITLE", "SCOPE");
    println!("{}", "-".repeat(96));
    for item in &items {
        let display_name = item
            .name
            .as_deref()
            .or(item.title.as_deref())
            .unwrap_or("");
        println!(
            "{:<6} {:<16} {:<32} {:<16} {}",
            item.id,
            item.item_type,
            display_name,
            scope_label(item.is_global, item.linked),
            item.categories.join(", ")
        );
    }

    Ok(())
}

/// `linked`/`global`/`global+linked` discriminator for the SCOPE column.
fn scope_label(is_global: bool, linked: bool) -> &'static str {
    match (linked, is_global) {
        (true, true) => "global+linked",
        (true, false) => "linked",
        (false, true) => "global",
        (false, false) => "",
    }
}
