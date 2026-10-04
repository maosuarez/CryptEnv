//! Availability guarantees around the master password (vault-lock-and-runtime-
//! availability): the Argon2 derivation never runs on an async worker thread
//! and never while the vault lock is held, failed unlocks are throttled fairly,
//! and `auto_lock_timeout` is range-checked.
//!
//! Unlock and change-password are two-phase around the derivation:
//! 1. (lock) snapshot the key material and the lock epoch, release;
//! 2. (`spawn_blocking`, no lock) derive and verify;
//! 3. (lock) re-check that nothing changed in between (epoch + key material),
//!    then commit. A lock, reset, restore or re-key during phase 2 aborts the
//!    commit instead of resurrecting a stale key.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, OwnedMutexGuard};
use zeroize::Zeroizing;

use super::{
    decrypt_item, migrate_literal_vars_to_items, Category, SharedState, UnlockPayload, VaultItem,
};
use crate::biometric::{self, wrap, HelloError, HelloSigner};
use crate::crypto::{self, VaultKey};

// ─── auto_lock_timeout ────────────────────────────────────────────────────────

/// Largest accepted `auto_lock_timeout`, in minutes (24 h).
pub const AUTO_LOCK_MAX_MINUTES: u64 = 1440;
/// Used when nothing (or something unparsable / negative) is stored.
pub const DEFAULT_AUTO_LOCK_MINUTES: u64 = 5;

/// Accepts `0` ("never auto-lock") or `1..=1440` minutes.
pub fn validate_auto_lock(minutes: i64) -> Result<u64, String> {
    match u64::try_from(minutes) {
        Ok(m) if m <= AUTO_LOCK_MAX_MINUTES => Ok(m),
        _ => Err(format!(
            "auto_lock_timeout must be 0 (never) or between 1 and {AUTO_LOCK_MAX_MINUTES} minutes"
        )),
    }
}

/// Timeout in minutes to act on for a stored setting. Values written by
/// earlier versions that are out of range are clamped instead of trusted, so
/// a bad stored value can neither overflow nor disable the lock by accident.
pub fn effective_auto_lock(stored: Option<&str>) -> u64 {
    match stored.and_then(|v| v.trim().parse::<i64>().ok()) {
        Some(v) => u64::try_from(v)
            .map(|m| m.min(AUTO_LOCK_MAX_MINUTES))
            .unwrap_or(DEFAULT_AUTO_LOCK_MINUTES),
        None => DEFAULT_AUTO_LOCK_MINUTES,
    }
}

// ─── Unlock throttle ──────────────────────────────────────────────────────────

const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Delay after the `failures`-th consecutive failure: 1 s doubling to 60 s.
fn backoff(failures: u32) -> Duration {
    let exp = failures.saturating_sub(1).min(6);
    Duration::from_secs(1u64 << exp).min(MAX_BACKOFF)
}

#[derive(Default)]
struct ThrottleState {
    failures: u32,
    next_allowed: Option<Instant>,
}

/// Penalty for wrong master passwords, shared by the REST `/unlock` and the
/// GUI unlock so neither path bypasses the other. Only failed password
/// verifications count; a success resets it. Password attempts are also
/// serialized (the `gate`) so concurrent guesses cannot all pass the check
/// before the first failure is recorded.
#[derive(Default)]
pub struct UnlockThrottle {
    state: StdMutex<ThrottleState>,
    gate: Arc<Mutex<()>>,
}

impl UnlockThrottle {
    fn lock_state(&self) -> std::sync::MutexGuard<'_, ThrottleState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `Err(wait)` while the backoff after the last failure is still running.
    pub fn check(&self, now: Instant) -> Result<(), Duration> {
        match self.lock_state().next_allowed {
            Some(t) if now < t => Err(t - now),
            _ => Ok(()),
        }
    }

    pub fn record_failure(&self, now: Instant) {
        let mut st = self.lock_state();
        st.failures = st.failures.saturating_add(1);
        st.next_allowed = Some(now + backoff(st.failures));
    }

    pub fn record_success(&self) {
        *self.lock_state() = ThrottleState::default();
    }

    /// Starts a password attempt: waits for any attempt in flight, then
    /// refuses with the remaining wait while throttled. Unless the caller
    /// ends the attempt with `succeeded` or `not_counted`, dropping it counts
    /// as a failure, so a client that disconnects mid-derivation cannot dodge
    /// the penalty.
    pub async fn begin(self: &Arc<Self>) -> Result<Attempt, Duration> {
        let gate = self.gate.clone().lock_owned().await;
        self.check(Instant::now())?;
        Ok(Attempt { throttle: self.clone(), counts_as_failure: true, _gate: gate })
    }
}

