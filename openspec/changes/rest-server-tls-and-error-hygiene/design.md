## Context

`ensure_tls_config` returns a `RustlsConfig` from `cert.pem`/`key.pem` in the app data dir, regenerating only on expiry. The CLI trusts the certificate through `CRYPTENV_CERT_PATH`, which points at `cert.pem`; `setup wsl` copies or points to it. Errors go through `err_json(status, &e, code)`, and ~40 sites pass raw `e`.

## Decisions

### D1. Generation directories plus a pointer
The layout is `tls/gen-<ts>/{cert.pem,key.pem}` and `tls/current`, a text file naming the generation.

Generation:
1. Create `gen-<ts>/`.
2. Write the key with `OpenOptions::create_new().mode(0o600)` and the cert.
3. `fsync` both.
4. Write `current.tmp` and rename it to `current` (a single atomic switch).
5. Copy the cert to the legacy `cert.pem` path (temp file + rename) so that `CRYPTENV_CERT_PATH` consumers keep working.
6. Old generations are removed after a successful start.

Migration: when `current` is missing, the legacy `cert.pem`/`key.pem` are validated; if valid, they are imported as the first generation.

*Rejected:* writing a single combined PEM file. rustls loads separate files through `from_pem_file`; it would be possible with `from_pem` too, but the pointer approach also keeps the CLI's anchor path stable.

### D2. Pair validation
Load with `rustls_pemfile` into `CertificateDer` and `PrivateKeyDer`, then build a `CertifiedKey` with `any_supported_type`, then `keys_match()` (rustls ≥0.23.x). Errors → regenerate once; if that fails too, report the status as Failed.

### D3. Parser hardening
- `decode_asn1_length`: `n <= size_of::<usize>()`, `checked_shl` / `checked_add`.
- `read_tlv`: `1usize.checked_add(header_len)?.checked_add(len)?`.
- Time parsing works on `&[u8]` with `is_ascii_digit` checks, using `std::str::from_utf8` only after the ASCII validation.
- A fuzz-style unit test feeds random bytes (a proptest-lite loop over the `rand` crate, 10k iterations) and must not panic.

### D4. Server status
`ApiState`-independent `static API_STATUS: RwLock<ApiStatus>`, set by `start_server`. The Tauri command `api_status()` reads it; `tls_regenerate()` deletes `current` and restarts the server task. The GUI Settings banner polls it on mount.

### D5. `internal_error(e: impl Display) -> Response`
It generates an 8-hex-char correlation id, logs `error id=<id> <e>` through the existing logging (`eprintln!`/`log`), and returns `{error:"internal error", code:"INTERNAL_ERROR", id}`. All raw echoes are replaced. A grep test fails on `err_json(..., &e,` patterns in `api/mod.rs`.

## Security & Threat Model

- The key is never world-readable.
- Error bodies don't leak schema or paths to local callers (including the MCP principal).
- Self-healing prevents a local denial of service by corrupting `cert.pem`. The attacker would already need user-level write access, but the app now recovers.

## Risks / Trade-offs

- [Regeneration changes the cert; WSL clients that copied the old cert fail TLS] → The GUI banner after a regeneration suggests re-running `setup wsl`; the cert *path* is unchanged for clients that point at it.
