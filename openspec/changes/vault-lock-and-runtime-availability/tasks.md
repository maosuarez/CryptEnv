## 1. Lock discipline

- [ ] 1.1 Split `project_export` so the dialog runs after the guard is dropped. Verify with a test: while a fake dialog future is pending, `state.try_lock()` succeeds.
- [ ] 1.2 Two-phase unlock (GUI and REST) and change-password with `spawn_blocking` KDF and an epoch re-check. Verify with a test: during a slow (test-injected) KDF, another task acquires the vault lock; a lock during the KDF aborts the commit.
- [ ] 1.3 Grep review for `state.lock().await` combined with `dialog`/`blocking_`/`derive_` in the same function; record the findings here and fix any found. Verify with the recorded note.

## 2. Throttle and settings

- [ ] 2.1 Implement `UnlockThrottle` shared by REST and GUI unlock (failures only, exponential to 60 s, reset on success, 400 before throttle). Verify with unit tests for malformed spam, 5 failures → 16 s wait, and success reset.
- [ ] 2.2 Add `validate_auto_lock` (0 | 1–1440) in `PUT /settings` and `vault_save_settings`, clamp on read, and saturating duration math in `session_ttl` and the auto-lock loop. Verify with tests: `i64::MAX` rejected; a stored bad value clamps; no panic in `session_ttl(u64::MAX)`.

## 3. Startup and wipe

- [ ] 3.1 Make hotkey registration failure non-fatal: `HotkeyState.unavailable` plus a `hotkey_status` command and a GUI notice. Verify with a unit test of the fallback path (mock registrar error → setup Ok) and a manual check with Ctrl+Alt+Z taken by another app on Windows.
- [ ] 3.2 Rename-first `wipe_and_reset` with WAL handling, shared connection options, zero+fsync+remove of the renamed files, and a startup sweep of `vault.db.wipe-*`. Verify with tests: a rename failure (simulated via a read-only dir on unix) keeps the DB usable; success leaves no `wipe-*` after the sweep.
- [ ] 3.3 Add startup recovery mode: no `expect` on DB open; `RecoveryScreen` with move-aside (timestamped rename) and quit; restart after move-aside. Verify with a manual test that corrupts `vault.db` and launches (`pnpm tauri dev`), and with a unit test of the move-aside naming.

## 4. Docs and verification

- [ ] 4.1 Document the `auto_lock_timeout` range, the throttle behavior (429 + Retry-After) and the recovery screen in `docs/index.html` / `docs/reference.md`. Verify by review.
- [ ] 4.2 Run `cargo clippy --all-targets && cargo test` and `pnpm build`. All pass.
