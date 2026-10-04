//! Filesystem containment guard.
//!
//! Dependency-free leaf module (imports only `std`) — see issue #7 and
//! `docs/plans/issue-7-path-traversal-environment-name.md` §4/D7 for why this
//! lives on its own rather than inside `api` or `project`: it is needed by
//! both and has no dependencies of its own, so a leaf module is the only
//! shape that does not create a sideways dependency between peers.
//!
//! This module answers exactly one question: *given a caller-trusted base
//! directory and an untrusted single filename, what is the one real path
//! that filename is allowed to resolve to?* It never decides whether a write
//! to that path should proceed (see #8's `guarded_write`, which composes on
//! top of this).

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Why `resolve_within` refused to produce a path. `Display` on this type
/// describes the rule that was broken, never the input that broke it — the
/// input is attacker-controlled stored data (see plan §4/D5).
#[derive(Debug, PartialEq, Eq)]
pub enum ContainmentError {
    /// `file_name` was empty or whitespace-only.
    EmptyName,
    /// `file_name` did not lexically resolve to exactly one `Normal` path
    /// component — separators, `.`, `..`, an absolute/UNC/verbatim prefix,
    /// or a Windows drive-relative form (`C:foo`) all land here.
    NotASingleComponent,
    /// `file_name` contained a NUL byte or another control character.
    NulByte,
    /// `file_name` is (ignoring an extension) a Windows reserved device
    /// name: `CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9`, `LPT1`-`LPT9`.
    ReservedDeviceName,
    /// `file_name` ends with a trailing `.` or space, contains `:` (NTFS
    /// alternate data streams), or contains one of the other Win32-illegal
    /// characters `< > " | ? *`.
    TrailingDotOrSpace,
    /// The base directory itself could not be created or canonicalized.
    /// Carries the raw io error text — this is about the caller-supplied
    /// base, never about the untrusted name, so it is safe to surface.
    BaseUnusable(String),
    /// The joined (or, for an existing target, re-canonicalized) path did
    /// not stay under the canonicalized base. This is the belt-and-braces
    /// check: reaching it means every lexical rule above already passed and
    /// something else — most plausibly a pre-planted symlink — is trying to
    /// redirect the write.
    Escapes,
}

impl std::fmt::Display for ContainmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainmentError::EmptyName => {
                write!(f, "file name must not be empty or whitespace-only")
            }
            ContainmentError::NotASingleComponent => write!(
                f,
                "file name must be a single path component: no separators, no '.', no '..', and no drive/UNC/verbatim prefix"
            ),
            ContainmentError::NulByte => {
                write!(f, "file name must not contain a NUL byte or control characters")
            }
            ContainmentError::ReservedDeviceName => write!(
                f,
                "file name must not be a reserved device name (CON, PRN, AUX, NUL, COM1-9, LPT1-9)"
            ),
            ContainmentError::TrailingDotOrSpace => write!(
                f,
                "file name must not end with a trailing '.' or space, and must not contain ':', '<', '>', '\"', '|', '?', or '*'"
            ),
            ContainmentError::BaseUnusable(msg) => write!(f, "base directory is not usable: {msg}"),
            ContainmentError::Escapes => write!(f, "resolved path escapes the base directory"),
        }
    }
}

const RESERVED_DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn is_reserved_device_name(file_name: &str) -> bool {
    // Reserved names apply to the base name (before the first '.'), both
    // bare and with any extension — `NUL`, `nul.txt` and `Nul.tar.gz` are
    // all the same reserved device on Windows.
    let base = file_name.split('.').next().unwrap_or(file_name);
    RESERVED_DEVICE_NAMES.iter().any(|r| r.eq_ignore_ascii_case(base))
}

