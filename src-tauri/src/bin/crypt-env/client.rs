use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Built-in REST base URL, used when `CRYPTENV_API_URL` is unset or empty.
pub const DEFAULT_API_BASE: &str = "https://127.0.0.1:47821";

/// Process-wide resolved REST base URL. Set once by [`init_api_base`] at
/// command entry so every request in an invocation targets the same endpoint.
static API_BASE_CACHE: OnceLock<String> = OnceLock::new();

// ─── Error types ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum CliError {
    Api(String),
    Io(std::io::Error),
    ConnectionRefused,
    Unauthorized,
    NotFound(String),
    VaultLocked,
    /// Invalid runtime configuration (malformed `CRYPTENV_API_URL`, unreadable
    /// `CRYPTENV_CERT_PATH`, `setup wsl` preconditions). Messages name only
    /// environment-variable names and file paths — never secret values.
    Config(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::Api(msg) => write!(f, "API error: {msg}"),
            CliError::Io(e) => write!(f, "I/O error: {e}"),
            CliError::ConnectionRefused => write!(
                f,
                "Error: vault is not running. Open the application and try again."
            ),
            CliError::Unauthorized => write!(f, "Error: unauthorized (invalid token)"),
            CliError::NotFound(name) => write!(f, "Error: '{}' not found in vault", name),
            CliError::VaultLocked => write!(f, "Error: vault is locked"),
            CliError::Config(msg) => write!(f, "Configuration error: {msg}"),
        }
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::Io(e)
    }
}

// ─── REST endpoint resolution ────────────────────────────────────────────────

/// Outcome of endpoint resolution, before the non-loopback warning is emitted.
#[derive(Debug)]
struct ApiBaseResolution {
    /// Base URL to use (trailing slashes trimmed).
    url: String,
    /// `Some(host)` when the host is not a loopback address and the caller
    /// should print a one-line stderr warning.
    non_loopback_host: Option<String>,
}

/// Pure resolver: given the raw `CRYPTENV_API_URL` value (if any), returns the
/// base URL to use, or a [`CliError::Config`] naming the variable when the value
/// is not an absolute `http`/`https` URL.
fn resolve_api_base_from(raw: Option<&str>) -> Result<ApiBaseResolution, CliError> {
    match raw {
        Some(value) if !value.trim().is_empty() => {
            let trimmed = value.trim();
            let host = http_url_host(trimmed).ok_or_else(|| {
                CliError::Config(
                    "CRYPTENV_API_URL must be an absolute http:// or https:// URL".to_string(),
                )
            })?;
            let url = trimmed.trim_end_matches('/').to_string();
            let non_loopback_host = if host_is_loopback(&host) {
                None
            } else {
                Some(host)
            };
            Ok(ApiBaseResolution {
                url,
                non_loopback_host,
            })
        }
        _ => Ok(ApiBaseResolution {
            url: DEFAULT_API_BASE.to_string(),
            non_loopback_host: None,
        }),
    }
}

/// Resolves the REST base URL from `CRYPTENV_API_URL`, printing a single stderr
/// warning when the configured host is not a loopback address.
pub fn resolve_api_base() -> Result<String, CliError> {
    let resolution = resolve_api_base_from(std::env::var("CRYPTENV_API_URL").ok().as_deref())?;
    if let Some(host) = &resolution.non_loopback_host {
        eprintln!(
            "warning: CRYPTENV_API_URL host '{host}' is not a loopback address; \
             the vault is expected to be reachable only over localhost"
        );
    }
    Ok(resolution.url)
}

/// Resolves and caches the process-wide REST base URL. Call once at command
/// entry so a malformed `CRYPTENV_API_URL` surfaces as a normal `Result`
/// before any request is made.
pub fn init_api_base() -> Result<(), CliError> {
    let resolved = resolve_api_base()?;
    let _ = API_BASE_CACHE.set(resolved);
    Ok(())
}

/// Returns the resolved REST base URL. Falls back to [`DEFAULT_API_BASE`] when
/// [`init_api_base`] was not called (e.g. in unit tests).
pub fn api_base() -> &'static str {
    API_BASE_CACHE
        .get_or_init(|| DEFAULT_API_BASE.to_string())
        .as_str()
}

