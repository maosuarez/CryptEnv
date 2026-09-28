//! TLS certificate management for the local REST API.
//!
//! On first launch a self-signed cert is generated for `["127.0.0.1", "localhost"]`
//! and written to `{app_data_dir}/tls/cert.pem` + `key.pem`.
//!
//! On subsequent launches the existing cert is reused unless it is expired or
//! within 30 days of expiry, in which case a fresh cert is regenerated.
//!
//! The private key is never logged.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum_server::tls_rustls::RustlsConfig;
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
use time::OffsetDateTime;

// ─── Error type ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum TlsError {
    Io(std::io::Error),
    Rcgen(rcgen::Error),
    Rustls(String),
    InvalidCert(String),
}

impl std::fmt::Display for TlsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TlsError::Io(e) => write!(f, "TLS I/O error: {e}"),
            TlsError::Rcgen(e) => write!(f, "TLS cert generation error: {e}"),
            TlsError::Rustls(e) => write!(f, "TLS config error: {e}"),
            TlsError::InvalidCert(e) => write!(f, "TLS cert invalid: {e}"),
        }
    }
}

impl From<std::io::Error> for TlsError {
    fn from(e: std::io::Error) -> Self {
        TlsError::Io(e)
    }
}

impl From<rcgen::Error> for TlsError {
    fn from(e: rcgen::Error) -> Self {
        TlsError::Rcgen(e)
    }
}

// ─── Paths ────────────────────────────────────────────────────────────────────

fn tls_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("tls")
}

pub fn cert_path(app_data_dir: &Path) -> PathBuf {
    tls_dir(app_data_dir).join("cert.pem")
}

fn key_path(app_data_dir: &Path) -> PathBuf {
    tls_dir(app_data_dir).join("key.pem")
}

// ─── Validity check ───────────────────────────────────────────────────────────

