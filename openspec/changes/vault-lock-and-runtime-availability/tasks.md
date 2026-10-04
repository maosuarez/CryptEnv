## 1. Lock discipline

- [x] 1.1 Split `project_export` so the dialog runs after the guard is dropped. Verify with a test: while a fake dialog future is pending, `state.try_lock()` succeeds.
- [x] 1.2 Two-phase unlock (GUI and REST) and change-password with `spawn_blocking` KDF and an epoch re-check. Verify with a test: during a slow (test-injected) KDF, another task acquires the vault lock; a lock during the KDF aborts the commit.
- [x] 1.3 Grep review for `state.lock().await` combined with `dialog`/`blocking_`/`derive_` in the same function; record the findings here and fix any found. Verify with the recorded note.

**1.3 findings (grep review of `state.lock().await` next to `dialog` / `blocking_` / `derive_` / `unlock_vault_crypto`):**
- `project_export`: already released the lock before the dialog on main; now split into `export_project_with` (dialog injected) so a test pins it (1.1).
- `vault_unlock`, `biometric_unlock`, `vault_change_password`, REST `/unlock`: KDF under the lock. Fixed (1.2, `vault/unlock.rs`).
- `biometric_enroll`: verified the password with `unlock_vault_crypto` under the lock. Fixed (`unlock::verify_password`).
- `vault_import_backup` / `vault_import_backup_data` (`backup::restore`): up to two KDFs (current and backup password) under the lock. Fixed: `backup::restore_shared` derives off the lock and re-checks the epoch before the swap, which still needs the lock; `restore` is kept under `#[cfg(test)]` for the unit tests.
- Other dialogs (`project_pick_env_path`, `project_pick_root_dir`, `project_import`, `share_export_file`, `share_import_file`): already `spawn_blocking` with no guard held. Relay key derivations already use `derive_relay_key_async`; package export/import KDFs run in `spawn_blocking` outside the lock. Nothing else found.

## 2. Throttle and settings

- [x] 2.1 Implement `UnlockThrottle` shared by REST and GUI unlock (failures only, exponential to 60 s, reset on success, 400 before throttle). Verify with unit tests for malformed spam, 5 failures → 16 s wait, and success reset.
- [x] 2.2 Add `validate_auto_lock` (0 | 1–1440) in `PUT /settings` and `vault_save_settings`, clamp on read, and saturating duration math in `session_ttl` and the auto-lock loop. Verify with tests: `i64::MAX` rejected; a stored bad value clamps; no panic in `session_ttl(u64::MAX)`.

## 3. Startup and wipe

- [ ] 3.1 (implemented and unit-tested; the manual Windows check with Ctrl+Alt+Z taken by another app is still open) Make hotkey registration failure non-fatal: `HotkeyState.unavailable` plus a `hotkey_status` command and a GUI notice. Verify with a unit test of the fallback path (mock registrar error → setup Ok) and a manual check with Ctrl+Alt+Z taken by another app on Windows.
- [x] 3.2 Rename-first `wipe_and_reset` with WAL handling, shared connection options, zero+fsync+remove of the renamed files, and a startup sweep of `vault.db.wipe-*`. Verify with tests: a rename failure (simulated via a read-only dir on unix) keeps the DB usable; success leaves no `wipe-*` after the sweep.
- [ ] 3.3 (implemented; move-aside naming unit-tested; the manual `pnpm tauri dev` corrupt-`vault.db` run and `app.restart()` check on Windows are still open) Add startup recovery mode: no `expect` on DB open; `RecoveryScreen` with move-aside (timestamped rename) and quit; restart after move-aside. Verify with a manual test that corrupts `vault.db` and launches (`pnpm tauri dev`), and with a unit test of the move-aside naming.

## 4. Docs and verification

- [x] 4.1 Document the `auto_lock_timeout` range, the throttle behavior (429 + Retry-After) and the recovery screen in `docs/index.html` / `docs/reference.md`. Verify by review.
- [x] 4.2 Run `cargo clippy --all-targets && cargo test` and `pnpm build`. All pass.
