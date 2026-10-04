pub mod crypto;
pub mod lan;
pub mod package;
pub mod protocol;
pub mod relay;

use rand::RngCore;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

use crypto::PairRole;
use package::PlainItem;

/// Pairing attempts with a wrong code allowed per session before it aborts.
const MAX_FAILED_ATTEMPTS: u8 = 3;
/// How long a session waits for a peer to connect, and then for fingerprint confirmation.
const SESSION_WAIT: Duration = Duration::from_secs(300);
/// Listener poll interval; bounds how long cancellation takes to be observed.
const LISTENER_POLL: Duration = Duration::from_millis(500);

// ─── Error type ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ShareError {
    /// Vault-layer error (DB or crypto)
    Vault(String),
    /// Network I/O error
    Io(String),
    /// Wire protocol error (bad message, malformed data)
    Protocol(String),
    /// mDNS discovery error
    Discovery(String),
    /// Cryptographic error
    Crypto(String),
    /// Operation timed out
    Timeout,
    /// Remote peer sent an error
    Remote(String),
    /// Session is in an unexpected state
    InvalidState(String),
    /// Relay backend lacks the v2 RPCs; the user must apply the relay SQL v2
    RelaySchemaOutdated,
    /// Peer failed key confirmation (wrong pairing code)
    PairingFailed,
    /// Peer speaks a different LAN protocol version
    VersionMismatch,
    /// A share session is already running
    SessionActive,
    /// The session was cancelled
    Cancelled,
}

impl std::fmt::Display for ShareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShareError::Vault(e) => write!(f, "vault error: {e}"),
            ShareError::Io(e) => write!(f, "I/O error: {e}"),
            ShareError::Protocol(e) => write!(f, "protocol error: {e}"),
            ShareError::Discovery(e) => write!(f, "discovery error: {e}"),
            ShareError::Crypto(e) => write!(f, "crypto error: {e}"),
            ShareError::Timeout => write!(f, "operation timed out"),
            ShareError::Remote(e) => write!(f, "peer error: {e}"),
            ShareError::InvalidState(e) => write!(f, "invalid session state: {e}"),
            ShareError::RelaySchemaOutdated => write!(
                f,
                "RELAY_SCHEMA_OUTDATED: apply relay SQL v2 in your Supabase SQL editor (Settings → Internet Sharing shows the SQL)"
            ),
            ShareError::PairingFailed => {
                write!(f, "pairing failed: the peer did not prove knowledge of the pairing code")
            }
            ShareError::VersionMismatch => write!(f, "{}", lan::VERSION_MISMATCH_HINT),
            ShareError::SessionActive => {
                write!(f, "a share session is already active; cancel it first")
            }
            ShareError::Cancelled => write!(f, "session cancelled"),
        }
    }
}

// ─── Session state ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum ShareSessionState {
    Listening,
    Connecting,
    AwaitingFingerprint,
    Active,
    Done,
    Failed(String),
    Cancelled,
}

impl ShareSessionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ShareSessionState::Listening => "listening",
            ShareSessionState::Connecting => "connecting",
            ShareSessionState::AwaitingFingerprint => "awaiting_fingerprint",
            ShareSessionState::Active => "active",
            ShareSessionState::Done => "done",
            ShareSessionState::Failed(_) => "failed",
            ShareSessionState::Cancelled => "cancelled",
        }
    }

    /// Done, Failed and Cancelled sessions no longer own any resources and
    /// may be replaced by a new session.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ShareSessionState::Done | ShareSessionState::Failed(_) | ShareSessionState::Cancelled
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShareDirection {
    Sending,
    Receiving,
}

impl ShareDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            ShareDirection::Sending => "sending",
            ShareDirection::Receiving => "receiving",
        }
    }
}

pub struct ShareSession {
    /// Monotonic id from `ShareState`; background tasks exit when it no longer matches.
    pub id: u64,
    pub pairing_code: String,
    /// Session key from the PAKE handshake, wrapped in Zeroizing — cleared on drop.
    pub shared_key: Option<Zeroizing<[u8; 32]>>,
    /// Copy of the vault key used to read/import items. Dropped (and zeroized)
    /// when the session ends, is cancelled, or the vault locks.
    pub vault_key: Option<Zeroizing<[u8; 32]>>,
    /// Set on cancel so blocking socket reads in background threads abort.
    pub cancel: Arc<AtomicBool>,
    /// Pairing attempts that failed key confirmation (sender side).
    pub failed_attempts: u8,
    pub state: ShareSessionState,
    pub created_at: Instant,
    pub last_active: Instant,
    /// Item IDs to send (sender side only).
    pub items_to_send: Vec<i64>,
    pub direction: ShareDirection,
    /// Fingerprint (`XXXX-XXXX-XXXX-XXXX`) computed after the handshake.
    pub fingerprint: Option<String>,
    /// Received plain items (receiver side, after Done).
    pub received_items: Vec<PlainItem>,
    /// Names of items received (available after Done for the API response).
    pub received_names: Vec<String>,
    /// Names of received items whose key collided with one already linked
    /// in the target environment — the item itself was still imported
    /// (owned by the project), just not linked over the existing var, so
    /// nothing the receiver already had gets silently repointed. See
    /// `import_plain_items_into_vault`.
    pub skipped_keys: Vec<String>,
    /// Non-fatal informational note for the frontend, e.g. firewall warning.
    pub note: Option<String>,
    /// Project + environment context this session is scoped to, when the
    /// caller provided one (the HTTP API always does; the GUI's Tauri-command
    /// LAN share flow currently does not). On the receiving side, when set,
    /// imported items are owned by `project_id` and linked into
    /// `environment_id`'s vars under their own name — see
    /// `import_plain_items_into_vault`.
    pub project_id: Option<i64>,
    pub environment_id: Option<i64>,
}

