## 1. Platform layer

- [ ] 1.1 Add the `windows` crate features for `Security_Credentials`, `Security_Cryptography` and `Storage_Streams`, and implement `hello_create`, `hello_open`, `hello_sign(challenge)`, `hello_delete` and `hello_supported` in `biometric/mod.rs`; remove the DPAPI functions. Verify with `cargo check --target x86_64-pc-windows-msvc` (via `cargo xwin check` from WSL or on Windows) and `cargo check` on Linux (the non-windows stubs compile).
- [ ] 1.2 Add the wrap/unwrap helpers (HKDF + AES-GCM, zeroizing), testable without Windows by using a fake signer. Verify with unit tests: the round trip works; a different signature → unwrap fails.

## 2. Vault integration

- [ ] 2.1 Enroll: self-test (sig1 == sig2), store the v2 blob. Unlock: sign, unwrap, verify the token, commit via the shared unlock path. Verify with unit tests using the fake signer, including mismatched signatures → enrollment refused.
- [ ] 2.2 Password change and disable delete the credential and blob, and set the notice flag. Verify with a unit test.
- [ ] 2.3 Startup legacy cleanup: delete a non-v2 blob and set the notice flag. Verify with a unit test using a v1-shaped hex blob.

## 3. GUI and docs

- [ ] 3.1 Add the re-enroll notice on the lock screen and in Settings, plus i18n. Verify with `pnpm build`.
- [ ] 3.2 Update the security section of `docs/index.html` (what is stored, what Hello protects, the re-enroll note). Verify by review.
- [ ] 3.3 Manual Windows verification: enroll, lock, Hello unlock; cancel the prompt → stays locked; change the password → biometric disabled; upgrade from a 1.0.6 profile → notice shown and the old blob removed. Record the results here.
