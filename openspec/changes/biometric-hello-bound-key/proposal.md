## Why

Biometric unlock stores the **master password** protected only with user-scope DPAPI: no entropy and no prompt flag (`vault/mod.rs:1477`, `biometric/mod.rs`). Windows Hello is only a UI check made before the unlock. Any process running as the user can read `settings.biometric_blob` from `vault.db`, call `CryptUnprotectData`, and get the plaintext master password silently, without a Hello prompt (CORE-4). In addition, `vault_change_password` never updates or clears the blob. After a password change, biometric unlock breaks permanently and the *old* password stays recoverable on disk.

## What Changes

- **Hello-bound key instead of a stored password.** Enrollment creates a Windows Hello `KeyCredential` (a TPM-backed key when available) and signs a random challenge with it; producing that signature requires a Hello gesture. A wrapping key is derived from the signature and used to encrypt the *vault key*, not the password. The stored blob holds only `{version: 2, challenge, wrapped_vault_key}`. The master password is never stored.
- **Unlock** asks Hello to sign the same challenge, which prompts the user. It derives the wrapping key, unwraps the vault key, and verifies it against the vault's verify token.
- **Enrollment self-test.** Enrollment signs the challenge twice and refuses if the signatures differ, because a non-deterministic signature scheme would make unwrapping impossible.
- **Password change clears the enrollment** and tells the user to re-enroll. Disabling the feature deletes both the Hello credential and the blob.
- **Legacy blobs are deleted.** On first launch after the upgrade, an existing v1 (DPAPI-password) blob is deleted securely, and the user is told to re-enroll. **BREAKING** for existing biometric users; they must re-enroll once.

## Capabilities

### New Capabilities
- `biometric-unlock`: what biometric unlock stores, what it requires at unlock time, and how it interacts with password changes and upgrades.

### Modified Capabilities
<!-- none -->

## Impact

- `biometric/mod.rs` (KeyCredentialManager create/open/sign/delete; DPAPI removed), `vault/mod.rs` (enroll, unlock, disable, change-password hook, legacy cleanup at startup), and `Cargo.toml` (the `windows` crate feature `Security_Credentials` and `Security_Cryptography_Core`/`Storage_Streams` for the buffers).
- GUI: re-enroll notice after an upgrade or password change; i18n.
- Docs: the security section on biometric unlock in `docs/index.html`.
- Windows-only; other platforms are unchanged (not available).

## Non-Goals

- macOS Touch ID or Linux biometrics.
- Protecting against malware that can drive the Hello UI with the user present and consenting.
- Changing the Argon2 parameters or the vault key derivation.
