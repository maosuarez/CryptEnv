//! Shell assignment formatter shared by the CLI (`inject`) and the GUI
//! ("copy as"). The emitted text, evaluated by the target shell, assigns
//! exactly the stored value and never executes any part of it.

use std::fmt;

/// Characters PowerShell treats as a single quote.
const POWERSHELL_QUOTES: [char; 5] = ['\'', '\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    PowerShell,
    Bash,
    Zsh,
    Sh,
}

impl Shell {
    /// Parses the names accepted by `--shell` and the GUI: pwsh/powershell/ps1, bash, zsh, sh.
    pub fn parse(name: &str) -> Option<Shell> {
        match name {
            "pwsh" | "powershell" | "ps1" => Some(Shell::PowerShell),
            "bash" => Some(Shell::Bash),
            "zsh" => Some(Shell::Zsh),
            "sh" => Some(Shell::Sh),
            _ => None,
        }
    }
}

/// Errors never carry the value, only the key name.
#[derive(Debug, PartialEq, Eq)]
pub enum ShellFmtError {
    InvalidKey(String),
    UnsupportedValue(String),
}

impl fmt::Display for ShellFmtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShellFmtError::InvalidKey(k) => write!(f, "'{k}' is not a valid variable name"),
            ShellFmtError::UnsupportedValue(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for ShellFmtError {}

/// `^[A-Za-z_][A-Za-z0-9_]*$`
pub fn is_valid_shell_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Formats a shell variable assignment for `shell`. The key must be a valid
/// variable name; the value is always quoted, never concatenated raw.
pub fn format_assignment(shell: &Shell, key: &str, value: &str) -> Result<String, ShellFmtError> {
    if !is_valid_shell_key(key) {
        return Err(ShellFmtError::InvalidKey(key.to_string()));
    }
    match shell {
        Shell::PowerShell => {
            // Single-quoted PowerShell strings are literal; only the quote characters need doubling.
            let mut escaped = String::with_capacity(value.len());
            for c in value.chars() {
                escaped.push(c);
                if POWERSHELL_QUOTES.contains(&c) {
                    escaped.push(c);
                }
            }
            Ok(format!("$env:{key} = '{escaped}'"))
        }
        Shell::Bash | Shell::Zsh | Shell::Sh => {
            if value.contains('\0') {
                return Err(ShellFmtError::UnsupportedValue(format!(
                    "the value of '{key}' contains a NUL byte, which shell variables cannot hold"
                )));
            }
            if !value.contains(['\n', '\r']) {
                // End quote, escaped quote, reopen quote.
                return Ok(format!("export {key}='{}'", value.replace('\'', "'\\''")));
            }
            if *shell == Shell::Sh {
                return Err(ShellFmtError::UnsupportedValue(format!(
                    "the value of '{key}' contains a line break, which sh output cannot represent; use --shell bash or zsh"
                )));
            }
            Ok(format!("export {key}={}", ansi_c_quote(value)))
        }
    }
}

/// bash/zsh `$'...'` quoting: backslash, quote and every control character escaped.
fn ansi_c_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 3);
    out.push_str("$'");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_ascii_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// GUI "copy as": `env` uses the dotenv serializer, the rest the shell formatter.