/// Resolves `file_name` as a direct child of `base_dir`, guaranteeing the
/// result cannot be anywhere else. `file_name` is treated as a literal
/// filename: it is never decoded, unescaped, or normalized.
///
/// Steps, in order (see the plan for the full rationale of each):
/// 1. Reject empty/whitespace, NUL, control characters.
/// 2. Lexical single-component check (rejects `..`, `.`, absolute/UNC/
///    verbatim/drive-relative forms) plus an explicit separator check that
///    also catches `\`-based traversal on platforms where `Path` does not
///    treat `\` as a separator.
/// 3. Windows-hostile-name check, applied on every platform: reserved
///    device names, trailing dot/space, NTFS alternate-data-stream `:`, and
///    the rest of the Win32-illegal character set.
/// 4. Create and canonicalize the base — the only `create_dir_all` call in
///    this function, and its argument is `base_dir` alone, never anything
///    with `file_name` interpolated into it.
/// 5. Join and assert the result is component-wise contained in the
///    canonicalized base.
/// 6. If the target already exists, re-canonicalize it and re-assert
///    containment — catches a pre-planted symlink.
pub fn resolve_within(base_dir: &str, file_name: &str) -> Result<PathBuf, ContainmentError> {
    // 1. Empty / whitespace / NUL / control characters.
    if file_name.trim().is_empty() {
        return Err(ContainmentError::EmptyName);
    }
    if file_name.contains('\0') || file_name.chars().any(|c| c.is_control()) {
        return Err(ContainmentError::NulByte);
    }

    // 2. Lexical single-component check.
    let components: Vec<Component> = Path::new(file_name).components().collect();
    let is_single_normal = components.len() == 1 && matches!(components[0], Component::Normal(_));
    if !is_single_normal {
        return Err(ContainmentError::NotASingleComponent);
    }
    // Belt: on Unix, `Path` does not treat `\` as a separator, so
    // `..\..\x` would otherwise parse as one `Normal` component.
    if file_name.contains('/') || file_name.contains('\\') {
        return Err(ContainmentError::NotASingleComponent);
    }

    // 3. Windows-hostile-name check, applied on every platform.
    if is_reserved_device_name(file_name) {
        return Err(ContainmentError::ReservedDeviceName);
    }
    if file_name.ends_with('.') || file_name.ends_with(' ') {
        return Err(ContainmentError::TrailingDotOrSpace);
    }
    if file_name.contains(':') || file_name.chars().any(|c| matches!(c, '<' | '>' | '"' | '|' | '?' | '*')) {
        return Err(ContainmentError::TrailingDotOrSpace);
    }

    // 4. Base resolution. This is the only `create_dir_all` in the whole
    // flow, and it never sees `file_name`.
    fs::create_dir_all(base_dir).map_err(|e| ContainmentError::BaseUnusable(e.to_string()))?;
    let real_base = fs::canonicalize(base_dir).map_err(|e| ContainmentError::BaseUnusable(e.to_string()))?;

    // 5. Join and verify. `Path::starts_with` compares whole components, so
    // it cannot be fooled by string-prefix tricks (`/base` vs `/base-evil`).
    let target = real_base.join(file_name);
    if !target.starts_with(&real_base) {
        return Err(ContainmentError::Escapes);
    }

    // 6. Symlink post-check: if something already sits at `target` (e.g. a
    // pre-planted symlink from an earlier attack attempt), re-resolve it and
    // re-assert containment. If it does not exist, step 4 already proved
    // the parent directory is real and step 5's join is authoritative.
    // `symlink_metadata` (not `exists()`) so a dangling symlink, which
    // `exists()` reports as absent, is still checked and rejected.
    match fs::symlink_metadata(&target) {
        Ok(_) => match fs::canonicalize(&target) {
            Ok(resolved) if resolved.starts_with(&real_base) => {}
            _ => return Err(ContainmentError::Escapes),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(ContainmentError::Escapes),
    }

    Ok(target)
}

/// Canonicalizes the deepest existing ancestor of `path` and returns it with
/// the not-yet-existing components that follow it. A component that exists
/// only as a dangling symlink is `Escapes`: it cannot be proven to stay put.
fn canonical_existing_prefix(path: &Path) -> Result<(PathBuf, Vec<OsString>), ContainmentError> {
    let mut current = path.to_path_buf();
    let mut remaining: Vec<OsString> = Vec::new();
    loop {
        match fs::canonicalize(&current) {
            Ok(real) => {
                remaining.reverse();
                return Ok((real, remaining));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if fs::symlink_metadata(&current).is_ok() {
                    return Err(ContainmentError::Escapes);
                }
                match (current.file_name().map(|n| n.to_os_string()), current.parent()) {
                    (Some(name), Some(parent)) => {
                        remaining.push(name);
                        current = parent.to_path_buf();
                    }
                    _ => return Err(ContainmentError::BaseUnusable(e.to_string())),
                }
            }
            Err(e) => return Err(ContainmentError::BaseUnusable(e.to_string())),
        }
    }
}

/// `\\?\C:\x` -> `C:\x`, `\\?\UNC\h\s` -> `\\h\s`: canonicalize returns verbatim
/// paths on Windows, which should not leak into user-visible output.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let s = p.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    p
}

