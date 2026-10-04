## 1. Backend exec core

- [x] 1.1 Create an `exec` module with param validation and substitution. Verify with unit tests: allowed charset passes; `;`, `$()`, backtick, space, `%`, `^`, `!`, `&` and `|` are rejected.
- [x] 1.2 Implement the child env builder (env_clear + per-OS baseline + injected keys). Verify with a unit test that spawns `env`/`set` and asserts an unrelated backend var is absent.
- [x] 1.3 Implement bounded capture (64 KiB per stream, stdin null) and char-boundary truncation to 2000 characters. Verify with unit tests: multi-byte output at byte 2000 does not panic; a 1 MiB output stays capped.
- [x] 1.4 Implement redaction with aho-corasick over raw, base64 (std and url-safe) and hex forms, for values of length ≥ 4. Verify with unit tests for each encoding.
- [ ] 1.5 Implement the process-tree kill: Unix process group + killpg; Windows Job Object kill-on-close. Verify with a unix test where a `sh -c 'sleep 300 & sleep 300'` run is fully killed on timeout (short test timeout), plus a Windows manual check.

## 2. API

- [x] 2.1 Add `POST /exec` and `DELETE /exec/:runId` (semaphore of 4, queue of 8 → 429; values decrypted outside the vault lock; epoch-bound runs killed on lock). Verify with API tests: an injected secret printed → redacted; timeout reports `timedOut`; a lock kills the run.
- [x] 2.2 Move `generate_env` backend-side: private 0700 dir, 0600 `create_new`, TTL/lock/exit deletion, startup sweep, Approve policy for MCP. Verify with tests: file mode 0600, deleted on an epoch change, sweep removes stale files.

## 3. WSL bridge

- [ ] 3.1 Implement the `clientContext` WSL spawn with the stdin-fed env prelude (no secrets in argv or env of `wsl.exe`). Verify with a unit test of the prelude and argv construction, and a manual Windows test from a WSL MCP client.

## 4. MCP binary

- [x] 4.1 `inject_env` / `inject_env_by_name` record key names in session state (no `set_var`); `run_command` calls `/exec` with `injectKeys` and `clientContext`; `generate_env` calls the backend and handles pending approval. Remove every `/reveal` call from the MCP binary. Verify with `grep -n "reveal\|set_var" src-tauri/src/bin/crypt-env-mcp.rs` returning nothing relevant, and with the MCP unit tests.
- [x] 4.2 On stdio EOF, cancel in-flight runs (`DELETE /exec/:runId`), and sweep legacy `/tmp/crypt_env_*.env` files owned by the user at startup. Verify with a unit test for the sweep filter.

## 5. Docs and verification

- [x] 5.1 Document the param charset, redaction, timeout, WSL behavior and the breaking changes in `docs/index.html` / `docs/reference.md`. Verify by review.
- [x] 5.2 Run `cargo clippy --all-targets && cargo test`. All pass.
- [x] 5.3 `generate_env` writes values through `envfile::serialize_line` (shared quoting); only values containing NUL are skipped. Verify with `generated_env_quotes_multiline_and_special_values`.
