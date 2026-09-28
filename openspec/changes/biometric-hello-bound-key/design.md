## Context

The current flow:
- **Enroll:** a `UserConsentVerifier` prompt, then `dpapi_protect(password)` → `settings.biometric_blob`.
- **Unlock:** a prompt, then `dpapi_unprotect` → run the normal password unlock.

`UserConsentVerifier` only returns a boolean; it cannot gate a key. The WinRT `KeyCredentialManager` (Windows Hello for Business / "Microsoft Passport") creates a per-app asymmetric key protected by Hello (and the TPM when present). `RequestSignAsync` shows the Hello prompt.

## Goals / Non-Goals

**Goals:** a vault key that is only recoverable with a Hello gesture; nothing reusable on disk.

**Non-Goals:** cross-platform biometrics.

## Decisions

### D1. Key wrapping from a deterministic Hello signature
- **Enroll:**
  1. `KeyCredentialManager::RequestCreateAsync("CryptEnv.Vault", ReplaceExisting)`.
  2. `challenge = random 32 bytes`.
  3. `sig1 = RequestSignAsync(challenge)` and `sig2 = RequestSignAsync(challenge)` (two prompts, or one if Windows caches the gesture within its window). Require `sig1 == sig2`.
  4. `wrap = HKDF-SHA256(ikm=sig1, salt=challenge, info="cryptenv-bio-v2")`.
  5. `blob = AES-256-GCM(wrap, vault_key)`.
  6. Store `{"v":2,"challenge":hex,"wrapped":hex(nonce||ct)}` in `settings.biometric_blob`.
- **Unlock:** `OpenAsync("CryptEnv.Vault")` → sign the challenge → derive → decrypt → verify the token → commit the key (same code path as a password unlock, from phase 3 of `vault-lock-and-runtime-availability` D1).

KeyCredential keys are RSA-2048 with PKCS#1 v1.5 signatures, which are deterministic. This is the scheme KeePassXC-style Hello integrations rely on; the self-test guards against a platform change.

*Rejected:*
- DPAPI with `CRYPTPROTECT_PROMPTONPROTECT`: the prompt is a password or consent dialog, not Hello-bound, and malware can still unprotect silently with `CRYPTPROTECT_UI_FORBIDDEN` unset.
- Storing the password under a Hello key: unnecessary exposure. The vault key is enough and can't be reused elsewhere.

### D2. Password change
In `vault_change_password`, after a successful commit: `KeyCredentialManager::DeleteAsync("CryptEnv.Vault")` (best effort), clear the setting, and set `settings.biometric_reenroll_notice = "1"`.

### D3. Legacy cleanup
At startup, if `biometric_blob` is non-empty and does not parse as v2 JSON, overwrite the setting with an empty string, run `VACUUM` (in the `secure_delete` context from `vault-db-transactional-integrity`), and set the notice flag. This needs no unlock.

### D4. WinRT buffers
`CryptographicBuffer::CreateFromByteArray` / `CopyToByteArray` (feature `Security_Cryptography`). Signatures and derived keys go into `Zeroizing`. All WinRT async calls run in `spawn_blocking` (as today).

## Security & Threat Model

| Adversary | v1 | v2 |
|---|---|---|
| User-level malware, user absent | Recovers the master password silently | Must trigger a visible Hello prompt; without a gesture, nothing |
| Malware with the user present, who approves a spoofed prompt | Password | Vault key for this session (not the password) |
| Disk or backup theft | DPAPI needs the user's logon secret; offline attacks possible | Needs the TPM or Hello key; not exportable |

## Risks / Trade-offs

- [Two Hello prompts during enrollment] → Acceptable once; the UI explains it.
- [Windows versions without KeyCredential support] → `KeyCredentialManager::IsSupportedAsync` is checked; the feature is hidden otherwise.
- [BREAKING re-enroll] → A one-time notice.

## Migration Plan

Automatic deletion of legacy blobs at startup; the user re-enrolls from Settings. Rollback to 1.0.6: the biometric blob (v2) is ignored by the old code, which treats it as corrupt, and the user uses the password.
