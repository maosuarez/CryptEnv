# key-material-handling Specification

## Purpose
Defines how cryptographic keys, passwords and revealed secret values are held in memory, so that they don't outlive their use in any crypt-env binary (desktop backend, CLI/TUI, MCP server).

## Requirements

### Requirement: Keys are held only in zeroizing containers

The vault key, and every key derived from a password or passphrase, MUST be held only in a container that overwrites its bytes when dropped. It MUST NOT be copied into a plain array or buffer that outlives the operation using it. Cryptographic library state derived from these keys (key-schedule and KDF working memory) SHALL be zeroized when released.

#### Scenario: Handler finishes
- **WHEN** a REST handler decrypts items with the vault key and returns
- **THEN** no copy of the key made by that handler remains in unwiped memory

### Requirement: Passwords are wiped after use

Every master password, backup password or share passphrase received by any interface (GUI command, REST body, CLI prompt, TUI input) MUST be held in a zeroizing container from the moment it is received, and wiped as soon as the operation completes, on success and on error. Input buffers SHALL NOT reallocate in a way that leaves unwiped copies.

#### Scenario: TUI login cancelled
- **WHEN** the user types part of the password in the TUI and presses Esc
- **THEN** the typed characters are wiped before the TUI exits

### Requirement: Revealed values have a single bounded copy

When a secret value is revealed in the TUI, at most one plaintext copy SHALL be held by crypt-env for display, and it SHALL be wiped when the reveal closes. Rendering MUST NOT create a new copy per frame. User-facing text SHALL NOT claim stronger guarantees than those provided.

#### Scenario: Reveal held open
- **WHEN** a value is revealed in the TUI for 10 seconds and then closed
- **THEN** crypt-env held one plaintext copy, which is wiped on close

### Requirement: Regression guard

The build's test suite SHALL fail if code outside the crypto module declares a raw 32-byte key type in a function signature or struct field, or dereferences a zeroizing key into a plain copy.

#### Scenario: Regression introduced
- **WHEN** a contributor adds `fn f(key: [u8; 32])` in `api/mod.rs`
- **THEN** `cargo test` fails with a message pointing to `VaultKey`
