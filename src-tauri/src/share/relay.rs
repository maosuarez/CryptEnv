use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::package::PlainItem;
use super::ShareError;
use crate::crypto::VaultKey;
use crate::share::crypto::{decrypt_message, encrypt_message};

// ─── Relay session structs ────────────────────────────────────────────────────

#[derive(Serialize)]
struct RelayPutArgs<'a> {
    p_code_hash: &'a str,
    p_payload: &'a str,
}

#[derive(Serialize)]
struct RelayClaimArgs<'a> {
    p_code_hash: &'a str,
}

/// PostgREST error body. Only the machine code is ever read — never the
/// message/details/hint, which could echo request data.
#[derive(Deserialize)]
struct PostgrestError {
    code: Option<String>,
}

/// Process-wide limit on concurrent Argon2id relay derivations (32 MiB each).
static RELAY_KDF: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

/// Relay schema version this client speaks (see `relay_schema_version()` in the SQL).
pub const RELAY_SCHEMA_VERSION: i32 = 2;

// ─── Key derivation ───────────────────────────────────────────────────────────

/// Derive a 32-byte relay encryption key from (code, passphrase) deterministically.
/// salt = SHA-256(code) so both sides can reproduce without extra transmission.
pub fn derive_relay_key(code: &str, passphrase: &str) -> Result<VaultKey, ShareError> {
    let salt: [u8; 32] = Sha256::digest(code.as_bytes()).into(); // key-hygiene: not-a-key (salt)
    let params = Params::new(32768, 2, 2, Some(32))
        .map_err(|e| ShareError::Crypto(e.to_string()))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(passphrase.as_bytes(), &salt, &mut *key)
        .map_err(|e| ShareError::Crypto(e.to_string()))?;
    Ok(VaultKey::new(key))
}

/// Runs `f` on a blocking thread after acquiring a permit from `sem`, so at
/// most `sem`'s permits run concurrently and none run on async worker threads.
async fn run_bounded<T, F>(sem: &'static tokio::sync::Semaphore, f: F) -> Result<T, ShareError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ShareError> + Send + 'static,
{
    let _permit = sem
        .acquire()
        .await
        .map_err(|e| ShareError::Crypto(e.to_string()))?;
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ShareError::Crypto(format!("key derivation task failed: {e}")))?
}

/// Async, bounded wrapper around `derive_relay_key`. All async callers MUST use
/// this one: it keeps Argon2id off the runtime workers and caps concurrency at 2.
pub async fn derive_relay_key_async(code: &str, passphrase: &str) -> Result<VaultKey, ShareError> {
    let code = code.to_owned();
    let passphrase = Zeroizing::new(passphrase.to_owned());
    run_bounded(&RELAY_KDF, move || derive_relay_key(&code, &passphrase)).await
}

/// Lookup hash stored by the relay instead of the code itself:
/// hex(SHA-256("cryptenv-relay-lookup-v1:" || code)). The prefix keeps it
/// distinct from the Argon2 salt (SHA-256(code)).
pub fn code_hash(code: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"cryptenv-relay-lookup-v1:");
    h.update(code.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

// ─── Payload encryption ───────────────────────────────────────────────────────

pub fn encrypt_items(items: &[PlainItem], key: &VaultKey) -> Result<String, ShareError> {
    let json = Zeroizing::new(
        serde_json::to_vec(items).map_err(|e| ShareError::Protocol(e.to_string()))?,
    );
    let ciphertext = encrypt_message(key, &json);
    Ok(B64.encode(&ciphertext))
}

pub fn decrypt_payload(payload: &str, key: &VaultKey) -> Result<Vec<PlainItem>, ShareError> {
    let ciphertext = B64
        .decode(payload)
        .map_err(|e| ShareError::Protocol(format!("base64 decode: {e}")))?;
    let plaintext = decrypt_message(key, &ciphertext)?;
    serde_json::from_slice(&plaintext).map_err(|e| ShareError::Protocol(e.to_string()))
}

// ─── Project bundle (structure + values for N environments at once) ──────────
// A project bundle carries an entire project's definition — its environments
// and the decrypted values of every item they reference — so a teammate can
// reconstruct a ready-to-inject multi-environment project in one step. Items
// are hoisted to the bundle root and deduped by name, referenced from each
// environment's vars by name (see docs/plans/issue-4 D1): this is what lets
// "the same item linked into 3 environments" be told apart from "3 items
// that happen to share a name" on receive, which the old per-environment
// `WorkspaceBundle` shape (removed) could not distinguish.

/// One variable in a shared environment — always references a bundled item
/// by name. Unlike the legacy workspace format, there is no inline literal:
/// `environment_vars.item_id` is mandatory post-migration, so every var this
/// bundle can even represent already resolves to a real item (D3).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectBundleVar {
    pub key: String,
    pub item_name: String,
}

