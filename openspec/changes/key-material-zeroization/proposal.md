## Why

Project rule: "the master password and derived keys exist only in memory during an unlocked session and are zeroized on drop or lock". The audit found that rule broken in many places (CORE-9, API-10, MCP-11, CLI-9):
- **Raw key copies.** The vault key is copied out of `Zeroizing` as a plain `[u8;32]` in `do_unlock`, `project/mod.rs:915`, change-password (`old_key`/`new_key`), restore (`backup_key`), `handle_unlock` (`api/mod.rs:629-636`), and the share, relay and inject handlers (`**k` at `api/mod.rs:1914,2560,2856,2962,3150,3301`). There are 17 such sites in total.
- **Plain `String` passwords.** Master passwords arrive as `String` and are never wiped: Tauri unlock, change-password, biometric enroll, import, `UnlockBody.master_password`, the CLI `rpassword` results (`client.rs:526,551,596,642,685`), and the TUI password buffer, which also leaves fragments behind every time it grows and is not wiped when Esc quits.
- **Plaintext buffers.** Plaintext from re-encryption during a password change is not zeroized.
- **Crate features off.** `argon2` and `aes-gcm` are built without their `zeroize` features, so the 64 MiB Argon2 memory block and the AES round keys are freed with their contents intact.
- **TUI reveal copies.** The TUI reveal view copies the plaintext value into a new `String` on every 100 ms frame (`tui.rs:649`), which leaves about 100 heap copies after 10 seconds, even though the UI claims "value is wiped from memory".

## What Changes

- **One key type.** Introduce `VaultKey(Zeroizing<[u8;32]>)`, which is not `Copy`, and use it end to end. Functions take `&VaultKey`; nothing returns or stores a raw `[u8;32]` vault or relay key.
- **One password type.** Introduce a `SecretString` newtype (`Zeroizing<String>`, with `Deserialize`) for every password or passphrase input: Tauri commands, REST bodies, the CLI and the TUI.
- **Enable crate zeroization.** Enable `argon2/zeroize` and `aes-gcm/zeroize`. Derivation output goes straight into `VaultKey`.
- **Zeroize plaintext buffers** in re-encryption, import and export.
- **TUI.** The password input is pre-allocated (`String::with_capacity(256)`, so it never reallocates) and wiped on submit, cancel or quit. The reveal renders from a borrowed `&str` of a single `Zeroizing<String>` copy that is wiped when the reveal closes. The on-screen wording is corrected to describe what is actually guaranteed.
- **A CI guard** (a grep-based test) that fails if `[u8; 32]` key signatures or `**key` / `**k` derefs reappear outside `crypto/`.

## Capabilities

### New Capabilities
- `key-material-handling`: lifetime and zeroization rules for the vault key, derived keys, passwords and revealed plaintext, across all binaries.

### Modified Capabilities
<!-- none -->

## Impact

- **Crate:** `crypto/`, `vault/mod.rs`, `api/mod.rs`, `project/mod.rs`, `share/*`, `vault/share_commands.rs`, `project/relay_commands.rs`, `bin/crypt-env/{client.rs,commands/tui.rs}`, `bin/crypt-env-mcp.rs` (passphrase params), `biometric/`.
- **Cargo:** feature flags on `argon2` and `aes-gcm`. No new crates (`zeroize` is already a dependency).
- There is no behavior change visible to users, apart from corrected TUI wording.
- **Ordering:** best applied after the other changes that touch these handlers (MCP, share, backup), to minimise conflicts; alternatively first, with the others rebasing onto `VaultKey`.

## Non-Goals

- Memory locking (`mlock`/`VirtualLock`) or guard pages.
- Protecting against an attacker who can read process memory while the vault is unlocked.
- Zeroizing the webview (JS) heap. The frontend holds revealed values only transiently; see `gui-lock-hygiene`.
