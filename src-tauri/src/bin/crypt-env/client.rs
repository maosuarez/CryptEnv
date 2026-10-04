use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// Built-in REST base URL, used when `CRYPTENV_API_URL` is unset or empty.
pub const DEFAULT_API_BASE: &str = "https://127.0.0.1:47821";

/// Process-wide resolved REST base URL. Set once by [`init_api_base`] at
/// command entry so every request in an invocation targets the same endpoint.
static API_BASE_CACHE: OnceLock<String> = OnceLock::new();

/// Set by the TUI while it owns the terminal (raw mode / alternate screen).
/// While set, [`prompt_password`] refuses to read from the terminal and the
/// caller must collect the password through the TUI's own modal instead.
static NON_INTERACTIVE: AtomicBool = AtomicBool::new(false);

/// Marks the process as unable to prompt on the terminal (`true`) or restores
/// the default (`false`).
pub fn set_non_interactive(on: bool) {
    NON_INTERACTIVE.store(on, Ordering::SeqCst);
}

pub fn is_non_interactive() -> bool {
    NON_INTERACTIVE.load(Ordering::SeqCst)
}

/// The only place the client reads the master password from the terminal.
/// Returns [`CliError::SessionRequired`] instead of prompting when the TUI
/// owns the terminal.
pub fn prompt_password() -> Result<String, CliError> {
    if is_non_interactive() {
        return Err(CliError::SessionRequired);
    }
    rpassword::prompt_password("Master password: ").map_err(CliError::Io)
}

// ─── Error types ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum CliError {
    Api(String),
    Io(std::io::Error),
    ConnectionRefused,
    NotFound(String),
    VaultLocked,
    /// An authenticated call needs a session and the terminal cannot be used
    /// to ask for the password (the TUI owns it). The TUI maps this to its
    /// password modal.
    SessionRequired,
    /// Invalid runtime configuration (malformed `CRYPTENV_API_URL`, unreadable
    /// `CRYPTENV_CERT_PATH`, `setup wsl` preconditions). Messages name only
    /// environment-variable names and file paths — never secret values.
    Config(String),
    /// A multi-item command finished with some failures (already reported
    /// per key name); the process exits with status 2.
    Partial(String),
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
            CliError::NotFound(name) => write!(f, "Error: '{}' not found in vault", name),
            CliError::VaultLocked => write!(f, "Error: vault is locked"),
            CliError::SessionRequired => write!(f, "Error: a password session is required"),
            CliError::Config(msg) => write!(f, "Configuration error: {msg}"),
            CliError::Partial(msg) => write!(f, "{msg}"),
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
    /// Present on `/items` responses since issue #13 (defaults to `false`
    /// when absent, e.g. responses from before this change).
    #[serde(default, rename = "isGlobal")]
    pub is_global: bool,
}

#[derive(Deserialize, Debug)]
struct RevealResponse {
    value: String,
}

// ─── Session token ────────────────────────────────────────────────────────────

/// This terminal's session-token file: `<token path>.<terminal hash>` (see
/// `terminal`). `CRYPTENV_TOKEN_PATH` / the platform default only choose the
/// base location.
pub fn token_file_path() -> Option<PathBuf> {
    token_path().map(|base| crate::terminal::per_terminal_path(&base, &crate::terminal::terminal_id()))
}

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
    let path = token_file_path()?;
    let token = std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    // Mark it recently used so stale-file pruning leaves it alone.
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
    Some(token)
}

/// Writes `content` to `path` and restricts permissions to owner-only on Unix.
///
/// On Windows, `%APPDATA%` inherits user-only NTFS ACLs from the OS, so no
/// additional ACL manipulation is needed via std. The write itself is best-effort.
fn write_token_file(path: &Path, content: &str) -> std::io::Result<()> {
    // Preferred path: private temp file renamed into place, so the token is
    // never visible with group/other bits. Where that is impossible (a
    // filesystem that refuses the rename, e.g. DrvFs), fall back to the plain
    // write + best-effort harden; a write that cannot succeed still errors.
    match write_token_file_atomic(path, content) {
        Ok(()) => Ok(()),
        Err(_) => write_token_file_inner(path, content, harden_token_permissions),
    }
}

