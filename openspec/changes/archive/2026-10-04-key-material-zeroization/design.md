## Context

`VaultState.key: Option<Zeroizing<[u8;32]>>` is correct at rest. The leaks happen where handlers do `let k = **key;` to move the key into `spawn_blocking` or into helper signatures typed `&[u8;32]` / `[u8;32]`. Passwords are deserialized straight into `String`.

## Goals / Non-Goals

**Goals:** type-level prevention; no raw copies; wipe passwords.

**Non-Goals:** OS memory locking.

## Decisions

### D1. `VaultKey` newtype
`pub struct VaultKey(Zeroizing<[u8;32]>)` with `Clone` (a clone is also zeroizing on drop), no `Copy`, no `Deref` to an array, and `fn expose(&self) -> &[u8;32]` usable only inside the `crypto` module (`pub(crate)`, with a lint comment). `VaultState.key: Option<VaultKey>`. Handlers that need the key inside `spawn_blocking` clone the `VaultKey` (an `Arc<VaultKey>` where it is shared).

*Rejected:* the `secrecy` crate (adds a dependency; `Zeroizing` already covers this).

### D2. `SecretString`
`#[derive(Deserialize)] #[serde(transparent)] pub struct SecretString(Zeroizing<String>)`. It is used in Tauri command parameters and the REST bodies. The CLI wraps `rpassword` results right away (the `rpassword` String is moved into `Zeroizing`, so the move keeps a single allocation). The TUI input is `Zeroizing<String>` with `with_capacity(256)`, and input is refused beyond 256 characters.

### D3. Crate features
`argon2 = { version = "0.5", features = ["zeroize"] }` and `aes-gcm = { version = "0.10", features = ["zeroize"] }`. Argon2's `hash_password_into` output goes into a `VaultKey` buffer (`Zeroizing::new([0u8;32])`, filled in place).

### D4. TUI reveal
`Modal::Reveal { value: Zeroizing<String> }` is rendered with `Paragraph::new(value.as_str())` (borrowed). ratatui's buffer cells still copy the glyphs into the frame buffer; that cannot be avoided without forking ratatui. The wording changes to "value is cleared from crypt-env's memory when closed; the terminal may retain it in scrollback".

### D5. Guard test
A test in `src-tauri/tests/key_hygiene.rs` scans `src/**/*.rs` (excluding `crypto/` and tests) for the regexes `\[u8;\s*32\]` in `fn`/struct contexts and `\*\*(key|k)\b`, and fails with guidance. This is crude but effective, and cheap.

## Security & Threat Model

This reduces the window in which memory disclosure (a crash dump, swap, a later heap read by a compromised dependency) exposes keys or passwords after use. It does not protect while the vault is unlocked (stated as a Non-Goal).

## Risks / Trade-offs

- [Large mechanical diff that conflicts with the other changes] → Apply after them, or first with a quick rebase; the change is purely type-level.
- [Regex guard false positives, e.g. SHA-256 digests typed `[u8;32]`] → An allowlist comment marker `// key-hygiene: not-a-key`.