#[tauri::command]
pub fn shell_format_assignment(shell: String, key: String, value: String) -> Result<String, String> {
    if shell == "env" {
        return crate::envfile::serialize_line(&key, &value)
            .ok_or_else(|| ShellFmtError::InvalidKey(key).to_string());
    }
    let target = Shell::parse(&shell).ok_or_else(|| format!("unknown shell '{shell}'"))?;
    format_assignment(&target, &key, &value).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;

    /// Values every shell must round-trip exactly; payloads create `sentinel` if they execute.
    fn corpus(sentinel: &Path) -> Vec<String> {
        let s = sentinel.display().to_string();
        vec![
            String::new(),
            "plain".into(),
            "with space".into(),
            "it's".into(),
            "'\u{2018}\u{2019}\u{201A}\u{201B}'".into(),
            format!("\u{2019}; New-Item {s} ;\u{2019}"),
            format!("\u{2019}; touch {s} ;\u{2019}"),
            format!("x$(touch {s})"),
            format!("`touch {s}`"),
            format!("a;touch {s};b"),
            format!("a&touch {s}&b"),
            format!("a|touch {s}|b"),
            "$HOME ${HOME} $env:HOME".into(),
            "\"quoted\" \\ back\\slash".into(),
            "caf\u{e9} \u{1F511} \u{65e5}\u{672c}".into(),
            "a\tb".into(),
            "-n".into(),
            "a".repeat(64 * 1024),
        ]
    }

    fn newline_corpus() -> Vec<String> {
        vec![
            "line1\nline2".into(),
            "a\r\nb".into(),
            "trail\n".into(),
            "it's\nmulti \\n literal".into(),
            "ctl\u{1}\u{7f}\r".into(),
        ]
    }

    #[test]
    fn rejects_invalid_keys() {
        for k in ["", "1A", "A-B", "A B", "A;rm -rf ~;B", "A.B", "a$(x)", "\u{e9}"] {
            for shell in [Shell::PowerShell, Shell::Bash, Shell::Zsh, Shell::Sh] {
                assert_eq!(
                    format_assignment(&shell, k, "v"),
                    Err(ShellFmtError::InvalidKey(k.to_string()))
                );
            }
        }
    }

    #[test]
    fn powershell_doubles_all_five_quotes() {
        let out = format_assignment(&Shell::PowerShell, "K", "'\u{2018}\u{2019}\u{201A}\u{201B}").unwrap();
        assert_eq!(
            out,
            "$env:K = '''\u{2018}\u{2018}\u{2019}\u{2019}\u{201A}\u{201A}\u{201B}\u{201B}'"
        );
    }

    #[test]
    fn powershell_keeps_newlines_verbatim() {
        let out = format_assignment(&Shell::PowerShell, "K", "a\nb").unwrap();
        assert_eq!(out, "$env:K = 'a\nb'");
    }

    #[test]
    fn posix_single_quote_form() {
        assert_eq!(
            format_assignment(&Shell::Bash, "K", "it's $(x)").unwrap(),
            "export K='it'\\''s $(x)'"
        );
        assert_eq!(format_assignment(&Shell::Sh, "K", "").unwrap(), "export K=''");
    }

    #[test]
    fn bash_and_zsh_use_ansi_c_for_newlines() {
        for shell in [Shell::Bash, Shell::Zsh] {
            assert_eq!(
                format_assignment(&shell, "K", "a\nb\r'\\\u{1}").unwrap(),
                "export K=$'a\\nb\\r\\'\\\\\\x01'"
            );
        }
    }

    #[test]
    fn sh_refuses_newlines_without_echoing_the_value() {
        let err = format_assignment(&Shell::Sh, "K", "sekret\nvalue").unwrap_err();
        assert!(matches!(err, ShellFmtError::UnsupportedValue(_)));
        assert!(!err.to_string().contains("sekret"));
    }

    #[test]
    fn nul_is_refused_for_posix() {
        assert!(format_assignment(&Shell::Bash, "K", "a\0b").is_err());
    }

    #[test]
    fn command_matches_formatter_and_env_format() {
        assert_eq!(
            shell_format_assignment("bash".into(), "K".into(), "a b".into()).unwrap(),
            "export K='a b'"
        );
        assert_eq!(
            shell_format_assignment("env".into(), "K".into(), "a b".into()).unwrap(),
            "K='a b'"
        );
        assert!(shell_format_assignment("fish".into(), "K".into(), "v".into()).is_err());
        assert!(shell_format_assignment("env".into(), "A;B".into(), "v".into()).is_err());
    }

    fn available(bin: &str) -> bool {
        Command::new(bin).arg("--version").output().is_ok()
            || Command::new(bin).arg("-c").arg(":").output().is_ok()
    }

    /// Evaluates the assignment in a real shell and returns the value read back.
    fn run_in_shell(bin: &str, shell: &Shell, value: &str, dir: &Path, sentinel: &Path) -> Vec<u8> {
        let assignment = format_assignment(shell, "KEY", value).unwrap();
        let out = dir.join("out.bin");
        let _ = std::fs::remove_file(&out);
        let (script_name, script, args): (&str, String, Vec<&str>) = if *shell == Shell::PowerShell {
            (
                "t.ps1",
                format!(
                    "{assignment}\n[System.IO.File]::WriteAllText('{}', $env:KEY)\n",
                    out.display()
                ),
                vec!["-NoProfile", "-File"],
            )
        } else {
            (
                "t.sh",
                format!("{assignment}\nprintf %s \"$KEY\" > '{}'\n", out.display()),
                vec![],
            )
        };
        let path = dir.join(script_name);
        std::fs::write(&path, script).unwrap();
        let status = Command::new(bin).args(&args).arg(&path).status().unwrap();
        assert!(status.success(), "{bin} failed");
        assert!(!sentinel.exists(), "{bin} executed content from the value");
        std::fs::read(&out).unwrap()
    }

    fn round_trip(bin: &str, shell: Shell) {
        if !available(bin) {
            eprintln!("shellfmt: skipping {bin} (not installed)");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let sentinel = dir.path().join("pwned");
        let mut values = corpus(&sentinel);
        if shell != Shell::Sh {
            values.extend(newline_corpus());
        }
        for v in values {
            let got = run_in_shell(bin, &shell, &v, dir.path(), &sentinel);
            assert_eq!(got, v.as_bytes(), "{bin} round trip differs for a {}-byte value", v.len());
        }
    }

    #[test]
    fn bash_round_trip_and_no_execution() {
        round_trip("bash", Shell::Bash);
    }

    #[test]
    fn zsh_round_trip_and_no_execution() {
        round_trip("zsh", Shell::Zsh);
    }

    #[test]
    fn sh_round_trip_and_no_execution() {
        round_trip("sh", Shell::Sh);
    }

    #[test]
    fn pwsh_round_trip_and_no_execution() {
        round_trip("pwsh", Shell::PowerShell);
    }
}
