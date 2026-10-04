## Why

Several defects can freeze the whole vault, including auto-lock, or stop the app from starting:
- **CORE-1:** `project_export` holds the global vault mutex while the modal "Save project template" dialog is open (`project/mod.rs:1064`). The auto-lock task waits on the same mutex, so **the vault never auto-locks while the dialog is open**. REST, MCP and CLI are blocked for that time too.
- **API-7 / CORE-14:** Argon2id (64 MiB, t=3, p=4) runs synchronously on async worker threads *while holding the vault mutex*: REST `/unlock` (`api/mod.rs:629`), and the GUI unlock and change-password. Every attempt stalls every other interface.
- **API-8:** the `/unlock` rate limiter is one global counter, incremented *before* authentication (`api/mod.rs:596-617`). Any local process can send 5 bogus requests a minute and lock the real CLI out indefinitely.
- **API-4 / CORE-14:** `auto_lock_timeout` is never range-checked.
  - `session_ttl` computes `minutes * 60` (`api/mod.rs:59`), which wraps in release builds; `Instant + ttl` then panics *after* the key has been set, leaving the vault unlocked with no CLI session.
  - The auto-lock loop's `timeout_mins * 60` (`lib.rs:163`) overflows too: it panics in debug and wraps in release.
- **CORE-2:** if registering the default hotkey fails (another app owns Ctrl+Alt+Z), the error is returned from `setup` with `?` (`lib.rs:183`), so **the app cannot start**, and the user can't reach Settings to change the hotkey.
- **CORE-10:** `wipe_and_reset` closes the pool and zero-fills the file before `remove_file` (`db/mod.rs:2091`). If the delete fails (an antivirus scanner or indexer holding the file on Windows), the pool stays closed for the rest of the session. On next launch the zero-filled `vault.db` fails to open, and `lib.rs:103` panics on `expect` at **every start**. The `-wal`/`-shm` files are ignored, and the zero-fill is never flushed.

## What Changes

- **No lock across dialogs or user interaction.** `project_export` builds the JSON under the lock, releases it, and then shows the dialog. A regression test asserts that the lock is free while the dialog is open.
- **KDF off the lock and off the runtime.** Every Argon2 derivation (unlock, change-password, backup verify) runs in `spawn_blocking` *without* holding the vault mutex. The mutex is only taken to read the salt and token, and to commit the key.
- **Fair unlock rate limiting.** Only *failed* unlock attempts count, and the penalty is an exponential backoff (1 s doubling up to 60 s) rather than a hard 5-per-minute lockout. A successful unlock resets it.
- **Bounded auto-lock timeout.** `auto_lock_timeout` accepts 0 ("never auto-lock") or 1–1440 minutes; other values are rejected with a validation error. All duration arithmetic uses checked or saturating operations.
- **Hotkey failure never blocks startup.** A failed registration is logged and reported to the GUI as a non-fatal notice ("hotkey unavailable — choose another in Settings"). The app starts without a global hotkey.
- **Safe wipe.**
  1. Checkpoint and close.
  2. *Rename* `vault.db` (plus `-wal`/`-shm`) aside.
  3. Create the fresh DB.
  4. Zero-fill, `fsync` and delete the renamed files. If that fails, it is retried at every startup.

  A failed rename leaves the current DB open and usable.
- **No panic when the database can't be opened at startup.** The app starts into a recovery screen offering "Move the damaged vault aside and start fresh" (a timestamped rename, never a delete) or "Quit".

## Capabilities

### New Capabilities
- `vault-availability`: responsiveness and startup guarantees. The vault lock is never held across user interaction or key derivation; auto-lock always fires; configuration bounds hold; recovery from an unopenable database.

### Modified Capabilities
- `desktop-ui`: *Cross-Platform Global Shortcut Registration and Lifecycle*. A registration failure at startup must not prevent the app from starting.

## Impact

- `project/mod.rs` (`project_export`), `vault/mod.rs` (unlock, change-password, settings validation), `api/mod.rs` (`/unlock`, rate limiter, `session_ttl`, `PUT /settings` validation), `lib.rs` (startup DB open, auto-lock loop, hotkey), `db/mod.rs` (`wipe_and_reset`, pending-wipe sweep), `hotkey.rs`.
- GUI: recovery screen, hotkey-unavailable notice, i18n.
- Docs: the `auto_lock_timeout` range in `docs/index.html` (REST `/settings`) and the recovery behavior.

## Non-Goals

- Changing Argon2 parameters.
- Changing the meaning of `auto_lock_timeout = 0` (still "never"). Blocking MCP from setting it is handled in `mcp-token-capabilities`.
- Automatic repair of corrupt databases.
