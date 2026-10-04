## 1. Relay client

- [x] 1.1 Add the `code_hash` helper and replace upload/download/delete with the `relay_put`/`relay_claim` RPC calls, with timeouts and strict status checks. Verify with unit tests using a local mock HTTP server: claim returns the payload once; a null result → "not found"; 404/PGRST202 → `RelaySchemaOutdated`.
- [x] 1.2 Update the call sites in `vault/share_commands.rs`, `project/relay_commands.rs` and `api/mod.rs` (no separate delete; map the new error). Verify with `cargo check` and the existing relay tests.
- [x] 1.3 Add the `RELAY_KDF` semaphore and move every `derive_relay_key` call into `spawn_blocking`. Verify with `grep -n derive_relay_key` (every call site wrapped) and a unit test that the semaphore limits concurrency to 2.

## 2. GUI

- [ ] 2.1 Replace `RELAY_SQL` in `Settings.tsx` with the v2 SQL, and add a "Check relay schema" action (calls `relay_schema_version`). Add i18n strings. Verify with `pnpm build` and a manual check against a v1 and a v2 project.
- [ ] 2.2 Render `RelaySchemaOutdated` in the send/receive modals with the copyable SQL. Verify manually.

## 3. Docs and verification

- [x] 3.1 Document the v2 SQL, the migration steps and the single-use/TTL guarantees in `docs/index.html` and the relevant `docs/*.md`. Verify by review.
- [ ] 3.2 Manual end-to-end against a real Supabase project: a second claim fails; an expired row (`expires_at` edited) is not returned; `select=*` with the anon key returns nothing. Record the results here.
- [x] 3.3 Run `cargo clippy --all-targets && cargo test`. All pass.
