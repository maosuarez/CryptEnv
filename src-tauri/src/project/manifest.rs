//! `.crypt-env.yaml` — the project manifest committed at a project's root
//! (cli-tui-parity design D1/D8). Shared by the CLI (`init`, `config`) and
//! the GUI (`project_write_yaml`) so both emit byte-identical files.
//!
//! The manifest only ever carries metadata — name, description, categories,
//! environments and their target paths. It never contains a secret value.
//! Environment paths are relative to the manifest's directory (preferred) or
//! absolute.

use serde::{Deserialize, Serialize};
use std::path::Path;

use super::Project;

pub const FILE_NAME: &str = ".crypt-env.yaml";

const HEADER: &str = "# Managed by crypt-env (https://maosuarez.com). Contains NO secret values — safe to commit.\n# Environment paths are relative to this file's directory. Sync with the vault: `crypt-env config`.\n";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub project: ManifestProject,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestProject {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, alias = "tags", skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    #[serde(default)]
    pub environments: Vec<ManifestEnvironment>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestEnvironment {
    pub name: String,
    #[serde(rename = "isDefault", default)]
    pub is_default: bool,
    #[serde(default)]
    pub paths: Vec<String>,
}

impl Manifest {
    /// The environment marked `isDefault`, else the first one.
    pub fn default_environment(&self) -> Option<&ManifestEnvironment> {
        let envs = &self.project.environments;
        envs.iter().find(|e| e.is_default).or_else(|| envs.first())
    }
}

/// Parses and validates manifest text. Errors describe the broken rule;
/// they never echo file content beyond names.
pub fn parse(content: &str) -> Result<Manifest, String> {
    let m: Manifest = serde_yaml::from_str(content).map_err(|e| format!("invalid YAML — {e}"))?;
    validate(&m)?;
    Ok(m)
}

pub fn validate(m: &Manifest) -> Result<(), String> {
    if m.project.name.trim().is_empty() {
        return Err("'project.name' is required and must not be empty".into());
    }
    let mut seen: Vec<String> = Vec::new();
    for e in &m.project.environments {
        let lower = e.name.to_lowercase();
        if seen.contains(&lower) {
            return Err(format!("environment '{}' is listed more than once", e.name));
        }
        seen.push(lower);
        if e.paths.iter().any(|p| p.trim().is_empty()) {
            return Err(format!("environment '{}' has an empty path", e.name));
        }
    }
    if m.project.environments.iter().filter(|e| e.is_default).count() > 1 {
        return Err("only one environment may have 'isDefault: true'".into());
    }
    Ok(())
}

/// Serializes with the standard header comment.
pub fn to_yaml(m: &Manifest) -> Result<String, String> {
    let body = serde_yaml::to_string(m).map_err(|e| e.to_string())?;
    Ok(format!("{HEADER}{body}"))
}

/// Reads and parses a manifest file.
pub fn read_file(path: &Path) -> Result<Manifest, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&content).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes `m` to `<dir>/.crypt-env.yaml` via a uniquely named temp file in the
/// same directory, then renames it over the target. The temp file is created
/// exclusively (random name, `O_EXCL` / `CREATE_NEW`), so a planted file or
/// symlink at any fixed name is never opened; the rename replaces a symlink at
/// the target itself instead of following it. On failure the temp file is
/// removed on drop and an existing manifest stays intact.
pub fn write_file(dir: &Path, m: &Manifest) -> Result<std::path::PathBuf, String> {
    use std::io::Write as _;

    let target = dir.join(FILE_NAME);
    let content = to_yaml(m)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    tmp.write_all(content.as_bytes()).map_err(|e| format!("{}: {e}", target.display()))?;
    tmp.as_file().sync_all().map_err(|e| format!("{}: {e}", target.display()))?;
    // The manifest is meant to be committed: keep the usual shared mode
    // rather than the temp file's owner-only 0600.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o644));
    }
    tmp.persist(&target).map_err(|e| format!("{}: {}", target.display(), e.error))?;
    Ok(target)
}

/// Builds the manifest for a vault project. Absolute environment paths that
/// sit inside the project root are rewritten relative to it (with `/`
/// separators); other absolute paths go through `map_absolute` (the CLI uses
/// it to translate host paths back into WSL paths) and relative ones are
/// kept as stored. The default environment is listed first.
pub fn from_project(p: &Project, map_absolute: &dyn Fn(&str) -> String) -> Manifest {
    let mut envs: Vec<&super::Environment> = p.environments.iter().collect();
    envs.sort_by_key(|e| !e.is_default);
    let root = p.root_path.as_deref();
    Manifest {
        project: ManifestProject {
            name: p.name.clone(),
            description: p.description.clone().filter(|d| !d.trim().is_empty()),
            categories: p.categories.clone(),
            environments: envs
                .into_iter()
                .map(|e| ManifestEnvironment {
                    name: e.name.clone(),
                    is_default: e.is_default,
                    paths: e
                        .paths
                        .iter()
                        .map(|path| match root.and_then(|r| relativize(r, path)) {
                            Some(rel) => rel,
                            None if is_absolute_any(path) => map_absolute(path),
                            None => path.clone(),
                        })
                        .collect(),
                })
                .collect(),
        },
    }
}

