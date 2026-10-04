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
