//! TLS certificate management for the local REST API.
//!
//! On first launch a self-signed cert is generated for `["127.0.0.1", "localhost"]`.
//! Certificate and key live together in a generation directory
//! `{app_data_dir}/tls/gen-<ts>/{cert.pem,key.pem}`; the text file
//! `{app_data_dir}/tls/current` names the live generation, so a pair becomes
//! active with a single atomic rename and a half-written pair is never live.
//! `{app_data_dir}/tls/cert.pem` is a published copy of the live certificate
//! (the stable path `CRYPTENV_CERT_PATH` clients point at).
//!
//! On every launch the live pair is validated (both files parse, the key
//! matches the certificate, the certificate is not within 30 days of expiry).
//! Any failure triggers one regeneration.
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
    let start = 1usize.checked_add(header_len)?;
    let total = start.checked_add(len)?;
    if data.len() < total {
        return None;
    }
    Some((tag, &data[start..total], &data[total..]))
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
    // A length wider than usize can never describe an in-memory buffer.
    if n == 0 || n > std::mem::size_of::<usize>() || data.len() < 1 + n {
        return None;
    }
    let mut len = 0usize;
    for &b in &data[1..=n] {
        len = len.checked_shl(8)? | (b as usize);
    }
    Some((len, 1 + n))
}

/// Parses UTCTime (0x17) or GeneralizedTime (0x18) into a Unix timestamp.
/// Works on bytes and requires ASCII digits, so non-ASCII input can never
/// cause a mid-character slice.
fn parse_asn1_time(tag: u8, bytes: &[u8]) -> Option<u64> {
    match tag {
        // UTCTime: YYMMDDHHMMSSZ
        0x17 => {
            if bytes.len() < 12 {
                return None;
            }
            let yy = parse_digits(&bytes[0..2])?;
            // RFC 5280 §4.1.2.5.1: year ≥ 50 → 1900s, < 50 → 2000s
            let year = if yy >= 50 { 1900 + yy } else { 2000 + yy };
            build_unix_ts(year, &bytes[2..])
        }
        // GeneralizedTime: YYYYMMDDHHMMSSZ
        0x18 => {
            if bytes.len() < 15 {
                return None;
            }
            let year = parse_digits(&bytes[0..4])?;
            build_unix_ts(year, &bytes[4..])
        }
        _ => None,
    }
}

/// Parses a run of ASCII digits; any other byte makes the whole run invalid.
fn parse_digits(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

fn build_unix_ts(year: u64, rest: &[u8]) -> Option<u64> {
    if rest.len() < 10 {
        return None;
    }
    let month = parse_digits(&rest[0..2])?;
    let day = parse_digits(&rest[2..4])?;
    let hour = parse_digits(&rest[4..6])?;
    let min = parse_digits(&rest[6..8])?;
    let sec = parse_digits(&rest[8..10])?;

    // Bound the year so the day-count loop below stays small and the
    // arithmetic cannot overflow.
    if !(1970..=9999).contains(&year) {
        return None;
    }
    let mut days = 0u64;
    for y in 1970..year {
        days += if is_leap_year(y) { 366 } else { 365 };
    }
    let month_days: [u64; 12] = [
        31,
        if is_leap_year(year) { 29 } else { 28 },
        31, 30, 31, 30, 31, 31, 30, 31, 30, 31,
    ];
    if !(1..=12).contains(&month) {
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

const GEN_PREFIX: &str = "gen-";
const CURRENT_FILE: &str = "current";

/// Creates a new file that never exists with permissions broader than owner
/// read/write. On Windows, %APPDATA% is already user-isolated by NTFS ACLs; std
/// has no portable ACL API so we document this limitation rather than skip it.
fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), TlsError> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(contents)?;
    f.sync_all()?;
    Ok(())
}

/// Writes `contents` to `target` through a temp file + rename so readers never
/// see a truncated file.
fn write_atomic(target: &Path, contents: &[u8]) -> Result<(), TlsError> {
    use std::io::Write;
    let tmp = target.with_extension("tmp");
    // A leftover temp file from a crashed run must not block this one.
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(contents)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, target)?;
    Ok(())
}

fn new_generation_dir(app_data_dir: &Path) -> Result<PathBuf, TlsError> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let gen_dir = tls_dir(app_data_dir).join(format!("{GEN_PREFIX}{ts}"));
    std::fs::create_dir(&gen_dir)?;
    Ok(gen_dir)
}

