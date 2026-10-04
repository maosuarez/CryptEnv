//! Shared filesystem-write policy for every crypt-env sink that writes an
//! `.env`-shaped file to a caller- or owner-supplied path (`POST /fill`,
//! `POST /environments/:id/example`, `POST /environments/:id/inject`, and
//! the `TempEnvFile` RAII guard in `api::mod`).
//!
//! Deliberately a `std`-only leaf module: it knows nothing about `db`,
//! `vault`, `crypto`, `project` or `api`, so it is unit-testable without a
//! database or a running server, and importable from any future caller
//! (CLI, TUI) without dragging `sqlx` behind it.
//!
//! Scope boundary (issue #8 vs issue #7): this module governs what may be
//! done to a path once it has been resolved and sanitised — existence
//! check, backup, marker, permissions. It does NOT validate path
//! construction or containment (e.g. traversal through `output_dir`) —
//! that is issue #7's territory, landing separately in the `write_target`
//! resolution blocks upstream of every call into this module. A path that
//! has escaped its intended directory but does not yet exist classifies as
//! `Target::Absent` here and is allowed — this gate does not stop
//! traversal, it stops silent clobbering of a resolved path.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// First-line marker prefix identifying a file crypt-env owns. Detection is
/// prefix-only, never equality: renaming a project or environment must
/// never invalidate an existing marker, and no "marker mismatch" error can
/// exist.
pub const MARKER_PREFIX: &str = "# crypt-env:";

/// Leading text of the message `commit`/`inspect` callers surface as a
/// `409 TARGET_EXISTS`. Exposed so callers that build a message from
/// several paths at once (multi-path inject) can still be recognised by a
/// handler that only has a `String` error to pattern-match on.
pub const TARGET_EXISTS_PREFIX: &str = "refusing to overwrite existing file not managed by crypt-env:";

/// Leading text of the message surfaced as a `409 BACKUP_EXISTS`.
pub const BACKUP_EXISTS_PREFIX: &str = "backup already exists, refusing to overwrite it:";

/// Leading text of the message surfaced as a `409 TARGET_SYMLINK`.
pub const SYMLINK_PREFIX: &str = "refusing to write through a symlink:";

/// Classification of a resolved write target. `Foreign` deliberately
/// carries no content — a file crypt-env doesn't own is never inspected
/// beyond "is it ours", so its bytes can never leak into an error message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Nothing exists at this path yet.
    Absent,
    /// A file crypt-env previously wrote (first non-empty line starts with
    /// `MARKER_PREFIX`). Carries its full content so callers that need to
    /// merge against it (inject) don't have to re-read it.
    Managed(String),
    /// A file exists but was not created by crypt-env — an unrelated file,
    /// or one whose marker the user removed.
    Foreign,
    /// The final path component is a symlink (or a Windows reparse point).
    /// Never read through and never written: secrets must not follow a link.
    Symlink,
}

/// Whether the written file should be created at owner-only permissions
/// (Unix `0o600`) or left to inherit the containing directory's default —
/// see §4.6 of the issue #8 plan for why `/example` uses `Inherit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    Private0600,
    Inherit,
}

#[derive(Debug, Clone, Copy)]
pub struct WriteOptions {
    pub overwrite: bool,
    pub mode: FileMode,
}

#[derive(Debug, Clone)]
pub struct Committed {
    /// Whether a file already existed at the target path before this call.
    pub pre_existed: bool,
    /// Path of the `.bak` copy, if one was created (only ever happens when
    /// a `Foreign` target is overwritten).
    pub backup: Option<PathBuf>,
}

/// Custom error type — no `unwrap()` anywhere in this module, and `Io`
/// stores an `std::io::ErrorKind` rather than the full `io::Error` so the
/// rendered message can never carry an OS string beyond the path the
/// caller already supplied.
#[derive(Debug)]
pub enum EnvFileError {
    TargetExists(PathBuf),
    BackupExists(PathBuf),
    /// The target is a symlink / reparse point; nothing was written.
    Symlink(PathBuf),
    Io(PathBuf, io::ErrorKind),
}

