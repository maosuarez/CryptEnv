//! WSL bridge: running a command in the caller's WSL distribution from a
//! Windows backend (`wsl.exe -d <distro> --cd <dir> --exec ...`).
//!
//! Secrets must not appear in `wsl.exe`'s argv (visible to every local
//! process) nor in `WSLENV` (which would need the values in the Windows
//! environment). They travel over the child's **stdin** instead: argv is fixed
//! to `/bin/sh -s`, and the script that sh reads from stdin exports the values,
//! then `exec`s the command with its own stdin pointed at `/dev/null`. The
//! command text itself also rides in that script, which keeps `wsl.exe` argv
//! free of anything that needs Windows command-line quoting.

use zeroize::Zeroizing;

use super::ExecError;

/// Where inside WSL the command runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslTarget {
    pub distro: String,
    /// Linux-side working directory (`--cd`).
    pub cwd: Option<String>,
    /// Linux-side `$HOME` for the cleared environment.
    pub home: Option<String>,
}

const LINUX_BASELINE_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// WSL distribution names: letters, digits, `.`, `_`, `-`.
pub fn is_valid_distro(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Absolute POSIX path without control characters.
pub fn is_valid_linux_path(p: &str) -> bool {
    p.starts_with('/') && p.len() <= 4096 && !p.chars().any(|c| c.is_control())
}

/// POSIX single-quote escaping: sound for every byte except NUL.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Fixed `wsl.exe` argv. Contains no secret and no command text.
pub fn build_argv(target: &WslTarget) -> Result<Vec<String>, ExecError> {
    if !is_valid_distro(&target.distro) {
        return Err(ExecError::InvalidContext("wslDistro".to_string()));
    }
    let mut argv = vec!["-d".to_string(), target.distro.clone()];
    if let Some(cwd) = &target.cwd {
        if !is_valid_linux_path(cwd) {
            return Err(ExecError::InvalidContext("cwd".to_string()));
        }
        argv.push("--cd".to_string());
        argv.push(cwd.clone());
    }
    argv.extend(
        [
            "--exec",
            "/usr/bin/env",
            "-i",
            &format!("PATH={LINUX_BASELINE_PATH}"),
            "LANG=C.UTF-8",
            "/bin/sh",
            "-s",
        ]
        .map(String::from),
    );
    Ok(argv)
}

/// The script written to the child's stdin. Fails on values containing NUL
/// (not representable in a shell variable).
pub fn build_script(
    target: &WslTarget,
    command: &str,
    env: &[(String, Zeroizing<String>)],
) -> Result<Zeroizing<String>, ExecError> {
    let mut script = Zeroizing::new(String::new());
    if let Some(home) = &target.home {
        if !is_valid_linux_path(home) {
            return Err(ExecError::InvalidContext("home".to_string()));
        }
        script.push_str(&format!("export HOME={}\n", sh_quote(home)));
    }
    for (key, value) in env {
        if value.contains('\0') {
            return Err(ExecError::UnsupportedValue(key.clone()));
        }
        script.push_str(&format!("export {key}={}\n", sh_quote(value)));
    }
    if command.contains('\0') {
        return Err(ExecError::InvalidContext("command".to_string()));
    }
    script.push_str(&format!("exec /bin/sh -c {} </dev/null\n", sh_quote(command)));
    Ok(script)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> WslTarget {
        WslTarget { distro: "Ubuntu-22.04".into(), cwd: Some("/home/u/my proj".into()), home: Some("/home/u".into()) }
    }

    #[test]
    fn argv_is_fixed_and_carries_no_secret_or_command() {
        let argv = build_argv(&target()).unwrap();
        assert_eq!(
            argv,
            vec![
                "-d", "Ubuntu-22.04", "--cd", "/home/u/my proj", "--exec", "/usr/bin/env", "-i",
                "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin", "LANG=C.UTF-8",
                "/bin/sh", "-s"
            ]
        );
    }

    #[test]
    fn bad_distro_or_cwd_is_rejected() {
        let mut t = target();
        t.distro = "Ubuntu; rm".into();
        assert!(build_argv(&t).is_err());
        let mut t = target();
        t.cwd = Some("relative/dir".into());
        assert!(build_argv(&t).is_err());
    }

    #[test]
    fn script_exports_secrets_and_quotes_them() {
        let env = vec![("DB_PASSWORD".to_string(), Zeroizing::new("it's a $(secret)".to_string()))];
        let script = build_script(&target(), "printenv DB_PASSWORD", &env).unwrap();
        assert!(script.contains("export HOME='/home/u'\n"));
        assert!(script.contains("export DB_PASSWORD='it'\\''s a $(secret)'\n"));
        assert!(script.ends_with("exec /bin/sh -c 'printenv DB_PASSWORD' </dev/null\n"));
    }

    #[test]
    fn nul_in_a_value_is_rejected() {
        let env = vec![("K".to_string(), Zeroizing::new("a\0b".to_string()))];
        assert!(build_script(&target(), "true", &env).is_err());
    }

    #[test]
    fn secrets_never_reach_argv() {
        let env = vec![("DB_PASSWORD".to_string(), Zeroizing::new("hunter2!".to_string()))];
        let argv = build_argv(&target()).unwrap().join(" ");
        assert!(!argv.contains("hunter2!"));
        assert!(build_script(&target(), "true", &env).unwrap().contains("hunter2!"));
    }
}