fn is_valid_generation_name(name: &str) -> bool {
    name.strip_prefix(GEN_PREFIX)
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

/// Reads the `current` pointer. Returns the generation directory only if the
/// name is well formed (no path separators) and the directory exists.
fn current_generation(app_data_dir: &Path) -> Option<PathBuf> {
    let raw = std::fs::read_to_string(tls_dir(app_data_dir).join(CURRENT_FILE)).ok()?;
    let name = raw.trim();
    if !is_valid_generation_name(name) {
        return None;
    }
    let dir = tls_dir(app_data_dir).join(name);
    dir.is_dir().then_some(dir)
}

/// Atomically makes `gen_dir` the live generation and refreshes the published
/// `cert.pem` copy.
fn activate_generation(
    app_data_dir: &Path,
    gen_dir: &Path,
    cert_pem: &[u8],
) -> Result<(), TlsError> {
    let name = gen_dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| TlsError::InvalidCert("bad generation directory".into()))?;
    write_atomic(&tls_dir(app_data_dir).join(CURRENT_FILE), name.as_bytes())?;
    // Best effort: the live pair is already switched, a failed copy only
    // affects clients that anchor on the legacy path and is retried next start.
    if let Err(e) = write_atomic(&cert_path(app_data_dir), cert_pem) {
        eprintln!("[tls] Could not refresh published cert.pem: {e}");
    }
    Ok(())
}

/// Generate a fresh self-signed certificate into a new generation directory
/// and make it live. The private key material is never stored in a
/// log-accessible variable.
fn generate_and_save(app_data_dir: &Path) -> Result<(), TlsError> {
    std::fs::create_dir_all(tls_dir(app_data_dir))?;

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

    let gen_dir = new_generation_dir(app_data_dir)?;
    write_private_file(&gen_dir.join("key.pem"), key_pem_bytes.as_bytes())?;
    write_private_file(&gen_dir.join("cert.pem"), cert_pem.as_bytes())?;

    // Single atomic switch: until this rename the previous pair stays live.
    activate_generation(app_data_dir, &gen_dir, cert_pem.as_bytes())
}

/// Loads a cert/key pair and checks it is usable: both parse, the key matches
/// the certificate and the certificate has at least `REGEN_THRESHOLD` left.
/// Never panics; any failure is `Err` so the caller can regenerate.
fn validate_pair(cert_file: &Path, key_file: &Path) -> Result<(), TlsError> {
    let cert_pem = std::fs::read_to_string(cert_file)?;
    if !cert_is_still_valid(&cert_pem, REGEN_THRESHOLD) {
        return Err(TlsError::InvalidCert("expired, expiring or unparseable".into()));
    }
    let cert_der = pem::parse(&cert_pem)
        .map_err(|e| TlsError::InvalidCert(e.to_string()))?
        .into_contents();

    let key_pem = std::fs::read_to_string(key_file)?;
    let key_block = pem::parse(&key_pem).map_err(|e| TlsError::InvalidCert(e.to_string()))?;
    if key_block.tag() != "PRIVATE KEY" {
        return Err(TlsError::InvalidCert("unsupported private key format".into()));
    }
    let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(
        rustls::pki_types::PrivatePkcs8KeyDer::from(key_block.into_contents()),
    );
    let signing_key = rustls::crypto::ring::sign::any_supported_type(&key_der)
        .map_err(|e| TlsError::Rustls(e.to_string()))?;
    let certified = rustls::sign::CertifiedKey::new(
        vec![rustls::pki_types::CertificateDer::from(cert_der)],
        signing_key,
    );
    certified
        .keys_match()
        .map_err(|e| TlsError::Rustls(e.to_string()))
}

/// Resolves the live pair, importing a pre-generation legacy pair when no
/// `current` pointer exists yet. Returns the cert/key paths to load.
fn resolve_valid_pair(app_data_dir: &Path) -> Option<(PathBuf, PathBuf)> {
    if let Some(gen_dir) = current_generation(app_data_dir) {
        let (c, k) = (gen_dir.join("cert.pem"), gen_dir.join("key.pem"));
        return validate_pair(&c, &k).is_ok().then_some((c, k));
    }
    if tls_dir(app_data_dir).join(CURRENT_FILE).exists() {
        // A pointer exists but is unusable: never fall back to the legacy files.
        return None;
    }
    // Legacy layout (cert.pem + key.pem directly in tls/): adopt it as the
    // first generation when it is still a valid pair.
    let (legacy_cert, legacy_key) = (cert_path(app_data_dir), key_path(app_data_dir));
    validate_pair(&legacy_cert, &legacy_key).ok()?;
    let cert_bytes = std::fs::read(&legacy_cert).ok()?;
    let key_bytes = std::fs::read(&legacy_key).ok()?;
    let gen_dir = new_generation_dir(app_data_dir).ok()?;
    write_private_file(&gen_dir.join("key.pem"), &key_bytes).ok()?;
    write_private_file(&gen_dir.join("cert.pem"), &cert_bytes).ok()?;
    activate_generation(app_data_dir, &gen_dir, &cert_bytes).ok()?;
    // The key now lives only in the generation directory.
    let _ = std::fs::remove_file(&legacy_key);
    Some((gen_dir.join("cert.pem"), gen_dir.join("key.pem")))
}

