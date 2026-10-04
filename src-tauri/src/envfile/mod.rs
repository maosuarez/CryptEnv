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

/// Leading text of the message surfaced as a `409 NOT_REGULAR_FILE`.
pub const NOT_REGULAR_PREFIX: &str = "refusing to use a target that is not a regular file:";

/// Largest existing target `inspect` reads. A bigger file is classified
/// `Foreign` without being read in full.
pub const MAX_INSPECT_BYTES: u64 = 1024 * 1024;

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
    /// The target exists but is a device, FIFO, socket or directory.
    NotRegularFile(PathBuf),
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
            EnvFileError::NotRegularFile(path) => write!(f, "{}", not_regular_message(path)),
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

/// The exact "target is not a regular file" message. Names only the path.
pub fn not_regular_message(path: &Path) -> String {
    format!("{NOT_REGULAR_PREFIX} {}", path.display())
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
/// what is needed to decide `Managed` vs `Foreign`, and never reads more than
/// `MAX_INSPECT_BYTES`. Devices, FIFOs, sockets and directories are refused
/// before being opened for reading.
pub fn inspect(path: &Path) -> Result<Target, EnvFileError> {
    // `symlink_metadata` does not follow the final component, so a link is
    // reported as such instead of being read through.
    match std::fs::symlink_metadata(path) {
        Ok(meta) if is_link(&meta) => return Ok(Target::Symlink),
        Ok(meta) if !meta.file_type().is_file() => {
            return Err(EnvFileError::NotRegularFile(path.to_path_buf()))
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Target::Absent),
        Err(e) => return Err(EnvFileError::Io(path.to_path_buf(), e.kind())),
    }

    use std::io::Read as _;
    let io_err = |e: io::Error| EnvFileError::Io(path.to_path_buf(), e.kind());

    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Belt and braces against the path being swapped for a FIFO or a
        // link between the check above and this open: neither blocks nor
        // is followed.
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = match options.open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Target::Absent),
        #[cfg(unix)]
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => return Ok(Target::Symlink),
        Err(e) => return Err(io_err(e)),
    };
    // Re-check on the opened handle: what was opened is what was checked.
    if !file.metadata().map_err(io_err)?.file_type().is_file() {
        return Err(EnvFileError::NotRegularFile(path.to_path_buf()));
    }

    let mut bytes = Vec::new();
    file.take(MAX_INSPECT_BYTES + 1).read_to_end(&mut bytes).map_err(io_err)?;
    if bytes.len() as u64 > MAX_INSPECT_BYTES {
        // Too large to be a file crypt-env wrote; never read in full.
        return Ok(Target::Foreign);
    }
    match String::from_utf8(bytes) {
        Ok(content) if is_managed(&content) => Ok(Target::Managed(content)),
        // Content deliberately dropped here — a `Foreign` file's bytes are
        // never carried forward into an error or a response. A non-UTF-8
        // file is by definition not one of ours; never truncate it silently.
        _ => Ok(Target::Foreign),
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

    write_atomic(path, opts.mode, |file| {
        use std::io::Write as _;
        file.write_all(final_content.as_bytes())
    })?;

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

/// Atomically replaces `path`: the body is written to an exclusively
/// created temp file (`.cenv-*`) in the same directory, `fsync`ed, and
/// renamed over the target. Any failure leaves the previous file intact and
/// removes the temp file (it is dropped, and so deleted, on every error
/// path). `write_body` is the seam tests use to simulate a mid-write failure.
///
/// The temp file is created 0600 on Unix, so a secret never exists at umask
/// permissions. `Private0600` keeps 0600; `Inherit` keeps the mode of the
/// file being replaced, or 0644 for a new file. On Windows the temp file
/// inherits the directory ACL (same reasoning as `TempEnvFile::create_guarded`
/// in `api/mod.rs`).
fn write_atomic(
    path: &Path,
    mode: FileMode,
    write_body: impl FnOnce(&mut std::fs::File) -> io::Result<()>,
) -> Result<(), EnvFileError> {
    let io_err = |e: io::Error| EnvFileError::Io(path.to_path_buf(), e.kind());

    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let mut tmp = tempfile::Builder::new()
        .prefix(".cenv-")
        .tempfile_in(dir)
        .map_err(io_err)?;

    write_body(tmp.as_file_mut()).map_err(io_err)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if matches!(mode, FileMode::Inherit) {
            let bits = match std::fs::symlink_metadata(path) {
                Ok(meta) => meta.permissions().mode() & 0o777,
                Err(_) => 0o644,
            };
            tmp.as_file()
                .set_permissions(std::fs::Permissions::from_mode(bits))
                .map_err(io_err)?;
        }
    }
    #[cfg(not(unix))]
    let _ = mode;

    tmp.as_file().sync_all().map_err(io_err)?;

    // Re-check right before the rename: a target swapped for a link or a
    // special file since `inspect` is refused, never replaced.
    match std::fs::symlink_metadata(path) {
        Ok(meta) if is_link(&meta) => return Err(EnvFileError::Symlink(path.to_path_buf())),
        Ok(meta) if !meta.file_type().is_file() => {
            return Err(EnvFileError::NotRegularFile(path.to_path_buf()))
        }
        _ => {}
    }

    tmp.persist(path).map_err(|e| io_err(e.error))?;

    // Best effort: persist the rename itself. The content is already in
    // place, so a failure here must not be reported as a failed write.
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }

    Ok(())
}

