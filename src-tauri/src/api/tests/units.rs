//! Pure-function unit tests for `api::mod` helpers that don't need a router
//! or a `TestVault` — `redact_item`, `validate_create`, `validate_update`,
//! `environment_item_ids`. Phase 1 of the plan.

use super::super::{environment_item_ids, redact_item, validate_create, validate_update};
use crate::project::{Environment, EnvironmentVar};
use crate::vault::VaultItem;

fn full_item() -> VaultItem {
    VaultItem {
        id: 1,
        item_type: "secret".to_string(),
        name: Some("DB_HOST".to_string()),
        value: Some("localhost".to_string()),
        url: Some("https://example.com".to_string()),
        username: Some("user".to_string()),
        password: Some("hunter2".to_string()),
        title: Some("title".to_string()),
        description: Some("desc".to_string()),
        command: Some("echo hi".to_string()),
        shell: Some("bash".to_string()),
        categories: Some(vec!["db".to_string()]),
        notes: Some("some notes".to_string()),
        content: Some("secret content".to_string()),
        created: "0".to_string(),
        is_global: Some(false),
    }
}

fn empty_item(item_type: &str) -> VaultItem {
    VaultItem {
        id: 0,
        item_type: item_type.to_string(),
        name: None,
        value: None,
        url: None,
        username: None,
        password: None,
        title: None,
        description: None,
        command: None,
        shell: None,
        categories: None,
        notes: None,
        content: None,
        created: String::new(),
        is_global: None,
    }
}

// ─── redact_item (3) ────────────────────────────────────────────────────

#[test]
fn redact_item_strips_value() {
    let item = redact_item(full_item());
    assert!(item.value.is_none());
}

#[test]
fn redact_item_strips_password() {
    let item = redact_item(full_item());
    assert!(item.password.is_none());
}

#[test]
fn redact_item_strips_content_but_keeps_other_fields() {
    let item = redact_item(full_item());
    assert!(item.content.is_none());
    // Non-secret fields must survive redaction untouched.
    assert_eq!(item.name.as_deref(), Some("DB_HOST"));
    assert_eq!(item.url.as_deref(), Some("https://example.com"));
    assert_eq!(item.username.as_deref(), Some("user"));
}

// ─── validate_create (8) ────────────────────────────────────────────────

#[test]
fn validate_create_accepts_a_valid_item() {
    let mut item = empty_item("secret");
    item.name = Some("DB_HOST".to_string());
    item.value = Some("localhost".to_string());
    assert!(validate_create(&item).is_ok());
}

#[test]
fn validate_create_rejects_empty_name() {
    let mut item = empty_item("secret");
    item.value = Some("v".to_string());
    assert!(validate_create(&item).is_err());
}

#[test]
fn validate_create_rejects_name_over_255_chars() {
    let mut item = empty_item("secret");
    item.name = Some("a".repeat(256));
    item.value = Some("v".to_string());
    assert!(validate_create(&item).is_err());
}

#[test]
fn validate_create_rejects_empty_type() {
    let mut item = empty_item("");
    item.name = Some("n".to_string());
    item.value = Some("v".to_string());
    assert!(validate_create(&item).is_err());
}

#[test]
fn validate_create_rejects_unknown_type() {
    let mut item = empty_item("not-a-real-type");
    item.name = Some("n".to_string());
    item.value = Some("v".to_string());
    assert!(validate_create(&item).is_err());
}

#[test]
fn validate_create_rejects_missing_value() {
    let mut item = empty_item("secret");
    item.name = Some("n".to_string());
    // value stays None
    assert!(validate_create(&item).is_err());
}

#[test]
fn validate_create_rejects_empty_value() {
    let mut item = empty_item("secret");
    item.name = Some("n".to_string());
    item.value = Some(String::new());
    assert!(validate_create(&item).is_err());
}

#[test]
fn validate_create_rejects_category_over_100_chars() {
    let mut item = empty_item("secret");
    item.name = Some("n".to_string());
    item.value = Some("v".to_string());
    item.categories = Some(vec!["a".repeat(101)]);
    assert!(validate_create(&item).is_err());
}

// ─── validate_update (4) ─────────────────────────────────────────────────

#[test]
fn validate_update_accepts_a_fully_empty_partial_update() {
    let item = empty_item("");
    assert!(validate_update(&item).is_ok());
}

#[test]
fn validate_update_rejects_explicit_empty_name() {
    let mut item = empty_item("");
    item.name = Some(String::new());
    assert!(validate_update(&item).is_err());
}

#[test]
fn validate_update_rejects_unknown_type_when_present() {
    let item = empty_item("not-a-real-type");
    assert!(validate_update(&item).is_err());
}

#[test]
fn validate_update_rejects_explicit_empty_value() {
    let mut item = empty_item("");
    item.value = Some(String::new());
    assert!(validate_update(&item).is_err());
}

// ─── environment_item_ids (2) ────────────────────────────────────────────

fn make_env(vars: Vec<(i64, i64)>) -> Environment {
    Environment {
        id: 1,
        project_id: 1,
        name: "production".to_string(),
        is_default: true,
        paths: vec![],
        vars: vars
            .into_iter()
            .map(|(id, item_id)| EnvironmentVar { id, key: format!("KEY_{id}"), item_id })
            .collect(),
        created: "0".to_string(),
        updated: "0".to_string(),
    }
}

#[test]
fn environment_item_ids_collects_all_linked_items() {
    let env = make_env(vec![(1, 10), (2, 20), (3, 30)]);
    let ids = environment_item_ids(&env);
    assert_eq!(ids.len(), 3);
    assert!(ids.contains(&10) && ids.contains(&20) && ids.contains(&30));
}

#[test]
fn environment_item_ids_is_empty_for_an_environment_with_no_vars() {
    let env = make_env(vec![]);
    assert!(environment_item_ids(&env).is_empty());
}

/// Internal errors must go through `internal_error()` (generic body plus a
/// correlation id); echoing the raw error text in a 500 leaks SQL, schema and
/// file paths.
#[test]
fn no_raw_error_echo_in_internal_error_responses() {
    let sources = [
        ("api/mod.rs", include_str!("../mod.rs")),
        // exec_routes.rs is excluded on purpose: its `EXEC_FAILED` body carries
        // `ExecError` text, which is sanitised by construction (never a path,
        // command or value).
        ("api/auth.rs", include_str!("../auth.rs")),
        ("api/approvals.rs", include_str!("../approvals.rs")),
        ("api/mcp_servers.rs", include_str!("../mcp_servers.rs")),
    ];
    for (name, src) in sources {
        let squashed: String = src.chars().filter(|c| !c.is_whitespace()).collect();
        for needle in [
            "err_json(StatusCode::INTERNAL_SERVER_ERROR,&e",
            "err_json(StatusCode::INTERNAL_SERVER_ERROR,&format!",
        ] {
            assert!(!squashed.contains(needle), "{name} echoes a raw error: {needle}");
        }
    }
}