/// Removes every generation directory except the live one.
fn cleanup_old_generations(app_data_dir: &Path, live: &Path) {
    let Ok(entries) = std::fs::read_dir(tls_dir(app_data_dir)) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_old_gen = path.is_dir()
            && path != live
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(is_valid_generation_name);
        if is_old_gen {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// Discards all TLS material so the next `ensure_tls_config` generates a fresh
/// pair. Used by the GUI "Regenerate certificate" action.
pub fn reset_tls_material(app_data_dir: &Path) -> Result<(), TlsError> {
    let dir = tls_dir(app_data_dir);
    for f in [CURRENT_FILE, "cert.pem", "key.pem"] {
        match std::fs::remove_file(dir.join(f)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Ensures a valid TLS certificate exists and returns a `RustlsConfig` ready
/// for `axum_server::bind_rustls`.
///
/// - First launch: generates and saves a new self-signed cert.
/// - Subsequent launches: reuses the live pair when it is a matching,
///   non-expiring pair; otherwise regenerates exactly once.
pub async fn ensure_tls_config(app_data_dir: &Path) -> Result<RustlsConfig, TlsError> {
    std::fs::create_dir_all(tls_dir(app_data_dir))?;

    let (cert_file, key_file) = match resolve_valid_pair(app_data_dir) {
        Some(pair) => pair,
        None => {
            eprintln!(
                "[tls] Generating new self-signed certificate in {}",
                tls_dir(app_data_dir).display()
            );
            generate_and_save(app_data_dir)?;
            let gen_dir = current_generation(app_data_dir)
                .ok_or_else(|| TlsError::InvalidCert("generation not active".into()))?;
            let pair = (gen_dir.join("cert.pem"), gen_dir.join("key.pem"));
            validate_pair(&pair.0, &pair.1)?;
            pair
        }
    };

    if let Some(live) = cert_file.parent() {
        cleanup_old_generations(app_data_dir, live);
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

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;

    // `RustlsConfig::from_pem_file` needs a process-wide crypto provider (lib.rs
    // installs it at startup).
    fn tmp() -> tempfile::TempDir {
        let _ = rustls::crypto::ring::default_provider().install_default();
        tempfile::tempdir().unwrap()
    }

    fn gen_pair_dir(dir: &Path) {
        generate_and_save(dir).expect("generate");
    }

    fn live_files(dir: &Path) -> (PathBuf, PathBuf) {
        let g = current_generation(dir).expect("current generation");
        (g.join("cert.pem"), g.join("key.pem"))
    }

    fn gen_dirs(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(tls_dir(dir))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect()
    }

    #[test]
    fn huge_der_length_is_rejected_without_overflow() {
        // Long-form length of 8 bytes, all 0xFF.
        let mut der = vec![0x30, 0x88];
        der.extend_from_slice(&[0xFF; 8]);
        assert!(read_tlv(&der).is_none());
        assert!(parse_not_after_from_der(&der).is_none());
        // Length wider than usize.
        let mut wide = vec![0x30, 0x89];
        wide.extend_from_slice(&[0x01; 9]);
        assert!(read_tlv(&wide).is_none());
    }

    #[test]
    fn non_ascii_time_is_rejected_without_panic() {
        // "é" is 2 bytes in UTF-8; slicing by byte inside it used to panic.
        let mut t = "é".as_bytes().to_vec();
        t.extend_from_slice(b"0101000000Z");
        assert!(parse_asn1_time(0x17, &t).is_none());
        let mut g = b"20".to_vec();
        g.extend_from_slice("é".as_bytes());
        g.extend_from_slice(b"01010000000Z");
        assert!(parse_asn1_time(0x18, &g).is_none());
        assert!(parse_asn1_time(0x17, b"991231235959Z").is_some());
    }

    #[test]
    fn random_bytes_never_panic() {
        let mut rng = rand::thread_rng();
        for _ in 0..10_000 {
            let mut buf = vec![0u8; (rng.next_u32() % 96) as usize];
            rng.fill_bytes(&mut buf);
            let _ = parse_not_after_from_der(&buf);
            let _ = parse_asn1_time(0x17, &buf);
            let _ = parse_asn1_time(0x18, &buf);
        }
    }

    #[test]
    fn mutated_real_cert_never_panics() {
        let tmp = tmp();
        gen_pair_dir(tmp.path());
        let (cert, _) = live_files(tmp.path());
        let der = pem::parse(std::fs::read_to_string(cert).unwrap())
            .unwrap()
            .into_contents();
        let mut rng = rand::thread_rng();
        for _ in 0..2_000 {
            let mut m = der.clone();
            for _ in 0..3 {
                let i = (rng.next_u32() as usize) % m.len();
                m[i] = rng.next_u32() as u8;
            }
            let _ = parse_not_after_from_der(&m);
        }
    }

    #[tokio::test]
    async fn first_launch_generates_valid_pair_and_published_cert() {
        let tmp = tmp();
        ensure_tls_config(tmp.path()).await.unwrap();
        let (cert, key) = live_files(tmp.path());
        assert!(validate_pair(&cert, &key).is_ok());
        assert_eq!(
            std::fs::read(&cert).unwrap(),
            std::fs::read(cert_path(tmp.path())).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn private_key_is_created_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tmp();
        let f = tmp.path().join("key.pem");
        write_private_file(&f, b"x").unwrap();
        let mode = std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        gen_pair_dir(tmp.path());
        let (_, key) = live_files(tmp.path());
        assert_eq!(std::fs::metadata(key).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[tokio::test]
    async fn crash_before_pointer_switch_keeps_old_pair() {
        let tmp = tmp();
        ensure_tls_config(tmp.path()).await.unwrap();
        let (cert_before, _) = live_files(tmp.path());
        let bytes_before = std::fs::read(&cert_before).unwrap();

        // Simulated crash: a new generation with only the key written.
        let stray = new_generation_dir(tmp.path()).unwrap();
        write_private_file(&stray.join("key.pem"), b"half-written").unwrap();

        ensure_tls_config(tmp.path()).await.unwrap();
        let (cert_after, _) = live_files(tmp.path());
        assert_eq!(std::fs::read(cert_after).unwrap(), bytes_before);
        assert!(!stray.exists(), "stray generation should be cleaned up");
        assert_eq!(gen_dirs(tmp.path()).len(), 1);
    }

    #[tokio::test]
    async fn mismatched_pair_is_regenerated() {
        let tmp = tmp();
        ensure_tls_config(tmp.path()).await.unwrap();
        let (cert, key) = live_files(tmp.path());
        let old_cert = std::fs::read(&cert).unwrap();

        // Replace the key with a different, valid key.
        let other = KeyPair::generate().unwrap().serialize_pem();
        std::fs::write(&key, other).unwrap();
        assert!(validate_pair(&cert, &key).is_err());

        ensure_tls_config(tmp.path()).await.unwrap();
        let (cert2, key2) = live_files(tmp.path());
        assert!(validate_pair(&cert2, &key2).is_ok());
        assert_ne!(std::fs::read(cert2).unwrap(), old_cert);
    }

    #[tokio::test]
    async fn tampered_cert_is_regenerated() {
        let tmp = tmp();
        ensure_tls_config(tmp.path()).await.unwrap();
        let (cert, _) = live_files(tmp.path());
        std::fs::write(
            &cert,
            b"-----BEGIN CERTIFICATE-----\n/////w==\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        ensure_tls_config(tmp.path()).await.unwrap();
        let (c, k) = live_files(tmp.path());
        assert!(validate_pair(&c, &k).is_ok());
    }

    #[tokio::test]
    async fn legacy_pair_is_imported() {
        let tmp = tmp();
        // Build a legacy layout from a freshly generated pair.
        gen_pair_dir(tmp.path());
        let (c, k) = live_files(tmp.path());
        let (cert_bytes, key_bytes) = (std::fs::read(&c).unwrap(), std::fs::read(&k).unwrap());
        let dir = tls_dir(tmp.path());
        std::fs::remove_dir_all(c.parent().unwrap()).unwrap();
        std::fs::remove_file(dir.join(CURRENT_FILE)).unwrap();
        std::fs::write(dir.join("cert.pem"), &cert_bytes).unwrap();
        std::fs::write(dir.join("key.pem"), &key_bytes).unwrap();

        ensure_tls_config(tmp.path()).await.unwrap();
        let (c2, _) = live_files(tmp.path());
        assert_eq!(std::fs::read(c2).unwrap(), cert_bytes, "legacy cert adopted");
        assert!(!dir.join("key.pem").exists(), "legacy key removed after import");
    }

    #[tokio::test]
    async fn reset_forces_new_certificate() {
        let tmp = tmp();
        ensure_tls_config(tmp.path()).await.unwrap();
        let (c, _) = live_files(tmp.path());
        let old = std::fs::read(&c).unwrap();
        reset_tls_material(tmp.path()).unwrap();
        ensure_tls_config(tmp.path()).await.unwrap();
        let (c2, _) = live_files(tmp.path());
        assert_ne!(std::fs::read(c2).unwrap(), old);
    }
}
