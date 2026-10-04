//! Output-path confinement for the MCP principal.
//!
//! An MCP caller may write generated files only inside a registered project
//! root, and may never overwrite a file crypt-env does not manage. Session
//! callers (the user) are unaffected.

use axum::response::Response;
use std::path::{Path, PathBuf};

use super::auth::mcp_forbidden;
use super::ApiState;
use crate::{envfile, fsguard};

/// Every non-empty `root_path` registered on a project.
pub(super) async fn project_roots(state: &ApiState) -> Vec<PathBuf> {
    let vault = state.vault.lock().await;
    vault
        .db
        .list_projects()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| p.root_path)
        .filter(|r| !r.trim().is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Resolves the absolute `target` under one of `roots`, proving after symlink
/// resolution that it stays inside (`fsguard::contain_relative`). `None` when
/// the target is relative, equals a root, or is outside every root.
pub(super) fn contain_in_roots(roots: &[PathBuf], target: &Path) -> Option<PathBuf> {
    if !target.is_absolute() {
        return None;
    }
    for root in roots {
        let mut bases = vec![root.clone()];
        if let Ok(real) = root.canonicalize() {
            bases.push(real);
        }
        for base in bases {
            if let Ok(rel) = target.strip_prefix(&base) {
                if let Some(rel) = rel.to_str() {
                    if let Ok(resolved) = fsguard::contain_relative(root, rel) {
                        return Some(resolved);
                    }
                }
            }
        }
    }
    None
}

/// Checks a file the MCP principal wants written. `Err` is the ready 403.
pub(super) async fn authorize_file(state: &ApiState, path: &Path, overwrite: bool) -> Result<(), Response> {
    let roots = project_roots(state).await;
    let Some(resolved) = contain_in_roots(&roots, path) else {
        return Err(mcp_forbidden("output path is outside every registered project root"));
    };
    if overwrite {
        // Fail closed: an unreadable target is treated like a foreign one.
        match envfile::inspect(&resolved) {
            Ok(envfile::Target::Foreign) | Err(_) => {
                return Err(mcp_forbidden("overwriting a file that crypt-env does not manage"));
            }
            Ok(_) => {}
        }
    }
    Ok(())
}

/// Checks a directory the MCP principal wants a file generated in.
pub(super) async fn authorize_dir(state: &ApiState, dir: &str) -> Result<(), Response> {
    let roots = project_roots(state).await;
    // A probe file name proves the directory is a root or inside one.
    if contain_in_roots(&roots, &Path::new(dir).join("probe")).is_none() {
        return Err(mcp_forbidden("output directory is outside every registered project root"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_root_paths_resolve_and_outside_paths_do_not() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let roots = vec![root.path().to_path_buf()];
        assert!(contain_in_roots(&roots, &root.path().join("apps/.env")).is_some());
        assert!(contain_in_roots(&roots, &outside.path().join(".env")).is_none());
        assert!(contain_in_roots(&roots, Path::new("/home/u/.bashrc")).is_none());
        assert!(contain_in_roots(&roots, &root.path().join("../escape/.env")).is_none());
        assert!(contain_in_roots(&roots, Path::new("relative/.env")).is_none());
        assert!(contain_in_roots(&roots, root.path()).is_none(), "the root itself is not a file");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escaping_the_root_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
        let roots = vec![root.path().to_path_buf()];
        assert!(contain_in_roots(&roots, &root.path().join("link/.env")).is_none());
    }
}
