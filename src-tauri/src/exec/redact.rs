//! Output redaction for backend-side command execution.
//!
//! Every injected secret value is replaced with `[REDACTED:<KEY>]`, together
//! with its standard base64 (padded and unpadded), URL-safe base64 (unpadded)
//! and hex (lower and upper case) encodings. Matching runs on the raw captured
//! bytes, before UTF-8 decoding, so a secret is caught even when the child
//! emits invalid UTF-8 around it.
//!
//! Values shorter than [`MIN_REDACTED_LEN`] are not redacted: the false-positive
//! rate on such short strings (`1`, `on`, `dev`) would make the output useless.

use aho_corasick::{AhoCorasick, MatchKind};
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use base64::Engine;
use zeroize::Zeroizing;

pub const MIN_REDACTED_LEN: usize = 4;

pub struct Redactor {
    automaton: Option<AhoCorasick>,
    /// One replacement per pattern, same order as the automaton patterns.
    replacements: Vec<Vec<u8>>,
    /// Longest pattern in bytes; used to trim a possible secret prefix off a
    /// truncated capture.
    longest: usize,
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Redactor {
    pub fn new(secrets: &[(String, Zeroizing<String>)]) -> Redactor {
        let mut patterns: Vec<Zeroizing<Vec<u8>>> = Vec::new();
        let mut replacements: Vec<Vec<u8>> = Vec::new();
        for (key, value) in secrets {
            if value.len() < MIN_REDACTED_LEN {
                continue;
            }
            let raw = value.as_bytes();
            let replacement = format!("[REDACTED:{key}]").into_bytes();
            let forms: [Zeroizing<String>; 6] = [
                Zeroizing::new(value.to_string()),
                Zeroizing::new(STANDARD.encode(raw)),
                Zeroizing::new(STANDARD_NO_PAD.encode(raw)),
                Zeroizing::new(URL_SAFE_NO_PAD.encode(raw)),
                Zeroizing::new(hex_lower(raw)),
                Zeroizing::new(hex_lower(raw).to_uppercase()),
            ];
            for form in forms {
                patterns.push(Zeroizing::new(form.as_bytes().to_vec()));
                replacements.push(replacement.clone());
            }
        }
        let longest = patterns.iter().map(|p| p.len()).max().unwrap_or(0);
        let automaton = if patterns.is_empty() {
            None
        } else {
            AhoCorasick::builder()
                .match_kind(MatchKind::LeftmostLongest)
                .build(patterns.iter().map(|p| p.as_slice()))
                .ok()
        };
        Redactor { automaton, replacements, longest }
    }

    /// Replaces every secret occurrence in `haystack`.
    pub fn redact(&self, haystack: &[u8]) -> Vec<u8> {
        match &self.automaton {
            Some(ac) => {
                let reps: Vec<&[u8]> = self.replacements.iter().map(|r| r.as_slice()).collect();
                ac.replace_all_bytes(haystack, &reps)
            }
            None => haystack.to_vec(),
        }
    }

    /// Bytes to drop from the end of a capture that was cut at the size cap: a
    /// secret straddling the cut would otherwise leave an unredacted prefix.
    pub fn truncation_guard(&self) -> usize {
        self.longest.saturating_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secrets(v: &str) -> Vec<(String, Zeroizing<String>)> {
        vec![("DB_PASSWORD".to_string(), Zeroizing::new(v.to_string()))]
    }

    fn redacted(v: &str, out: &str) -> String {
        String::from_utf8(Redactor::new(&secrets(v)).redact(out.as_bytes())).unwrap()
    }

    #[test]
    fn raw_value_is_redacted() {
        assert_eq!(redacted("hunter2!", "pw=hunter2! end"), "pw=[REDACTED:DB_PASSWORD] end");
    }

    #[test]
    fn base64_standard_and_url_safe_are_redacted() {
        let v = "s3cr3t??>>value";
        for enc in [STANDARD.encode(v), STANDARD_NO_PAD.encode(v), URL_SAFE_NO_PAD.encode(v)] {
            let out = redacted(v, &format!("x {enc} y"));
            assert!(!out.contains(&enc), "{enc} leaked: {out}");
            assert!(out.contains("[REDACTED:DB_PASSWORD]"));
        }
    }

    #[test]
    fn hex_is_redacted() {
        let v = "hunter2!";
        let hex = hex_lower(v.as_bytes());
        let out = redacted(v, &format!("h={hex}"));
        assert_eq!(out, "h=[REDACTED:DB_PASSWORD]");
    }

    #[test]
    fn short_values_are_not_redacted() {
        assert_eq!(redacted("abc", "abc abc"), "abc abc");
    }

    #[test]
    fn invalid_utf8_around_a_secret_is_handled() {
        let r = Redactor::new(&secrets("hunter2!"));
        let mut input = vec![0xff, 0xfe];
        input.extend_from_slice(b"hunter2!");
        input.push(0xff);
        let out = r.redact(&input);
        assert!(String::from_utf8_lossy(&out).contains("[REDACTED:DB_PASSWORD]"));
        assert!(!out.windows(8).any(|w| w == b"hunter2!"));
    }

    #[test]
    fn longest_match_wins_over_a_contained_shorter_secret() {
        let secrets = vec![
            ("A".to_string(), Zeroizing::new("abcd".to_string())),
            ("B".to_string(), Zeroizing::new("abcdefgh".to_string())),
        ];
        let out = Redactor::new(&secrets).redact(b"abcdefgh");
        assert_eq!(out, b"[REDACTED:B]");
    }
}