impl ShareSession {
    /// Change state; entering a terminal state drops the key copies.
    fn set_state(&mut self, state: ShareSessionState) {
        if state.is_terminal() {
            self.vault_key = None;
            self.shared_key = None;
        }
        self.state = state;
        self.last_active = Instant::now();
    }
}

/// Top-level share state — at most one non-terminal session at a time.
pub struct ShareState {
    pub session: Arc<Mutex<Option<ShareSession>>>,
    next_id: AtomicU64,
    /// Socket timeouts; shortened by tests.
    pub lan: lan::LanConfig,
}

impl ShareState {
    pub fn new() -> Self {
        Self::with_config(lan::LanConfig::default())
    }

    pub fn with_config(lan: lan::LanConfig) -> Self {
        ShareState {
            session: Arc::new(Mutex::new(None)),
            next_id: AtomicU64::new(1),
            lan,
        }
    }
}

// ─── Session helpers ──────────────────────────────────────────────────────────

/// Register a new session. Rejected with `SessionActive` while another session
/// is still running; finished sessions are replaced.
async fn begin_session(
    share_state: &Arc<ShareState>,
    pairing_code: String,
    direction: ShareDirection,
    state: ShareSessionState,
    items_to_send: Vec<i64>,
    vault_key: Zeroizing<[u8; 32]>,
    project_id: Option<i64>,
    environment_id: Option<i64>,
) -> Result<(u64, Arc<AtomicBool>), ShareError> {
    let mut guard = share_state.session.lock().await;
    if guard.as_ref().is_some_and(|s| !s.state.is_terminal()) {
        return Err(ShareError::SessionActive);
    }
    let id = share_state.next_id.fetch_add(1, Ordering::Relaxed);
    let cancel = Arc::new(AtomicBool::new(false));
    *guard = Some(ShareSession {
        id,
        pairing_code,
        shared_key: None,
        vault_key: Some(vault_key),
        cancel: cancel.clone(),
        failed_attempts: 0,
        state,
        created_at: Instant::now(),
        last_active: Instant::now(),
        items_to_send,
        direction,
        fingerprint: None,
        received_items: Vec::new(),
        received_names: Vec::new(),
        skipped_keys: Vec::new(),
        note: None,
        project_id,
        environment_id,
    });
    Ok((id, cancel))
}

/// Run `f` on the session only if it is still the session with `id`.
/// Returns `None` when the session is gone or has been replaced.
async fn with_session<R>(
    share_state: &Arc<ShareState>,
    id: u64,
    f: impl FnOnce(&mut ShareSession) -> R,
) -> Option<R> {
    let mut guard = share_state.session.lock().await;
    match guard.as_mut() {
        Some(s) if s.id == id => Some(f(s)),
        _ => None,
    }
}

/// State of session `id`, or `None` when it no longer exists / was replaced.
async fn session_status(share_state: &Arc<ShareState>, id: u64) -> Option<ShareSessionState> {
    with_session(share_state, id, |s| s.state.clone()).await
}

/// Mark session `id` failed unless it already finished (never clobbers Cancelled/Done).
async fn fail_session(share_state: &Arc<ShareState>, id: u64, message: String) {
    with_session(share_state, id, |s| {
        if !s.state.is_terminal() {
            s.set_state(ShareSessionState::Failed(message));
        }
    })
    .await;
}

/// Record the handshake result and move to AwaitingFingerprint.
/// Returns false when the session was cancelled/replaced meanwhile.
async fn set_awaiting_fingerprint(
    share_state: &Arc<ShareState>,
    id: u64,
    fingerprint: &str,
    shared_key: &Zeroizing<[u8; 32]>,
) -> bool {
    with_session(share_state, id, |s| {
        if s.state.is_terminal() {
            return false;
        }
        s.fingerprint = Some(fingerprint.to_string());
        s.shared_key = Some(shared_key.clone());
        s.set_state(ShareSessionState::AwaitingFingerprint);
        true
    })
    .await
    .unwrap_or(false)
}

enum ConfirmOutcome {
    Confirmed,
    /// The user rejected the fingerprint (or the session was cancelled).
    Rejected,
    /// The session ended or was replaced; the task must exit silently.
    Stop,
}

