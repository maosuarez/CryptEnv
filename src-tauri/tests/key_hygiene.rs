// Regression guard for the `key-material-handling` spec: outside `src/crypto/`,
// production code must not declare a raw 32-byte key type nor copy a zeroizing
// key out with `**key` / `**k`. Keys travel as `VaultKey` (see
// `src/crypto/mod.rs`).
//
// Crude on purpose (line-based regexes): it only has to make the easy mistake
// loud. Scanning stops at a file's first `#[cfg(test)]`, and `tests` / test
// support files are skipped.
//
// Allowlist: a line that carries `// key-hygiene: not-a-key` (salts, digests).
// `Zeroizing<[u8; 32]>` is accepted: it already wipes on drop.

use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

const ALLOW_MARKER: &str = "key-hygiene: not-a-key";

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Test-only sources that are exempt from the scan.
fn is_exempt(path: &Path) -> bool {
    let p = path.to_string_lossy().replace('\\', "/");
    p.contains("/src/crypto/")
        || p.contains("/src/test_support/")
        || p.contains("/src/api/tests/")
        || p.ends_with("/testing.rs")
}

/// Returns `line_no: reason` for each violation in `source`.
fn scan(source: &str) -> Vec<String> {
    let raw_array = Regex::new(r"\[u8;\s*32\]").expect("valid regex");
    let key_deref = Regex::new(r"\*\*(key|k)\b").expect("valid regex");
    let mut found = Vec::new();

    for (i, line) in source.lines().enumerate() {
        if line.contains("#[cfg(test)]") {
            break;
        }
        let code = line.trim_start();
        if code.starts_with("//") || line.contains(ALLOW_MARKER) {
            continue;
        }
        let n = i + 1;
        for m in raw_array.find_iter(line) {
            if !line[..m.start()].ends_with("Zeroizing<") {
                found.push(format!(
                    "{n}: raw `[u8; 32]` key type; use `crypto::VaultKey` (or mark `// {ALLOW_MARKER}` for salts/digests)"
                ));
            }
        }
        if key_deref.is_match(line) {
            found.push(format!(
                "{n}: `**key` / `**k` copies the key out of its container; clone the `VaultKey` instead"
            ));
        }
    }
    found
}

#[test]
fn no_raw_key_arrays_or_key_derefs_outside_crypto() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs(&root, &mut files);
    assert!(!files.is_empty(), "no sources found under {}", root.display());

    let mut violations = Vec::new();
    for file in files.iter().filter(|f| !is_exempt(f)) {
        let Ok(source) = fs::read_to_string(file) else { continue };
        for v in scan(&source) {
            violations.push(format!("{}:{v}", file.display()));
        }
    }
    assert!(
        violations.is_empty(),
        "key material hygiene violations (see openspec key-material-zeroization):\n{}",
        violations.join("\n")
    );
}

#[test]
fn guard_flags_a_raw_key_signature() {
    let hits = scan("fn f(key: [u8; 32]) {}\n");
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].contains("VaultKey"));
    assert_eq!(scan("fn f(key: &[u8;32]) {}").len(), 1);
}

#[test]
fn guard_flags_key_derefs() {
    assert_eq!(scan("let v = **key;").len(), 1);
    assert_eq!(scan("Some(k) => **k,").len(), 1);
    assert!(scan("let keys = **keys;").is_empty());
}

#[test]
fn guard_accepts_zeroizing_comments_markers_and_test_modules() {
    assert!(scan("shared_key: Zeroizing<[u8; 32]>,").is_empty());
    assert!(scan("// fn f(key: [u8; 32])").is_empty());
    assert!(scan("let salt: [u8; 32] = d.into(); // key-hygiene: not-a-key (salt)").is_empty());
    assert!(scan("#[cfg(test)]\nmod t { fn f(k: [u8; 32]) { let _ = **k; } }").is_empty());
}