/// One environment's shape, values-free of anything machine-specific.
/// Deliberately has no `paths` field (D6) — absolute filesystem paths from
/// the sender's machine are meaningless (and identity-leaking) on the
/// receiver's, so the field does not exist in the wire type rather than
/// being included-but-ignored.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EnvironmentBundle {
    pub name: String,
    pub is_default: bool,
    pub vars: Vec<ProjectBundleVar>,
}

/// A complete, self-contained project ready to import: N environments plus
/// the deduped set of items they reference.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectBundle {
    /// Discriminator so a project payload is never mistaken for a bare item list.
    pub kind: String,
    /// Format version, checked on decrypt (D2) — bumping it is free now and
    /// impossible to retrofit later, so it's checked from day one even
    /// though only version 1 currently exists.
    pub version: u32,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub template: String,
    pub environments: Vec<EnvironmentBundle>,
    /// Decrypted values, deduped by name. Matched to vars by `item_name`.
    pub items: Vec<PlainItem>,
}

impl ProjectBundle {
    pub const KIND: &'static str = "project";
    pub const VERSION: u32 = 1;
}

pub fn encrypt_project(bundle: &ProjectBundle, key: &VaultKey) -> Result<String, ShareError> {
    let json = Zeroizing::new(
        serde_json::to_vec(bundle).map_err(|e| ShareError::Protocol(e.to_string()))?,
    );
    let ciphertext = encrypt_message(key, &json);
    Ok(B64.encode(&ciphertext))
}

/// Decrypts and validates a project bundle: rejects a wrong `kind` (e.g. a
/// payload produced by `encrypt_items`) and an unknown `version` before
/// returning, so a format change never has to repeat this discriminator dance.
pub fn decrypt_project(payload: &str, key: &VaultKey) -> Result<ProjectBundle, ShareError> {
    let ciphertext = B64
        .decode(payload)
        .map_err(|e| ShareError::Protocol(format!("base64 decode: {e}")))?;
    let plaintext = decrypt_message(key, &ciphertext)?;
    let bundle: ProjectBundle =
        serde_json::from_slice(&plaintext).map_err(|e| ShareError::Protocol(e.to_string()))?;
    if bundle.kind != ProjectBundle::KIND {
        return Err(ShareError::Protocol(
            "this code is not a project package (use the items receive flow instead)".into(),
        ));
    }
    if bundle.version != ProjectBundle::VERSION {
        return Err(ShareError::Protocol(format!(
            "this package was created by a newer version of CryptEnv (format v{}); update to receive it",
            bundle.version
        )));
    }
    Ok(bundle)
}

// ─── Code generation ──────────────────────────────────────────────────────────

pub fn generate_share_code() -> String {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::thread_rng();
    let mut buf = [0u8; 8];
    rng.fill_bytes(&mut buf);
    let chars: String = buf
        .iter()
        .map(|b| ALPHA[(*b as usize) % ALPHA.len()] as char)
        .collect();
    format!("{}-{}", &chars[..4], &chars[4..])
}

// ─── Supabase relay calls (blocking reqwest, SQL v2 RPCs) ─────────────────────

fn relay_client() -> Result<reqwest::blocking::Client, ShareError> {
    reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| ShareError::Io(e.to_string()))
}

/// POST to `/rest/v1/rpc/<function>`. A missing function (HTTP 404 /
/// PGRST202) means the project still has the v1 schema.
fn relay_rpc<B: Serialize>(
    supabase_url: &str,
    anon_key: &str,
    function: &str,
    body: &B,
) -> Result<reqwest::blocking::Response, ShareError> {
    let res = relay_client()?
        .post(format!(
            "{}/rest/v1/rpc/{}",
            supabase_url.trim_end_matches('/'),
            function
        ))
        .header("apikey", anon_key)
        .header("Authorization", format!("Bearer {}", anon_key))
        .header("Content-Type", "application/json")
        .json(body)
        .send()
        .map_err(|e| ShareError::Io(format!("relay {function} request failed: {}", e.without_url())))?;

    if res.status().is_success() {
        return Ok(res);
    }
    let status = res.status().as_u16();
    let pg_code = res
        .json::<PostgrestError>()
        .ok()
        .and_then(|e| e.code)
        .unwrap_or_default();
    if status == 404 || pg_code == "PGRST202" {
        return Err(ShareError::RelaySchemaOutdated);
    }
    if pg_code.is_empty() {
        Err(ShareError::Io(format!("relay {function} failed ({status})")))
    } else {
        Err(ShareError::Io(format!("relay {function} failed ({status}, {pg_code})")))
    }
}