/// `path` relative to `root` (with `/` separators) when it lies strictly
/// inside it; `None` otherwise. Accepts either separator on both sides.
pub fn relativize(root: &str, path: &str) -> Option<String> {
    let norm = |s: &str| s.replace('\\', "/");
    let root_n = norm(root);
    let root_n = root_n.trim_end_matches('/');
    let path_n = norm(path);
    let rest = path_n.strip_prefix(root_n)?.strip_prefix('/')?;
    if rest.is_empty() || rest.split('/').any(|c| c == "..") {
        return None;
    }
    Some(rest.to_string())
}

/// Absolute on this host, or a Windows drive/UNC path regardless of host.
pub fn is_absolute_any(path: &str) -> bool {
    let b = path.as_bytes();
    Path::new(path).is_absolute()
        || path.starts_with("\\\\")
        || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Environment;

    fn env(name: &str, default: bool, paths: &[&str]) -> Environment {
        Environment {
            id: 0,
            project_id: 1,
            name: name.into(),
            is_default: default,
            paths: paths.iter().map(|s| s.to_string()).collect(),
            vars: vec![],
            created: "0".into(),
            updated: "0".into(),
        }
    }

    fn project() -> Project {
        Project {
            id: 1,
            name: "svc".into(),
            description: Some("Auth".into()),
            template: "generic".into(),
            created: "0".into(),
            updated: "0".into(),
            environments: vec![
                env("production", false, &["/repo/svc/apps/api/.env.production", "/elsewhere/.env"]),
                env("default", true, &[".env"]),
            ],
            categories: vec!["backend".into()],
            root_path: Some("/repo/svc".into()),
        }
    }

    #[test]
    fn roundtrip_preserves_content() {
        let m = from_project(&project(), &|p| p.to_string());
        let text = to_yaml(&m).unwrap();
        assert!(text.starts_with("# Managed by crypt-env"));
        assert_eq!(parse(&text).unwrap(), m);
    }

    #[test]
    fn from_project_relativizes_inside_root_and_lists_default_first() {
        let m = from_project(&project(), &|p| format!("mapped:{p}"));
        assert_eq!(m.project.environments[0].name, "default");
        assert_eq!(
            m.project.environments[1].paths,
            vec!["apps/api/.env.production".to_string(), "mapped:/elsewhere/.env".to_string()]
        );
    }

    #[test]
    fn relativize_handles_windows_separators() {
        assert_eq!(
            relativize(r"\\wsl.localhost\U\home\a", r"\\wsl.localhost\U\home\a\web\.env").as_deref(),
            Some("web/.env")
        );
        assert_eq!(relativize("/a/b", "/a/bc/.env"), None);
        assert_eq!(relativize("/a/b", "/a/b"), None);
    }

    #[test]
    fn parse_accepts_tags_alias_and_rejects_bad_input() {
        let ok = parse("project:\n  name: x\n  tags: [a]\n  environments:\n    - name: default\n      isDefault: true\n").unwrap();
        assert_eq!(ok.project.categories, vec!["a".to_string()]);
        assert!(parse("project:\n  name: ''\n").is_err());
        assert!(parse("project:\n  name: x\n  secret: y\n").is_err(), "unknown fields rejected");
        assert!(parse("project:\n  name: x\n  environments:\n    - name: a\n    - name: A\n").is_err());
        assert!(parse(
            "project:\n  name: x\n  environments:\n    - {name: a, isDefault: true}\n    - {name: b, isDefault: true}\n"
        )
        .is_err());
        assert!(parse("not: [valid").is_err());
    }

    #[test]
    fn default_environment_falls_back_to_first() {
        let m = parse("project:\n  name: x\n  environments:\n    - name: dev\n    - name: prod\n").unwrap();
        assert_eq!(m.default_environment().unwrap().name, "dev");
    }

    #[test]
    fn write_file_writes_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let m = from_project(&project(), &|p| p.to_string());
        let path = write_file(dir.path(), &m).unwrap();
        assert_eq!(read_file(&path).unwrap(), m);
        assert!(!dir.path().join(".crypt-env.yaml.tmp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn write_file_never_follows_a_planted_tmp_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, "keep me").unwrap();
        std::os::unix::fs::symlink(&victim, dir.path().join(".crypt-env.yaml.tmp")).unwrap();

        let m = from_project(&project(), &|p| p.to_string());
        let path = write_file(dir.path(), &m).unwrap();

        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me");
        assert_eq!(read_file(&path).unwrap(), m);
    }

    #[cfg(unix)]
    #[test]
    fn write_file_replaces_a_symlinked_manifest_without_following_it() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, "keep me").unwrap();
        std::os::unix::fs::symlink(&victim, dir.path().join(FILE_NAME)).unwrap();

        let m = from_project(&project(), &|p| p.to_string());
        write_file(dir.path(), &m).unwrap();

        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me");
        assert!(!std::fs::symlink_metadata(dir.path().join(FILE_NAME)).unwrap().file_type().is_symlink());
    }

    #[test]
    fn write_file_failure_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        // A non-empty directory at the target makes the final rename fail.
        let blocker = dir.path().join(FILE_NAME);
        std::fs::create_dir(&blocker).unwrap();
        std::fs::write(blocker.join("x"), "x").unwrap();

        let m = from_project(&project(), &|p| p.to_string());
        assert!(write_file(dir.path(), &m).is_err());

        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![FILE_NAME.to_string()]);
    }
}