pub struct Attempt {
    throttle: Arc<UnlockThrottle>,
    counts_as_failure: bool,
    _gate: OwnedMutexGuard<()>,
}

impl Attempt {
    /// The password was verified: reset the throttle.
    pub fn succeeded(mut self) {
        self.counts_as_failure = false;
        self.throttle.record_success();
    }

    /// Ended for a reason other than a wrong password (vault not initialized,
    /// storage error, ...): leaves the throttle untouched.
    pub fn not_counted(mut self) {
        self.counts_as_failure = false;
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        if self.counts_as_failure {
            self.throttle.record_failure(Instant::now());
        }
    }
}

// ─── Two-phase unlock ─────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum UnlockError {
    /// Too many recent failures; retry after this long.
    Throttled(Duration),
    /// No vault has been created yet (and the caller does not create one).
    NotInitialized,
    IncorrectPassword,
    /// The vault was locked, reset, restored or re-keyed while the key was
    /// being derived; nothing was committed.
    Aborted,
    Other(String),
}

impl UnlockError {
    /// Text for the GUI. Never contains key material or the password.
    pub fn message(&self) -> String {
        match self {
            UnlockError::Throttled(wait) => format!(
                "too many failed attempts, try again in {} s",
                wait.as_secs().max(1)
            ),
            UnlockError::NotInitialized => "vault not initialized".to_string(),
            UnlockError::IncorrectPassword => crypto::INCORRECT_PASSWORD.to_string(),
            UnlockError::Aborted => "the vault changed while unlocking, try again".to_string(),
            UnlockError::Other(e) => e.clone(),
        }
    }
}

/// Result of the derivation phase.
pub struct Derived {
    key: VaultKey,
    /// Salt and verify token of a brand-new vault (first-run setup).
    new_meta: Option<(String, String)>,
}

pub struct UnlockOutcome {
    /// Lock epoch the key was committed in.
    pub epoch: u64,
    pub payload: Option<UnlockPayload>,
}

/// Production derivation: verifies `password` against `meta`, or creates the
/// key material of a new vault when there is none.
fn derive_for_unlock(
    password: &[u8],
    meta: Option<(String, String)>,
) -> Result<Derived, UnlockError> {
    match meta {
        Some((salt, token)) => match crypto::unlock_vault_crypto(password, &salt, &token) {
            Ok(key) => Ok(Derived { key, new_meta: None }),
            Err(e) if e == crypto::INCORRECT_PASSWORD => Err(UnlockError::IncorrectPassword),
            Err(e) => Err(UnlockError::Other(e)),
        },
        None => {
            let (salt, token, key) =
                crypto::init_vault_crypto(password).map_err(UnlockError::Other)?;
            Ok(Derived { key, new_meta: Some((salt, token)) })
        }
    }
}

/// Unlocks (or, with `allow_init`, first-run initializes) the vault with
/// `password`. `want_payload` additionally loads and decrypts all items and
/// categories for the GUI.
pub async fn unlock_with_password(
    shared: &SharedState,
    password: &[u8],
    allow_init: bool,
    want_payload: bool,
) -> Result<UnlockOutcome, UnlockError> {
    let password = Zeroizing::new(password.to_vec());
    unlock_with_kdf(shared, allow_init, want_payload, move |meta| {
        derive_for_unlock(&password, meta)
    })
    .await
}

/// Unlocks with Windows Hello: the stored challenge is signed (Hello prompt),
/// the vault key unwrapped and verified against the verify token. Runs as the
/// derivation phase of the shared unlock, so the epoch re-check and the final
/// `set_key` commit are the same as for a password unlock. A cancelled prompt
/// or a failed unwrap is not a wrong password and does not count against the
/// password throttle.
pub async fn unlock_with_biometric(
    shared: &SharedState,
    signer: Arc<dyn HelloSigner>,
    want_payload: bool,
) -> Result<UnlockOutcome, UnlockError> {
    let blob = shared
        .lock()
        .await
        .db
        .get_setting(biometric::BLOB_SETTING)
        .await
        .map_err(UnlockError::Other)?
        .filter(|v| !v.is_empty())
        .ok_or_else(|| UnlockError::Other("biometric unlock is not enrolled".to_string()))?;
    let enrollment = wrap::Enrollment::parse(&blob).ok_or_else(|| {
        UnlockError::Other(HelloError::Unusable.message())
    })?;

    unlock_with_kdf(shared, false, want_payload, move |meta| {
        let (_, token) = meta.ok_or(UnlockError::NotInitialized)?;
        let key = wrap::unlock_key(&*signer, &enrollment, &token)
            .map_err(|e| UnlockError::Other(e.message()))?;
        Ok(Derived { key, new_meta: None })
    })
    .await
}