/// Extracts the lowercased host from `s` when it is an absolute `http`/`https`
/// URL with a non-empty host; otherwise `None`. Hand-rolled to avoid a new
/// dependency (see `design.md` D2).
fn http_url_host(s: &str) -> Option<String> {
    let rest = s
        .strip_prefix("http://")
        .or_else(|| s.strip_prefix("https://"))?;
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() {
        return None;
    }
    // Drop any userinfo prefix.
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    if host_port.is_empty() {
        return None;
    }
    // IPv6 literal in brackets, optionally followed by `:port`.
    let host = if let Some(after_bracket) = host_port.strip_prefix('[') {
        let end = after_bracket.find(']')?;
        &after_bracket[..end]
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };
    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

/// True when `host` is a loopback address: `localhost`, `::1`, or anything in
/// `127.0.0.0/8`.
fn host_is_loopback(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(v4) = host.parse::<std::net::Ipv4Addr>() {
        return v4.is_loopback();
    }
    if let Ok(v6) = host.parse::<std::net::Ipv6Addr>() {
        return v6.is_loopback();
    }
    false
}

// ─── API response types ───────────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
pub struct UnlockResponse {
    pub token: String,
}

#[derive(Deserialize, Debug)]
pub struct ApiError {
    pub error: String,
}

#[derive(Deserialize, Debug)]
pub struct ItemSummary {
    pub id: i64,
    #[serde(rename = "type")]
    pub item_type: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    /// Present on `/items` responses since issue #13 (defaults to `false`
    /// when absent, e.g. responses from before this change).
    #[serde(default, rename = "isGlobal")]
    pub is_global: bool,
    /// `true` iff this item is linked into the queried environment. Also
    /// new since issue #13 — defaults to `false` when absent.
    #[serde(default)]
    pub linked: bool,
}

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
pub struct CommandDetail {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub placeholders: Vec<String>,
    /// Present on `/commands` list responses since issue #13. `GET
    /// /commands/:id` (unscoped, untouched by this change) omits it, so this
    /// defaults to `false` there.
    #[serde(default, rename = "isGlobal")]
    pub is_global: bool,
    /// `true` iff this command is linked into the queried environment.
    /// Meaningless outside a scoped `/commands` list — defaults to `false`.
    #[serde(default)]
    pub linked: bool,
}

/// Given the full `/commands` list for a scope, finds the command matching
/// `name` case-insensitively. When both a linked command and a global
/// (unlinked) command share the same name, the linked one wins — matching
/// `/fill`'s and `/inject`'s materialization-only semantics — and a warning
/// naming the shadowed global's id is printed to stderr.
pub fn resolve_command_by_name(commands: Vec<CommandDetail>, name: &str) -> Option<CommandDetail> {
    let name_lower = name.to_lowercase();
    let mut matches: Vec<CommandDetail> = commands
        .into_iter()
        .filter(|c| c.name.to_lowercase() == name_lower)
        .collect();

    if matches.len() > 1 {
        if let Some(linked_idx) = matches.iter().position(|c| c.linked) {
            let linked = matches.remove(linked_idx);
            for shadowed in matches.iter().filter(|c| !c.linked) {
                eprintln!(
                    "warning: command '{}' also exists as a global item (id {}) — using the linked one",
                    name, shadowed.id
                );
            }
            return Some(linked);
        }
    }

    matches.into_iter().next()
}

#[derive(Deserialize, Debug)]
struct RevealResponse {
    value: String,
}

// ─── Session token ────────────────────────────────────────────────────────────

fn token_path() -> Option<PathBuf> {
    token_path_from(
        std::env::var("CRYPTENV_TOKEN_PATH").ok().as_deref(),
        std::env::var("APPDATA").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// Pure resolver for the session-token path: a non-empty `CRYPTENV_TOKEN_PATH`
/// wins verbatim; otherwise the historical `APPDATA` → `HOME/.local/share`
/// probing is used.
fn token_path_from(
    explicit: Option<&str>,
    appdata: Option<&str>,
    home: Option<&str>,
) -> Option<PathBuf> {
    if let Some(explicit) = explicit {
        if !explicit.trim().is_empty() {
            return Some(PathBuf::from(explicit.trim()));
        }
    }
    if let Some(appdata) = appdata {
        return Some(
            PathBuf::from(appdata)
                .join("com.maosuarez.cryptenv")
                .join(".cli_token"),
        );
    }
    if let Some(home) = home {
        return Some(
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("com.maosuarez.cryptenv")
                .join(".cli_token"),
        );
    }
    None
}

pub fn read_token() -> Option<String> {
    let path = token_path()?;
    std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Writes `content` to `path` and restricts permissions to owner-only on Unix.
///
/// On Windows, `%APPDATA%` inherits user-only NTFS ACLs from the OS, so no
/// additional ACL manipulation is needed via std. The write itself is best-effort.
fn write_token_file(path: &Path, content: &str) -> std::io::Result<()> {
    write_token_file_inner(path, content, harden_token_permissions)
}

/// Testable core of [`write_token_file`]: writes `content`, then runs `harden`
/// (best-effort). Only a failed content write is propagated — a failure inside
/// `harden` never affects the result (see `design.md` D4).
fn write_token_file_inner<F: FnOnce(&Path)>(
    path: &Path,
    content: &str,
    harden: F,
) -> std::io::Result<()> {
    std::fs::write(path, content)?;
    harden(path);
    Ok(())
}

/// Best-effort permission tightening. On non-Windows targets a `/mnt/c` (DrvFs)
/// path under WSL cannot honor Unix mode bits; the token content is already
/// written, so a failure here is logged and swallowed rather than propagated.
fn harden_token_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) =
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        {
            eprintln!(
                "warning: could not restrict permissions on {}: {e}",
                path.display()
            );
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

pub fn save_token(token: &str) {
    if let Some(path) = token_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = write_token_file(&path, token);
    }
}

pub fn clear_token() {
    if let Some(path) = token_path() {
        let _ = std::fs::remove_file(&path);
    }
}

// ─── HTTP client ──────────────────────────────────────────────────────────────

/// Where the TLS trust anchor comes from.
enum CertSource {
    /// From a non-empty `CRYPTENV_CERT_PATH`: used verbatim with no fallback —
    /// an unreadable file or invalid PEM here is a hard error (see `design.md`
    /// D3).
    Explicit(PathBuf),
    /// Probed from the platform data directories, or `None` when none could be
    /// determined. Preserves the pre-existing best-effort behavior.
    Probed(Option<PathBuf>),
}

/// Probes `APPDATA` → `XDG_DATA_HOME` → `HOME/.local/share` for the cert the
/// Tauri app writes. Mirrors the path used by `src-tauri/src/tls/mod.rs`.
fn probe_cert_path() -> Option<PathBuf> {
    // Windows: %APPDATA%\com.maosuarez.cryptenv\tls\cert.pem
    // Linux/macOS: $HOME/.local/share/com.maosuarez.cryptenv/tls/cert.pem (or XDG)
    if let Ok(appdata) = std::env::var("APPDATA") {
        return Some(
            PathBuf::from(appdata)
                .join("com.maosuarez.cryptenv")
                .join("tls")
                .join("cert.pem"),
        );
    }
    // Fallback for Linux/macOS (XDG data home).
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return Some(
            PathBuf::from(xdg)
                .join("com.maosuarez.cryptenv")
                .join("tls")
                .join("cert.pem"),
        );
    }
    if let Ok(home) = std::env::var("HOME") {
        return Some(
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("com.maosuarez.cryptenv")
                .join("tls")
                .join("cert.pem"),
        );
    }
    None
}

/// Chooses the cert source: a non-empty `CRYPTENV_CERT_PATH` short-circuits the
/// probe entirely.
fn classify_cert_source(explicit: Option<&str>, probed: Option<PathBuf>) -> CertSource {
    match explicit {
        Some(v) if !v.trim().is_empty() => CertSource::Explicit(PathBuf::from(v.trim())),
        _ => CertSource::Probed(probed),
    }
}

fn resolve_cert_source() -> CertSource {
    classify_cert_source(
        std::env::var("CRYPTENV_CERT_PATH").ok().as_deref(),
        probe_cert_path(),
    )
}

/// Reads the explicit override cert, mapping any failure to a `CliError` that
/// names the configured path (never a secret value).
fn read_explicit_cert(path: &Path) -> Result<Vec<u8>, CliError> {
    std::fs::read(path).map_err(|e| {
        CliError::Config(format!(
            "cannot read TLS certificate at CRYPTENV_CERT_PATH ({}): {e}",
            path.display()
        ))
    })
}

/// Historical probed-path behavior: pin the cert if it loads, else fall back to
/// a plain client so the caller surfaces a clear connection error.
fn build_pinned_or_plain(path: Option<&Path>) -> reqwest::blocking::Client {
    (|| -> Result<reqwest::blocking::Client, Box<dyn std::error::Error>> {
        let path = path.ok_or("cannot determine cert path")?;
        let pem_bytes = std::fs::read(path)?;
        let cert = reqwest::Certificate::from_pem(&pem_bytes)?;
        Ok(reqwest::blocking::ClientBuilder::new()
            .add_root_certificate(cert)
            // Do NOT use danger_accept_invalid_certs — we load the actual cert.
            .build()?)
    })()
    .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

/// Build a `reqwest` blocking client that trusts exactly the vault's cert.
///
/// - `CRYPTENV_CERT_PATH` set: the file is loaded verbatim; an unreadable file
///   or invalid PEM is a hard `CliError` naming the path — never a fallback to
///   the platform probe or to an unverified client.
/// - Otherwise: identical to the historical behavior — the probed cert is
///   pinned when present, and a plain client is used when it is absent (the
///   request then fails with a clear connection error, surfaced as
///   `CliError::ConnectionRefused`).
///
/// Certificate verification is never disabled on any branch.
pub fn http_client() -> Result<reqwest::blocking::Client, CliError> {
    match resolve_cert_source() {
        CertSource::Explicit(path) => {
            let pem_bytes = read_explicit_cert(&path)?;
            let cert = reqwest::Certificate::from_pem(&pem_bytes).map_err(|e| {
                CliError::Config(format!(
                    "CRYPTENV_CERT_PATH ({}) is not a valid PEM certificate: {e}",
                    path.display()
                ))
            })?;
            reqwest::blocking::ClientBuilder::new()
                .add_root_certificate(cert)
                .build()
                .map_err(|e| CliError::Api(e.to_string()))
        }
        CertSource::Probed(path) => Ok(build_pinned_or_plain(path.as_deref())),
    }
}

/// POST /unlock — returns session token.
pub fn api_unlock(password: &str) -> Result<String, CliError> {
    let client = http_client()?;
    let body = serde_json::json!({ "master_password": password });

    let resp = client
        .post(format!("{}/unlock", api_base()))
        .json(&body)
        .send()
        .map_err(|e| {
            if e.is_connect() {
                CliError::ConnectionRefused
            } else {
                CliError::Api(e.to_string())
            }
        })?;

    if resp.status().is_success() {
        let data: UnlockResponse = resp.json().map_err(|e| CliError::Api(e.to_string()))?;
        Ok(data.token)
    } else {
        let err: ApiError = resp.json().unwrap_or(ApiError { error: "unknown error".into() });
        Err(CliError::Api(err.error))
    }
}

/// Returns a valid token: uses saved one or prompts for password.
pub fn get_auth_token() -> Result<String, CliError> {
    if let Some(token) = read_token() {
        return Ok(token);
    }

    let password = rpassword::prompt_password("Master password: ").map_err(CliError::Io)?;
    let token = api_unlock(&password)?;
    save_token(&token);
    Ok(token)
}

/// Authenticated GET. Handles 401 by clearing token and retrying once.
pub fn authenticated_get(url: &str) -> Result<reqwest::blocking::Response, CliError> {
    let token = get_auth_token()?;
    let client = http_client()?;

    let resp = client
        .get(url)
        .header("X-Vault-Token", &token)
        .send()
        .map_err(|e| {
            if e.is_connect() {
                CliError::ConnectionRefused
            } else {
                CliError::Api(e.to_string())
            }
        })?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        clear_token();
        let new_password = rpassword::prompt_password("Master password: ").map_err(CliError::Io)?;
        let new_token = api_unlock(&new_password)?;
        save_token(&new_token);

        let resp2 = client
            .get(url)
            .header("X-Vault-Token", &new_token)
            .send()
            .map_err(|e| {
                if e.is_connect() {
                    CliError::ConnectionRefused
                } else {
                    CliError::Api(e.to_string())
                }
            })?;

        return Ok(resp2);
    }

    Ok(resp)
}

/// Authenticated POST with JSON body. Handles 401 with one retry.
pub fn authenticated_post(
    url: &str,
    body: &serde_json::Value,
) -> Result<reqwest::blocking::Response, CliError> {
    let token = get_auth_token()?;
    let client = http_client()?;

    let resp = client
        .post(url)
        .header("X-Vault-Token", &token)
        .json(body)
        .send()
        .map_err(|e| {
            if e.is_connect() {
                CliError::ConnectionRefused
            } else {
                CliError::Api(e.to_string())
            }
        })?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        clear_token();
        let new_password = rpassword::prompt_password("Master password: ").map_err(CliError::Io)?;
        let new_token = api_unlock(&new_password)?;
        save_token(&new_token);

        let resp2 = client
            .post(url)
            .header("X-Vault-Token", &new_token)
            .json(body)
            .send()
            .map_err(|e| {
                if e.is_connect() {
                    CliError::ConnectionRefused
                } else {
                    CliError::Api(e.to_string())
                }
            })?;

        return Ok(resp2);
    }

    Ok(resp)
}

/// Authenticated PUT with JSON body. Handles 401 with one retry.
#[allow(dead_code)]
pub fn authenticated_put(
    url: &str,
    body: &serde_json::Value,
) -> Result<reqwest::blocking::Response, CliError> {
    let token = get_auth_token()?;
    let client = http_client()?;

    let resp = client
        .put(url)
        .header("X-Vault-Token", &token)
        .json(body)
        .send()
        .map_err(|e| {
            if e.is_connect() {
                CliError::ConnectionRefused
            } else {
                CliError::Api(e.to_string())
            }
        })?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        clear_token();
        let new_password = rpassword::prompt_password("Master password: ").map_err(CliError::Io)?;
        let new_token = api_unlock(&new_password)?;
        save_token(&new_token);

        let resp2 = client
            .put(url)
            .header("X-Vault-Token", &new_token)
            .json(body)
            .send()
            .map_err(|e| {
                if e.is_connect() {
                    CliError::ConnectionRefused
                } else {
                    CliError::Api(e.to_string())
                }
            })?;

        return Ok(resp2);
    }

    Ok(resp)
}

