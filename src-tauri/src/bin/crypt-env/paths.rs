//! Host ↔ local path translation (cli-tui-parity design D9).
//!
//! The vault stores paths as the GUI host sees them. When this CLI is a
//! native Linux binary inside WSL (`WSL_DISTRO_NAME` set) and the vault runs
//! on Windows, absolute paths are translated: `/home/u/app` ↔
//! `\\wsl.localhost\<distro>\home\u\app` and `/mnt/c/x` ↔ `C:\x`. Set
//! `CRYPTENV_PATH_TRANSLATION=off` when the vault itself runs inside the
//! distro. Pure string rewriting — no `wslpath` process — assuming the
//! default `/mnt` automount root.

use std::path::Path;

/// The current WSL distro when translation applies, else `None`.
pub fn wsl_distro() -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    if std::env::var("CRYPTENV_PATH_TRANSLATION").map(|v| v.eq_ignore_ascii_case("off")).unwrap_or(false) {
        return None;
    }
    std::env::var("WSL_DISTRO_NAME").ok().filter(|d| !d.trim().is_empty())
}

/// Local absolute path → the form the vault host uses.
pub fn to_host(local: &Path) -> String {
    let s = local.to_string_lossy().into_owned();
    match wsl_distro() {
        Some(d) => linux_to_windows(&d, &s),
        None => s,
    }
}

/// Vault-host absolute path → local form (unchanged when not translatable).
pub fn to_local(host: &str) -> String {
    match wsl_distro() {
        Some(d) => windows_to_linux(&d, host).unwrap_or_else(|| host.to_string()),
        None => host.to_string(),
    }
}

pub fn linux_to_windows(distro: &str, path: &str) -> String {
    if !path.starts_with('/') {
        return path.to_string();
    }
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() >= 2 && parts[0] == "mnt" && parts[1].len() == 1 && parts[1].chars().all(|c| c.is_ascii_alphabetic()) {
        let drive = parts[1].to_ascii_uppercase();
        return format!("{drive}:\\{}", parts[2..].join("\\"));
    }
    format!("\\\\wsl.localhost\\{distro}\\{}", parts.join("\\"))
}

pub fn windows_to_linux(distro: &str, path: &str) -> Option<String> {
    let b = path.as_bytes();
    if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/') {
        let rest = path[3..].replace('\\', "/");
        let drive = (b[0] as char).to_ascii_lowercase();
        return Some(format!("/mnt/{drive}/{}", rest.trim_start_matches('/')).trim_end_matches('/').to_string());
    }
    let norm = path.replace('/', "\\");
    for prefix in ["\\\\wsl.localhost\\", "\\\\wsl$\\"] {
        if let Some(rest) = norm.strip_prefix(prefix) {
            let (d, tail) = rest.split_once('\\').unwrap_or((rest, ""));
            if !d.eq_ignore_ascii_case(distro) {
                return None;
            }
            return Some(format!("/{}", tail.replace('\\', "/")));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_paths_map_to_unc_or_drive() {
        assert_eq!(linux_to_windows("Ubuntu", "/home/u/app"), r"\\wsl.localhost\Ubuntu\home\u\app");
        assert_eq!(linux_to_windows("Ubuntu", "/mnt/c/Users/u/app"), r"C:\Users\u\app");
        assert_eq!(linux_to_windows("Ubuntu", "rel/x"), "rel/x");
    }

    #[test]
    fn windows_paths_map_back() {
        assert_eq!(windows_to_linux("Ubuntu", r"\\wsl.localhost\Ubuntu\home\u\app").as_deref(), Some("/home/u/app"));
        assert_eq!(windows_to_linux("ubuntu", r"\\wsl$\Ubuntu\home\u").as_deref(), Some("/home/u"));
        assert_eq!(windows_to_linux("Ubuntu", r"C:\Users\u\app").as_deref(), Some("/mnt/c/Users/u/app"));
        assert_eq!(windows_to_linux("Ubuntu", r"\\wsl.localhost\Debian\x"), None);
        assert_eq!(windows_to_linux("Ubuntu", "/already/linux"), None);
    }
}
