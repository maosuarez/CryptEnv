## 1. Token and session

- [x] 1.1 Atomic 0600 token write via tempfile + persist, 0700 dir creation, DrvFs fallback. Verify with a unix test: after `save_token`, the mode is 0600 and the dir 0700; a watcher test (a loop stat during the write) never observes group/other bits.
- [x] 1.2 `session_alive` clears only on 401; 5xx/429/network are errors. Verify with unit tests using a mock server per status.
- [x] 1.3 `add` summary and non-zero exit on partial failure. Verify with a unit test through the client seam.

## 2. Terminal identity

- [ ] 2.1 Windows `native_id` with the console host pid and creation time, and a per-process fallback. Verify with `cargo xwin check` or a Windows build, plus a manual test: close and reopen a terminal → prompts; two detached processes → separate sessions.
- [x] 2.2 WSL launcher id includes the leader start time. Verify by updating the existing launcher script tests in `crates/crypt-env-setup` (the id contains 4 colon-separated fields).

## 3. TUI

- [x] 3.1 `TerminalGuard` right after raw mode is enabled. Verify with a test that simulates a `Terminal::new` failure through a seam and asserts the guard's drop ran.
- [ ] 3.2 Workspace cache, filter regex cache, 5 s HTTP timeout in TUI mode, "working…" status. Verify with reducer tests (no `find_config` call on idle ticks, via a counter seam) and a manual test with the backend stopped.

## 4. Docs and verification

- [x] 4.1 Update `docs/cli.md` (`add` exit code 2 on partial failure; Windows re-auth note). Verify by review.
- [x] 4.2 Run `cargo clippy --all-targets && cargo test` (including the `crypt-env-setup` crate). All pass.
