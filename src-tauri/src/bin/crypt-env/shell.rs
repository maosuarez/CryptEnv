//! Shell detection for the CLI. The assignment formatter lives in the library
//! crate (`crypt_env_lib::shellfmt`) so the GUI shares it.

pub use crypt_env_lib::shellfmt::{format_assignment, Shell};

pub fn detect_shell() -> Shell {
    if std::env::var("PSModulePath").is_ok() {
        return Shell::PowerShell;
    }
    match std::env::var("SHELL").unwrap_or_default().to_lowercase() {
        s if s.contains("zsh") => Shell::Zsh,
        s if s.contains("bash") => Shell::Bash,
        _ => Shell::Sh,
    }
}