// ─── dotenv grammar ───────────────────────────────────────────────────────────

/// A variable name crypt-env is willing to write: `^[A-Za-z_][A-Za-z0-9_.]*$`.
pub fn is_valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

fn is_bare_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | ':' | '@' | '+' | ',' | '=' | '-')
}

/// Serializes a value so common dotenv parsers read back exactly `value`:
/// bare when only safe characters, single-quoted (literal) when there is no
/// newline, carriage return or `'`, otherwise double-quoted with `\`, `"`,
/// `$`, newline and carriage return escaped. A value can never end its own
/// line or open a comment.
pub fn serialize_value(value: &str) -> String {
    if value.chars().all(is_bare_char) {
        return value.to_string();
    }
    if !value.contains(['\n', '\r', '\'']) {
        return format!("'{value}'");
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `KEY=VALUE` for one variable, or `None` when `key` is not a valid name
/// (the caller reports it by name instead of writing it).
pub fn serialize_line(key: &str, value: &str) -> Option<String> {
    if !is_valid_key(key) {
        return None;
    }
    Some(format!("{key}={}", serialize_value(value)))
}

/// Parses the dotenv grammar `serialize_value` writes (plus `export`
/// prefixes and `#` comments): bare, single-quoted (literal, may span
/// lines) and double-quoted (`\\`, `\"`, `\$`, `\n`, `\r` escapes, may span
/// lines). Lines without a usable `KEY=` are skipped. No interpolation.
pub fn parse_dotenv(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < content.len() {
        let line_end = content[pos..].find('\n').map_or(content.len(), |i| pos + i);
        let line = &content[pos..line_end];
        let trimmed = line.trim_start();
        let decl = trimmed.strip_prefix("export ").map_or(trimmed, str::trim_start);
        let eq = match decl.find('=') {
            Some(i) if !trimmed.starts_with('#') => i,
            _ => {
                pos = line_end + 1;
                continue;
            }
        };
        let key = decl[..eq].trim();
        let after_eq = &decl[eq + 1..];
        let blanks = after_eq.len() - after_eq.trim_start_matches([' ', '\t']).len();
        // `decl` is a suffix of `line`, so the value's absolute offset is
        // the end of the line minus what follows the blanks after `=`.
        let value_start = line_end - (after_eq.len() - blanks);
        let rest = &content[value_start..];
        let (value, consumed) = match rest.chars().next() {
            Some('\'') => parse_single_quoted(rest),
            Some('"') => parse_double_quoted(rest),
            _ => {
                let raw = &content[value_start..line_end];
                let raw = raw.find(" #").map_or(raw, |i| &raw[..i]);
                (raw.trim().to_string(), line_end - value_start)
            }
        };
        if !key.is_empty() {
            out.push((key.to_string(), value));
        }
        // Skip whatever trails the value on its last line.
        let end = value_start + consumed;
        pos = content[end..].find('\n').map_or(content.len(), |i| end + i + 1);
    }
    out
}

/// `s` starts with `'`. Returns the literal text up to the closing quote and
/// the number of bytes consumed including both quotes.
fn parse_single_quoted(s: &str) -> (String, usize) {
    let body = &s[1..];
    match body.find('\'') {
        Some(i) => (body[..i].to_string(), i + 2),
        None => (body.to_string(), s.len()),
    }
}

/// `s` starts with `"`. Returns the unescaped text up to the closing quote
/// and the number of bytes consumed including both quotes.
fn parse_double_quoted(s: &str) -> (String, usize) {
    let mut value = String::new();
    let mut chars = s.char_indices().skip(1);
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return (value, i + 1),
            '\\' => match chars.next() {
                Some((_, 'n')) => value.push('\n'),
                Some((_, 'r')) => value.push('\r'),
                Some((_, e @ ('\\' | '"' | '$'))) => value.push(e),
                Some((_, other)) => {
                    value.push('\\');
                    value.push(other);
                }
                None => value.push('\\'),
            },
            c => value.push(c),
        }
    }
    (value, s.len())
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

#[cfg(test)]
mod content_safety_tests {
    use super::*;
    use rand::{rngs::StdRng, Rng, SeedableRng};

    fn roundtrip(value: &str) -> String {
        let line = serialize_line("K", value).expect("valid key");
        let parsed = parse_dotenv(&format!("{line}\n"));
        assert_eq!(parsed.len(), 1, "one variable for {value:?}, got {parsed:?} from {line:?}");
        assert_eq!(parsed[0].0, "K");
        parsed[0].1.clone()
    }

    #[test]
    fn serialize_picks_bare_single_and_double_forms() {
        assert_eq!(serialize_value("abc/def_1.2:3@x+y,z=w-v"), "abc/def_1.2:3@x+y,z=w-v");
        assert_eq!(serialize_value(""), "");
        assert_eq!(serialize_value("has space"), "'has space'");
        assert_eq!(serialize_value("a$b#c"), "'a$b#c'");
        assert_eq!(serialize_value("it's"), "\"it's\"");
        assert_eq!(serialize_value("a\nb"), "\"a\\nb\"");
        assert_eq!(serialize_value("a\\b\"c$d\r"), "\"a\\\\b\\\"c\\$d\\r\"");
    }

    #[test]
    fn injection_attempt_in_value_yields_one_variable() {
        let value = "x\nADMIN=1";
        let line = serialize_line("K", value).unwrap();
        assert!(!line.contains('\n'), "serialized line must be a single physical line");
        let parsed = parse_dotenv(&format!("{line}\n"));
        assert_eq!(parsed, vec![("K".to_string(), value.to_string())]);
    }

    #[test]
    fn pem_value_is_one_double_quoted_line_and_roundtrips() {
        let pem = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBg$kq\n-----END PRIVATE KEY-----\n";
        let line = serialize_line("PEM", pem).unwrap();
        assert!(line.starts_with("PEM=\"") && !line.contains('\n'));
        assert_eq!(roundtrip(pem), pem);
    }

    #[test]
    fn dollar_and_single_quote_roundtrip() {
        assert_eq!(roundtrip("pa$$word"), "pa$$word");
        assert_eq!(roundtrip("it's $HOME"), "it's $HOME");
    }

    #[test]
    fn key_validation() {
        for ok in ["A", "_a", "A_B1", "a.b"] {
            assert!(is_valid_key(ok), "{ok}");
        }
        for bad in ["", "1A", "A B", "A-B", "A=B", "A\nB", "é", "A#"] {
            assert!(!is_valid_key(bad), "{bad:?}");
            assert!(serialize_line(bad, "v").is_none());
        }
    }

    #[test]
    fn parser_handles_comments_export_and_trailing_text() {
        let src = "# c\n\nexport A=1\nB='x y' # note\nC=\"l1\\nl2\"\nD=bare # comment\nnoequals\n";
        assert_eq!(
            parse_dotenv(src),
            vec![
                ("A".into(), "1".into()),
                ("B".into(), "x y".into()),
                ("C".into(), "l1\nl2".into()),
                ("D".into(), "bare".into()),
            ]
        );
    }

    #[test]
    fn property_serialize_then_parse_returns_original() {
        let alphabet: Vec<char> = "aZ09 _-./:@+,=$#'\"\\\n\r\t!&*()<>`~;|é☃".chars().collect();
        let mut rng = StdRng::seed_from_u64(0xC0DE);
        for _ in 0..5000 {
            let len = rng.gen_range(0..24);
            let value: String = (0..len).map(|_| alphabet[rng.gen_range(0..alphabet.len())]).collect();
            assert_eq!(roundtrip(&value), value, "value {value:?}");
        }
    }

    #[test]
    fn multiple_lines_parse_independently() {
        let mut content = String::new();
        for (k, v) in [("A", "x\ny"), ("B", "it's"), ("C", "plain"), ("D", "a b")] {
            content.push_str(&serialize_line(k, v).unwrap());
            content.push('\n');
        }
        assert_eq!(
            parse_dotenv(&content),
            vec![
                ("A".into(), "x\ny".into()),
                ("B".into(), "it's".into()),
                ("C".into(), "plain".into()),
                ("D".into(), "a b".into()),
            ]
        );
    }

    /// Runs `program`; `None` when the tool is missing or the check cannot
    /// run, so the compatibility tests skip instead of failing.
    fn run_tool(program: &str, args: &[&str]) -> Option<String> {
        let out = std::process::Command::new(program).args(args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn fixture(dir: &Path, vars: &[(&str, &str)]) -> PathBuf {
        let file = dir.join("fixture.env");
        let mut content = String::new();
        for (k, v) in vars {
            content.push_str(&serialize_line(k, v).unwrap());
            content.push('\n');
        }
        std::fs::write(&file, content).unwrap();
        file
    }

    #[test]
    fn compat_fixture_bash_source_reads_bare_and_single_quoted() {
        let dir = tempfile::tempdir().unwrap();
        let file = fixture(dir.path(), &[("BARE", "abc/def:1"), ("SPACE", "a b $HOME #x"), ("EMPTY", "")]);
        let script = format!("set -a; . '{}'; printf '%s|%s|%s' \"$BARE\" \"$SPACE\" \"$EMPTY\"", file.display());
        let Some(out) = run_tool("bash", &["-c", &script]) else { return };
        assert_eq!(out, "abc/def:1|a b $HOME #x|");
    }

    #[test]
    fn compat_fixture_python_dotenv() {
        let dir = tempfile::tempdir().unwrap();
        let pem = "-----BEGIN-----\nab\"c\\d\n-----END-----";
        let file = fixture(dir.path(), &[("A", "plain"), ("B", "a b"), ("C", pem), ("D", "it's")]);
        let code = "import sys,json;from dotenv import dotenv_values;print(json.dumps(dotenv_values(sys.argv[1])))";
        let Some(out) = run_tool("python3", &["-c", code, file.to_str().unwrap()]) else { return };
        let got: std::collections::BTreeMap<String, String> = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(got["A"], "plain");
        assert_eq!(got["B"], "a b");
        assert_eq!(got["C"], pem);
        assert_eq!(got["D"], "it's");
    }

    #[test]
    fn compat_fixture_node_dotenv() {
        let dir = tempfile::tempdir().unwrap();
        let pem = "-----BEGIN-----\nabc\n-----END-----";
        let file = fixture(dir.path(), &[("A", "plain"), ("B", "a b"), ("C", pem), ("D", "it's")]);
        let code = "const fs=require('fs');const d=require('dotenv');console.log(JSON.stringify(d.parse(fs.readFileSync(process.argv[1]))))";
        let Some(out) = run_tool("node", &["-e", code, file.to_str().unwrap()]) else { return };
        let got: std::collections::BTreeMap<String, String> = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(got["A"], "plain");
        assert_eq!(got["B"], "a b");
        assert_eq!(got["C"], pem);
        assert_eq!(got["D"], "it's");
    }

    // ── inspect gate ──

    #[test]
    fn inspect_refuses_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(inspect(dir.path()), Err(EnvFileError::NotRegularFile(_))));
    }

    #[test]
    #[cfg(unix)]
    fn inspect_refuses_dev_zero_quickly() {
        let start = std::time::Instant::now();
        let r = inspect(Path::new("/dev/zero"));
        assert!(matches!(r, Err(EnvFileError::NotRegularFile(_))), "{r:?}");
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    #[cfg(unix)]
    fn inspect_refuses_fifo_without_blocking() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("pipe");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        let p = fifo.clone();
        std::thread::spawn(move || {
            let _ = tx.send(inspect(&p));
        });
        let r = rx.recv_timeout(std::time::Duration::from_secs(2)).expect("inspect blocked on FIFO");
        assert!(matches!(r, Err(EnvFileError::NotRegularFile(_))), "{r:?}");
    }

    #[test]
    fn inspect_treats_file_over_1mib_as_foreign() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.env");
        // Begins with the marker but exceeds the bound: still never read in full.
        let mut content = format!("{MARKER_PREFIX} managed file\n").into_bytes();
        content.resize(2 * 1024 * 1024, b'a');
        std::fs::write(&path, content).unwrap();
        assert_eq!(inspect(&path).unwrap(), Target::Foreign);
    }

    // ── atomic write ──

    fn cenv_temp_files(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".cenv-"))
            .collect()
    }

    #[test]
    fn failed_write_keeps_previous_content_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(&path, "previous\n").unwrap();

        let r = write_atomic(&path, FileMode::Private0600, |f| {
            use std::io::Write as _;
            f.write_all(b"partial secret")?;
            Err(io::Error::other("disk full"))
        });
        assert!(matches!(r, Err(EnvFileError::Io(..))));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous\n");
        assert!(cenv_temp_files(dir.path()).is_empty());
    }

    #[test]
    fn successful_commit_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        let opts = WriteOptions { overwrite: false, mode: FileMode::Private0600 };
        commit(&path, "A=1\n", &marker_line("p", "e"), &opts).unwrap();
        assert!(cenv_temp_files(dir.path()).is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn new_file_is_created_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        let opts = WriteOptions { overwrite: false, mode: FileMode::Private0600 };
        commit(&path, "A=1\n", &marker_line("p", "e"), &opts).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    #[cfg(unix)]
    fn inherit_mode_keeps_existing_file_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env.example");
        std::fs::write(&path, format!("{MARKER_PREFIX} x\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o664)).unwrap();
        let opts = WriteOptions { overwrite: false, mode: FileMode::Inherit };
        commit(&path, "A=\n", &marker_line("p", "e"), &opts).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o664);
    }

    #[test]
    #[cfg(unix)]
    fn target_swapped_for_symlink_before_rename_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        let victim = dir.path().join("victim");
        std::fs::write(&victim, "keep").unwrap();
        let r = write_atomic(&path, FileMode::Private0600, |f| {
            use std::io::Write as _;
            // Simulates the race: the link appears while the body is written.
            std::os::unix::fs::symlink(&victim, &path)?;
            f.write_all(b"secret")
        });
        assert!(matches!(r, Err(EnvFileError::Symlink(_))), "{r:?}");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        assert!(cenv_temp_files(dir.path()).is_empty());
    }
}