/// Authenticated DELETE. Handles 401 with one retry.
#[allow(dead_code)]
pub fn authenticated_delete(url: &str) -> Result<reqwest::blocking::Response, CliError> {
    let token = get_auth_token()?;
    let client = http_client()?;

    let resp = client
        .delete(url)
        .header("X-Vault-Token", &token)
        .send()
        .map_err(|e| {
            if e.is_connect() {
                CliError::ConnectionRefused
            } else {
                CliError::Api(e.to_string())
            }
        })?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        clear_token();
        let new_password = rpassword::prompt_password("Master password: ").map_err(CliError::Io)?;
        let new_token = api_unlock(&new_password)?;
        save_token(&new_token);

        let resp2 = client
            .delete(url)
            .header("X-Vault-Token", &new_token)
            .send()
            .map_err(|e| {
                if e.is_connect() {
                    CliError::ConnectionRefused
                } else {
                    CliError::Api(e.to_string())
                }
            })?;

        return Ok(resp2);
    }

    Ok(resp)
}

/// GET /items, scoped to a project+environment — the API requires scope
/// params on every call now. `scope_query` is a pre-built query string
/// (e.g. `project=<enc>&environment=<enc>`, see
/// `commands::scope::ResolvedScope::to_query_string`), without a leading
/// `?`. Used for duplicate detection and for listing/browsing items.
pub fn api_list_items(scope_query: &str) -> Result<Vec<ItemSummary>, CliError> {
    let url = format!("{}/items?{scope_query}", api_base());
    let resp = authenticated_get(&url)?;

    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        return Err(CliError::Api(format!("HTTP {}", resp.status())));
    }

    resp.json().map_err(|e| CliError::Api(e.to_string()))
}