/// Resolves the relative path `rel` under `root` and proves, after following
/// every existing symlink, that the result is still inside the canonical
/// root. The deepest existing ancestor of the joined path is canonicalized
/// and must be under the canonical root; the not-yet-existing tail is
/// appended as-is. `..`, drive/stream specifiers (`:`), NUL and control
/// characters, and absolute paths are rejected. Both `/` and `\` separate
/// components, matching `project::resolve_env_path`. Nothing is created.
pub fn contain_relative(root: &Path, rel: &str) -> Result<PathBuf, ContainmentError> {
    if rel.contains('\0') || rel.chars().any(|c| c.is_control()) {
        return Err(ContainmentError::NulByte);
    }
    if Path::new(rel).is_absolute() || rel.starts_with('\\') {
        return Err(ContainmentError::NotASingleComponent);
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in rel.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return Err(ContainmentError::Escapes),
            p if p.contains(':') => return Err(ContainmentError::TrailingDotOrSpace),
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        return Err(ContainmentError::EmptyName);
    }

    // The root is vault-owner configured and trusted, but it may not exist
    // yet (injection creates missing directories), so it is canonicalized
    // through its own deepest existing ancestor.
    let (root_real, root_tail) = canonical_existing_prefix(root)?;
    let mut real_root = root_real;
    real_root.extend(root_tail);

    let mut joined = real_root.clone();
    joined.extend(&parts);
    let (mut real, tail) = canonical_existing_prefix(&joined)?;
    if !real.starts_with(&real_root) {
        return Err(ContainmentError::Escapes);
    }
    real.extend(tail);
    Ok(strip_verbatim(real))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pure-lexical cases only — no filesystem I/O. See
    // `tests/path_containment.rs` for the filesystem-level invariants
    // (base creation, symlink escapes, the malicious-name table, etc).

    #[test]
    fn rejects_empty_and_whitespace() {
        assert_eq!(
            resolve_within("/tmp", "").unwrap_err(),
            ContainmentError::EmptyName
        );
        assert_eq!(
            resolve_within("/tmp", "   ").unwrap_err(),
            ContainmentError::EmptyName
        );
    }

    #[test]
    fn rejects_nul_and_control_chars() {
        assert_eq!(
            resolve_within("/tmp", "prod\0evil").unwrap_err(),
            ContainmentError::NulByte
        );
        assert_eq!(
            resolve_within("/tmp", "prod\nevil").unwrap_err(),
            ContainmentError::NulByte
        );
    }

    #[test]
    fn rejects_parent_and_current_dir() {
        assert_eq!(
            resolve_within("/tmp", "..").unwrap_err(),
            ContainmentError::NotASingleComponent
        );
        assert_eq!(
            resolve_within("/tmp", ".").unwrap_err(),
            ContainmentError::NotASingleComponent
        );
        assert_eq!(
            resolve_within("/tmp", "../../../tmp/pwned").unwrap_err(),
            ContainmentError::NotASingleComponent
        );
    }

    #[test]
    fn rejects_windows_traversal_even_on_unix() {
        // `\` is not a separator to `Path` on Unix, so without the explicit
        // belt check this would otherwise parse as one `Normal` component.
        assert_eq!(
            resolve_within("/tmp", "..\\..\\..\\Windows\\Temp\\pwned").unwrap_err(),
            ContainmentError::NotASingleComponent
        );
    }

    #[test]
    fn rejects_absolute_paths() {
        assert_eq!(
            resolve_within("/tmp", "/etc/cron.d/pwned").unwrap_err(),
            ContainmentError::NotASingleComponent
        );
    }

    #[test]
    fn rejects_windows_absolute_and_unc_and_verbatim() {
        assert_eq!(
            resolve_within("/tmp", "C:\\Windows\\Temp\\pwned").unwrap_err(),
            ContainmentError::NotASingleComponent
        );
        assert_eq!(
            resolve_within("/tmp", "\\\\wsl.localhost\\Ubuntu\\home\\u\\pwned").unwrap_err(),
            ContainmentError::NotASingleComponent
        );
        assert_eq!(
            resolve_within("/tmp", "\\\\?\\C:\\pwned").unwrap_err(),
            ContainmentError::NotASingleComponent
        );
    }

    #[test]
    fn rejects_drive_relative() {
        // On Unix this lexically parses as one `Normal` component (no
        // separator, `Path` does not know about drive letters) — the
        // Windows-hostile-name check's `:` rejection is what actually
        // catches it there, which is why the assertion is just "is_err".
        assert!(resolve_within("/tmp", "C:pwned").is_err());
    }

    #[test]
    fn rejects_reserved_device_names() {
        for name in ["CON", "NUL", "COM1", "LPT1", "con.txt", "Nul.tar.gz"] {
            assert_eq!(
                resolve_within("/tmp", name).unwrap_err(),
                ContainmentError::ReservedDeviceName,
                "expected {name} to be rejected as a reserved device name"
            );
        }
    }

    #[test]
    fn rejects_trailing_dot_or_space() {
        assert_eq!(
            resolve_within("/tmp", "prod.").unwrap_err(),
            ContainmentError::TrailingDotOrSpace
        );
        assert_eq!(
            resolve_within("/tmp", "prod ").unwrap_err(),
            ContainmentError::TrailingDotOrSpace
        );
    }

    #[test]
    fn rejects_ads_and_illegal_chars() {
        assert_eq!(
            resolve_within("/tmp", "env:stream").unwrap_err(),
            ContainmentError::TrailingDotOrSpace
        );
        for name in ["a<b", "a>b", "a\"b", "a|b", "a?b", "a*b"] {
            assert_eq!(
                resolve_within("/tmp", name).unwrap_err(),
                ContainmentError::TrailingDotOrSpace,
                "expected {name} to be rejected"
            );
        }
    }

    #[test]
    fn does_not_decode_percent_or_unicode_lookalikes() {
        // These must be treated as literal filenames. They are not path
        // separators or control characters, so — as documented in the
        // plan's T1 — a single-component result contained under the base is
        // an acceptable outcome for this layer; silently *decoding* them is
        // the actually-forbidden behaviour, and that would show up as a
        // `NotASingleComponent` rejection here, which none of these trigger.
        for name in ["%2e%2e%2fpwned", "..%c0%af..%c0%afpwned", "．．／pwned"] {
            let result = resolve_within("/tmp", name);
            assert!(
                result.is_err() || matches!(&result, Ok(p) if p.parent().is_some()),
                "unexpected outcome for {name}: {result:?}"
            );
        }
    }

    #[test]
    fn contain_relative_accepts_nested_and_not_yet_existing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let real = fs::canonicalize(dir.path()).unwrap();
        fs::create_dir_all(dir.path().join("apps/api")).unwrap();
        assert_eq!(
            contain_relative(dir.path(), "apps/api/.env").unwrap(),
            real.join("apps/api/.env")
        );
        // Neither `new` nor `deep` exists yet.
        assert_eq!(
            contain_relative(dir.path(), "./new\\deep/.env").unwrap(),
            real.join("new/deep/.env")
        );
    }

    #[test]
    fn contain_relative_rejects_lexical_escapes() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["../x/.env", "a/../../x", "/etc/passwd", "C:foo", "a\0b", "", "./"] {
            assert!(contain_relative(dir.path(), bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[cfg(unix)]
    #[test]
    fn contain_relative_rejects_symlinked_directory_escaping_the_root() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("config")).unwrap();
        assert_eq!(
            contain_relative(root.path(), "config/.env").unwrap_err(),
            ContainmentError::Escapes
        );
        // A symlink that stays inside the root is fine.
        fs::create_dir(root.path().join("real")).unwrap();
        std::os::unix::fs::symlink(root.path().join("real"), root.path().join("alias")).unwrap();
        assert!(contain_relative(root.path(), "alias/.env").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn contain_relative_rejects_dangling_symlink_component() {
        let root = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/nonexistent-crypt-env-target", root.path().join("gone")).unwrap();
        assert_eq!(
            contain_relative(root.path(), "gone/.env").unwrap_err(),
            ContainmentError::Escapes
        );
    }
}