/// [`unlock_with_password`] with the derivation injected (tests use a slow one).
async fn unlock_with_kdf<F>(
    shared: &SharedState,
    allow_init: bool,
    want_payload: bool,
    kdf: F,
) -> Result<UnlockOutcome, UnlockError>
where
    F: FnOnce(Option<(String, String)>) -> Result<Derived, UnlockError> + Send + 'static,
{
    let throttle = shared.lock().await.throttle.clone();
    let attempt = throttle.begin().await.map_err(UnlockError::Throttled)?;

    // Phase 1: snapshot under the lock.
    let (meta, epoch) = {
        let s = shared.lock().await;
        match s.db.get_meta().await {
            Ok(meta) => (meta, s.epoch),
            Err(e) => {
                attempt.not_counted();
                return Err(UnlockError::Other(e));
            }
        }
    };
    if meta.is_none() && !allow_init {
        attempt.not_counted();
        return Err(UnlockError::NotInitialized);
    }

    // Phase 2: derive off the runtime and off the lock.
    let snapshot = meta.clone();
    let derived = match tokio::task::spawn_blocking(move || kdf(snapshot)).await {
        Ok(Ok(d)) => d,
        Ok(Err(UnlockError::IncorrectPassword)) => {
            // Dropping `attempt` records the failure.
            return Err(UnlockError::IncorrectPassword);
        }
        Ok(Err(e)) => {
            attempt.not_counted();
            return Err(e);
        }
        Err(_) => {
            attempt.not_counted();
            return Err(UnlockError::Other("key derivation task failed".to_string()));
        }
    };
    if meta.is_some() {
        attempt.succeeded();
    } else {
        attempt.not_counted();
    }

    // Phase 3: commit, if the vault is still what phase 1 saw.
    let mut s = shared.lock().await;
    if s.epoch != epoch {
        return Err(UnlockError::Aborted);
    }
    let current_meta = s.db.get_meta().await.map_err(UnlockError::Other)?;
    if current_meta != meta {
        return Err(UnlockError::Aborted);
    }
    if let Some((salt, token)) = &derived.new_meta {
        s.db.init_vault(salt, token).await.map_err(UnlockError::Other)?;
    }

    // The key is published to `VaultState` only after every fallible step has
    // succeeded: REST/MCP treat `key.is_some()` as "unlocked", so a failure
    // below must leave the vault locked.
    let key: &VaultKey = &derived.key;
    migrate_literal_vars_to_items(&s.db, key).await.map_err(UnlockError::Other)?;

    let payload = if want_payload {
        Some(load_payload(&s.db, key).await.map_err(UnlockError::Other)?)
    } else {
        None
    };

    s.set_key(Some(derived.key));
    s.touch();
    Ok(UnlockOutcome { epoch: s.epoch, payload })
}

async fn load_payload(db: &crate::db::VaultDb, key: &VaultKey) -> Result<UnlockPayload, String> {
    let raw = db.list_items().await?;
    let items: Vec<VaultItem> = raw
        .into_iter()
        .filter_map(|(id, _, data, _, is_global)| decrypt_item(key, id, &data, is_global).ok())
        .collect();
    let categories = db
        .list_categories()
        .await?
        .into_iter()
        .map(|c| Category {
            id: c.cid,
            name: c.name,
            color: c.color,
            description: c.description,
        })
        .collect();
    Ok(UnlockPayload { items, categories })
}

// ─── Password checks and change-password off the lock ────────────────────────

/// Verifies `password` against the stored key material without holding the
/// vault lock during the derivation. Does not touch the unlock throttle (the
/// caller is already authenticated or is an enrolment flow).
pub async fn verify_password(shared: &SharedState, password: &[u8]) -> Result<(), String> {
    let (salt, token) = {
        let s = shared.lock().await;
        s.db.get_meta().await?.ok_or_else(|| "vault not initialized".to_string())?
    };
    let password = Zeroizing::new(password.to_vec());
    tokio::task::spawn_blocking(move || {
        // The derived key is dropped (and wiped) right away.
        crypto::unlock_vault_crypto(&password, &salt, &token).map(|_| ())
    })
    .await
    .map_err(|_| "key derivation task failed".to_string())?
}

