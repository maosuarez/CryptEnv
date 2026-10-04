//! Router-level tests for `POST /fill` — the line-preservation guarantee is
//! R3 in the plan: a template key not linked into the scoped environment
//! must survive byte-identical rather than being blanked out.

use crate::test_support::{req, router, unlocked_vault};

async fn fill(v: &crate::test_support::TestVault, app: &axum::Router, template: &str) -> serde_json::Value {
    let uri = format!("/fill?environment_id={}", v.env_id);
    let body = serde_json::json!({ "template": template });
    let (status, json) = req(app, "POST", &uri, Some(&v.token), Some(body)).await;
    assert_eq!(status.as_u16(), 200, "fill failed: {json:?}");
    json
}

#[tokio::test]
async fn key_present_in_environment_is_substituted() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let json = fill(&v, &app, "DB_HOST=old-value\n").await;
    assert_eq!(json.get("content").and_then(|c| c.as_str()), Some("DB_HOST=localhost\n"));
    assert_eq!(json.get("injected").and_then(|n| n.as_i64()), Some(1));
}

#[tokio::test]
async fn key_absent_from_environment_leaves_line_byte_identical() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let json = fill(&v, &app, "UNKNOWN_KEY=some-local-value\n").await;
    assert_eq!(
        json.get("content").and_then(|c| c.as_str()),
        Some("UNKNOWN_KEY=some-local-value\n"),
        "a key not linked into scope must not be blanked out"
    );
    assert_eq!(json.get("not_found").and_then(|n| n.as_i64()), Some(1));
    let missing = json.get("missing_keys").and_then(|m| m.as_array()).unwrap();
    assert_eq!(missing[0].as_str(), Some("UNKNOWN_KEY"));
}

#[tokio::test]
async fn comments_and_blank_lines_are_preserved() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let template = "# a comment\n\nDB_HOST=x\n";
    let json = fill(&v, &app, template).await;
    let content = json.get("content").and_then(|c| c.as_str()).unwrap();
    assert!(content.starts_with("# a comment\n\n"));
}

#[tokio::test]
async fn trailing_newline_presence_is_preserved() {
    let v = unlocked_vault().await;
    let app = router(&v);

    let with_newline = fill(&v, &app, "DB_HOST=x\n").await;
    assert!(with_newline.get("content").and_then(|c| c.as_str()).unwrap().ends_with('\n'));

    let without_newline = fill(&v, &app, "DB_HOST=x").await;
    assert!(!without_newline.get("content").and_then(|c| c.as_str()).unwrap().ends_with('\n'));
}

#[tokio::test]
async fn crlf_input_is_processed_without_corruption() {
    // Current behaviour: `body.template.lines()` (Rust's line splitter)
    // strips both `\r` and `\n` terminators and the output is always
    // rejoined with plain `\n` — CRLF input is normalized to LF output, not
    // preserved byte-for-byte. Pinned here so a future change to preserve
    // CRLF is a deliberate, visible diff.
    let v = unlocked_vault().await;
    let app = router(&v);
    let json = fill(&v, &app, "DB_HOST=old\r\n").await;
    assert_eq!(json.get("content").and_then(|c| c.as_str()), Some("DB_HOST=localhost\n"));
}

#[tokio::test]
async fn duplicate_keys_in_template_are_each_substituted() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let json = fill(&v, &app, "DB_HOST=a\nDB_HOST=b\n").await;
    assert_eq!(json.get("content").and_then(|c| c.as_str()), Some("DB_HOST=localhost\nDB_HOST=localhost\n"));
    assert_eq!(json.get("injected").and_then(|n| n.as_i64()), Some(2));
}

#[tokio::test]
async fn empty_template_input_is_handled() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let json = fill(&v, &app, "").await;
    assert_eq!(json.get("content").and_then(|c| c.as_str()), Some(""));
    assert_eq!(json.get("injected").and_then(|n| n.as_i64()), Some(0));
    assert_eq!(json.get("not_found").and_then(|n| n.as_i64()), Some(0));
}

#[tokio::test]
async fn unresolvable_scope_is_422_before_any_file_is_touched() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let output_path = v.dir.path().join("should-not-exist.env");
    let body = serde_json::json!({
        "template": "DB_HOST=x\n",
        "output_path": output_path.to_str().unwrap(),
    });
    // No environment_id / project / environment at all.
    let (status, json) = req(&app, "POST", "/fill", Some(&v.token), Some(body)).await;
    assert_eq!(status.as_u16(), 422);
    assert_eq!(json.get("code").and_then(|c| c.as_str()), Some("VALIDATION_ERROR"));
    assert!(!output_path.exists(), "scope must be resolved before any file write is attempted");
}

#[cfg(unix)]
#[tokio::test]
async fn fill_to_a_symlink_output_path_is_409_target_symlink() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let dir = tempfile::tempdir().unwrap();
    let victim = dir.path().join("victim.txt");
    std::fs::write(&victim, "keep").unwrap();
    let link = dir.path().join(".env");
    std::os::unix::fs::symlink(&victim, &link).unwrap();

    let uri = format!("/fill?environment_id={}", v.env_id);
    let body = serde_json::json!({ "template": "DB_HOST=x\n", "output_path": link.to_str().unwrap(), "overwrite": true });
    let (status, json) = req(&app, "POST", &uri, Some(&v.token), Some(body)).await;

    assert_eq!(status.as_u16(), 409, "{json:?}");
    assert_eq!(json.get("code").and_then(|c| c.as_str()), Some("TARGET_SYMLINK"));
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
}
