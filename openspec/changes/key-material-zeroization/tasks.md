## 1. Types and crates

- [ ] 1.1 Add `VaultKey` and `SecretString` in `crypto/`, and enable the `zeroize` features on `argon2` and `aes-gcm`. Verify with `cargo check` and unit tests: a `VaultKey` clone is independent; `SecretString` deserializes from JSON.
- [ ] 1.2 Make key derivation return `VaultKey` directly. Verify with the existing crypto tests passing.

## 2. Backend migration

- [ ] 2.1 Migrate `VaultState.key`, `vault/mod.rs` (unlock, change-password incl. zeroized re-encryption buffers, restore, import/export) and `project/mod.rs` to `VaultKey`/`SecretString`. Verify with `cargo check` and `cargo test`.
- [ ] 2.2 Migrate the `api/mod.rs` handlers (unlock body, share/relay/inject/fill) and `share/*`, `vault/share_commands.rs`, `project/relay_commands.rs`, `biometric/`. Verify with `grep -rn '\*\*k\b\|\*\*key\b' src-tauri/src` returning nothing, and with `cargo test`.

## 3. CLI/TUI/MCP

- [ ] 3.1 Wrap every `rpassword` result in `Zeroizing` in `client.rs`; the MCP passphrase params use `SecretString`. Verify with `cargo check` and a grep showing no bare `prompt_password(...)?` bound to a `String`.
- [ ] 3.2 TUI: pre-allocated zeroizing password input with a 256-char cap, wiped on submit/Esc/quit; single-copy `Zeroizing` reveal rendered by borrow; corrected wording. Verify with TUI reducer tests (Esc wipes: the buffer is empty after the event) and a manual check.

## 4. Guard and verification

- [ ] 4.1 Add the `tests/key_hygiene.rs` regex guard with the allowlist marker. Verify that it fails on a deliberately inserted `fn f(key: [u8; 32])` and passes on the clean tree.
- [ ] 4.2 Run `cargo clippy --all-targets && cargo test`. All pass.
