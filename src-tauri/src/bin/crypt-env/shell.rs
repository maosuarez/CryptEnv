pub enum Shell {
    PowerShell,
    Bash,
    Zsh,
    Sh,
}

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

/// Formats a shell variable assignment with safe quoting.
/// The value is single-quote-escaped — never concatenated unsafely.
pub fn format_assignment(shell: &Shell, key: &str, value: &str) -> String {
    match shell {
        Shell::PowerShell => {
            // PowerShell single-quote escaping: double the single quotes
            let escaped = value.replace('\'', "''");
            format!("$env:{} = '{}'", key, escaped)
        }
        _ => {
            // POSIX single-quote escaping: end quote, escaped quote, reopen quote
            let escaped = value.replace('\'', "'\\''");
            format!("export {}='{}'", key, escaped)
        }
    }
}
