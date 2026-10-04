//! Strict `{{param}}` substitution for stored commands.
//!
//! One conservative allowlist on every platform instead of per-shell quoting:
//! POSIX single-quote escaping is sound, but `cmd.exe` quoting (`%VAR%`, `^`,
//! `!` with delayed expansion) is not reliably escapable.

use std::collections::BTreeMap;

use super::ExecError;

/// Maximum accepted length of one parameter value.
pub const MAX_PARAM_LEN: usize = 256;

fn is_allowed_value_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | ':' | '@' | '=' | '+' | ',' | '-')
}

/// `^[A-Za-z0-9._/:@=+,-]{0,256}$`
pub fn validate_param_value(name: &str, value: &str) -> Result<(), ExecError> {
    if value.len() > MAX_PARAM_LEN {
        return Err(ExecError::InvalidParam {
            name: name.to_string(),
            reason: format!("longer than {MAX_PARAM_LEN} characters"),
        });
    }
    if !value.chars().all(is_allowed_value_char) {
        return Err(ExecError::InvalidParam {
            name: name.to_string(),
            reason: "contains characters outside [A-Za-z0-9._/:@=+,-]".to_string(),
        });
    }
    Ok(())
}

/// Placeholder names are the same `[A-Za-z0-9_]+` the template parser accepts.
fn validate_param_name(name: &str) -> Result<(), ExecError> {
    if name.is_empty() || name.len() > 64 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(ExecError::InvalidParam {
            name: name.chars().take(64).collect(),
            reason: "parameter names must match [A-Za-z0-9_]{1,64}".to_string(),
        });
    }
    Ok(())
}

/// Validates every supplied parameter (even ones the template does not use)
/// and substitutes `{{name}}` placeholders. Placeholders without a supplied
/// value are left untouched.
pub fn substitute(template: &str, params: &BTreeMap<String, String>) -> Result<String, ExecError> {
    for (name, value) in params {
        validate_param_name(name)?;
        validate_param_value(name, value)?;
    }
    let mut resolved = template.to_string();
    for (name, value) in params {
        resolved = resolved.replace(&format!("{{{{{name}}}}}"), value);
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(v: &str) -> BTreeMap<String, String> {
        BTreeMap::from([("p".to_string(), v.to_string())])
    }

    #[test]
    fn allowed_charset_passes() {
        for ok in ["", "abc", "a.b_c/d:e@f=g+h,i-j", "prod-1", "https://x.io/a"] {
            assert!(substitute("echo {{p}}", &one(ok)).is_ok(), "{ok:?}");
        }
        assert_eq!(substitute("echo {{p}} {{p}}", &one("a-b")).unwrap(), "echo a-b a-b");
    }

    #[test]
    fn shell_metacharacters_are_rejected() {
        for bad in [
            "; printenv DB_PASSWORD", "$(id)", "`id`", "a b", "%PATH%", "a^b", "a!b", "a&b", "a|b",
            "a>b", "a<b", "a\"b", "a'b", "a\\b", "a\nb", "a*b", "a(b", "a;b", "a$b", "a{b", "é", "a?b",
        ] {
            assert!(substitute("echo {{p}}", &one(bad)).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn too_long_value_is_rejected() {
        assert!(substitute("{{p}}", &one(&"a".repeat(MAX_PARAM_LEN))).is_ok());
        assert!(substitute("{{p}}", &one(&"a".repeat(MAX_PARAM_LEN + 1))).is_err());
    }

    #[test]
    fn unused_params_are_still_validated_and_the_error_names_the_param() {
        let err = substitute("echo hi", &one("a b")).unwrap_err();
        assert!(err.to_string().contains("'p'"), "{err}");
    }

    #[test]
    fn bad_parameter_name_is_rejected() {
        let params = BTreeMap::from([("a b".to_string(), "x".to_string())]);
        assert!(substitute("echo", &params).is_err());
    }
}