impl fmt::Display for EnvFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EnvFileError::TargetExists(path) => {
                write!(f, "{}", refuse_message(std::slice::from_ref(path)))
            }
            EnvFileError::BackupExists(path) => write!(f, "{}", backup_exists_message(path)),
            EnvFileError::Symlink(path) => write!(f, "{}", symlink_message(path)),
            EnvFileError::Io(path, kind) => {
                write!(f, "io error on {}: {kind:?}", path.display())
            }
        }
    }
}

impl std::error::Error for EnvFileError {}

/// The exact refusal message (§1.3 of the plan), generalised to N paths —
/// used both by `EnvFileError::TargetExists`'s `Display` (single path) and
/// by multi-path callers (inject) that must refuse several paths at once
/// before any of them are touched.
///
/// Contains nothing derived from the victim file's contents — no excerpt,
/// no length, no hash, no mtime. Only the paths the caller already supplied.
pub fn refuse_message(paths: &[PathBuf]) -> String {
    let list = paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{TARGET_EXISTS_PREFIX} {list}\n — pass overwrite: true to replace it (a .bak copy of the current contents is kept first)"
    )
}

/// The exact "target is a symlink" message. Names only the path.
pub fn symlink_message(path: &Path) -> String {
    format!(
        "{SYMLINK_PREFIX} {}\n — point the environment at the real file instead of a link",
        path.display()
    )
}

/// The exact "a previous `.bak` already exists" message.
pub fn backup_exists_message(path: &Path) -> String {
    format!("{BACKUP_EXISTS_PREFIX} {}", path.display())
}

/// Builds the marker line crypt-env prepends to every file it writes.
/// No timestamp, no version, nothing parsed on read: a timestamp would
/// dirty a committed `.env.example` on every inject, and any parsed field
/// invites someone to start depending on its shape.
pub fn marker_line(project: &str, environment: &str) -> String {
    format!("{MARKER_PREFIX} managed file (project: {project}, environment: {environment})")
}

/// First non-empty line, trimmed, starts with `MARKER_PREFIX`. Nothing
/// after the prefix is inspected.
fn is_managed(content: &str) -> bool {
    content
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().starts_with(MARKER_PREFIX))
        .unwrap_or(false)
}

/// Symlink on every platform; on Windows also any other reparse point
/// (junctions, mount points).
fn is_link(meta: &std::fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

fn bak_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".bak");
    PathBuf::from(name)
}

/// Classifies a resolved write target. Never touches its contents beyond
/// what is needed to decide `Managed` vs `Foreign`.
pub fn inspect(path: &Path) -> Result<Target, EnvFileError> {
    // `symlink_metadata` does not follow the final component, so a link is
    // reported as such instead of being read through.
    match std::fs::symlink_metadata(path) {
        Ok(meta) if is_link(&meta) => return Ok(Target::Symlink),
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Target::Absent),
        Err(e) => return Err(EnvFileError::Io(path.to_path_buf(), e.kind())),
    }
    match std::fs::read_to_string(path) {
        Ok(content) if is_managed(&content) => Ok(Target::Managed(content)),
        // Content deliberately dropped here — a `Foreign` file's bytes are
        // never carried forward into an error or a response.
        Ok(_) => Ok(Target::Foreign),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Target::Absent),
        // Non-UTF-8 existing file: a binary file is by definition not one
        // of ours. Never truncate it silently.
        Err(e) if e.kind() == io::ErrorKind::InvalidData => Ok(Target::Foreign),
        // This replaces the old `unwrap_or_default()` at
        // `project/mod.rs:328`, which turned an EACCES/EISDIR into a
        // silent "treat as empty, create new".
        Err(e) => Err(EnvFileError::Io(path.to_path_buf(), e.kind())),
    }
}

