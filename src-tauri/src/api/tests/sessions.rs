//! Per-terminal sliding CLI sessions (cli-tui-parity design D4).

use std::time::{Duration, Instant};

use super::super::{session_ttl, SessionStore, MAX_SESSIONS};
use crate::test_support::{req, router, session_token, unlocked_vault};

#[test]
fn session_slides_on_use_and_lapses_when_idle() {
    let mut store = SessionStore::default();
    let t0 = Instant::now();
    let ttl = Duration::from_secs(300);
    store.insert("a".into(), ttl, t0, 0);

    // Used at 4 min → renewed until 9 min.
    assert!(store.touch("a", t0 + Duration::from_secs(240), 0));
    assert!(store.touch("a", t0 + Duration::from_secs(530), 0), "renewed window still open");
    // Idle for a full ttl → gone.
    assert!(!store.touch("a", t0 + Duration::from_secs(530 + 301), 0));
    assert!(!store.touch("b", t0, 0));
}

#[test]
fn sessions_coexist_and_are_bounded() {
    let mut store = SessionStore::default();
    let t0 = Instant::now();
    let ttl = Duration::from_secs(300);
    store.insert("first".into(), ttl, t0, 0);
    store.insert("second".into(), ttl, t0 + Duration::from_secs(1), 0);
    assert!(store.touch("first", t0 + Duration::from_secs(2), 0));
    assert!(store.touch("second", t0 + Duration::from_secs(2), 0));

    for i in 0..MAX_SESSIONS {
        store.insert(format!("t{i}"), ttl, t0 + Duration::from_secs(10 + i as u64), 0);
    }
    assert_eq!(store.entries.len(), MAX_SESSIONS);
    assert!(!store.touch("first", t0 + Duration::from_secs(100), 0), "least recently used evicted");
}

#[test]
fn never_auto_lock_still_bounds_sessions() {
    assert_eq!(session_ttl(0), Duration::from_secs(300));
    assert_eq!(session_ttl(15), Duration::from_secs(900));
}

#[tokio::test]
async fn two_unlocks_do_not_invalidate_each_other() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let a = session_token(&app, &v).await;
    let b = session_token(&app, &v).await;
    assert_ne!(a, b);
    for t in [&a, &b] {
        let (status, _) = req(&app, "GET", "/projects", Some(t), None).await;
        assert_eq!(status, axum::http::StatusCode::OK);
    }
    let (status, _) = req(&app, "GET", "/projects", Some("not-a-session"), None).await;
    assert_eq!(status, axum::http::StatusCode::UNAUTHORIZED);
}

#[test]
fn stale_epoch_session_is_not_renewed_and_is_removed() {
    let mut store = SessionStore::default();
    let t0 = Instant::now();
    let ttl = Duration::from_secs(300);
    store.insert("old".into(), ttl, t0, 1);
    store.insert("new".into(), ttl, t0, 2);

    assert!(!store.touch("old", t0 + Duration::from_secs(1), 2));
    assert_eq!(store.entries.len(), 1, "stale entry dropped");
    assert!(store.touch("new", t0 + Duration::from_secs(1), 2));
}

