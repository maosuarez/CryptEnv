//! Per-terminal sliding CLI sessions (cli-tui-parity design D4).

use std::time::{Duration, Instant};

use super::super::{session_ttl, SessionStore, MAX_SESSIONS};
use crate::test_support::{req, router, session_token, unlocked_vault};

#[test]
fn session_slides_on_use_and_lapses_when_idle() {
    let mut store = SessionStore::default();
    let t0 = Instant::now();
    let ttl = Duration::from_secs(300);
    store.insert("a".into(), ttl, t0);

    // Used at 4 min → renewed until 9 min.
    assert!(store.touch("a", t0 + Duration::from_secs(240)));
    assert!(store.touch("a", t0 + Duration::from_secs(530)), "renewed window still open");
    // Idle for a full ttl → gone.
    assert!(!store.touch("a", t0 + Duration::from_secs(530 + 301)));
    assert!(!store.touch("b", t0));
}

#[test]
fn sessions_coexist_and_are_bounded() {
    let mut store = SessionStore::default();
    let t0 = Instant::now();
    let ttl = Duration::from_secs(300);
    store.insert("first".into(), ttl, t0);
    store.insert("second".into(), ttl, t0 + Duration::from_secs(1));
    assert!(store.touch("first", t0 + Duration::from_secs(2)));
    assert!(store.touch("second", t0 + Duration::from_secs(2)));

    for i in 0..MAX_SESSIONS {
        store.insert(format!("t{i}"), ttl, t0 + Duration::from_secs(10 + i as u64));
    }
    assert_eq!(store.entries.len(), MAX_SESSIONS);
    assert!(!store.touch("first", t0 + Duration::from_secs(100)), "least recently used evicted");
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