/// Writes `content` to `path`, applying the full policy: existence gate,
/// `.bak` backup when overwriting a `Foreign` target, marker prepend
/// (idempotent), and permission mode.
pub fn commit(
    path: &Path,
    content: &str,
    marker: &str,
    opts: &WriteOptions,
) -> Result<Committed, EnvFileError> {
    let target = inspect(path)?;
    if matches!(target, Target::Symlink) {
        // Checked before the `.bak` step: a backup must never be copied
        // through a link either.
        return Err(EnvFileError::Symlink(path.to_path_buf()));
    }
    let pre_existed = !matches!(target, Target::Absent);

    let mut backup: Option<PathBuf> = None;

    if matches!(target, Target::Foreign) {
        if !opts.overwrite {
            return Err(EnvFileError::TargetExists(path.to_path_buf()));
        }

        // Back up only when about to modify a Foreign target — never for
        // Absent or Managed, so crypt-env never scatters a second plaintext
        // copy of its own secret-bearing content across the disk.
        let bak = bak_path(path);
        let bak_exists = bak
            .try_exists()
            .map_err(|e| EnvFileError::Io(bak.clone(), e.kind()))?;
        if bak_exists {
            return Err(EnvFileError::BackupExists(bak));
        }
        std::fs::copy(path, &bak).map_err(|e| EnvFileError::Io(bak.clone(), e.kind()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&bak, perms)
                .map_err(|e| EnvFileError::Io(bak.clone(), e.kind()))?;
        }

        backup = Some(bak);
    }

    // Prepend the marker unless `content` already carries it — idempotence:
    // inject's merge output already re-emits an existing marker line
    // untouched, so this must not double it.
    let final_content = if content.starts_with(MARKER_PREFIX) {
        content.to_string()
    } else {
        format!("{marker}\n{content}")
    };

    write_with_mode(path, &final_content, opts.mode)?;

    Ok(Committed { pre_existed, backup })
}

/// Opens `path` for create+truncate at the requested mode without ever
/// following a symlink at the final component. Unix: `O_NOFOLLOW` (race-free,
/// `ELOOP` becomes `EnvFileError::Symlink`). Windows: `symlink_metadata`
/// immediately before the open; reparse points are refused (a small TOCTOU
/// window remains, see the harden-cli-manifest-and-sessions design).
pub fn open_nofollow(path: &Path, mode: FileMode) -> Result<std::fs::File, EnvFileError> {
    use std::fs::OpenOptions;

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
        if matches!(mode, FileMode::Private0600) {
            options.mode(0o600);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            if is_link(&meta) {
                return Err(EnvFileError::Symlink(path.to_path_buf()));
            }
        }
    }

    options.open(path).map_err(|e| {
        #[cfg(unix)]
        if e.raw_os_error() == Some(libc::ELOOP) {
            return EnvFileError::Symlink(path.to_path_buf());
        }
        EnvFileError::Io(path.to_path_buf(), e.kind())
    })
}