/// Poll until the user confirms the fingerprint, up to `SESSION_WAIT`.
async fn wait_for_confirmation(
    share_state: &Arc<ShareState>,
    id: u64,
) -> Result<ConfirmOutcome, ShareError> {
    let deadline = Instant::now() + SESSION_WAIT;
    loop {
        if Instant::now() >= deadline {
            return Err(ShareError::Timeout);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        match session_status(share_state, id).await {
            Some(ShareSessionState::Active) => return Ok(ConfirmOutcome::Confirmed),
            Some(ShareSessionState::Cancelled) => return Ok(ConfirmOutcome::Rejected),
            Some(ShareSessionState::AwaitingFingerprint) => continue,
            _ => return Ok(ConfirmOutcome::Stop),
        }
    }
}

/// Clone the session's vault key, if the session still holds one.
async fn session_vault_key(
    share_state: &Arc<ShareState>,
    id: u64,
) -> Option<Zeroizing<[u8; 32]>> {
    with_session(share_state, id, |s| s.vault_key.clone())
        .await
        .flatten()
}

// ─── Pairing code generation ──────────────────────────────────────────────────

/// Generate a cryptographically random 6-digit numeric pairing code.
pub fn generate_pairing_code() -> String {
    let mut buf = [0u8; 4];
    rand::thread_rng().fill_bytes(&mut buf);
    let n = u32::from_le_bytes(buf) % 1_000_000;
    format!("{:06}", n)
}

// ─── Public functions ─────────────────────────────────────────────────────────

/// Start a sender session: generate a session, start the mDNS listener in a
/// background Tokio task, and return the pairing code.
///
/// Rejected with `ShareError::SessionActive` while another session is running.
///
/// The background task:
/// 1. Accepts TCP connections one at a time and runs the SPAKE2 handshake;
///    wrong-code peers are dropped and counted (3 strikes abort the session).
/// 2. Sets state to AwaitingFingerprint with the fingerprint.
/// 3. Waits for the fingerprint to be confirmed (state → Active) or cancelled.
/// 4. If Active: sends items, sets state Done.
/// 5. If not: sends Error, state stays Cancelled or Failed.
///
/// The vault key lives in the session (zeroized when it ends) so the task can
/// decrypt items independently and a lock/cancel can drop it.
pub async fn start_listen_session(
    share_state: Arc<ShareState>,
    item_ids: Vec<i64>,
    vault_key: Zeroizing<[u8; 32]>,
    db: Arc<Mutex<crate::vault::VaultState>>,
    project_id: Option<i64>,
    environment_id: Option<i64>,
) -> Result<String, ShareError> {
    let pairing_code = generate_pairing_code();

    let (id, cancel) = begin_session(
        &share_state,
        pairing_code.clone(),
        ShareDirection::Sending,
        ShareSessionState::Listening,
        item_ids.clone(),
        vault_key,
        project_id,
        environment_id,
    )
    .await?;

    let state_clone = share_state.clone();
    let code_clone = pairing_code.clone();

    tokio::spawn(async move {
        let result =
            run_send_background(state_clone.clone(), id, cancel, code_clone, item_ids, db).await;

        if let Err(e) = result {
            fail_session(&state_clone, id, e.to_string()).await;
        }
    });

    Ok(pairing_code)
}

/// Accept connections on `listener` until one completes the pairing handshake.
///
/// Returns `Ok(None)` when the session ended (cancelled, replaced, aborted by
/// too many wrong-code attempts, or version mismatch) — the session state has
/// already been updated in those cases. Silent or garbled peers are dropped
/// without counting as attempts, so a port scanner cannot burn the session.
async fn accept_and_pair(
    share_state: &Arc<ShareState>,
    id: u64,
    listener: &std::net::TcpListener,
    pairing_code: &str,
    cfg: &lan::LanConfig,
    accept_timeout: Duration,
) -> Result<Option<(std::net::TcpStream, Zeroizing<[u8; 32]>, String)>, ShareError> {
    // Non-blocking so we can poll with cancellation checks.
    listener
        .set_nonblocking(true)
        .map_err(|e| ShareError::Io(e.to_string()))?;
    let start = Instant::now();

    loop {
        if start.elapsed() >= accept_timeout {
            fail_session(share_state, id, "pairing code expired".into()).await;
            return Err(ShareError::Timeout);
        }

        match session_status(share_state, id).await {
            Some(s) if !s.is_terminal() => {}
            _ => return Ok(None),
        }

        let stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                tokio::time::sleep(LISTENER_POLL).await;
                continue;
            }
            Err(e) => return Err(ShareError::Io(format!("accept: {e}"))),
        };

        // The listener is non-blocking for polling and accepted streams may
        // inherit that; the handshake reads need blocking mode with timeouts.
        stream
            .set_nonblocking(false)
            .map_err(|e| ShareError::Io(format!("set stream blocking: {e}")))?;

        let code = pairing_code.to_string();
        let cfg_clone = cfg.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let mut stream = stream;
            lan::pair_handshake(&mut stream, &code, PairRole::Sender, &cfg_clone)
                .map(|(key, fp)| (stream, key, fp))
        })
        .await
        .map_err(|e| ShareError::Io(e.to_string()))?;

        match outcome {
            Ok((stream, key, fp)) => return Ok(Some((stream, key, fp))),
            Err(ShareError::PairingFailed) => {
                let attempts = with_session(share_state, id, |s| {
                    s.failed_attempts += 1;
                    s.failed_attempts
                })
                .await;
                match attempts {
                    None => return Ok(None),
                    Some(n) if n >= MAX_FAILED_ATTEMPTS => {
                        fail_session(
                            share_state,
                            id,
                            "too many failed pairing attempts".into(),
                        )
                        .await;
                        return Ok(None);
                    }
                    Some(_) => {}
                }
            }
            Err(ShareError::VersionMismatch) => {
                fail_session(share_state, id, ShareError::VersionMismatch.to_string()).await;
                return Ok(None);
            }
            Err(ShareError::Cancelled) => return Ok(None),
            // Silent, slow or garbled peer: dropped; keep waiting for the real receiver.
            Err(_) => {}
        }
    }
}

