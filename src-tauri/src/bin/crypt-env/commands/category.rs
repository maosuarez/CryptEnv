use clap::{Args, Subcommand};
use serde::Deserialize;

use crate::client::{authenticated_delete, authenticated_get, authenticated_post, authenticated_put, CliError};

// ─── CLI argument structs ─────────────────────────────────────────────────────

#[derive(Args)]
pub struct CategoryArgs {
    #[command(subcommand)]
    pub cmd: CategoryCmd,
}

#[derive(Subcommand)]
pub enum CategoryCmd {
    /// List all categories
    List,
    /// Create a new category
    Create {
        /// Category name
        #[arg(long)]
        name: String,
        /// Category color (e.g. #FF5733)
        #[arg(long)]
        color: String,
    },
    /// Edit an existing category
    Edit {
        /// Category id
        #[arg(long)]
        id: String,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// New color
        #[arg(long)]
        color: Option<String>,
    },
    /// Delete a category by id
    Delete {
        /// Category id
        #[arg(long)]
        id: String,
    },
}

// ─── Response types ───────────────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct CategoryResponse {
    id: String,
    name: String,
    color: String,
}

// ─── Entry point ──────────────────────────────────────────────────────────────

pub fn run(args: CategoryArgs) -> Result<(), CliError> {
    match args.cmd {
        CategoryCmd::List => run_list(),
        CategoryCmd::Create { name, color } => run_create(name, color),
        CategoryCmd::Edit { id, name, color } => run_edit(id, name, color),
        CategoryCmd::Delete { id } => run_delete(id),
    }
}

// ─── List ─────────────────────────────────────────────────────────────────────

fn run_list() -> Result<(), CliError> {
    let resp = authenticated_get(&format!("{base}/categories", base = crate::client::api_base()))?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("list failed: {text}")));
    }

    let cats: Vec<CategoryResponse> = resp.json().map_err(|e| CliError::Api(e.to_string()))?;

    if cats.is_empty() {
        println!("No categories found.");
        return Ok(());
    }

    println!("{:<38} {:<30} {}", "ID", "Name", "Color");
    println!("{}", "-".repeat(75));
    for cat in cats {
        println!("{:<38} {:<30} {}", cat.id, cat.name, cat.color);
    }

    Ok(())
}

// ─── Create ──────────────────────────────────────────────────────────────────

fn run_create(name: String, color: String) -> Result<(), CliError> {
    let body = serde_json::json!({ "name": name, "color": color });
    let resp = authenticated_post(&format!("{base}/categories", base = crate::client::api_base()), &body)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("create failed: {text}")));
    }

    let cat: CategoryResponse = resp.json().map_err(|e| CliError::Api(e.to_string()))?;
    println!("Category created:");
    println!("  ID:    {}", cat.id);
    println!("  Name:  {}", cat.name);
    println!("  Color: {}", cat.color);

    Ok(())
}

// ─── Edit ─────────────────────────────────────────────────────────────────────

fn run_edit(id: String, name: Option<String>, color: Option<String>) -> Result<(), CliError> {
    if name.is_none() && color.is_none() {
        return Err(CliError::Api(
            "at least one of --name or --color must be provided".to_string(),
        ));
    }

    let mut body = serde_json::json!({});
    if let Some(n) = name {
        body["name"] = serde_json::json!(n);
    }
    if let Some(c) = color {
        body["color"] = serde_json::json!(c);
    }

    let resp = authenticated_put(&format!("{base}/categories/{id}", base = crate::client::api_base()), &body)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(CliError::NotFound(format!("category '{id}'")));
    }
    if !resp.status().is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(CliError::Api(format!("edit failed: {text}")));
    }

    let cat: CategoryResponse = resp.json().map_err(|e| CliError::Api(e.to_string()))?;
    println!("Category updated:");
    println!("  ID:    {}", cat.id);
    println!("  Name:  {}", cat.name);
    println!("  Color: {}", cat.color);

    Ok(())
}

// ─── Delete ───────────────────────────────────────────────────────────────────

fn run_delete(id: String) -> Result<(), CliError> {
    let resp = authenticated_delete(&format!("{base}/categories/{id}", base = crate::client::api_base()))?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(CliError::NotFound(format!("category '{id}'")));
    }
    if resp.status() == reqwest::StatusCode::NO_CONTENT || resp.status().is_success() {
        println!("Category '{id}' deleted.");
        return Ok(());
    }

    let text = resp.text().unwrap_or_default();
    Err(CliError::Api(format!("delete failed: {text}")))
}