/// Creates (or truncates) `path` at open time with the requested mode —
/// not write-then-`chmod`, which leaves a window where a secret file exists
/// at umask permissions and, if the `chmod` fails, returns `Err` after the
/// target is already truncated with no guard armed.
fn write_with_mode(path: &Path, content: &str, mode: FileMode) -> Result<(), EnvFileError> {
    use std::io::Write as _;

    let mut file = open_nofollow(path, mode)?;
    file.write_all(content.as_bytes())
        .map_err(|e| EnvFileError::Io(path.to_path_buf(), e.kind()))?;

    // `.mode()` only applies at creation (subject to umask); for an
    // already-existing target it has no effect. Tighten to 0o600 if the
    // current mode is group/other-readable — never loosen a mode the user
    // deliberately set looser than that.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if matches!(mode, FileMode::Private0600) {
            if let Ok(meta) = file.metadata() {
                let current = meta.permissions().mode() & 0o777;
                if current & 0o077 != 0 {
                    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
                }
            }
        }
    }
    // `#[cfg(windows)]`: no ACL API is applied here — the file inherits the
    // containing directory's ACL, the same reasoning already documented at
    // `api/mod.rs` for `TempEnvFile::create`.

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn write_file(path: &Path, content: &str) {
        std::fs::write(path, content).expect("test setup: write file");
    }

    fn read_file(path: &Path) -> Vec<u8> {
        std::fs::read(path).expect("test setup: read file")
    }

    #[test]
    fn inspect_absent_path_returns_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.env");
        assert_eq!(inspect(&path).unwrap(), Target::Absent);
    }

    #[test]
    fn inspect_marked_file_returns_managed_with_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        let content = "# crypt-env: managed file (project: p, environment: e)\nKEY=1\n";
        write_file(&path, content);
        match inspect(&path).unwrap() {
            Target::Managed(c) => assert_eq!(c, content),
            other => panic!("expected Managed, got {other:?}"),
        }
    }

    #[test]
    fn inspect_unmarked_file_returns_foreign_without_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("victim.yaml");
        write_file(&path, "important: config\n");
        assert_eq!(inspect(&path).unwrap(), Target::Foreign);
    }

    #[test]
    #[cfg(unix)]
    fn inspect_unreadable_path_returns_io_error_not_absent() {
        use std::os::unix::fs::PermissionsExt;

        // Running as root (common in CI containers) bypasses permission
        // bits entirely, which would make this test spuriously fail.
        if unsafe { libc_geteuid() } == 0 {
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked.env");
        write_file(&path, "KEY=1\n");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

        let result = inspect(&path);
        // Restore permissions so tempdir cleanup can remove the file.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        match result {
            Err(EnvFileError::Io(p, kind)) => {
                assert_eq!(p, path);
                assert_eq!(kind, io::ErrorKind::PermissionDenied);
            }
            other => panic!("expected Io(PermissionDenied), got {other:?}"),
        }
    }

    #[cfg(unix)]
    unsafe fn libc_geteuid() -> u32 {
        extern "C" {
            fn geteuid() -> u32;
        }
        geteuid()
    }

    #[test]
    fn marker_detected_after_leading_blank_lines_and_ignores_trailing_text() {
        let content = "\n\n  # crypt-env: managed file (project: p, environment: e) extra trailing text\nKEY=1\n";
        assert!(is_managed(content));
    }

    #[test]
    fn commit_prepends_marker_and_is_idempotent_across_two_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        let marker = marker_line("p", "e");
        let opts = WriteOptions { overwrite: false, mode: FileMode::Private0600 };

        let committed = commit(&path, "KEY=1", &marker, &opts).unwrap();
        assert!(!committed.pre_existed);
        let first = String::from_utf8(read_file(&path)).unwrap();
        assert_eq!(first, format!("{marker}\nKEY=1"));

        // Second write passes content that already carries the marker
        // (as inject's merge output does) — must not be duplicated.
        let committed2 = commit(&path, &first, &marker, &opts).unwrap();
        assert!(committed2.pre_existed);
        let second = String::from_utf8(read_file(&path)).unwrap();
        assert_eq!(second, first);
        assert_eq!(second.matches(MARKER_PREFIX).count(), 1);
    }

    #[test]
    fn commit_refuses_foreign_target_without_overwrite_and_leaves_bytes_identical() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("victim.yaml");
        write_file(&path, "important: config\n");
        let before = read_file(&path);

        let marker = marker_line("p", "e");
        let opts = WriteOptions { overwrite: false, mode: FileMode::Private0600 };
        let err = commit(&path, "KEY=1", &marker, &opts).unwrap_err();
        assert!(matches!(err, EnvFileError::TargetExists(p) if p == path));

        assert_eq!(read_file(&path), before);
    }

    #[test]
    fn commit_with_overwrite_creates_bak_holding_original_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("victim.yaml");
        let original = b"important: config\n".to_vec();
        write_file(&path, "important: config\n");

        let marker = marker_line("p", "e");
        let opts = WriteOptions { overwrite: true, mode: FileMode::Private0600 };
        let committed = commit(&path, "KEY=1", &marker, &opts).unwrap();

        let bak = committed.backup.expect("expected a .bak to be created");
        assert_eq!(read_file(&bak), original);
        assert_eq!(String::from_utf8(read_file(&path)).unwrap(), format!("{marker}\nKEY=1"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&bak).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn commit_refuses_when_bak_already_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("victim.yaml");
        write_file(&path, "important: config\n");
        let bak = bak_path(&path);
        write_file(&bak, "someone else's earlier backup\n");

        let marker = marker_line("p", "e");
        let opts = WriteOptions { overwrite: true, mode: FileMode::Private0600 };
        let err = commit(&path, "KEY=1", &marker, &opts).unwrap_err();
        assert!(matches!(err, EnvFileError::BackupExists(p) if p == bak));
    }

    #[test]
    fn commit_never_creates_bak_for_absent_or_managed_target() {
        let dir = tempfile::tempdir().unwrap();
        let marker = marker_line("p", "e");

        // Absent, with overwrite true or false — no backup either way.
        let absent_path = dir.path().join("fresh.env");
        let opts_overwrite = WriteOptions { overwrite: true, mode: FileMode::Private0600 };
        let committed = commit(&absent_path, "KEY=1", &marker, &opts_overwrite).unwrap();
        assert!(committed.backup.is_none());
        assert!(!bak_path(&absent_path).exists());

        // Managed — re-writing our own file never backs it up either.
        let managed_path = dir.path().join(".env");
        let opts_no_overwrite = WriteOptions { overwrite: false, mode: FileMode::Private0600 };
        commit(&managed_path, "KEY=1", &marker, &opts_no_overwrite).unwrap();
        let committed2 = commit(&managed_path, &format!("{marker}\nKEY=2"), &marker, &opts_no_overwrite).unwrap();
        assert!(committed2.backup.is_none());
        assert!(!bak_path(&managed_path).exists());
    }

    #[test]
    fn temp_env_file_style_flush_used_in_tests_compiles() {
        // Sanity check that `std::io::Write` is in scope for the helper
        // above; not itself a behavioural assertion.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("noop");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"x").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_target_is_refused_and_nothing_is_touched() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim.txt");
        write_file(&victim, "important: config\n");
        let link = dir.path().join(".env");
        std::os::unix::fs::symlink(&victim, &link).unwrap();

        assert_eq!(inspect(&link).unwrap(), Target::Symlink);

        let marker = marker_line("p", "e");
        for overwrite in [false, true] {
            let opts = WriteOptions { overwrite, mode: FileMode::Private0600 };
            let err = commit(&link, "KEY=1", &marker, &opts).unwrap_err();
            assert!(matches!(err, EnvFileError::Symlink(p) if p == link));
        }
        assert_eq!(read_file(&victim), b"important: config\n");
        assert_eq!(std::fs::read_link(&link).unwrap(), victim);
        assert!(!bak_path(&link).exists(), "no .bak from a symlink");
        assert!(!bak_path(&victim).exists());
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_target_is_refused_without_creating_its_destination() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("not-yet.txt");
        let link = dir.path().join(".env");
        std::os::unix::fs::symlink(&dest, &link).unwrap();

        let marker = marker_line("p", "e");
        let opts = WriteOptions { overwrite: true, mode: FileMode::Inherit };
        assert!(matches!(commit(&link, "KEY=1", &marker, &opts), Err(EnvFileError::Symlink(_))));
        assert!(!dest.exists());
    }
}