/// Output of the change-password derivation phase.
pub struct Rekey {
    old_key: VaultKey,
    new_salt: String,
    new_token: String,
    new_key: VaultKey,
}

fn derive_for_rekey(
    current: &[u8],
    new: &[u8],
    meta: (String, String),
) -> Result<Rekey, String> {
    let (salt, token) = meta;
    let old_key = crypto::unlock_vault_crypto(current, &salt, &token)?;
    let (new_salt, new_token, new_key) = crypto::init_vault_crypto(new)?;
    Ok(Rekey { old_key, new_salt, new_token, new_key })
}

/// Re-keys the vault from `current_password` to `new_password`.
pub async fn change_password(
    shared: &SharedState,
    current_password: &str,
    new_password: &str,
) -> Result<(), String> {
    change_password_with_signer(shared, current_password, new_password, biometric::platform_signer())
        .await
}

/// [`change_password`] with the Hello signer injected (tests use a fake one).
pub(super) async fn change_password_with_signer(
    shared: &SharedState,
    current_password: &str,
    new_password: &str,
    signer: Arc<dyn HelloSigner>,
) -> Result<(), String> {
    let current = Zeroizing::new(current_password.as_bytes().to_vec());
    let new = Zeroizing::new(new_password.as_bytes().to_vec());
    change_password_with_kdf(shared, move |meta| derive_for_rekey(&current, &new, meta)).await?;
    // The biometric enrollment wraps the old vault key: drop it and ask the
    // user to re-enroll. The password change itself has already succeeded.
    if let Err(e) = super::biometric_enrollment::invalidate(shared, signer, true).await {
        eprintln!("could not clear biometric enrollment after password change: {e}");
    }
    Ok(())
}