/// POST /items/:id/reveal — returns the secret value of an item.
pub fn api_reveal(item_id: i64, token: &str) -> Result<String, CliError> {
    let client = http_client()?;
    // confirm: true is required by the API to acknowledge the reveal action
    let body = serde_json::json!({ "confirm": true });
    let resp = client
        .post(format!("{}/items/{}/reveal", api_base(), item_id))
        .header("X-Vault-Token", token)
        .json(&body)
        .send()
        .map_err(|e| {
            if e.is_connect() {
                CliError::ConnectionRefused
            } else {
                CliError::Api(e.to_string())
            }
        })?;

    if resp.status().is_success() {
        let data: RevealResponse = resp.json().map_err(|e| CliError::Api(e.to_string()))?;
        Ok(data.value)
    } else if resp.status() == reqwest::StatusCode::NOT_FOUND {
        Err(CliError::NotFound("item".to_string()))
    } else {
        let err: ApiError = resp.json().unwrap_or(ApiError { error: "error".into() });
        Err(CliError::Api(err.error))
    }
}

/// Searches for an item by exact name (case-insensitive) within a
/// project+environment scope and returns (id, secret value). `scope_query`
/// is a pre-built query string (see `commands::scope::ResolvedScope`),
/// without a leading `?`.
pub fn find_and_reveal(name: &str, scope_query: &str) -> Result<(i64, String), CliError> {
    let token = get_auth_token()?;
    let url = format!("{}/items?search={}&{scope_query}", api_base(), urlencod(name));

    let client = http_client()?;
    let resp = client
        .get(&url)
        .header("X-Vault-Token", &token)
        .send()
        .map_err(|e| {
            if e.is_connect() {
                CliError::ConnectionRefused
            } else {
                CliError::Api(e.to_string())
            }
        })?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(CliError::Unauthorized);
    }
    if resp.status() == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if !resp.status().is_success() {
        let code = resp.status();
        return Err(CliError::Api(format!("HTTP error {code}")));
    }

    let items: Vec<ItemSummary> = resp.json().map_err(|e| CliError::Api(e.to_string()))?;
    let name_lower = name.to_lowercase();

    let found = items.into_iter().find(|item| {
        item.name
            .as_deref()
            .map(|n| n.to_lowercase() == name_lower)
            .unwrap_or(false)
            || item
                .title
                .as_deref()
                .map(|t| t.to_lowercase() == name_lower)
                .unwrap_or(false)
    });

    let item = found.ok_or_else(|| CliError::NotFound(name.to_string()))?;
    let value = api_reveal(item.id, &token)?;
    Ok((item.id, value))
}

