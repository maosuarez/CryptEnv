//! Router-level tests for `/items` CRUD + `/items/:id/reveal`.

use crate::test_support::{read_item, req, router, unlocked_vault};

#[tokio::test]
async fn list_scoped_to_environment_only_returns_linked_items() {
    let v = unlocked_vault().await;
    let app = router(&v);
    // `include_global=false` is the exact-linked set — the pre-issue-#13
    // filter, and what /fill and /inject will actually materialize. The
    // default (`true`) now unions unlinked globals in; that union is covered
    // by `api::scope_tests` and the #13 cases below.
    let (status, json) = req(&app, "GET", &format!("/items?environment_id={}&include_global=false", v.env_id), Some(&v.token), None).await;
    assert_eq!(status.as_u16(), 200);
    let items = json.as_array().unwrap();
    // 3 linked into "production"; the 4th seeded item (SHARED_TOKEN, global)
    // is not linked into any environment var, so it must not appear here.
    assert_eq!(items.len(), 3);
    assert!(items.iter().all(|i| i.get("name").and_then(|n| n.as_str()) != Some("SHARED_TOKEN")));
}

#[tokio::test]
async fn list_never_returns_secret_fields() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let (_, json) = req(&app, "GET", &format!("/items?environment_id={}", v.env_id), Some(&v.token), None).await;
    for item in json.as_array().unwrap() {
        assert!(item.get("value").is_none(), "value must never be present on the wire");
        assert!(item.get("password").is_none(), "password must never be present on the wire");
        assert!(item.get("content").is_none(), "content must never be present on the wire");
    }
}

#[tokio::test]
async fn list_search_filters_within_scope() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let uri = format!("/items?environment_id={}&search=db_host", v.env_id);
    let (status, json) = req(&app, "GET", &uri, Some(&v.token), None).await;
    assert_eq!(status.as_u16(), 200);
    let items = json.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].get("name").and_then(|n| n.as_str()), Some("DB_HOST"));
}

#[tokio::test]
async fn get_item_nonexistent_returns_404() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let (status, json) = req(&app, "GET", "/items/999999", Some(&v.token), None).await;
    assert_eq!(status.as_u16(), 404);
    assert_eq!(json.get("code").and_then(|c| c.as_str()), Some("NOT_FOUND"));
}

#[tokio::test]
async fn create_item_persists_encrypted_blob_not_plaintext() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let body = serde_json::json!({
        "type": "secret",
        "name": "NEW_SECRET",
        "value": "super-secret-plaintext",
    });
    let uri = format!("/items?environment_id={}", v.env_id);
    let (status, json) = req(&app, "POST", &uri, Some(&v.token), Some(body)).await;
    assert_eq!(status.as_u16(), 201);
    assert!(json.get("value").is_none(), "response must not echo the plaintext value");

    let id = json.get("id").and_then(|i| i.as_i64()).expect("created item has an id");
    let decrypted = read_item(&v, id).await;
    assert_eq!(decrypted.value.as_deref(), Some("super-secret-plaintext"));

    // The raw DB column must never contain the plaintext.
    let raw_state = v.state.lock().await;
    let raw = raw_state.db.list_items().await.unwrap();
    let (_, _, data, _, _) = raw.into_iter().find(|(rid, ..)| *rid == id).unwrap();
    assert!(!data.contains("super-secret-plaintext"), "ciphertext must not contain the plaintext value");
}

#[tokio::test]
async fn create_item_missing_name_is_422_naming_the_field() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let body = serde_json::json!({
        "type": "secret",
        "value": "v",
    });
    let uri = format!("/items?environment_id={}", v.env_id);
    let (status, json) = req(&app, "POST", &uri, Some(&v.token), Some(body)).await;
    assert_eq!(status.as_u16(), 422);
    assert_eq!(json.get("code").and_then(|c| c.as_str()), Some("VALIDATION_ERROR"));
    let msg = json.get("error").and_then(|e| e.as_str()).unwrap_or_default();
    assert!(msg.starts_with("name:"), "error must name the offending field: {msg}");
}

#[tokio::test]
async fn update_item_partial_update_preserves_other_fields() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let id = v.item_ids[0]; // DB_HOST

    // `type` has no `#[serde(default)]` on `VaultItem`, so it must be resent
    // even on a "partial" update — otherwise axum's Json extractor itself
    // rejects the body (422) before `validate_update` ever runs.
    let body = serde_json::json!({ "type": "secret", "value": "updated-host" });
    let (status, _) = req(&app, "PUT", &format!("/items/{id}"), Some(&v.token), Some(body)).await;
    assert_eq!(status.as_u16(), 200);

    let decrypted = read_item(&v, id).await;
    assert_eq!(decrypted.value.as_deref(), Some("updated-host"));
    assert_eq!(decrypted.name.as_deref(), Some("DB_HOST"), "unrelated field must survive a partial update");
}

#[tokio::test]
async fn delete_item_unlinks_and_removes() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let id = v.item_ids[0]; // DB_HOST, linked into v.env_id

    let (status, _) = req(&app, "DELETE", &format!("/items/{id}"), Some(&v.token), None).await;
    assert_eq!(status.as_u16(), 204);

    let state = v.state.lock().await;
    let raw = state.db.list_items().await.unwrap();
    assert!(raw.iter().find(|(rid, ..)| *rid == id).is_none(), "item row must be gone");
    let vars = state.db.get_environment_vars(v.env_id).await.unwrap();
    assert!(vars.iter().find(|var| var.item_id == Some(id)).is_none(), "environment_vars link must be gone too");
}

#[tokio::test]
async fn reveal_item_is_the_only_endpoint_returning_plaintext() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let id = v.item_ids[1]; // DB_PASSWORD = "hunter2"

    let (status, json) = req(
        &app,
        "POST",
        &format!("/items/{id}/reveal"),
        Some(&v.token),
        Some(serde_json::json!({ "confirm": true })),
    )
    .await;
    assert_eq!(status.as_u16(), 200);
    assert_eq!(json.get("value").and_then(|v| v.as_str()), Some("hunter2"));

    // The same item via GET /items/:id must still be redacted.
    let (_, get_json) = req(&app, "GET", &format!("/items/{id}"), Some(&v.token), None).await;
    assert!(get_json.get("value").is_none());
}

#[tokio::test]
async fn reveal_item_nonexistent_returns_404() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let (status, _) = req(
        &app,
        "POST",
        "/items/999999/reveal",
        Some(&v.token),
        Some(serde_json::json!({ "confirm": true })),
    )
    .await;
    assert_eq!(status.as_u16(), 404);
}

#[tokio::test]
async fn concurrent_updates_to_different_fields_are_both_kept() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let id = v.item_ids[0];
    let uri = format!("/items/{id}");

    let value_body = serde_json::json!({ "type": "secret", "value": "new-value" });
    let desc_body = serde_json::json!({ "type": "secret", "description": "new-description" });
    let (a, b) = tokio::join!(
        req(&app, "PUT", &uri, Some(&v.token), Some(value_body)),
        req(&app, "PUT", &uri, Some(&v.token), Some(desc_body)),
    );
    assert_eq!(a.0.as_u16(), 200);
    assert_eq!(b.0.as_u16(), 200);

    let stored = read_item(&v, id).await;
    assert_eq!(stored.value.as_deref(), Some("new-value"));
    assert_eq!(stored.description.as_deref(), Some("new-description"));
}