/// Creates a uniquely named temp file next to `path` (0600 on Unix from the
/// moment it exists), writes and syncs the token, then renames it over `path`.
fn write_token_file_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let mut tmp = tempfile::Builder::new().prefix(".tok-").tempfile_in(dir)?;
    tmp.write_all(content.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Creates `dir` (and missing parents), owner-only (0700) on Unix.
fn create_token_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
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
    if let Some(base) = token_path() {
        crate::terminal::prune_stale(&base, std::time::SystemTime::now());
    }
    if let Some(path) = token_file_path() {
        if let Some(parent) = path.parent() {
            let _ = create_token_dir(parent);
        }
        let _ = write_token_file(&path, token);
    }
}

pub fn clear_token() {
    if let Some(path) = token_file_path() {
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

/// The TLS trust-anchor path this invocation uses (explicit or probed).
pub fn cert_path_in_use() -> Option<PathBuf> {
    match resolve_cert_source() {
        CertSource::Explicit(p) => Some(p),
        CertSource::Probed(p) => p,
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
        Ok(client_builder()
            .add_root_certificate(cert)
            // Do NOT use danger_accept_invalid_certs — we load the actual cert.
            .build()?)
    })()
    .unwrap_or_else(|_| client_builder().build().unwrap_or_else(|_| reqwest::blocking::Client::new()))
}

/// Request timeout while the TUI owns the terminal: a stalled backend must
/// not freeze its event loop for reqwest's default 30 s.
const TUI_HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Client builder shared by every constructor; applies [`TUI_HTTP_TIMEOUT`]
/// in non-interactive (TUI) mode.
fn client_builder() -> reqwest::blocking::ClientBuilder {
    let b = reqwest::blocking::ClientBuilder::new();
    if is_non_interactive() {
        b.timeout(TUI_HTTP_TIMEOUT)
    } else {
        b
    }
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
            client_builder()
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

/// Password gate for secret operations (`fill`, `sync`, `inject`, `search`,
/// collision reveal): passes when this terminal holds a live session (which
/// the check itself renews server-side), otherwise prompts for the master
/// password and opens a new session. The password lives only in a
/// `Zeroizing` buffer.
pub fn ensure_session() -> Result<(), CliError> {
    if session_alive()? {
        return Ok(());
    }
    let password = zeroize::Zeroizing::new(
        prompt_password()?,
    );
    authenticate(&password)
}

/// Whether this terminal's cached token is a live session. A successful
/// probe slides its expiry; a rejected token is deleted. Never prompts.
pub fn session_alive() -> Result<bool, CliError> {
    let Some(token) = read_token() else { return Ok(false) };
    let alive = probe_session(&http_client()?, api_base(), &token)?;
    if !alive {
        clear_token();
    }
    Ok(alive)
}

/// `GET /settings` with `token`: 2xx alive, 401 rejected (`Ok(false)`, the
/// caller deletes the token), 403 locked, anything else (5xx, 429, network)
/// an error that must leave the token in place.
fn probe_session(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
) -> Result<bool, CliError> {
    let resp = client
        .get(format!("{base}/settings"))
        .header("X-Vault-Token", token)
        .send()
        .map_err(|e| if e.is_connect() { CliError::ConnectionRefused } else { CliError::Api(e.to_string()) })?;
    match resp.status() {
        s if s.is_success() => Ok(true),
        reqwest::StatusCode::UNAUTHORIZED => Ok(false),
        reqwest::StatusCode::FORBIDDEN => Err(CliError::VaultLocked),
        s => Err(CliError::Api(format!("session check failed: HTTP {s}"))),
    }
}

/// Verifies `password` and caches the resulting session token. A wrong
/// password is an "authentication failed" error with no side effects.
pub fn authenticate(password: &str) -> Result<(), CliError> {
    let token = api_unlock(password).map_err(|e| match e {
        CliError::Api(msg) => CliError::Api(format!("authentication failed ({msg})")),
        other => other,
    })?;
    save_token(&token);
    Ok(())
}

/// Returns a valid token: uses saved one or prompts for password.
pub fn get_auth_token() -> Result<String, CliError> {
    if let Some(token) = read_token() {
        return Ok(token);
    }

    let password = prompt_password()?;
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
        let new_password = prompt_password()?;
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
        let new_password = prompt_password()?;
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
        let new_password = prompt_password()?;
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
        let new_password = prompt_password()?;
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

// ─── Typed project API ──────────────────────────────────────────────────────

pub use crypt_env_lib::project::{Environment, Project};

fn check_status(resp: reqwest::blocking::Response, what: &str) -> Result<reqwest::blocking::Response, CliError> {
    let status = resp.status();
    if status == reqwest::StatusCode::FORBIDDEN {
        return Err(CliError::VaultLocked);
    }
    if status.is_success() {
        return Ok(resp);
    }
    let msg = resp
        .json::<serde_json::Value>()
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_else(|| format!("HTTP {status}"));
    Err(CliError::Api(format!("{what}: {msg}")))
}

/// `GET /projects` — every project with environments, vars and root path.
pub fn fetch_projects() -> Result<Vec<Project>, CliError> {
    let resp = check_status(authenticated_get(&format!("{}/projects", api_base()))?, "list projects")?;
    resp.json().map_err(|e| CliError::Api(e.to_string()))
}

/// Case-insensitive project lookup by name.
pub fn find_project(name: &str) -> Result<Option<Project>, CliError> {
    let lower = name.to_lowercase();
    Ok(fetch_projects()?.into_iter().find(|p| p.name.to_lowercase() == lower))
}

/// `POST /projects` (create when `id == 0`, else update). Returns the id.
pub fn save_project(body: &serde_json::Value) -> Result<i64, CliError> {
    let resp = check_status(authenticated_post(&format!("{}/projects", api_base()), body)?, "save project")?;
    let v: serde_json::Value = resp.json().map_err(|e| CliError::Api(e.to_string()))?;
    v.get("id").and_then(|i| i.as_i64()).ok_or_else(|| CliError::Api("save project: missing id".into()))
}

/// Creates any of `names` missing from the vault's categories (projects only
/// store references to existing categories). Colors cycle the GUI preset.
pub fn ensure_categories(names: &[String]) -> Result<(), CliError> {
    const PRESET: [&str; 4] = ["#FF9900", "#10a37f", "#635bff", "#c9d1d9"];
    if names.is_empty() {
        return Ok(());
    }
    let url = format!("{}/categories", api_base());
    let existing: Vec<serde_json::Value> =
        check_status(authenticated_get(&url)?, "list categories")?.json().map_err(|e| CliError::Api(e.to_string()))?;
    let known: Vec<&str> = existing.iter().filter_map(|c| c.get("name").and_then(|n| n.as_str())).collect();
    let missing: Vec<&String> = names.iter().filter(|n| !known.contains(&n.as_str())).collect();
    for (i, name) in missing.into_iter().enumerate() {
        let color = PRESET[(known.len() + i) % PRESET.len()];
        check_status(
            authenticated_post(&url, &serde_json::json!({ "name": name, "color": color }))?,
            "create category",
        )?;
    }
    Ok(())
}

/// `POST /environments` — full replace of name/default/paths/vars.
pub fn save_environment(body: &serde_json::Value) -> Result<i64, CliError> {
    let resp = check_status(authenticated_post(&format!("{}/environments", api_base()), body)?, "save environment")?;
    let v: serde_json::Value = resp.json().map_err(|e| CliError::Api(e.to_string()))?;
    v.get("id").and_then(|i| i.as_i64()).ok_or_else(|| CliError::Api("save environment: missing id".into()))
}

/// Body for [`save_environment`] that keeps `env`'s current vars and
/// replaces only its paths (and default flag).
pub fn environment_body(env: &Environment, is_default: bool, paths: &[String]) -> serde_json::Value {
    serde_json::json!({
        "id": env.id,
        "projectId": env.project_id,
        "name": env.name,
        "isDefault": is_default,
        "paths": paths,
        "vars": env.vars.iter().map(|v| serde_json::json!({"key": v.key, "itemId": v.item_id})).collect::<Vec<_>>(),
    })
}

/// Result of `POST /environments/:id/inject`.
#[derive(Deserialize, Debug)]
pub struct InjectResult {
    pub paths: Vec<String>,
    pub written: Vec<String>,
    #[serde(default)]
    pub backups: Vec<String>,
    /// Keys the vault host could not write (undecryptable item or invalid
    /// name). Names only.
    #[serde(default, rename = "failedKeys")]
    pub failed_keys: Vec<String>,
}

/// Materializes an environment into its configured target files (written by
/// the vault host). `output_path` adds a caller-supplied target, which the
/// server refuses to overwrite when it is an unmanaged file.
pub fn inject_environment(env_id: i64, output_path: Option<&str>) -> Result<InjectResult, CliError> {
    let body = serde_json::json!({ "output_path": output_path, "overwrite": false });
    let resp = authenticated_post(&format!("{}/environments/{env_id}/inject", api_base()), &body)?;
    check_status(resp, "inject")?.json().map_err(|e| CliError::Api(e.to_string()))
}

/// Marks an item as global (reusable across projects) via `PUT /items/:id`;
/// only `isGlobal` is sent, every other field is kept server-side.
pub fn set_item_global(item_id: i64) -> Result<(), CliError> {
    let body = serde_json::json!({ "id": item_id, "type": "", "created": "", "isGlobal": true });
    check_status(authenticated_put(&format!("{}/items/{item_id}", api_base()), &body)?, "mark global")?;
    Ok(())
}

/// Reveals one item's value with the cached session token.
pub fn reveal_item(item_id: i64) -> Result<zeroize::Zeroizing<String>, CliError> {
    let token = get_auth_token()?;
    api_reveal(item_id, &token).map(zeroize::Zeroizing::new)
}

/// `GET /items` in a project/environment scope with an `include_global`
/// mode (`with` | `without` | `only`).
pub fn list_items(project: &str, environment: &str, include_global: &str) -> Result<Vec<ItemSummary>, CliError> {
    let url = format!(
        "{}/items?project={}&environment={}&include_global={}",
        api_base(),
        urlencod(project),
        urlencod(environment),
        urlencod(include_global)
    );
    check_status(authenticated_get(&url)?, "list items")?.json().map_err(|e| CliError::Api(e.to_string()))
}

// ─── Vault seam ───────────────────────────────────────────────────────────────

/// The vault operations the project commands and the TUI need, behind a trait
/// so their consent / binding logic can be unit-tested without a running app.
/// [`Live`] is the real implementation over the REST client above.
pub trait VaultApi {
    fn session_alive(&self) -> Result<bool, CliError>;
    fn ensure_session(&self) -> Result<(), CliError>;
    fn authenticate(&self, password: &str) -> Result<(), CliError>;
    fn fetch_projects(&self) -> Result<Vec<Project>, CliError>;
    fn find_project(&self, name: &str) -> Result<Option<Project>, CliError>;
    fn save_project(&self, body: &serde_json::Value) -> Result<i64, CliError>;
    fn save_environment(&self, body: &serde_json::Value) -> Result<i64, CliError>;
    fn ensure_categories(&self, names: &[String]) -> Result<(), CliError>;
    fn inject_environment(&self, env_id: i64) -> Result<InjectResult, CliError>;
    fn list_items(&self, project: &str, environment: &str, include_global: &str) -> Result<Vec<ItemSummary>, CliError>;
    fn reveal_item(&self, item_id: i64) -> Result<zeroize::Zeroizing<String>, CliError>;
}

/// [`VaultApi`] over the local REST API.
pub struct Live;

impl VaultApi for Live {
    fn session_alive(&self) -> Result<bool, CliError> {
        session_alive()
    }
    fn ensure_session(&self) -> Result<(), CliError> {
        ensure_session()
    }
    fn authenticate(&self, password: &str) -> Result<(), CliError> {
        authenticate(password)
    }
    fn fetch_projects(&self) -> Result<Vec<Project>, CliError> {
        fetch_projects()
    }
    fn find_project(&self, name: &str) -> Result<Option<Project>, CliError> {
        find_project(name)
    }
    fn save_project(&self, body: &serde_json::Value) -> Result<i64, CliError> {
        save_project(body)
    }
    fn save_environment(&self, body: &serde_json::Value) -> Result<i64, CliError> {
        save_environment(body)
    }
    fn ensure_categories(&self, names: &[String]) -> Result<(), CliError> {
        ensure_categories(names)
    }
    fn inject_environment(&self, env_id: i64) -> Result<InjectResult, CliError> {
        inject_environment(env_id, None)
    }
    fn list_items(&self, project: &str, environment: &str, include_global: &str) -> Result<Vec<ItemSummary>, CliError> {
        list_items(project, environment, include_global)
    }
    fn reveal_item(&self, item_id: i64) -> Result<zeroize::Zeroizing<String>, CliError> {
        reveal_item(item_id)
    }
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

    #[test]
    fn non_interactive_flag_makes_prompt_return_session_required() {
        set_non_interactive(true);
        let result = prompt_password();
        set_non_interactive(false);
        assert!(matches!(result, Err(CliError::SessionRequired)));
        assert!(!is_non_interactive());
    }

    /// One-shot HTTP server answering every request with `status`.
    fn mock_server(status: u16) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut conn, _)) = listener.accept() {
                let mut buf = [0u8; 2048];
                let _ = conn.read(&mut buf);
                let _ = write!(conn, "HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn probe_session_maps_each_status() {
        let c = reqwest::blocking::Client::new();
        assert!(matches!(probe_session(&c, &mock_server(200), "t"), Ok(true)));
        assert!(matches!(probe_session(&c, &mock_server(401), "t"), Ok(false)));
        assert!(matches!(probe_session(&c, &mock_server(403), "t"), Err(CliError::VaultLocked)));
        for status in [500, 503, 429] {
            assert!(
                matches!(probe_session(&c, &mock_server(status), "t"), Err(CliError::Api(m)) if m.contains(&status.to_string())),
                "{status} must be an error, not a rejection"
            );
        }
        // Nothing listening: a network error, not a rejection.
        let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", dead.local_addr().unwrap());
        drop(dead);
        assert!(probe_session(&c, &base, "t").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn token_is_written_private_in_a_private_directory() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("a").join("b");
        create_token_dir(&dir).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        let path = dir.join(".cli_token.0123456789abcdef");
        write_token_file(&path, "tok").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "tok");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn token_never_exists_with_group_or_other_bits_during_writes() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".cli_token.0123456789abcdef");
        let done = Arc::new(AtomicBool::new(false));
        let watcher = {
            let (dir, done) = (dir.path().to_path_buf(), done.clone());
            std::thread::spawn(move || {
                let mut loose = Vec::new();
                while !done.load(Ordering::SeqCst) {
                    for e in std::fs::read_dir(&dir).unwrap().flatten() {
                        if let Ok(m) = e.metadata() {
                            if m.permissions().mode() & 0o077 != 0 {
                                loose.push(e.file_name());
                            }
                        }
                    }
                }
                loose
            })
        };
        for i in 0..300 {
            write_token_file(&path, &format!("token-{i}")).unwrap();
        }
        done.store(true, Ordering::SeqCst);
        assert!(watcher.join().unwrap().is_empty(), "a token file was observed with group/other bits");
    }
}
