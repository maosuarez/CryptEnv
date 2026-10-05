//! `vault_changed` is signalled only after an authenticated, successful write.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::test_support::{req, router, unlocked_vault, TestVault};

fn count_signals(v: &TestVault) -> Arc<AtomicUsize> {
    let counter = Arc::new(AtomicUsize::new(0));
    let c = counter.clone();
    v.api.set_change_notifier(move || {
        c.fetch_add(1, Ordering::SeqCst);
    });
    counter
}

fn new_item_body() -> serde_json::Value {
    serde_json::json!({ "type": "secret", "name": "SYNC_TEST", "value": "s3cret" })
}

#[tokio::test]
async fn successful_write_signals_once() {
    let v = unlocked_vault().await;
    let signals = count_signals(&v);
    let app = router(&v);
    let uri = format!("/items?environment_id={}", v.env_id);
    let (status, _) = req(&app, "POST", &uri, Some(&v.token), Some(new_item_body())).await;
    assert_eq!(status.as_u16(), 201);
    assert_eq!(signals.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unauthenticated_write_does_not_signal() {
    let v = unlocked_vault().await;
    let signals = count_signals(&v);
    let app = router(&v);
    let uri = format!("/items?environment_id={}", v.env_id);
    let (status, _) = req(&app, "POST", &uri, None, Some(new_item_body())).await;
    assert_eq!(status.as_u16(), 401);
    assert_eq!(signals.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn failed_write_does_not_signal() {
    let v = unlocked_vault().await;
    let signals = count_signals(&v);
    let app = router(&v);
    let uri = format!("/items?environment_id={}", v.env_id);
    let (status, _) = req(&app, "POST", &uri, Some(&v.token), Some(serde_json::json!({ "type": "secret", "value": "v" }))).await;
    assert_eq!(status.as_u16(), 422);
    assert_eq!(signals.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn policy_denied_write_does_not_signal() {
    let v = unlocked_vault().await;
    let signals = count_signals(&v);
    let app = router(&v);
    let body = serde_json::json!({ "id": 0, "name": "x", "template": "generic", "categories": [] });
    let (status, _) = req(&app, "POST", "/projects", Some(&v.mcp_token), Some(body)).await;
    assert_eq!(status.as_u16(), 403);
    assert_eq!(signals.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn reads_do_not_signal() {
    let v = unlocked_vault().await;
    let signals = count_signals(&v);
    let app = router(&v);
    let (status, _) = req(&app, "GET", "/projects", Some(&v.token), None).await;
    assert_eq!(status.as_u16(), 200);
    assert_eq!(signals.load(Ordering::SeqCst), 0);
}

#[test]
fn every_signalling_route_exists_in_the_route_table() {
    for (method, path) in super::super::changes::CHANGING_ROUTES {
        assert!(
            super::super::api_routes().iter().any(|(m, p, _)| m == method && p == path),
            "{method} {path} is not a route"
        );
    }
}
