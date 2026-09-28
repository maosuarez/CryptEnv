## Why

The local REST server can fail permanently without telling the user, and it leaks internal error text:
- **API-6:** the TLS certificate and key are renamed into place one after the other (`tls/mod.rs:287-288`). A crash or rename failure between the two leaves a new certificate next to an old key. Later startups only check the certificate's expiry (`ensure_tls_config`), so `from_pem_file` keeps failing and `start_server` only logs and returns (`api/mod.rs:3427`). **The REST API, and with it the CLI, TUI and MCP, stays down permanently, with nothing shown to the user.** `key.pem.tmp` is also written at default permissions before the chmod to 0600 (`:275-284`).
- **API-11:** the hand-written DER parser can panic on a tampered or corrupted `cert.pem`:
  - `1 + header_len + len` overflows when the encoded length is huge (`tls/mod.rs:138`);
  - the length accumulator shifts without bounds (`:162`);
  - `s[0..2]`/`s[0..4]` slice by byte on possibly non-ASCII UTF-8 (`:177,187`).

  The crash kills the server's startup task.
- **Error echo:** about 40 handlers return raw `sqlx` or `std::io` error text in API responses (e.g. `delete_item` `api/mod.rs:1049`, the category handlers, `delete_environment`). The code's own comments say "never echo SQL". No secret values are exposed, but schema details and file paths are.

## What Changes

- **Validated key pair.** At startup the server loads the certificate *and* key, and checks that the key matches the certificate's public key and that the certificate is not expiring. Any failure (a mismatch, a parse error, a missing file) triggers one regeneration, and the server then starts.
- **Private, single-rename key material.** Certificate and key are written into a fresh generation directory with the key file created 0600 from the start, and a single atomic rename of a pointer file makes the pair live. A half-written pair is never live.
- **Panic-free expiry parsing.** Expiry is read with checked arithmetic, bounded lengths and byte-level (ASCII) time parsing; any malformed input is treated as "invalid, regenerate".
- **Visible server status.** The GUI can query it (`api_status`: running / failed with reason), and Settings shows a "REST API unavailable" banner with a *Regenerate certificate* action.
- **Generic error bodies.** Internal errors return a generic message with a stable error code and a random correlation id. Details go only to the local log, never secret values.

## Capabilities

### New Capabilities
- `rest-api-server`: startup self-healing of TLS material, resilience of certificate parsing, visible server status, and the hygiene of error responses.

### Modified Capabilities
<!-- none -->

## Impact

- `tls/mod.rs`, `api/mod.rs` (start_server status, a central `internal_error()` helper replacing raw error echoes), a new `api_status` / `tls_regenerate` Tauri command registered in `lib.rs`, and the Settings banner in the GUI plus i18n.
- The WSL/CLI cert anchor (`CRYPTENV_CERT_PATH`) path stays the same file path. The pointer indirection is internal, and the published `cert.pem` path is kept as a copy that is refreshed after each successful switch.
- Docs: a note in the troubleshooting section of `docs/index.html` (banner, regenerate, re-running `setup wsl` after a regeneration).

## Non-Goals

- Replacing the hand-written parser with a new crate dependency (`x509-parser`); hardening in place is enough.
- Changing the certificate lifetime or algorithm.