async fn run_send_background(
    share_state: Arc<ShareState>,
    id: u64,
    cancel: Arc<AtomicBool>,
    pairing_code: String,
    item_ids: Vec<i64>,
    db_state: Arc<Mutex<crate::vault::VaultState>>,
) -> Result<(), ShareError> {
    // Start mDNS listener — keep daemon alive in this scope
    let session_id = lan::generate_session_id();
    let (listener, port, mdns_daemon) = lan::start_listener(&session_id)?;

    // Inform the frontend that the app is listening on a LAN port and that
    // Windows Firewall may block inbound connections.  The user must allow the
    // app through the firewall (or add an inbound rule for the port) if they
    // see connection failures on the receiver side.
    with_session(&share_state, id, |s| {
        s.note = Some(format!(
            "Listening on port {port}. If the receiver cannot connect, \
             allow this app through Windows Firewall or add an inbound \
             TCP rule for port {port}."
        ));
    })
    .await;

    let cfg = share_state.lan.clone().with_cancel(cancel);
    let paired = accept_and_pair(&share_state, id, &listener, &pairing_code, &cfg, SESSION_WAIT).await?;
    // One peer only: stop accepting and stop advertising as soon as pairing is over.
    drop(listener);
    drop(mdns_daemon);
    let (stream, shared_key, fingerprint) = match paired {
        Some(p) => p,
        None => return Ok(()),
    };

    if !set_awaiting_fingerprint(&share_state, id, &fingerprint, &shared_key).await {
        return Ok(());
    }

    // Wait for user to confirm fingerprint (up to 5 minutes)
    match wait_for_confirmation(&share_state, id).await? {
        ConfirmOutcome::Confirmed => {}
        ConfirmOutcome::Rejected => {
            // Notify peer — use the local shared_key we derived during handshake
            let _ = tokio::task::spawn_blocking(move || {
                lan::sender_reject(stream, &shared_key, "user rejected fingerprint")
            })
            .await;
            return Ok(());
        }
        ConfirmOutcome::Stop => return Ok(()),
    }

    let vault_key = match session_vault_key(&share_state, id).await {
        Some(k) => k,
        None => return Ok(()),
    };

    // Decrypt items for sharing
    let db_items = {
        let vault_guard = db_state.lock().await;
        let raw = vault_guard
            .db
            .list_items()
            .await
            .map_err(|e| ShareError::Vault(e))?;
        raw
    };

    let mut plain_items = Vec::new();
    for (item_id, _, data, _, _) in &db_items {
        if !item_ids.contains(item_id) {
            continue;
        }
        let plaintext =
            crate::crypto::decrypt(&vault_key, data).map_err(|e| ShareError::Vault(e))?;
        let item: crate::vault::VaultItem =
            serde_json::from_slice(&plaintext).map_err(|e| ShareError::Protocol(e.to_string()))?;
        plain_items.push(PlainItem {
            item_type: item.item_type,
            name: item.name.unwrap_or_default(),
            value: item.value,
            username: item.username,
            password: item.password,
            url: item.url,
            notes: item.notes,
            category: item.categories.and_then(|v| v.into_iter().next()),
            command: item.command,
        });
    }
    drop(vault_key);

    // Cancelled (or the vault locked) while decrypting: send nothing.
    if session_status(&share_state, id).await != Some(ShareSessionState::Active) {
        return Ok(());
    }

    // Send items
    let shared_key_clone = shared_key.clone();
    let cfg_send = cfg.clone();
    tokio::task::spawn_blocking(move || {
        lan::sender_send_items(stream, &shared_key_clone, plain_items, &cfg_send)
    })
    .await
    .map_err(|e| ShareError::Io(e.to_string()))??;

    // Log and mark Done
    {
        let vault_guard = db_state.lock().await;
        let _ = vault_guard
            .db
            .log_share(
                "lan",
                "sending",
                &item_ids,
                Some(&fingerprint),
            )
            .await;
    }

    with_session(&share_state, id, |s| {
        if s.state == ShareSessionState::Active {
            s.set_state(ShareSessionState::Done);
        }
    })
    .await;

    Ok(())
}

/// Start a receiver session: discover and pair with a LAN peer (SPAKE2 handshake),
/// and set state to AwaitingFingerprint. Returns the fingerprint.
///
/// Rejected with `ShareError::SessionActive` while another session is running.
pub async fn connect_to_peer(
    share_state: Arc<ShareState>,
    pairing_code: String,
    vault_key: Zeroizing<[u8; 32]>,
    db: Arc<Mutex<crate::vault::VaultState>>,
    project_id: Option<i64>,
    environment_id: Option<i64>,
) -> Result<String, ShareError> {
    let (id, cancel) = begin_session(
        &share_state,
        pairing_code.clone(),
        ShareDirection::Receiving,
        ShareSessionState::Connecting,
        Vec::new(),
        vault_key,
        project_id,
        environment_id,
    )
    .await?;

    // Discover + pair via mDNS.  Use 60 s so slow mDNS propagation on
    // Windows (multicast routing delays) does not cause spurious timeouts.
    let cfg = share_state.lan.clone().with_cancel(cancel);
    let cfg_connect = cfg.clone();
    let paired = tokio::task::spawn_blocking(move || {
        lan::connect_and_pair(&pairing_code, 60, &cfg_connect)
    })
    .await
    .map_err(|e| ShareError::Io(e.to_string()))?;

    let (stream, shared_key, fingerprint) = match paired {
        Ok(p) => p,
        Err(e) => {
            fail_session(&share_state, id, e.to_string()).await;
            return Err(e);
        }
    };

    if !set_awaiting_fingerprint(&share_state, id, &fingerprint, &shared_key).await {
        return Err(ShareError::Cancelled);
    }

    // Background task: wait for confirmation, then receive items
    let state_clone = share_state.clone();
    let fp_clone = fingerprint.clone();
    tokio::spawn(async move {
        let result = run_receive_background(
            state_clone.clone(),
            id,
            cfg,
            stream,
            shared_key,
            fingerprint,
            db,
            project_id,
            environment_id,
        )
        .await;
        if let Err(e) = result {
            fail_session(&state_clone, id, e.to_string()).await;
        }
    });

    Ok(fp_clone)
}

async fn run_receive_background(
    share_state: Arc<ShareState>,
    id: u64,
    cfg: lan::LanConfig,
    stream: std::net::TcpStream,
    shared_key: Zeroizing<[u8; 32]>,
    fingerprint: String,
    db_state: Arc<Mutex<crate::vault::VaultState>>,
    project_id: Option<i64>,
    environment_id: Option<i64>,
) -> Result<(), ShareError> {
    // Wait for user to confirm fingerprint (up to 5 minutes)
    match wait_for_confirmation(&share_state, id).await? {
        ConfirmOutcome::Confirmed => {}
        ConfirmOutcome::Rejected | ConfirmOutcome::Stop => return Ok(()),
    }

    // Receive items (the sender's user may still be confirming on their side)
    let shared_key_clone = shared_key.clone();
    let items = tokio::task::spawn_blocking(move || {
        lan::receiver_receive_items(stream, &shared_key_clone, &cfg, SESSION_WAIT)
    })
    .await
    .map_err(|e| ShareError::Io(e.to_string()))??;

    // Cancelled (or the vault locked) while receiving: import nothing.
    if session_status(&share_state, id).await != Some(ShareSessionState::Active) {
        return Ok(());
    }
    let vault_key = match session_vault_key(&share_state, id).await {
        Some(k) => k,
        None => return Ok(()),
    };

    let item_ids: Vec<i64> = Vec::new(); // receiver doesn't know IDs before import

    // Import items into vault and log — acquire the mutex once for both operations
    let link = match (project_id, environment_id) {
        (Some(p), Some(e)) => Some((p, e)),
        _ => None,
    };
    let outcome = {
        let vault_guard = db_state.lock().await;
        let imported = import_plain_items_into_vault(&items, &vault_key, &vault_guard.db, link).await?;
        let _ = vault_guard
            .db
            .log_share("lan", "receiving", &item_ids, Some(&fingerprint))
            .await;
        imported
    };

    with_session(&share_state, id, |s| {
        s.received_names = outcome.names.clone();
        s.skipped_keys = outcome.skipped_keys.clone();
        s.set_state(ShareSessionState::Done);
    })
    .await;

    Ok(())
}

