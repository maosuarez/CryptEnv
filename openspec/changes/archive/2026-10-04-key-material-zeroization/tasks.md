## 1. Types and crates

- [x] 1.1 Add `VaultKey` and `SecretString` in `crypto/`, and enable the `zeroize` features on `argon2` and `aes-gcm`. Verify with `cargo check` and unit tests: a `VaultKey` clone is independent; `SecretString` deserializes from JSON.
  - Also enabled the crate's own `zeroize/serde` feature so `Zeroizing<String>` can deserialize/serialize. `VaultKey` has a redacted `Debug`, no `Copy`/`Deref`/`Serialize`/`Display` (trait-absence checked at compile time in `crypto::tests`), constant-time `ct_eq`, and `expose()` is `pub(crate)`. `SecretString` additionally lacks `Clone`.
  - `crypto::decrypt` now returns `Zeroizing<Vec<u8>>` (plaintext buffers wipe everywhere it is used).
- [x] 1.2 Make key derivation return `VaultKey` directly. Verify with the existing crypto tests passing.
  - `derive_key`, `init_vault_crypto`, `unlock_vault_crypto`, `share::crypto::derive_package_key` and `share::relay::derive_relay_key(_async)` all fill a `Zeroizing<[u8;32]>` in place and return `VaultKey`. The old `CryptoKey = [u8;32]` alias is gone.

## 2. Backend migration

- [x] 2.1 Migrate `VaultState.key`, `vault/mod.rs` (unlock, change-password incl. zeroized re-encryption buffers, restore, import/export) and `project/mod.rs` to `VaultKey`/`SecretString`. Verify with `cargo check` and `cargo test`.
  - Current equivalents: unlock/change-password live in `vault/unlock.rs` (`Derived`/`Rekey` hold `VaultKey`; re-encryption plaintext is the `Zeroizing` returned by `decrypt`), restore in `vault/backup.rs` (`restore_verified` takes `VaultKey`s), biometric enroll in `vault/biometric_enrollment.rs`. `set_key` takes `Option<VaultKey>` and compares with `ct_eq`.
  - Tauri commands take `SecretString` for `vault_unlock`, `vault_change_password`, `vault_import_backup(_data)` (master and current password), `biometric_enroll`, `share_import_file`, `share_relay_receive`, `project_relay_receive`. Inner functions keep `&str`/`&[u8]` and copy into `Zeroizing` (existing behavior).
  - Intermediate serialized item/bundle/backup-metadata JSON buffers are `Zeroizing`.
- [x] 2.2 Migrate the `api/mod.rs` handlers (unlock body, share/relay/inject/fill) and `share/*`, `vault/share_commands.rs`, `project/relay_commands.rs`, `biometric/`. Verify with `grep -rn '\*\*k\b\|\*\*key\b' src-tauri/src` returning nothing, and with `cargo test`.
  - `**k` / `**key` grep returns nothing. REST bodies `UnlockBody`, `ShareImportBody`, `RelayReceiveBody`, `ProjectRelayReceiveBody` use `SecretString`. LAN session keys (`shared_key`, `vault_key`, SPAKE2 `session_key`) are `VaultKey`; `encrypt_message`/`decrypt_message` take `&VaultKey` (decrypt returns `Zeroizing`). Generated passphrases are `Zeroizing<String>` end to end (export/relay results, `ApprovalSecret`, REST response structs).
  - `exec/` and `api/exec_routes.rs`, `clipboard.rs`, `envfile` already held values in `Zeroizing`; no change needed.

## 3. CLI/TUI/MCP

- [x] 3.1 Wrap every `rpassword` result in `Zeroizing` in `client.rs`; the MCP passphrase params use `SecretString`. Verify with `cargo check` and a grep showing no bare `prompt_password(...)?` bound to a `String`.
  - `client::prompt_password()` returns `Zeroizing<String>` (the `rpassword` String is moved in). `api_unlock` serializes a borrowed `&str` instead of cloning into a `json!` value; `RevealResponse.value` is `Zeroizing<String>` (and lost its `Debug` derive). MCP `share_import`/`relay_receive` hold the passphrase in `SecretString` and send a borrowed-field body.
- [x] 3.2 TUI: pre-allocated zeroizing password input with a 256-char cap, wiped on submit/Esc/quit; single-copy `Zeroizing` reveal rendered by borrow; corrected wording. Verify with TUI reducer tests (Esc wipes: the buffer is empty after the event) and a manual check.
  - Input is `Zeroizing<String>` with capacity 256, capped at 256 bytes (equal to 256 chars for ASCII; a byte cap is what guarantees no reallocation). Esc at Login previously quit without wiping; it now wipes first. Reveal renders `value.as_str()` by borrow (the per-frame `.to_string()` is gone). `docs/index.html` TUI reveal wording updated. Manual check (visual) is left to the Windows/terminal verification pass.

## 4. Guard and verification

- [x] 4.1 Add the `tests/key_hygiene.rs` regex guard with the allowlist marker. Verify that it fails on a deliberately inserted `fn f(key: [u8; 32])` and passes on the clean tree.
  - The guard accepts `Zeroizing<[u8; 32]>`, comments, `#[cfg(test)]` tails and `// key-hygiene: not-a-key` lines. The "fails on `fn f(key: [u8; 32])`" check is a unit test of the scanner (`guard_flags_a_raw_key_signature`) plus a manual insertion run.
- [x] 4.2 Run `cargo clippy --all-targets && cargo test`. All pass.
