//! Safe edits of third-party JSON configuration files (MCP host configs such as
//! `.mcp.json` or `claude_desktop_config.json`).
//!
//! Rules: never discard content we cannot parse strictly, write atomically
//! (temp file in the same directory + rename), keep a one-time `.bak` of an
//! existing file, and create new files owner-only (0600 on Unix).

use serde_json::{json, Value};
use std::io::Write as _;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum HostCfgError {
    /// The existing file is not strict JSON; it was left untouched. `snippet`
    /// is the entry to add by hand, with any secret replaced by a placeholder.
    Unparseable { path: PathBuf, snippet: String },
    /// The file parses but its structure cannot hold the entry.
    InvalidStructure(String),
    Io(String),
}

impl std::fmt::Display for HostCfgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostCfgError::Unparseable { path, snippet } => write!(
                f,
                "{} is not strict JSON (comments or trailing commas?), so it was left unchanged. \
                 Add this entry by hand:\n{snippet}",
                path.display()
            ),
            HostCfgError::InvalidStructure(m) | HostCfgError::Io(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for HostCfgError {}

/// Reads `path` as strict JSON. A missing file yields `{}`.
fn read_strict(path: &Path, snippet: impl FnOnce() -> String) -> Result<(Value, bool), HostCfgError> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map(|v| (v, true))
            .map_err(|_| HostCfgError::Unparseable { path: path.to_path_buf(), snippet: snippet() }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((json!({}), false)),
        Err(e) => Err(HostCfgError::Io(format!("error reading {}: {e}", path.display()))),
    }
}

/// Builds `{ key1: { key2: { ... leaf } } }` for the refusal snippet.
fn nest(keys: &[&str], leaf: &Value) -> Value {
    keys.iter().rev().fold(leaf.clone(), |acc, k| json!({ *k: acc }))
}

/// Atomic replace. The mode of an existing file is preserved; a new file is
/// created 0600 (tempfile's default on Unix). A `.bak` is written once, before
/// the first modification of an existing file.
fn write_atomic(path: &Path, value: &Value, existed: bool) -> Result<(), HostCfgError> {
    let io = |what: &str, e: std::io::Error| HostCfgError::Io(format!("{what} {}: {e}", path.display()));
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(dir).map_err(|e| io("cannot create directory for", e))?;

    if existed {
        let mut bak = path.as_os_str().to_owned();
        bak.push(".bak");
        let bak = PathBuf::from(bak);
        if !bak.exists() {
            std::fs::copy(path, &bak).map_err(|e| io("cannot write backup of", e))?;
        }
    }

    let text = serde_json::to_string_pretty(value)
        .map_err(|e| HostCfgError::Io(format!("error serializing config: {e}")))?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".cenv-hostcfg-")
        .tempfile_in(dir)
        .map_err(|e| io("cannot create temp file for", e))?;
    tmp.write_all(text.as_bytes()).map_err(|e| io("error writing", e))?;
    tmp.as_file().sync_all().map_err(|e| io("error syncing", e))?;

    #[cfg(unix)]
    if existed {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let _ = tmp
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(meta.permissions().mode() & 0o777));
        }
    }

    tmp.persist(path).map_err(|e| io("error replacing", e.error))?;
    Ok(())
}

/// Applies `edit` to the strict-JSON config at `path` and writes the result
/// atomically. If the file exists but is not strict JSON it is left untouched
/// and `Unparseable { snippet }` is returned.
pub fn update_json(
    path: &Path,
    snippet: impl FnOnce() -> String,
    edit: impl FnOnce(&mut Value) -> Result<(), HostCfgError>,
) -> Result<(), HostCfgError> {
    let (mut config, existed) = read_strict(path, snippet)?;
    edit(&mut config)?;
    write_atomic(path, &config, existed)
}

