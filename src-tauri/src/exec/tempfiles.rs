//! Private, short-lived files holding plaintext secrets (`generate_env`).
//!
//! Files live in a per-user private directory (`<app_data>/mcp-tmp`, mode 0700
//! on Unix; on Windows the per-user app-data ACL applies), are created with
//! `create_new` at mode 0600, and are deleted after [`GENERATED_TTL`], when the
//! vault locks, and at application exit. Leftovers from a crashed run are swept
//! at startup. Never a shared temp directory.

use rand::RngCore;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a generated file may live.
pub const GENERATED_TTL: Duration = Duration::from_secs(10 * 60);

struct GeneratedFile {
    path: PathBuf,
    created: Instant,
}

/// Registry of generated files. Lives in `VaultState` (like the LAN share slot)
/// so locking the vault can purge it without knowing about the API layer.
#[derive(Default)]
pub struct GeneratedFiles {
    dir: Mutex<Option<PathBuf>>,
    files: Mutex<Vec<GeneratedFile>>,
}

fn random_name() -> String {
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("crypt_env_{hex}.env")
}

/// Zero-overwrite then delete; best effort (a zero pass is defence in depth
/// only on journaling or copy-on-write filesystems).
fn wipe_and_remove(path: &Path) {
    if let Ok(meta) = std::fs::metadata(path) {
        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(path) {
            let _ = f.write_all(&vec![0u8; meta.len() as usize]);
            let _ = f.sync_all();
        }
    }
    let _ = std::fs::remove_file(path);
}

impl GeneratedFiles {
    pub fn new() -> GeneratedFiles {
        GeneratedFiles::default()
    }

    /// Sets the private directory and removes anything left in it by an
    /// earlier run.
    pub fn init(&self, dir: PathBuf) {
        sweep_dir(&dir);
        if let Ok(mut d) = self.dir.lock() {
            *d = Some(dir);
        }
    }

    /// Writes `content` to a fresh private file and tracks it for expiry.
    pub fn create(&self, content: &str) -> Result<PathBuf, String> {
        let dir = self
            .dir
            .lock()
            .map_err(|_| "temp registry unavailable".to_string())?
            .clone()
            .ok_or_else(|| "private temp directory is not configured".to_string())?;
        ensure_private_dir(&dir)?;

        let path = dir.join(random_name());
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&path).map_err(|e| format!("cannot create file: {e}"))?;
        if let Err(e) = file.write_all(content.as_bytes()).and_then(|_| file.flush()) {
            drop(file);
            wipe_and_remove(&path);
            return Err(format!("cannot write file: {e}"));
        }
        match self.files.lock() {
            Ok(mut files) => files.push(GeneratedFile { path: path.clone(), created: Instant::now() }),
            Err(_) => {
                wipe_and_remove(&path);
                return Err("temp registry unavailable".to_string());
            }
        }
        Ok(path)
    }

    /// Deletes files older than [`GENERATED_TTL`]; returns how many went.
    pub fn sweep_expired(&self) -> usize {
        let Ok(mut files) = self.files.lock() else { return 0 };
        let now = Instant::now();
        let mut removed = 0;
        files.retain(|f| {
            if now.duration_since(f.created) >= GENERATED_TTL {
                wipe_and_remove(&f.path);
                removed += 1;
                false
            } else {
                true
            }
        });
        removed
    }

    /// Deletes every tracked file (vault lock, application exit).
    pub fn purge_all(&self) {
        let Ok(mut files) = self.files.lock() else { return };
        for f in files.drain(..) {
            wipe_and_remove(&f.path);
        }
    }

    #[cfg(test)]
    pub fn backdate_all(&self, by: Duration) {
        if let Ok(mut files) = self.files.lock() {
            for f in files.iter_mut() {
                if let Some(t) = f.created.checked_sub(by) {
                    f.created = t;
                }
            }
        }
    }
}

fn ensure_private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create temp directory: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("cannot restrict temp directory: {e}"))?;
    }
    Ok(())
}

/// Removes every regular file in `dir` (startup sweep). Missing dir is fine.
pub fn sweep_dir(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            wipe_and_remove(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_is_private_and_lives_in_the_private_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = GeneratedFiles::new();
        let dir = tmp.path().join("mcp-tmp");
        reg.init(dir.clone());
        let path = reg.create("KEY=value\n").unwrap();
        assert_eq!(path.parent().unwrap(), dir);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "KEY=value\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        }
    }

    #[test]
    fn purge_all_deletes_files() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = GeneratedFiles::new();
        reg.init(tmp.path().join("mcp-tmp"));
        let path = reg.create("K=v").unwrap();
        reg.purge_all();
        assert!(!path.exists());
    }

    #[test]
    fn expired_files_are_swept_and_fresh_ones_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = GeneratedFiles::new();
        reg.init(tmp.path().join("mcp-tmp"));
        let path = reg.create("K=v").unwrap();
        assert_eq!(reg.sweep_expired(), 0);
        assert!(path.exists());
        reg.backdate_all(GENERATED_TTL + Duration::from_secs(1));
        assert_eq!(reg.sweep_expired(), 1);
        assert!(!path.exists());
    }

    #[test]
    fn startup_sweep_removes_stale_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("mcp-tmp");
        std::fs::create_dir_all(&dir).unwrap();
        let stale = dir.join("crypt_env_stale.env");
        std::fs::write(&stale, "K=v").unwrap();
        let reg = GeneratedFiles::new();
        reg.init(dir);
        assert!(!stale.exists());
    }

    #[test]
    fn create_without_a_configured_dir_fails() {
        assert!(GeneratedFiles::new().create("K=v").is_err());
    }
}