#[tokio::test]
async fn lock_does_not_renew_and_unlock_rejects_old_token() {
    use axum::http::StatusCode;
    let v = unlocked_vault().await;
    let app = router(&v);
    let t = session_token(&app, &v).await;
    let (status, _) = req(&app, "GET", "/projects", Some(&t), None).await;
    assert_eq!(status, StatusCode::OK);

    v.state.lock().await.set_key(None);
    let (status, _) = req(&app, "GET", "/projects", Some(&t), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Unlock again through the API: the pre-lock token is dead.
    let fresh = session_token(&app, &v).await;
    let (status, _) = req(&app, "GET", "/projects", Some(&t), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = req(&app, "GET", "/projects", Some(&fresh), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[test]
fn session_ttl_never_overflows() {
    assert_eq!(session_ttl(u64::MAX), Duration::from_secs(1440 * 60));
    let mut store = SessionStore::default();
    let t0 = Instant::now();
    store.insert("a".into(), session_ttl(u64::MAX), t0, 0);
    assert!(store.touch("a", t0, 0));
    // A ttl that cannot be added to an Instant yields an expired session, not a panic.
    store.insert("b".into(), Duration::MAX, t0, 0);
    assert!(!store.touch("b", t0, 0));
}

#[tokio::test]
async fn malformed_unlock_requests_do_not_lock_out_the_cli() {
    let v = unlocked_vault().await;
    let app = router(&v);
    for _ in 0..100 {
        let (status, _) =
            req(&app, "POST", "/unlock", None, Some(serde_json::json!({ "nope": 1 }))).await;
        assert!(status.is_client_error());
    }
    // Still immediately usable with the right password.
    let _ = session_token(&app, &v).await;
}

#[tokio::test]
async fn wrong_password_throttles_the_next_attempt_until_it_expires() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let wrong = serde_json::json!({ "master_password": "not-the-password" });
    let (status, _) = req(&app, "POST", "/unlock", None, Some(wrong)).await;
    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);

    // Even the right password waits out the backoff.
    let right = serde_json::json!({ "master_password": v.master_password });
    let (status, body) = req(&app, "POST", "/unlock", None, Some(right.clone())).await;
    assert_eq!(status, axum::http::StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["code"], "RATE_LIMITED");

    // The GUI path shares the throttle.
    let gui = crate::vault::unlock::unlock_with_password(
        &v.state, v.master_password.as_bytes(), true, false,
    )
    .await;
    assert!(matches!(gui, Err(crate::vault::unlock::UnlockError::Throttled(_))));

    // After the backoff a correct password succeeds and resets everything.
    let throttle = v.state.lock().await.throttle.clone();
    throttle.record_success();
    let (status, _) = req(&app, "POST", "/unlock", None, Some(right)).await;
    assert_eq!(status, axum::http::StatusCode::OK);
}

#[tokio::test]
async fn out_of_range_auto_lock_is_rejected_and_nothing_changes() {
    let v = unlocked_vault().await;
    let app = router(&v);
    let (status, before) = req(&app, "GET", "/settings", Some(&v.token), None).await;
    assert_eq!(status, axum::http::StatusCode::OK);

    for bad in [serde_json::json!(i64::MAX), serde_json::json!(-1), serde_json::json!(1441)] {
        let (status, _) = req(
            &app, "PUT", "/settings", Some(&v.token),
            Some(serde_json::json!({ "auto_lock_timeout": bad })),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
    }
    let (_, after) = req(&app, "GET", "/settings", Some(&v.token), None).await;
    assert_eq!(before["auto_lock_timeout"], after["auto_lock_timeout"]);

    for ok in [0, 1, 1440] {
        let (status, _) = req(
            &app, "PUT", "/settings", Some(&v.token),
            Some(serde_json::json!({ "auto_lock_timeout": ok })),
        )
        .await;
        assert_eq!(status, axum::http::StatusCode::OK);
    }
    // CLI sessions keep working afterwards.
    let _ = session_token(&app, &v).await;
}

#[tokio::test]
async fn stored_out_of_range_auto_lock_is_clamped_and_unlock_still_works() {
    let v = unlocked_vault().await;
    v.state
        .lock()
        .await
        .db
        .set_setting("auto_lock_timeout", "9223372036854775807")
        .await
        .unwrap();
    let app = router(&v);
    let (_, settings) = req(&app, "GET", "/settings", Some(&v.token), None).await;
    assert_eq!(settings["auto_lock_timeout"], 1440);
    // Previously `minutes * 60` wrapped and `Instant + ttl` panicked here.
    let t = session_token(&app, &v).await;
    let (status, _) = req(&app, "GET", "/projects", Some(&t), None).await;
    assert_eq!(status, axum::http::StatusCode::OK);
}