/// Sets `entry` under `config[keys[0]][keys[1]]...` (e.g. `["mcpServers", "cryptenv"]`),
/// changing nothing else. `placeholder` is what the refusal snippet shows in
/// place of `entry` (so a token never reaches an error message).
pub fn merge_json_entry(
    path: &Path,
    keys: &[&str],
    entry: &Value,
    placeholder: &Value,
) -> Result<(), HostCfgError> {
    let Some((leaf, parents)) = keys.split_last() else {
        return Err(HostCfgError::InvalidStructure("empty key path".to_string()));
    };
    let not_object = || HostCfgError::InvalidStructure(format!("{} has an unexpected structure (not a JSON object)", path.display()));
    update_json(
        path,
        || serde_json::to_string_pretty(&nest(keys, placeholder)).unwrap_or_default(),
        |root| {
            let mut cur = root;
            for k in parents {
                cur = cur
                    .as_object_mut()
                    .ok_or_else(not_object)?
                    .entry(*k)
                    .or_insert_with(|| json!({}));
            }
            cur.as_object_mut()
                .ok_or_else(not_object)?
                .insert((*leaf).to_string(), entry.clone());
            Ok(())
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: [&str; 2] = ["mcpServers", "cryptenv"];

    fn entry() -> Value {
        json!({ "command": "crypt-env-mcp", "env": { "CRYPTENV_TOKEN": "s3cr3t-token" } })
    }
    fn placeholder() -> Value {
        json!({ "command": "crypt-env-mcp", "env": { "CRYPTENV_TOKEN": "<your MCP token — shown in Settings>" } })
    }

    #[test]
    fn jsonc_input_is_left_unchanged_and_snippet_has_no_token() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".mcp.json");
        let original = "{\n // keep me\n \"mcpServers\": { \"a\": {}, },\n}\n";
        std::fs::write(&p, original).unwrap();

        let err = merge_json_entry(&p, &KEYS, &entry(), &placeholder()).unwrap_err();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), original);
        assert!(!dir.path().join(".mcp.json.bak").exists());
        let msg = err.to_string();
        assert!(matches!(err, HostCfgError::Unparseable { .. }));
        assert!(!msg.contains("s3cr3t-token"), "{msg}");
        assert!(msg.contains("your MCP token"), "{msg}");
    }

    #[test]
    fn valid_input_keeps_other_servers_and_writes_one_backup() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".mcp.json");
        std::fs::write(&p, r#"{"mcpServers":{"a":{"command":"x"},"b":{"command":"y"}},"other":1}"#).unwrap();

        merge_json_entry(&p, &KEYS, &entry(), &placeholder()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["a"]["command"], "x");
        assert_eq!(v["mcpServers"]["b"]["command"], "y");
        assert_eq!(v["other"], 1);
        assert_eq!(v["mcpServers"]["cryptenv"]["command"], "crypt-env-mcp");

        // A second write must not overwrite the first (original) backup.
        merge_json_entry(&p, &KEYS, &json!({"command": "z"}), &placeholder()).unwrap();
        let bak: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join(".mcp.json.bak")).unwrap()).unwrap();
        assert!(bak["mcpServers"].get("cryptenv").is_none());
    }

    #[test]
    fn non_object_root_is_refused_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.json");
        std::fs::write(&p, "[1,2]").unwrap();
        assert!(matches!(
            merge_json_entry(&p, &KEYS, &entry(), &placeholder()),
            Err(HostCfgError::InvalidStructure(_))
        ));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "[1,2]");
    }

    #[cfg(unix)]
    #[test]
    fn created_file_is_mode_0600_and_existing_mode_is_preserved() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join(".mcp.json");
        merge_json_entry(&p, &KEYS, &entry(), &placeholder()).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);

        let q = dir.path().join("existing.json");
        std::fs::write(&q, "{}").unwrap();
        std::fs::set_permissions(&q, std::fs::Permissions::from_mode(0o644)).unwrap();
        merge_json_entry(&q, &KEYS, &entry(), &placeholder()).unwrap();
        assert_eq!(std::fs::metadata(&q).unwrap().permissions().mode() & 0o777, 0o644);
    }
}