/// Serializes and encrypts a single `PlainItem` into vault-item ciphertext:
/// `(item_type, encrypted_data)`, ready for `db::upsert_item` or a batched
/// insert like `db::insert_received_project`. Extracted out of
/// `import_plain_items_into_vault`'s per-item body (issue #4 D7) so the
/// project-relay receive path — which needs ciphertext up front for a single
/// transactional multi-row insert, rather than one `db.upsert_item` call per
/// item — can share the encrypt step instead of duplicating it. `db` never
/// sees the vault key or a `PlainItem`, only the ciphertext this returns.
pub(crate) fn build_encrypted_item(
    plain: &PlainItem,
    vault_key: &[u8; 32],
    created: &str,
) -> Result<(String, String), ShareError> {
    let vault_item = crate::vault::VaultItem {
        id: 0,
        item_type: plain.item_type.clone(),
        name: Some(plain.name.clone()),
        value: plain.value.clone(),
        username: plain.username.clone(),
        password: plain.password.clone(),
        url: plain.url.clone(),
        notes: plain.notes.clone(),
        title: None,
        description: None,
        command: plain.command.clone(),
        shell: None,
        content: None,
        categories: Some(plain.category.iter().cloned().collect()),
        created: created.to_string(),
        is_global: None,
    };

    let json = serde_json::to_vec(&vault_item).map_err(|e| ShareError::Protocol(e.to_string()))?;
    let encrypted = crate::crypto::encrypt(vault_key, &json).map_err(ShareError::Vault)?;
    Ok((vault_item.item_type, encrypted))
}

/// Result of [`import_plain_items_into_vault`].
pub struct ImportOutcome {
    /// Names of every item actually imported into the vault (regardless of
    /// whether it ended up linked into an environment).
    pub names: Vec<String>,
    /// Names of items that were imported but NOT linked into the target
    /// environment, either because their key already had a different item
    /// linked to it there (a peer must never silently repoint a link the
    /// receiver already had — see Bug 3 in the security review) or because
    /// the name is empty / contains characters (`=`, newlines) that would
    /// corrupt a `KEY=value` .env line if it were ever written out.
    pub skipped_keys: Vec<String>,
}

/// Imports decrypted items received via LAN share, internet relay, or
/// `.vault` package import into the vault. When `link` is
/// `Some((project_id, environment_id))` — whenever the caller resolved a
/// project+environment scope for the request — each imported item is
/// additionally granted ownership of `project_id`, and linked into
/// `environment_id`'s vars under its own name UNLESS that name is unsafe or
/// already linked to a different item in that environment (see
/// `ImportOutcome::skipped_keys`); the item itself is still imported and
/// owned either way, just not silently overwriting an existing link. `link`
/// is `None` only when the caller has no project+environment context.
pub(crate) async fn import_plain_items_into_vault(
    items: &[PlainItem],
    vault_key: &[u8; 32],
    db: &crate::db::VaultDb,
    link: Option<(i64, i64)>,
) -> Result<ImportOutcome, ShareError> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string();

    // Keys already linked in the target environment, read once up front —
    // a peer sending an item whose name collides with one of these must not
    // silently repoint the receiver's existing link.
    let existing_keys: HashSet<String> = if let Some((_, environment_id)) = link {
        db.get_environment_vars(environment_id)
            .await
            .map_err(ShareError::Vault)?
            .into_iter()
            .map(|v| v.key)
            .collect()
    } else {
        HashSet::new()
    };

    let mut names = Vec::new();
    let mut skipped_keys = Vec::new();

    for plain in items {
        let (item_type, encrypted) = build_encrypted_item(plain, vault_key, &now_ts)?;

        let new_id = db
            .upsert_item(0, &item_type, &encrypted, &now_ts, false)
            .await
            .map_err(|e| ShareError::Vault(e))?;

        if let Some((project_id, environment_id)) = link {
            db.add_item_owner(new_id, project_id)
                .await
                .map_err(|e| ShareError::Vault(e))?;

            let key = plain.name.trim();
            let key_is_safe = !key.is_empty() && !key.contains('=') && !key.contains(['\n', '\r']);

            if key_is_safe && !existing_keys.contains(key) {
                db.upsert_environment_var(environment_id, key, new_id)
                    .await
                    .map_err(|e| ShareError::Vault(e))?;
            } else {
                skipped_keys.push(plain.name.clone());
            }
        }

        names.push(plain.name.clone());
    }

    Ok(ImportOutcome { names, skipped_keys })
}

/// Confirm (or reject) the fingerprint for the active session.
/// Moving to Active state unblocks the background task to proceed with the transfer.
pub async fn confirm_fingerprint(
    share_state: &Arc<ShareState>,
    confirmed: bool,
) -> Result<(), ShareError> {
    let mut guard = share_state.session.lock().await;
    match guard.as_mut() {
        None => Err(ShareError::InvalidState("no active session".into())),
        Some(s) => {
            if s.state != ShareSessionState::AwaitingFingerprint {
                return Err(ShareError::InvalidState(format!(
                    "session is in state {:?}, not AwaitingFingerprint",
                    s.state
                )));
            }
            s.set_state(if confirmed {
                ShareSessionState::Active
            } else {
                ShareSessionState::Cancelled
            });
            Ok(())
        }
    }
}