// ─── Utilities ────────────────────────────────────────────────────────────────

/// Minimal URL encoding for item names in query strings.
pub fn urlencod(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            ' ' => out.push('+'),
            c => {
                for byte in c.to_string().as_bytes() {
                    out.push_str(&format!("%{:02X}", byte));
                }
            }
        }
    }
    out
}

/// Parses --VAR=value style arguments into a HashMap.
pub fn parse_vars(vars: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for var in vars {
        let stripped = var.trim_start_matches('-');
        if let Some(eq_pos) = stripped.find('=') {
            let key = stripped[..eq_pos].to_string();
            let value = stripped[eq_pos + 1..].to_string();
            if !key.is_empty() {
                map.insert(key, value);
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Endpoint resolution (tasks 1.1, 1.4) ───────────────────────────────

    #[test]
    fn resolve_api_base_defaults_when_unset_or_empty() {
        for raw in [None, Some(""), Some("   ")] {
            let r = resolve_api_base_from(raw).unwrap();
            assert_eq!(r.url, DEFAULT_API_BASE);
            assert!(r.non_loopback_host.is_none());
        }
    }

    #[test]
    fn resolve_api_base_honors_loopback_override_without_warning() {
        for raw in [
            "https://127.0.0.1:47821",
            "http://localhost:8080",
            "https://127.9.9.9",
            "https://[::1]:47821",
        ] {
            let r = resolve_api_base_from(Some(raw)).unwrap();
            assert_eq!(r.url, raw.trim_end_matches('/'));
            assert!(
                r.non_loopback_host.is_none(),
                "{raw} should be treated as loopback"
            );
        }
    }

    #[test]
    fn resolve_api_base_trims_trailing_slash() {
        let r = resolve_api_base_from(Some("https://127.0.0.1:47821/")).unwrap();
        assert_eq!(r.url, "https://127.0.0.1:47821");
    }

    #[test]
    fn resolve_api_base_flags_non_loopback_host_once() {
        let r = resolve_api_base_from(Some("https://vault.example.com:47821")).unwrap();
        assert_eq!(r.non_loopback_host.as_deref(), Some("vault.example.com"));
        assert_eq!(r.url, "https://vault.example.com:47821");
    }

    #[test]
    fn resolve_api_base_rejects_malformed_values() {
        for raw in ["not-a-url", "ftp://127.0.0.1", "127.0.0.1:47821", "https://"] {
            let err = resolve_api_base_from(Some(raw)).unwrap_err();
            match err {
                CliError::Config(msg) => assert!(msg.contains("CRYPTENV_API_URL")),
                other => panic!("expected Config error for {raw:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn host_is_loopback_classifies_expected_hosts() {
        assert!(host_is_loopback("127.0.0.1"));
        assert!(host_is_loopback("127.255.255.254"));
        assert!(host_is_loopback("localhost"));
        assert!(host_is_loopback("LocalHost"));
        assert!(host_is_loopback("::1"));
        assert!(!host_is_loopback("10.0.0.5"));
        assert!(!host_is_loopback("example.com"));
        assert!(!host_is_loopback("0.0.0.0"));
    }

    // ─── TLS trust anchor (task 2.1) ───────────────────────────────────────

    #[test]
    fn cert_source_prefers_explicit_override_and_ignores_probe() {
        let probed = Some(PathBuf::from("/probed/cert.pem"));
        match classify_cert_source(Some("/explicit/cert.pem"), probed) {
            CertSource::Explicit(p) => assert_eq!(p, PathBuf::from("/explicit/cert.pem")),
            CertSource::Probed(_) => panic!("explicit override must win"),
        }
    }

    #[test]
    fn cert_source_falls_through_when_override_absent_or_blank() {
        let probed = || Some(PathBuf::from("/probed/cert.pem"));
        for explicit in [None, Some(""), Some("  ")] {
            match classify_cert_source(explicit, probed()) {
                CertSource::Probed(Some(p)) => assert_eq!(p, PathBuf::from("/probed/cert.pem")),
                _ => panic!("expected the probed path for {explicit:?}"),
            }
        }
    }

    #[test]
    fn read_explicit_cert_errors_name_the_configured_path() {
        let missing = Path::new("/definitely/not/here/cert.pem");
        match read_explicit_cert(missing) {
            Err(CliError::Config(msg)) => {
                assert!(msg.contains("CRYPTENV_CERT_PATH"));
                assert!(msg.contains("/definitely/not/here/cert.pem"));
            }
            other => panic!("expected Config error, got {other:?}"),
        }
    }

    // ─── Session token path & permissions (tasks 3.1, 3.2) ─────────────────

    #[test]
    fn token_path_from_prefers_explicit_override() {
        let p = token_path_from(Some("/custom/.cli_token"), Some("/app"), Some("/home"));
        assert_eq!(p, Some(PathBuf::from("/custom/.cli_token")));
    }

    #[test]
    fn token_path_from_ignores_blank_override_and_probes() {
        let p = token_path_from(Some("   "), None, Some("/home"));
        assert_eq!(
            p,
            Some(PathBuf::from(
                "/home/.local/share/com.maosuarez.cryptenv/.cli_token"
            ))
        );
    }

    #[test]
    fn token_path_from_uses_default_probe_order() {
        assert_eq!(
            token_path_from(None, Some("/app"), Some("/home")),
            Some(PathBuf::from("/app/com.maosuarez.cryptenv/.cli_token"))
        );
        assert_eq!(
            token_path_from(None, None, Some("/home")),
            Some(PathBuf::from(
                "/home/.local/share/com.maosuarez.cryptenv/.cli_token"
            ))
        );
        assert_eq!(token_path_from(None, None, None), None);
    }

    #[test]
    fn write_token_file_inner_ok_when_write_succeeds_even_if_harden_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".cli_token");
        // The closure stands in for a chmod-refusing filesystem: the real
        // `harden_token_permissions` swallows such failures internally.
        let r = write_token_file_inner(&path, "session-token", |_p| { /* chmod refused */ });
        assert!(r.is_ok());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "session-token");
    }

    #[test]
    fn write_token_file_inner_errors_and_skips_harden_when_write_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing-parent").join(".cli_token");
        let r = write_token_file_inner(&path, "session-token", |_p| {
            unreachable!("harden must not run when the content write fails")
        });
        assert!(r.is_err());
        assert!(!path.exists());
    }

    #[test]
    fn harden_token_permissions_never_panics_on_a_bogus_path() {
        // set_permissions fails here; the helper must swallow it silently.
        harden_token_permissions(Path::new("/nonexistent/cryptenv/.cli_token"));
    }
}
