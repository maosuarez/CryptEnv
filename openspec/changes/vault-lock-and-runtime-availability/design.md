## Context

`SharedState = Arc<tokio::Mutex<VaultState>>` is shared by the Tauri commands, the REST API, and the auto-lock loop (which ticks and locks when idle). Several operations do `let s = state.lock().await;` at the top and keep the guard until they return.

## Goals / Non-Goals

**Goals:** no lock across user interaction or KDF; no startup panics; bounded settings.

**Non-Goals:** a lock-free state redesign.

## Decisions

### D1. Two-phase unlock, change-password and export
- **Unlock:** phase 1 (lock) reads `(salt, token)` and clones the `db` handle for later; release. Phase 2 (`spawn_blocking`, no lock) derives the key and verifies the token. Phase 3 (lock) re-checks that the vault is still in the same state (epoch unchanged, from `harden-cli-manifest-and-sessions` D6), then commits the key and runs the post-unlock steps from `vault-db-transactional-integrity` D4.
- **Change-password:** the same pattern for both derivations; the re-encryption runs under the lock inside one transaction (it is fast).
- **`project_export`:** build the JSON under the lock, drop the guard, then `blocking_save_file` / the dialog.

A clippy-style review task greps for `state.lock().await` in functions that also call `dialog`, `blocking_` or `derive_` APIs.

### D2. Unlock throttle
`struct UnlockThrottle { failures: u32, next_allowed: Instant }`. On a failed verify: `failures += 1; next_allowed = now + min(2^(failures-1) s, 60 s)`. On success: reset. Attempts before `next_allowed` get 429 with `Retry-After`. Malformed bodies return 400 before the throttle is consulted. The throttle is shared by the REST `/unlock` and the GUI `vault_unlock`, so it cannot be bypassed through the other path.

### D3. Timeout validation
`fn validate_auto_lock(v: i64) -> Result<u64, ValidationError>` accepts `0 | 1..=1440`. It is used by `PUT /settings` and `vault_save_settings`. Values stored by earlier versions that are out of range are clamped on read (`get_auto_lock()`), so an existing bad value cannot crash. `session_ttl` and the auto-lock loop use `Duration::from_secs(u64::from(min).saturating_mul(60))`.

### D4. Hotkey
Default registration failure is logged; `HotkeyState.unavailable = true`. A Tauri command `hotkey_status` lets the GUI show the notice. `setup` no longer propagates the error.

### D5. Wipe
1. `PRAGMA wal_checkpoint(TRUNCATE)`, then `pool.close()`.
2. `rename(vault.db → vault.db.wipe-<ts>)` (and the `-wal`/`-shm` files if present). If this fails, reopen the pool on the original and return an error.
3. Open a fresh pool with the shared options (from `vault-db-transactional-integrity` D1, fixing the current `max_connections(1)`, no-pragma reopen).
4. Best effort on the renamed files: overwrite with zeros, `sync_all`, remove. Leftover `vault.db.wipe-*` files are retried by a startup sweep.

### D6. Startup recovery
`lib.rs` replaces `expect("failed to open vault database")` with a match. On error it manages `AppMode::Recovery { error }` instead of `VaultState`, registers only the recovery commands (`recovery_move_aside`, `recovery_quit`) and a minimal state, and the frontend renders `RecoveryScreen` when `app_mode()` returns recovery. After move-aside, the app restarts itself (`app.restart()`) to reinitialize normally. The other `expect`s in `setup` (app data dir) stay: the OS contract makes them unrecoverable. The same match style is applied there too, returning a Tauri setup error with a message rather than a panic.

## Security & Threat Model

- Auto-lock reliability is a security property: an unlocked vault left open because a dialog is open is now prevented.
- The throttle no longer lets a local process deny access; brute-force resistance stays (exponential backoff plus Argon2 cost).
- Recovery never deletes user data.

## Risks / Trade-offs

- [Two-phase unlock race: a lock or reset between phases] → The epoch re-check in phase 3 aborts the commit.
- [`app.restart()` behavior on Windows installer builds] → Covered by manual verification.

## Migration Plan

No data migration. Out-of-range stored timeouts are clamped on read.
