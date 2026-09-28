## Purpose

Defines how Windows Hello biometric unlock protects the vault key. Unlocking requires a Windows Hello gesture, and no reusable secret (in particular the master password) is recoverable from disk by a process running as the user.

## ADDED Requirements

### Requirement: No stored password

Biometric enrollment MUST NOT store the master password, or anything the master password can be recovered from, in any form. The stored enrollment data SHALL be only a random challenge and the vault key encrypted under a key that can be derived solely with a Windows Hello signature over that challenge.

#### Scenario: Blob stolen
- **WHEN** a process running as the user reads the stored enrollment data and has no Windows Hello gesture from the user
- **THEN** it cannot obtain the master password or the vault key

### Requirement: Unlock requires a Hello gesture

Biometric unlock SHALL obtain a Windows Hello signature over the stored challenge, which requires the user's Hello verification. It SHALL derive the wrapping key from the signature, decrypt the vault key, and verify it against the vault's verify token before unlocking. A failed or cancelled gesture SHALL leave the vault locked.

#### Scenario: Cancelled prompt
- **WHEN** the user cancels the Hello prompt
- **THEN** the vault stays locked, and the lock screen offers password entry

### Requirement: Enrollment self-test

Enrollment SHALL sign the challenge twice, and MUST refuse to enroll if the two signatures differ. In that case the user SHALL be told that biometric unlock is not supported on this device.

#### Scenario: Non-deterministic signer
- **WHEN** the platform returns different signatures for the same challenge
- **THEN** enrollment fails with a "not supported on this device" message, and nothing is stored

### Requirement: Enrollment invalidated by password change and disable

Changing the master password MUST delete the biometric enrollment (the stored data and the Hello credential) and notify the user to re-enroll. Disabling biometric unlock SHALL delete both.

#### Scenario: Password changed
- **WHEN** the user changes the master password while biometric unlock is enrolled
- **THEN** biometric unlock is no longer offered until the user re-enrolls, and no data from the old enrollment remains

### Requirement: Legacy enrollment removal

At startup, the application SHALL delete any enrollment data from the previous format, which stored the protected password, and SHALL notify the user once that biometric unlock needs re-enrollment.

#### Scenario: Upgrade from 1.0.6
- **WHEN** a user with biometric unlock enrolled on 1.0.6 starts the new version
- **THEN** the old blob is deleted, and the lock screen shows a one-time "re-enroll biometric unlock" notice