/// Cancel the active session immediately: aborts blocking socket reads, ends the
/// background tasks (which close the listener and unregister mDNS), and drops the
/// key copies. A session that already finished keeps its result state.
pub async fn cancel_session(share_state: &Arc<ShareState>) -> Result<(), ShareError> {
    let mut guard = share_state.session.lock().await;
    cancel_locked(guard.as_mut())
}

fn cancel_locked(session: Option<&mut ShareSession>) -> Result<(), ShareError> {
    match session {
        None => Err(ShareError::InvalidState("no active session".into())),
        Some(s) => {
            s.cancel.store(true, Ordering::Relaxed);
            if s.state.is_terminal() {
                s.vault_key = None;
                s.shared_key = None;
            } else {
                s.set_state(ShareSessionState::Cancelled);
            }
            Ok(())
        }
    }
}

/// Synchronous variant for `VaultState::set_key` (which cannot await): cancels
/// immediately when the session slot is free, otherwise hands the cancel to the
/// runtime. Needed when the vault key changes while a session holds a key copy.
pub fn cancel_all_blocking(share_state: &Arc<ShareState>) {
    if let Ok(mut guard) = share_state.session.try_lock() {
        let _ = cancel_locked(guard.as_mut());
        return;
    }
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let share = share_state.clone();
        handle.spawn(async move { cancel_all(&share).await });
    }
}

/// Cancel whatever session exists; used when the vault locks.
pub async fn cancel_all(share_state: &Arc<ShareState>) {
    let _ = cancel_session(share_state).await;
}

/// Export selected items as an encrypted package file.
/// Returns (passphrase, path).
pub async fn export_package(
    item_ids: &[i64],
    output_path: &std::path::Path,
    vault_state: &Arc<Mutex<crate::vault::VaultState>>,
) -> Result<String, ShareError> {
    let passphrase = crypto::generate_passphrase();

    let (vault_key, raw_items) = {
        let guard = vault_state.lock().await;
        let k = guard
            .key
            .as_ref()
            .ok_or_else(|| ShareError::Vault("vault is locked".into()))?;
        let key: [u8; 32] = **k;
        let raw = guard
            .db
            .list_items()
            .await
            .map_err(|e| ShareError::Vault(e))?;
        (key, raw)
    };

    let mut plain_items = Vec::new();
    for (id, _, data, _, _) in &raw_items {
        if !item_ids.contains(id) {
            continue;
        }
        let plaintext = crate::crypto::decrypt(&vault_key, data)
            .map_err(|e| ShareError::Vault(e))?;
        let item: crate::vault::VaultItem = serde_json::from_slice(&plaintext)
            .map_err(|e| ShareError::Protocol(e.to_string()))?;
        plain_items.push(PlainItem {
            item_type: item.item_type,
            name: item.name.unwrap_or_default(),
            value: item.value,
            username: item.username,
            password: item.password,
            url: item.url,
            notes: item.notes,
            category: item.categories.and_then(|v| v.into_iter().next()),
            command: item.command,
        });
    }

    // Run the file write on a blocking thread (file I/O)
    let pass_clone = passphrase.clone();
    let path_clone = output_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        package::export_package(&plain_items, &path_clone, &pass_clone)
    })
    .await
    .map_err(|e| ShareError::Io(e.to_string()))??;

    // Log
    {
        let guard = vault_state.lock().await;
        let _ = guard.db.log_share("package", "sending", item_ids, None).await;
    }

    Ok(passphrase)
}