/// Uploads a package via `relay_put`. The relay stores only `code_hash(code)`
/// and enforces the 1 MiB cap and 24 h expiry.
pub fn relay_upload(
    supabase_url: &str,
    anon_key: &str,
    code: &str,
    payload: &str,
) -> Result<(), ShareError> {
    let hash = code_hash(code);
    relay_rpc(
        supabase_url,
        anon_key,
        "relay_put",
        &RelayPutArgs { p_code_hash: &hash, p_payload: payload },
    )?;
    Ok(())
}

/// Atomically claims a package via `relay_claim`: the relay deletes the row
/// and returns the payload only if it exists and has not expired. This is
/// also the burn-after-read step; there is no separate delete.
pub fn relay_claim(
    supabase_url: &str,
    anon_key: &str,
    code: &str,
) -> Result<String, ShareError> {
    let hash = code_hash(code);
    let res = relay_rpc(
        supabase_url,
        anon_key,
        "relay_claim",
        &RelayClaimArgs { p_code_hash: &hash },
    )?;
    let payload: Option<String> = res
        .json()
        .map_err(|e| ShareError::Protocol(format!("relay claim: unexpected response ({e})")))?;
    payload.ok_or_else(|| ShareError::Remote("code not found or already used".into()))
}

/// Probes `relay_schema_version()`. Returns `RelaySchemaOutdated` on a v1 project.
pub fn relay_schema_version(supabase_url: &str, anon_key: &str) -> Result<i32, ShareError> {
    let res = relay_rpc(supabase_url, anon_key, "relay_schema_version", &serde_json::json!({}))?;
    let v: i32 = res
        .json()
        .map_err(|e| ShareError::Protocol(format!("relay schema probe: unexpected response ({e})")))?;
    if v < RELAY_SCHEMA_VERSION {
        return Err(ShareError::RelaySchemaOutdated);
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bundle() -> ProjectBundle {
        ProjectBundle {
            kind: ProjectBundle::KIND.to_string(),
            version: ProjectBundle::VERSION,
            name: "MyApp".to_string(),
            description: Some("a sample project".to_string()),
            template: "node".to_string(),
            environments: vec![
                EnvironmentBundle {
                    name: "local".to_string(),
                    is_default: true,
                    vars: vec![
                        ProjectBundleVar { key: "DB_HOST".to_string(), item_name: "db-host".to_string() },
                        ProjectBundleVar { key: "DB_PASSWORD".to_string(), item_name: "db-password".to_string() },
                    ],
                },
                EnvironmentBundle {
                    name: "production".to_string(),
                    is_default: false,
                    vars: vec![ProjectBundleVar {
                        key: "DB_HOST".to_string(),
                        item_name: "db-host".to_string(),
                    }],
                },
            ],
            items: vec![
                PlainItem {
                    item_type: "secret".to_string(),
                    name: "db-host".to_string(),
                    value: Some("localhost".to_string()),
                    username: None,
                    password: None,
                    url: None,
                    notes: None,
                    category: None,
                    command: None,
                },
                PlainItem {
                    item_type: "secret".to_string(),
                    name: "db-password".to_string(),
                    value: Some("hunter2".to_string()),
                    username: None,
                    password: None,
                    url: None,
                    notes: None,
                    category: None,
                    command: None,
                },
            ],
        }
    }

    #[test]
    fn project_bundle_roundtrip_preserves_structure() {
        let bundle = sample_bundle();
        let key = derive_relay_key("TEST-CODE", "correct horse battery staple").unwrap();

        let payload = encrypt_project(&bundle, &key).unwrap();
        let decrypted = decrypt_project(&payload, &key).unwrap();

        assert_eq!(decrypted.environments.len(), bundle.environments.len());
        for (a, b) in decrypted.environments.iter().zip(bundle.environments.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.is_default, b.is_default);
            let a_keys: Vec<&str> = a.vars.iter().map(|v| v.key.as_str()).collect();
            let b_keys: Vec<&str> = b.vars.iter().map(|v| v.key.as_str()).collect();
            assert_eq!(a_keys, b_keys);
        }
        let mut a_names: Vec<&str> = decrypted.items.iter().map(|i| i.name.as_str()).collect();
        let mut b_names: Vec<&str> = bundle.items.iter().map(|i| i.name.as_str()).collect();
        a_names.sort();
        b_names.sort();
        assert_eq!(a_names, b_names);
    }

    #[test]
    fn decrypt_project_rejects_items_payload() {
        let key = derive_relay_key("TEST-CODE", "correct horse battery staple").unwrap();
        let items = vec![PlainItem {
            item_type: "secret".to_string(),
            name: "loose-item".to_string(),
            value: Some("x".to_string()),
            username: None,
            password: None,
            url: None,
            notes: None,
            category: None,
            command: None,
        }];
        let payload = encrypt_items(&items, &key).unwrap();

        let result = decrypt_project(&payload, &key);
        match result {
            Err(ShareError::Protocol(_)) => {}
            other => panic!("expected ShareError::Protocol, got {other:?}"),
        }
    }

    #[test]
    fn decrypt_project_rejects_unknown_version() {
        let key = derive_relay_key("TEST-CODE", "correct horse battery staple").unwrap();
        let mut bundle = sample_bundle();
        bundle.version = 99;
        let payload = encrypt_project(&bundle, &key).unwrap();

        let err = decrypt_project(&payload, &key).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("v99"), "message should name the unsupported version, got: {msg}");
    }

    #[test]
    fn decrypt_project_rejects_wrong_passphrase() {
        let bundle = sample_bundle();
        let key = derive_relay_key("TEST-CODE", "correct horse battery staple").unwrap();
        let payload = encrypt_project(&bundle, &key).unwrap();

        let wrong_key = derive_relay_key("TEST-CODE", "totally different passphrase").unwrap();
        let result = decrypt_project(&payload, &wrong_key);
        match result {
            Err(ShareError::Crypto(_)) => {}
            other => panic!("expected ShareError::Crypto, got {other:?}"),
        }
    }

    // ─── Relay RPC tests (local mock HTTP server) ───────────────────────────

    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serves each canned (status, body) response to one connection, in order,
    /// and returns the base URL plus the raw requests received.
    fn mock_server(
        responses: Vec<(u16, &'static str)>,
    ) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut sock, _) = listener.accept().unwrap();
                let mut buf = vec![0u8; 8192];
                let n = sock.read(&mut buf).unwrap();
                let _ = tx.send(String::from_utf8_lossy(&buf[..n]).to_string());
                let resp = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                sock.write_all(resp.as_bytes()).unwrap();
            }
        });
        (url, rx)
    }

    #[test]
    fn code_hash_is_domain_separated_hex() {
        let h = code_hash("ABCD-1234");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
        let salt: [u8; 32] = Sha256::digest(b"ABCD-1234").into(); // key-hygiene: not-a-key (salt)
        let salt_hex: String = salt.iter().map(|b| format!("{b:02x}")).collect();
        assert_ne!(h, salt_hex);
        assert!(!h.contains("ABCD"));
    }

    #[test]
    fn claim_returns_payload_once_and_sends_hash_not_code() {
        let (url, rx) = mock_server(vec![(200, "\"c2VjcmV0\""), (200, "null")]);
        let first = relay_claim(&url, "anon", "ABCD-1234").unwrap();
        assert_eq!(first, "c2VjcmV0");
        let req = rx.recv().unwrap();
        assert!(req.starts_with("POST /rest/v1/rpc/relay_claim"));
        assert!(req.contains(&code_hash("ABCD-1234")));
        assert!(!req.contains("ABCD-1234"));
        match relay_claim(&url, "anon", "ABCD-1234") {
            Err(ShareError::Remote(m)) => assert!(m.contains("not found or already used")),
            other => panic!("expected Remote, got {other:?}"),
        }
    }

    #[test]
    fn missing_rpc_maps_to_schema_outdated() {
        let (url, _rx) = mock_server(vec![
            (404, "{\"code\":\"PGRST202\",\"message\":\"nope\"}"),
            (404, "{}"),
            (400, "{\"code\":\"PGRST202\"}"),
        ]);
        assert!(matches!(relay_claim(&url, "k", "A-B"), Err(ShareError::RelaySchemaOutdated)));
        assert!(matches!(relay_upload(&url, "k", "A-B", "p"), Err(ShareError::RelaySchemaOutdated)));
        assert!(matches!(relay_schema_version(&url, "k"), Err(ShareError::RelaySchemaOutdated)));
    }

    #[test]
    fn other_failures_surface_status_without_payload() {
        let (url, _rx) = mock_server(vec![(500, "{\"code\":\"XX000\",\"message\":\"secret-payload\"}")]);
        match relay_upload(&url, "k", "A-B", "secret-payload") {
            Err(ShareError::Io(m)) => {
                assert!(m.contains("500") && m.contains("XX000"));
                assert!(!m.contains("secret-payload"));
            }
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[test]
    fn schema_version_ok_on_v2() {
        let (url, _rx) = mock_server(vec![(200, "2")]);
        assert_eq!(relay_schema_version(&url, "k").unwrap(), 2);
    }

    #[tokio::test]
    async fn kdf_semaphore_limits_concurrency_to_two() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        static SEM: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..10 {
            let (running, peak) = (running.clone(), peak.clone());
            tasks.push(tokio::spawn(run_bounded(&SEM, move || {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(40));
                running.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })));
        }
        for t in tasks {
            t.await.unwrap().unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }
}