/// [`change_password`] with the derivation injected (tests use a slow one).
async fn change_password_with_kdf<F>(shared: &SharedState, kdf: F) -> Result<(), String>
where
    F: FnOnce((String, String)) -> Result<Rekey, String> + Send + 'static,
{
    // Phase 1.
    let (meta, epoch) = {
        let s = shared.lock().await;
        s.key.as_ref().ok_or("vault is locked")?;
        let meta = s.db.get_meta().await?.ok_or("vault_meta missing")?;
        (meta, s.epoch)
    };

    // Phase 2.
    let snapshot = meta.clone();
    let rekey = tokio::task::spawn_blocking(move || kdf(snapshot))
        .await
        .map_err(|_| "key derivation task failed".to_string())??;

    // Phase 3: re-encryption of every item is fast and runs inside one
    // transaction under the lock.
    let mut s = shared.lock().await;
    const CHANGED: &str = "the vault changed while the password was being verified, try again";
    if s.epoch != epoch || s.key.is_none() {
        return Err(CHANGED.to_string());
    }
    if s.db.get_meta().await?.as_ref() != Some(&meta) {
        return Err(CHANGED.to_string());
    }

    let raw = s.db.list_items().await?;
    let mut re_encrypted: Vec<(i64, String)> = Vec::with_capacity(raw.len());
    for (id, _, data, _, _) in &raw {
        let plaintext = crypto::decrypt(&rekey.old_key, data)?;
        let new_data = crypto::encrypt(&rekey.new_key, &plaintext)?;
        re_encrypted.push((*id, new_data));
    }

    // Atomic DB update: new salt/token + all re-encrypted items
    s.db.rekey(&rekey.new_salt, &rekey.new_token, re_encrypted).await?;

    s.set_key(Some(rekey.new_key));
    s.touch();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{locked_vault, unlocked_vault};
    use crate::vault::lock_vault;
    use std::sync::mpsc;

    /// A derivation that reports it started, then blocks until released.
    fn gated<T: Send + 'static>(
        started: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
        real: impl FnOnce() -> T + Send + 'static,
    ) -> impl FnOnce() -> T + Send + 'static {
        move || {
            let _ = started.send(());
            let _ = release.recv();
            real()
        }
    }

    async fn wait_started(rx: mpsc::Receiver<()>) {
        tokio::task::spawn_blocking(move || rx.recv())
            .await
            .unwrap()
            .unwrap();
    }

    async fn lock_is_free(shared: &SharedState) -> bool {
        tokio::time::timeout(Duration::from_secs(2), shared.lock()).await.is_ok()
    }

    // ── throttle ─────────────────────────────────────────────────────────────

    #[test]
    fn five_failures_wait_sixteen_seconds_and_success_resets() {
        let t = UnlockThrottle::default();
        let t0 = Instant::now();
        assert!(t.check(t0).is_ok());
        for _ in 0..5 {
            t.record_failure(t0);
        }
        let wait = t.check(t0).unwrap_err();
        assert_eq!(wait, Duration::from_secs(16));
        assert!(t.check(t0 + Duration::from_secs(15)).is_err());
        assert!(t.check(t0 + Duration::from_secs(16)).is_ok());

        t.record_success();
        assert!(t.check(t0).is_ok());
        t.record_failure(t0);
        assert_eq!(t.check(t0).unwrap_err(), Duration::from_secs(1));
    }

    #[test]
    fn backoff_is_capped_at_sixty_seconds() {
        assert_eq!(backoff(1), Duration::from_secs(1));
        assert_eq!(backoff(6), Duration::from_secs(32));
        assert_eq!(backoff(7), Duration::from_secs(60));
        assert_eq!(backoff(u32::MAX), Duration::from_secs(60));
    }

    #[tokio::test]
    async fn dropped_attempt_counts_but_not_counted_does_not() {
        let t = Arc::new(UnlockThrottle::default());
        t.begin().await.unwrap().not_counted();
        assert!(t.check(Instant::now()).is_ok());
        drop(t.begin().await.unwrap());
        assert!(t.check(Instant::now()).is_err());
    }

    #[tokio::test]
    async fn wrong_passwords_throttle_and_the_right_one_resets() {
        let v = locked_vault().await;
        {
            let r = unlock_with_password(&v.state, b"wrong", false, false).await;
            assert!(matches!(r, Err(UnlockError::IncorrectPassword)));
        }
        // Throttled even with the correct password until the wait passes.
        let r = unlock_with_password(&v.state, v.master_password.as_bytes(), false, false).await;
        assert!(matches!(r, Err(UnlockError::Throttled(_))), "{:?}", r.as_ref().err());

        // Elapse the backoff, then a correct unlock succeeds and resets it.
        let throttle = v.state.lock().await.throttle.clone();
        throttle.record_success();
        let ok = unlock_with_password(&v.state, v.master_password.as_bytes(), false, true)
            .await
            .unwrap();
        assert!(ok.payload.is_some());
        assert!(v.state.lock().await.key.is_some());
        assert!(throttle.check(Instant::now()).is_ok());
    }

    // ── auto_lock_timeout ────────────────────────────────────────────────────

    #[test]
    fn validate_auto_lock_bounds() {
        assert_eq!(validate_auto_lock(0), Ok(0));
        assert_eq!(validate_auto_lock(1), Ok(1));
        assert_eq!(validate_auto_lock(1440), Ok(1440));
        for bad in [-1, 1441, i64::MAX, i64::MIN] {
            assert!(validate_auto_lock(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn stored_bad_values_are_clamped() {
        assert_eq!(effective_auto_lock(None), 5);
        assert_eq!(effective_auto_lock(Some("garbage")), 5);
        assert_eq!(effective_auto_lock(Some("-3")), 5);
        assert_eq!(effective_auto_lock(Some("0")), 0);
        assert_eq!(effective_auto_lock(Some("30")), 30);
        assert_eq!(effective_auto_lock(Some("9223372036854775807")), 1440);
        assert_eq!(effective_auto_lock(Some("99999999999999999999999")), 5);
    }

    // ── two-phase unlock ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn vault_lock_is_free_during_a_slow_derivation() {
        let v = locked_vault().await;
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let pw = v.master_password.clone();
        let shared = v.state.clone();
        let task = tokio::spawn(async move {
            unlock_with_kdf(&shared, false, false, move |meta| {
                gated(started_tx, release_rx, move || derive_for_unlock(pw.as_bytes(), meta))()
            })
            .await
        });
        wait_started(started_rx).await;

        assert!(lock_is_free(&v.state).await, "vault lock held during the derivation");
        assert!(v.state.lock().await.key.is_none(), "key published before verification");

        release_tx.send(()).unwrap();
        task.await.unwrap().unwrap();
        assert!(v.state.lock().await.key.is_some());
    }

    #[tokio::test]
    async fn locking_during_the_derivation_aborts_the_commit() {
        let v = unlocked_vault().await;
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let pw = v.master_password.clone();
        let shared = v.state.clone();
        let task = tokio::spawn(async move {
            unlock_with_kdf(&shared, false, false, move |meta| {
                gated(started_tx, release_rx, move || derive_for_unlock(pw.as_bytes(), meta))()
            })
            .await
        });
        wait_started(started_rx).await;

        lock_vault(&v.state).await;
        release_tx.send(()).unwrap();

        let r = task.await.unwrap();
        assert!(matches!(r, Err(UnlockError::Aborted)), "{:?}", r.as_ref().err());
        assert!(v.state.lock().await.key.is_none(), "stale key committed after a lock");
    }

    #[tokio::test]
    async fn rekey_during_the_derivation_aborts_the_commit() {
        let v = locked_vault().await;
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let pw = v.master_password.clone();
        let shared = v.state.clone();
        let task = tokio::spawn(async move {
            unlock_with_kdf(&shared, false, false, move |meta| {
                gated(started_tx, release_rx, move || derive_for_unlock(pw.as_bytes(), meta))()
            })
            .await
        });
        wait_started(started_rx).await;

        {
            // Same effect as a wipe/restore while locked: other key material.
            let s = v.state.lock().await;
            let (salt, token, _) = crypto::init_vault_crypto(b"another").unwrap();
            s.db.rekey(&salt, &token, vec![]).await.unwrap();
        }
        release_tx.send(()).unwrap();

        let r = task.await.unwrap();
        assert!(matches!(r, Err(UnlockError::Aborted)), "{:?}", r.as_ref().err());
        assert!(v.state.lock().await.key.is_none());
    }

    #[tokio::test]
    async fn first_run_initializes_only_when_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::VaultDb::open(dir.path().join("vault.db").to_str().unwrap())
            .await
            .unwrap();
        let shared: SharedState = Arc::new(Mutex::new(crate::vault::VaultState::new(db)));

        let r = unlock_with_password(&shared, b"pw-1234567", false, false).await;
        assert!(matches!(r, Err(UnlockError::NotInitialized)));
        assert!(shared.lock().await.key.is_none());
        // A rejected attempt is not a password failure.
        assert!(shared.lock().await.throttle.check(Instant::now()).is_ok());

        unlock_with_password(&shared, b"pw-1234567", true, false).await.unwrap();
        assert!(shared.lock().await.key.is_some());
        lock_vault(&shared).await;
        unlock_with_password(&shared, b"pw-1234567", false, false).await.unwrap();
    }

    // ── change-password ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn vault_lock_is_free_during_change_password_derivation() {
        let v = unlocked_vault().await;
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let pw = v.master_password.clone();
        let shared = v.state.clone();
        let task = tokio::spawn(async move {
            change_password_with_kdf(&shared, move |meta| {
                gated(started_tx, release_rx, move || {
                    derive_for_rekey(pw.as_bytes(), b"new-password-1", meta)
                })()
            })
            .await
        });
        wait_started(started_rx).await;

        assert!(lock_is_free(&v.state).await, "vault lock held during the derivation");

        release_tx.send(()).unwrap();
        task.await.unwrap().unwrap();
        // The new password now unlocks the re-keyed vault.
        lock_vault(&v.state).await;
        unlock_with_password(&v.state, b"new-password-1", false, true).await.unwrap();
    }

    #[tokio::test]
    async fn locking_during_change_password_aborts_without_touching_the_db() {
        let v = unlocked_vault().await;
        let before = v.state.lock().await.db.get_meta().await.unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let pw = v.master_password.clone();
        let shared = v.state.clone();
        let task = tokio::spawn(async move {
            change_password_with_kdf(&shared, move |meta| {
                gated(started_tx, release_rx, move || {
                    derive_for_rekey(pw.as_bytes(), b"new-password-1", meta)
                })()
            })
            .await
        });
        wait_started(started_rx).await;

        lock_vault(&v.state).await;
        release_tx.send(()).unwrap();

        assert!(task.await.unwrap().is_err());
        let s = v.state.lock().await;
        assert!(s.key.is_none());
        assert_eq!(s.db.get_meta().await.unwrap(), before);
    }

    #[tokio::test]
    async fn wrong_current_password_is_rejected() {
        let v = unlocked_vault().await;
        let err = change_password(&v.state, "not-the-password", "new-password-1")
            .await
            .unwrap_err();
        assert_eq!(err, crypto::INCORRECT_PASSWORD);
        assert!(v.state.lock().await.key.is_some());
    }
}