/// Import an encrypted package file into the vault. `link`, when
/// `Some((project_id, environment_id))`, owns and links each imported item
/// into that project/environment the same way LAN/relay receive do (see
/// `import_plain_items_into_vault`) — the Tauri GUI import flow has no such
/// scope and passes `None`.
pub async fn import_package(
    path: &std::path::Path,
    passphrase: &str,
    vault_state: &Arc<Mutex<crate::vault::VaultState>>,
    link: Option<(i64, i64)>,
) -> Result<ImportOutcome, ShareError> {
    let path_clone = path.to_path_buf();
    let pass_clone = passphrase.to_string();

    // Decrypt package on a blocking thread
    let plain_items = tokio::task::spawn_blocking(move || {
        package::import_package(&path_clone, &pass_clone)
    })
    .await
    .map_err(|e| ShareError::Io(e.to_string()))??;

    // Import into vault
    let vault_key: [u8; 32] = {
        let guard = vault_state.lock().await;
        let k = guard
            .key
            .as_ref()
            .ok_or_else(|| ShareError::Vault("vault is locked".into()))?;
        **k
    };

    let guard = vault_state.lock().await;
    let outcome = import_plain_items_into_vault(&plain_items, &vault_key, &guard.db, link).await?;

    // Log
    let _ = guard.db.log_share("package", "receiving", &[], None).await;

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::VaultDb;

    fn plain(name: &str, value: &str) -> PlainItem {
        PlainItem {
            item_type: "secret".to_string(),
            name: name.to_string(),
            value: Some(value.to_string()),
            username: None,
            password: None,
            url: None,
            notes: None,
            category: None,
            command: None,
        }
    }

    async fn test_db() -> (tempfile::TempDir, VaultDb, [u8; 32]) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        let db = VaultDb::open(path.to_str().unwrap()).await.unwrap();
        let (_, _, key) = crate::crypto::init_vault_crypto(b"pw").unwrap();
        (dir, db, key)
    }

    #[tokio::test]
    async fn import_without_link_creates_unowned_items() {
        let (_dir, db, key) = test_db().await;
        let items = vec![plain("DB_HOST", "localhost")];

        let outcome = import_plain_items_into_vault(&items, &key, &db, None).await.unwrap();

        assert_eq!(outcome.names, vec!["DB_HOST".to_string()]);
        assert!(outcome.skipped_keys.is_empty());
        let raw = db.list_items().await.unwrap();
        assert_eq!(raw.len(), 1);
        let (id, ..) = raw[0].clone();
        assert!(db.list_owning_projects(id).await.unwrap().is_empty(), "no link => no ownership grant");
    }

    #[tokio::test]
    async fn import_with_link_grants_ownership_and_links_the_environment_var() {
        let (_dir, db, key) = test_db().await;
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let env_id = db.upsert_environment(0, project_id, "production", true).await.unwrap();
        let items = vec![plain("DB_HOST", "localhost")];

        let outcome = import_plain_items_into_vault(&items, &key, &db, Some((project_id, env_id))).await.unwrap();

        assert!(outcome.skipped_keys.is_empty());
        let vars = db.get_environment_vars(env_id).await.unwrap();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].key, "DB_HOST");
        let item_id = vars[0].item_id.unwrap();
        assert!(db.list_owning_projects(item_id).await.unwrap().contains(&project_id));
    }

    #[tokio::test]
    async fn import_skips_relinking_a_key_already_linked_to_a_different_item() {
        let (_dir, db, key) = test_db().await;
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let env_id = db.upsert_environment(0, project_id, "production", true).await.unwrap();
        // Pre-existing link the receiver already had.
        db.upsert_environment_var(env_id, "DB_HOST", 12345).await.unwrap();

        let items = vec![plain("DB_HOST", "incoming-value")];
        let outcome = import_plain_items_into_vault(&items, &key, &db, Some((project_id, env_id))).await.unwrap();

        assert_eq!(outcome.skipped_keys, vec!["DB_HOST".to_string()], "must not silently repoint an existing link");
        // The item is still imported (and owned) even though not linked.
        assert_eq!(outcome.names, vec!["DB_HOST".to_string()]);
        let vars = db.get_environment_vars(env_id).await.unwrap();
        assert_eq!(vars[0].item_id, Some(12345), "existing link must be untouched");
    }

    #[tokio::test]
    async fn import_skips_linking_an_unsafe_key_but_still_imports_the_item() {
        let (_dir, db, key) = test_db().await;
        let project_id = db.upsert_project(0, "demo", None, "generic").await.unwrap();
        let env_id = db.upsert_environment(0, project_id, "production", true).await.unwrap();
        let mut bad = plain("BAD=KEY", "value");
        bad.name = "BAD=KEY".to_string(); // contains '=', corrupts a KEY=value line

        let outcome = import_plain_items_into_vault(&[bad], &key, &db, Some((project_id, env_id))).await.unwrap();

        assert_eq!(outcome.skipped_keys, vec!["BAD=KEY".to_string()]);
        assert!(db.get_environment_vars(env_id).await.unwrap().is_empty());
        assert_eq!(db.list_items().await.unwrap().len(), 1, "item is still imported, just not linked");
    }

    #[tokio::test]
    async fn import_multiple_items_preserves_order_in_names() {
        let (_dir, db, key) = test_db().await;
        let items = vec![plain("A", "1"), plain("B", "2"), plain("C", "3")];

        let outcome = import_plain_items_into_vault(&items, &key, &db, None).await.unwrap();

        assert_eq!(outcome.names, vec!["A".to_string(), "B".to_string(), "C".to_string()]);
    }

    #[tokio::test]
    async fn imported_items_are_never_global() {
        let (_dir, db, key) = test_db().await;
        let items = vec![plain("DB_HOST", "localhost")];

        import_plain_items_into_vault(&items, &key, &db, None).await.unwrap();

        let raw = db.list_items().await.unwrap();
        let (_, _, _, _, is_global) = raw[0].clone();
        assert!(!is_global, "imported items must never be created as global");
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;
    use crate::db::VaultDb;
    use std::net::{SocketAddr, TcpListener, TcpStream};

    fn fast_cfg() -> lan::LanConfig {
        lan::LanConfig {
            io_timeout: Duration::from_millis(400),
            handshake_timeout: Duration::from_secs(5),
            cancel: None,
        }
    }

    fn key() -> Zeroizing<[u8; 32]> {
        Zeroizing::new([7u8; 32])
    }

    async fn start(state: &Arc<ShareState>) -> Result<(u64, Arc<AtomicBool>), ShareError> {
        begin_session(
            state,
            "123456".into(),
            ShareDirection::Sending,
            ShareSessionState::Listening,
            vec![],
            key(),
            None,
            None,
        )
        .await
    }

    fn local_listener() -> (TcpListener, SocketAddr) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        (l, addr)
    }

    /// Receiver-side handshake against `addr` using `code`, on a blocking thread.
    async fn peer_handshake(
        addr: SocketAddr,
        code: &'static str,
    ) -> Result<(Zeroizing<[u8; 32]>, String), ShareError> {
        tokio::task::spawn_blocking(move || {
            let mut s = TcpStream::connect(addr).map_err(|e| ShareError::Io(e.to_string()))?;
            // Generous timeout: the listener only polls for connections every 500 ms.
            let cfg = lan::LanConfig { io_timeout: Duration::from_secs(5), ..fast_cfg() };
            lan::pair_handshake(&mut s, code, PairRole::Receiver, &cfg)
        })
        .await
        .unwrap()
    }

    type PairResult =
        Result<Option<(std::net::TcpStream, Zeroizing<[u8; 32]>, String)>, ShareError>;

    fn spawn_accept(
        state: &Arc<ShareState>,
        id: u64,
        cancel: Arc<AtomicBool>,
        listener: TcpListener,
    ) -> tokio::task::JoinHandle<PairResult> {
        let state = state.clone();
        tokio::spawn(async move {
            let cfg = fast_cfg().with_cancel(cancel);
            accept_and_pair(&state, id, &listener, "123456", &cfg, Duration::from_secs(20)).await
        })
    }

    #[tokio::test]
    async fn second_start_is_rejected_while_first_is_active() {
        let state = Arc::new(ShareState::new());
        start(&state).await.unwrap();
        assert!(matches!(start(&state).await, Err(ShareError::SessionActive)));
        // The existing session is unaffected.
        let guard = state.session.lock().await;
        assert_eq!(guard.as_ref().unwrap().state, ShareSessionState::Listening);
    }

    #[tokio::test]
    async fn new_session_is_allowed_after_the_previous_one_ended() {
        let state = Arc::new(ShareState::new());
        let (a, _) = start(&state).await.unwrap();
        cancel_session(&state).await.unwrap();
        let (b, _) = start(&state).await.unwrap();
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn successful_pairing_yields_matching_fingerprints() {
        let state = Arc::new(ShareState::new());
        let (id, cancel) = start(&state).await.unwrap();
        let (listener, addr) = local_listener();
        let task = spawn_accept(&state, id, cancel, listener);

        let (_, peer_fp) = peer_handshake(addr, "123456").await.unwrap();
        let (_, _, sender_fp) = task.await.unwrap().unwrap().unwrap();
        assert_eq!(peer_fp, sender_fp);
    }

    #[tokio::test]
    async fn wrong_code_peers_hit_the_attempt_limit_and_abort_the_session() {
        let state = Arc::new(ShareState::new());
        let (id, cancel) = start(&state).await.unwrap();
        let (listener, addr) = local_listener();
        let task = spawn_accept(&state, id, cancel, listener);

        for _ in 0..MAX_FAILED_ATTEMPTS {
            assert!(matches!(
                peer_handshake(addr, "000000").await,
                Err(ShareError::PairingFailed)
            ));
        }
        assert!(task.await.unwrap().unwrap().is_none());
        let guard = state.session.lock().await;
        let s = guard.as_ref().unwrap();
        assert!(matches!(&s.state, ShareSessionState::Failed(m) if m.contains("too many")));
        assert!(s.fingerprint.is_none(), "no pairing completed");
        assert!(s.vault_key.is_none(), "key copy dropped when the session ends");
    }

    #[tokio::test]
    async fn silent_peer_is_dropped_and_session_keeps_waiting() {
        let state = Arc::new(ShareState::new());
        let (id, cancel) = start(&state).await.unwrap();
        let (listener, addr) = local_listener();
        let task = spawn_accept(&state, id, cancel, listener);

        let _silent = TcpStream::connect(addr).unwrap();
        // After the silent peer's io timeout, a legitimate peer still pairs.
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert!(peer_handshake(addr, "123456").await.is_ok());
        assert!(task.await.unwrap().unwrap().is_some());
        let guard = state.session.lock().await;
        assert_eq!(
            guard.as_ref().unwrap().failed_attempts,
            0,
            "silence is not a pairing attempt"
        );
    }

    #[tokio::test]
    async fn stale_task_does_nothing_after_cancel_and_a_new_session() {
        let state = Arc::new(ShareState::new());
        let (a, cancel_a) = start(&state).await.unwrap();
        let (listener, addr) = local_listener();
        let task_a = spawn_accept(&state, a, cancel_a, listener);

        cancel_session(&state).await.unwrap();
        let (b, _) = start(&state).await.unwrap();

        // Task A exits without pairing, and without touching session B.
        assert!(task_a.await.unwrap().unwrap().is_none());
        // Its listener is gone: nobody can pair with the cancelled session.
        assert!(TcpStream::connect(addr).is_err());
        let guard = state.session.lock().await;
        let s = guard.as_ref().unwrap();
        assert_eq!(s.id, b);
        assert_eq!(s.state, ShareSessionState::Listening);
        assert!(s.fingerprint.is_none());
        assert_eq!(s.failed_attempts, 0);
    }

    #[tokio::test]
    async fn cancel_interrupts_a_handshake_in_progress() {
        let state = Arc::new(ShareState::new());
        let (id, cancel) = start(&state).await.unwrap();
        let (listener, addr) = local_listener();
        let cfg = lan::LanConfig {
            io_timeout: Duration::from_secs(30),
            handshake_timeout: Duration::from_secs(60),
            cancel: Some(cancel),
        };
        let st = state.clone();
        let task = tokio::spawn(async move {
            accept_and_pair(&st, id, &listener, "123456", &cfg, Duration::from_secs(20)).await
        });
        let _silent = TcpStream::connect(addr).unwrap();
        tokio::time::sleep(Duration::from_millis(800)).await;

        let started = Instant::now();
        cancel_session(&state).await.unwrap();
        assert!(task.await.unwrap().unwrap().is_none());
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[tokio::test]
    async fn lock_cancels_session_closes_listener_and_drops_key() {
        let dir = tempfile::tempdir().unwrap();
        let db = VaultDb::open(dir.path().join("vault.db").to_str().unwrap())
            .await
            .unwrap();
        let vault: crate::vault::SharedState =
            Arc::new(Mutex::new(crate::vault::VaultState::new(db)));
        let share = vault.lock().await.share.clone();

        let (id, cancel) = start(&share).await.unwrap();
        let (listener, addr) = local_listener();
        let task = spawn_accept(&share, id, cancel, listener);
        assert!(share.session.lock().await.as_ref().unwrap().vault_key.is_some());

        crate::vault::lock_vault(&vault).await;

        assert!(task.await.unwrap().unwrap().is_none());
        assert!(TcpStream::connect(addr).is_err(), "listener port must be closed");
        let guard = share.session.lock().await;
        let s = guard.as_ref().unwrap();
        assert_eq!(s.state, ShareSessionState::Cancelled);
        assert!(s.vault_key.is_none());
        assert!(s.shared_key.is_none());
    }

    #[tokio::test]
    async fn finishing_a_session_drops_its_key_copies() {
        let state = Arc::new(ShareState::new());
        let (id, _) = start(&state).await.unwrap();
        fail_session(&state, id, "boom".into()).await;
        let guard = state.session.lock().await;
        assert!(guard.as_ref().unwrap().vault_key.is_none());
    }
}
