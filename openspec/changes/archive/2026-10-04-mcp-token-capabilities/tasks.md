## 1. Principal model

- [x] 1.1 Change `verify_token` to return `Principal::{Session, Mcp}` and add an `AuthedPrincipal` axum extractor. Migrate all handlers to it. Verify with `cargo check`, and with the existing API tests passing unchanged for session callers.
- [x] 1.2 Add an `mcp_policy` table and middleware (default Deny for unknown routes). Verify with a unit test that enumerates the router's routes and fails on any route without an explicit policy.

## 2. Denials

- [x] 2.1 Deny the MCP principal on reveal, share confirm, and security settings keys in `PUT /settings`. Verify with API tests: MCP → 403 `MCP_FORBIDDEN`, no body secrets; session → unchanged.
- [x] 2.2 Confine MCP `output_path` for `/fill` and `/environments/:id/example` to project roots (`fsguard::contain_relative`), and deny MCP overwrite of Foreign targets. Verify with API tests for `~/.bashrc`, an in-root path, and a Foreign overwrite. (Depends on `harden-cli-manifest-and-sessions` task 2.1.)

## 3. Approval flow

- [x] 3.1 Implement `api/approvals.rs` (store, 120 s expiry, max 8, epoch binding, clear on lock). Verify with unit tests for expiry, capacity (429), and a lock epoch change discarding entries.
- [x] 3.2 Route relay_send and share_export for MCP through approvals (202 + id), and add `GET /approvals/:id` returning non-secret meta. Verify with API tests: the MCP response bodies never contain the passphrase or code (grep assertions on the serialized JSON).
- [x] 3.3 Add `/mcp-servers` endpoints in the backend (the write logic moves out of the MCP binary), with Approve policy for MCP. Verify with a unit test that session writes succeed and MCP writes return 202.
- [x] 3.4 Add the Tauri event `approval://requested`, and the commands `approval_list` and `approval_resolve`, registered in `lib.rs`. Verify with `cargo check` and a command-level test that resolve executes the stored payload.

## 4. GUI

- [ ] 4.1 Add an approval modal: operation, item names/count, destination, Deny focused by default. Show the relay code and passphrase in the GUI after approval. Verify manually with `pnpm tauri dev`: approve and deny from an MCP call.
- [ ] 4.2 On an approval request, show the window and fire an OS notification. Verify manually with the window hidden to the tray.

## 5. MCP server

- [x] 5.1 Update the tools: remove share_confirm and the security keys from update_settings; relay_send/share_export/mcp_server tools return pending; add `crypt_env_approval_status`. Verify with the MCP unit tests in `crypt-env-mcp.rs` (tools/list snapshot, and a pending response shape).

## 6. Docs and verification

- [x] 6.1 Update the MCP section of `docs/index.html` and `docs/reference.md` (tool changes, approval flow, breaking notes). Verify by reviewing the rendered page.
- [x] 6.2 Run `cargo clippy --all-targets && cargo test`, and `pnpm build`. All pass.
