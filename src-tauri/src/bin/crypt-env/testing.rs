//! Test doubles shared by the command and TUI unit tests.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::client::{CliError, Environment, InjectResult, ItemSummary, Project, VaultApi};

/// In-memory [`VaultApi`]. Mutating calls are recorded in `calls` (as the
/// method name) so tests can assert that nothing was written.
pub struct FakeVault {
    pub projects: RefCell<Vec<Project>>,
    pub calls: RefCell<Vec<String>>,
    /// Shared so a test can expire the session after handing the vault to an `App`.
    pub alive: Rc<Cell<bool>>,
    pub inject_paths: RefCell<Vec<String>>,
}

impl FakeVault {
    pub fn new(projects: Vec<Project>) -> Self {
        FakeVault {
            projects: RefCell::new(projects),
            calls: RefCell::new(Vec::new()),
            alive: Rc::new(Cell::new(true)),
            inject_paths: RefCell::new(Vec::new()),
        }
    }

    fn record(&self, name: &str) {
        self.calls.borrow_mut().push(name.to_string());
    }

    /// Calls that would change vault state or write files.
    pub fn mutations(&self) -> Vec<String> {
        self.calls
            .borrow()
            .iter()
            .filter(|c| c.starts_with("save_") || c.as_str() == "ensure_categories" || c.as_str() == "inject_environment")
            .cloned()
            .collect()
    }
}

impl VaultApi for FakeVault {
    fn session_alive(&self) -> Result<bool, CliError> {
        self.record("session_alive");
        Ok(self.alive.get())
    }
    fn ensure_session(&self) -> Result<(), CliError> {
        self.record("ensure_session");
        Ok(())
    }
    fn authenticate(&self, _password: &str) -> Result<(), CliError> {
        self.record("authenticate");
        self.alive.set(true);
        Ok(())
    }
    fn fetch_projects(&self) -> Result<Vec<Project>, CliError> {
        Ok(self.projects.borrow().clone())
    }
    fn find_project(&self, name: &str) -> Result<Option<Project>, CliError> {
        let lower = name.to_lowercase();
        Ok(self.projects.borrow().iter().find(|p| p.name.to_lowercase() == lower).cloned())
    }
    fn save_project(&self, body: &serde_json::Value) -> Result<i64, CliError> {
        self.record("save_project");
        if let Some(id) = body.get("id").and_then(|i| i.as_i64()).filter(|i| *i != 0) {
            return Ok(id);
        }
        // Create: register it so a following fetch finds it.
        let name = body.get("name").and_then(|n| n.as_str()).unwrap_or("new");
        let mut created = project(name, body.get("rootPath").and_then(|r| r.as_str()), vec![]);
        created.id = 99;
        self.projects.borrow_mut().push(created);
        Ok(99)
    }
    fn save_environment(&self, body: &serde_json::Value) -> Result<i64, CliError> {
        self.record("save_environment");
        Ok(body.get("id").and_then(|i| i.as_i64()).unwrap_or(0))
    }
    fn ensure_categories(&self, _names: &[String]) -> Result<(), CliError> {
        self.record("ensure_categories");
        Ok(())
    }
    fn inject_environment(&self, _env_id: i64) -> Result<InjectResult, CliError> {
        self.record("inject_environment");
        Ok(InjectResult { paths: self.inject_paths.borrow().clone(), written: vec![], backups: vec![] })
    }
    fn list_items(&self, _p: &str, _e: &str, _g: &str) -> Result<Vec<ItemSummary>, CliError> {
        Ok(Vec::new())
    }
    fn reveal_item(&self, _item_id: i64) -> Result<zeroize::Zeroizing<String>, CliError> {
        Err(CliError::Api("not available in tests".into()))
    }
}

pub fn env(id: i64, name: &str, default: bool, paths: &[&str]) -> Environment {
    Environment {
        id,
        project_id: 1,
        name: name.into(),
        is_default: default,
        paths: paths.iter().map(|p| p.to_string()).collect(),
        vars: vec![],
        created: "0".into(),
        updated: "0".into(),
    }
}

pub fn project(name: &str, root: Option<&str>, envs: Vec<Environment>) -> Project {
    Project {
        id: 1,
        name: name.into(),
        description: None,
        template: "generic".into(),
        created: "0".into(),
        updated: "0".into(),
        environments: envs,
        categories: vec![],
        root_path: root.map(str::to_string),
    }
}