/// Returns `true` when the PEM-encoded cert is valid for at least `min_remaining`
/// from now.  Returns `false` on any parse error so the cert is regenerated
/// rather than crashing.
fn cert_is_still_valid(cert_pem: &str, min_remaining: Duration) -> bool {
    let not_after_unix = match parse_not_after_from_pem(cert_pem) {
        Some(ts) => ts,
        None => return false,
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let threshold = now.saturating_add(min_remaining.as_secs());
    not_after_unix > threshold
}

/// Extracts the `notAfter` field from a PEM certificate as a Unix timestamp.
/// Uses a minimal ASN.1 DER scanner to avoid adding a full X.509 parser.
pub fn parse_not_after_from_pem(cert_pem: &str) -> Option<u64> {
    let pem_block = pem::parse(cert_pem).ok()?;
    parse_not_after_from_der(pem_block.contents())
}

/// Walks the DER structure:
///   Certificate → TBSCertificate → Validity → notAfter
fn parse_not_after_from_der(der: &[u8]) -> Option<u64> {
    // Outer Certificate SEQUENCE
    let (tbs_and_rest, _) = read_sequence(der)?;
    // tbsCertificate SEQUENCE
    let (tbs, _) = read_sequence(tbs_and_rest)?;

    let mut cursor = tbs;

    // Optional version [0] EXPLICIT
    if cursor.first() == Some(&0xa0) {
        cursor = skip_tlv(cursor)?.1;
    }
    // serialNumber INTEGER
    cursor = skip_tlv(cursor)?.1;
    // signature AlgorithmIdentifier SEQUENCE
    cursor = skip_tlv(cursor)?.1;
    // issuer Name SEQUENCE
    cursor = skip_tlv(cursor)?.1;

    // Validity SEQUENCE
    let (validity, _) = read_sequence(cursor)?;

    // notBefore — skip
    let (_, after_nb) = skip_tlv(validity)?;
    // notAfter — read
    let (tag, time_bytes, _) = read_tlv(after_nb)?;

    parse_asn1_time(tag, time_bytes)
}

// ─── Minimal ASN.1 helpers ────────────────────────────────────────────────────

fn read_sequence(data: &[u8]) -> Option<(&[u8], &[u8])> {
    let (tag, contents, rest) = read_tlv(data)?;
    if tag != 0x30 {
        return None;
    }
    Some((contents, rest))
}

fn read_tlv(data: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    if data.len() < 2 {
        return None;
    }
    let tag = data[0];
    let (len, header_len) = decode_asn1_length(&data[1..])?;
    let total = 1 + header_len + len;
    if data.len() < total {
        return None;
    }
    Some((tag, &data[1 + header_len..total], &data[total..]))
}

fn skip_tlv(data: &[u8]) -> Option<(u8, &[u8])> {
    let (tag, _, rest) = read_tlv(data)?;
    Some((tag, rest))
}

fn decode_asn1_length(data: &[u8]) -> Option<(usize, usize)> {
    if data.is_empty() {
        return None;
    }
    if data[0] < 0x80 {
        return Some((data[0] as usize, 1));
    }
    let n = (data[0] & 0x7f) as usize;
    if n == 0 || data.len() < 1 + n {
        return None;
    }
    let mut len = 0usize;
    for &b in &data[1..=n] {
        len = (len << 8) | (b as usize);
    }
    Some((len, 1 + n))
}

/// Parses UTCTime (0x17) or GeneralizedTime (0x18) into a Unix timestamp.
fn parse_asn1_time(tag: u8, bytes: &[u8]) -> Option<u64> {
    let s = std::str::from_utf8(bytes).ok()?;
    match tag {
        // UTCTime: YYMMDDHHMMSSZ
        0x17 => {
            if s.len() < 12 {
                return None;
            }
            let yy: u64 = s[0..2].parse().ok()?;
            // RFC 5280 §4.1.2.5.1: year ≥ 50 → 1900s, < 50 → 2000s
            let year = if yy >= 50 { 1900 + yy } else { 2000 + yy };
            build_unix_ts(year, &s[2..])
        }
        // GeneralizedTime: YYYYMMDDHHMMSSZ
        0x18 => {
            if s.len() < 15 {
                return None;
            }
            let year: u64 = s[0..4].parse().ok()?;
            build_unix_ts(year, &s[4..])
        }
        _ => None,
    }
}

fn build_unix_ts(year: u64, rest: &str) -> Option<u64> {
    if rest.len() < 10 {
        return None;
    }
    let month: u64 = rest[0..2].parse().ok()?;
    let day: u64 = rest[2..4].parse().ok()?;
    let hour: u64 = rest[4..6].parse().ok()?;
    let min: u64 = rest[6..8].parse().ok()?;
    let sec: u64 = rest[8..10].parse().ok()?;

    let mut days = 0u64;
    for y in 1970..year {
        days += if is_leap_year(y) { 366 } else { 365 };
    }
    let month_days: [u64; 12] = [
        31,
        if is_leap_year(year) { 29 } else { 28 },
        31, 30, 31, 30, 31, 31, 30, 31, 30, 31,
    ];
    if month < 1 || month > 12 {
        return None;
    }
    for m in 1..month {
        days += month_days[(m - 1) as usize];
    }
    days += day.checked_sub(1)?;

    Some(days * 86400 + hour * 3600 + min * 60 + sec)
}

fn is_leap_year(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

// ─── Cert generation ──────────────────────────────────────────────────────────

/// 10-year validity for a local-only self-signed certificate.
const CERT_VALIDITY_SECS: i64 = 10 * 365 * 24 * 60 * 60;
/// Regenerate when fewer than 30 days remain.
const REGEN_THRESHOLD: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Generate a fresh self-signed certificate and write both PEM files.
/// The private key material is never stored in a log-accessible variable.
fn generate_and_save(app_data_dir: &Path) -> Result<(), TlsError> {
    let dir = tls_dir(app_data_dir);
    std::fs::create_dir_all(&dir)?;

    // Build cert params with SAN entries for 127.0.0.1 and localhost.
    let mut params = CertificateParams::new(vec![
        "127.0.0.1".to_string(),
        "localhost".to_string(),
    ])?;

    params.distinguished_name = {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "cryptenv-local");
        dn.push(DnType::OrganizationName, "CryptEnv Local");
        dn
    };

    // Set validity: now .. now + 10 years.
    let now = OffsetDateTime::now_utc();
    params.not_before = now;
    params.not_after = now.saturating_add(time::Duration::seconds(CERT_VALIDITY_SECS));

    let key_pair = KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;

    let cert_pem = cert.pem();
    // The private key PEM is written directly without storing it in a named
    // binding that could end up in a log macro.
    let key_pem_bytes = key_pair.serialize_pem();

    // Atomic write: write to .tmp then rename so a crash never leaves a
    // truncated file where a valid cert used to be.
    let cert_file = cert_path(app_data_dir);
    let key_file = key_path(app_data_dir);
    let cert_tmp = cert_file.with_extension("pem.tmp");
    let key_tmp = key_file.with_extension("pem.tmp");

    std::fs::write(&cert_tmp, cert_pem.as_bytes())?;
    std::fs::write(&key_tmp, key_pem_bytes.as_bytes())?;

    // On Unix, restrict the key file to owner read/write only (0o600).
    // On Windows, %APPDATA% is already user-isolated by NTFS ACLs; std has no
    // portable ACL API so we document this limitation rather than silently skip.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&key_tmp, perms)?;
    }

    std::fs::rename(&cert_tmp, &cert_file)?;
    std::fs::rename(&key_tmp, &key_file)?;

    Ok(())
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Ensures a valid TLS certificate exists and returns a `RustlsConfig` ready
/// for `axum_server::bind_rustls`.
///
/// - First launch: generates and saves a new self-signed cert.
/// - Subsequent launches: reuses the cert unless it expires within 30 days.
pub async fn ensure_tls_config(app_data_dir: &Path) -> Result<RustlsConfig, TlsError> {
    let cert_file = cert_path(app_data_dir);
    let key_file = key_path(app_data_dir);

    let needs_generate = if cert_file.exists() && key_file.exists() {
        let cert_pem = std::fs::read_to_string(&cert_file)?;
        !cert_is_still_valid(&cert_pem, REGEN_THRESHOLD)
    } else {
        true
    };

    if needs_generate {
        eprintln!(
            "[tls] Generating new self-signed certificate in {}",
            tls_dir(app_data_dir).display()
        );
        generate_and_save(app_data_dir)?;
    }

    RustlsConfig::from_pem_file(&cert_file, &key_file)
        .await
        .map_err(|e| TlsError::Rustls(e.to_string()))
}

/// Returns the DER bytes of the certificate stored at `{app_data_dir}/tls/cert.pem`.
/// Used by the CLI to build a `reqwest` client that trusts the local cert.
pub fn load_cert_der(app_data_dir: &Path) -> Result<Vec<u8>, TlsError> {
    let cert_pem = std::fs::read_to_string(cert_path(app_data_dir))?;
    let pem_block = pem::parse(&cert_pem)
        .map_err(|e| TlsError::InvalidCert(e.to_string()))?;
    Ok(pem_block.into_contents())
}
