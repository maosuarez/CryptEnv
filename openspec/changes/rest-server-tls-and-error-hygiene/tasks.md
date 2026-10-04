## 1. TLS

- [x] 1.1 Harden the DER/time parsing with checked arithmetic and ASCII byte parsing. Verify with unit tests for huge lengths and non-ASCII times, plus a 10k-iteration random-bytes no-panic test.
- [x] 1.2 Generation dirs + `current` pointer + 0600 `create_new` key + legacy `cert.pem` copy + legacy import + old-generation cleanup. Verify with tests: a simulated crash after the key write but before the pointer switch → the next ensure uses the old valid pair; the key mode is 0600 at creation (unix).
- [x] 1.3 Pair validation with `keys_match` → regenerate once. Verify with a test: a mismatched pair on disk → regenerated and loads.

## 2. Status and errors

- [ ] 2.1 `API_STATUS`, the `api_status` and `tls_regenerate` commands (registered in `lib.rs`), and the Settings banner with i18n. Verify with `cargo check`, `pnpm build`, and a manual test with port 47821 occupied.
  - Implemented (api/status.rs, `api_status`/`tls_regenerate` in lib.rs, `ApiStatusBanner` + i18n en/es/pt); `tsc --noEmit` and cargo check pass. Still pending: manual check with port 47821 occupied and `pnpm build` (not runnable from WSL).
- [x] 2.2 Add `internal_error()` with a correlation id and replace every raw error echo in `api/mod.rs`; add a grep test forbidding `err_json(` with a raw `&e`. Verify with the grep test and an API test (a forced DB error → generic body with `id`).

## 3. Docs and verification

- [x] 3.1 Troubleshooting notes in `docs/index.html` (banner, regenerate, re-run `setup wsl`). Verify by review.
- [x] 3.2 Run `cargo clippy --all-targets && cargo test`. All pass.
