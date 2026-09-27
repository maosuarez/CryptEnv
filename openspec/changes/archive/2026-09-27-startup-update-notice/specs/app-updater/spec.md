## Purpose

Defines how the CryptEnv desktop app discovers, announces and installs signed updates.

## ADDED Requirements

### Requirement: Automatic update check after unlock

The desktop app SHALL check for an update exactly once per app launch, triggered by the first successful vault unlock (password or biometric). Later unlocks within the same launch MUST NOT trigger another automatic check. The check MUST NOT block or delay the unlock flow.

#### Scenario: First unlock triggers a check

- **WHEN** the app is launched and the user unlocks the vault for the first time
- **THEN** the app calls `check_for_update` in the background

#### Scenario: Re-unlock does not re-check

- **WHEN** the vault auto-locks and the user unlocks it again in the same launch
- **THEN** no additional automatic update check is made

#### Scenario: Check failure is silent

- **WHEN** the automatic check fails (network error, unreachable endpoint, invalid manifest or signature)
- **THEN** no error toast or notice is shown
- **AND** the manual check in Settings remains available and still reports errors

### Requirement: Update available notice

When the automatic check reports a newer version, the app SHALL show a persistent in-app notice naming that version, with an INSTALL action and a LATER action. The notice MUST NOT auto-dismiss and MUST NOT be shown while the lock screen is displayed.

#### Scenario: Newer version found

- **WHEN** the automatic check returns version `X`
- **THEN** a notice reading that version `X` is available is shown on the unlocked screens

#### Scenario: No update

- **WHEN** the automatic check reports no newer version
- **THEN** no notice is shown

#### Scenario: LATER dismisses for this launch

- **WHEN** the user selects LATER
- **THEN** the notice is hidden and does not reappear until the app is relaunched

#### Scenario: Locked while notice pending

- **WHEN** the vault locks while the notice is showing and not dismissed
- **THEN** the notice is hidden on the lock screen and shown again after the next unlock

#### Scenario: INSTALL

- **WHEN** the user selects INSTALL
- **THEN** the app calls `install_update`, disables both actions and shows an installing state
- **AND** on success it tells the user to restart the app to apply the update
- **AND** on failure it shows the error in the notice and re-enables INSTALL

### Requirement: Updater commands never panic

The `check_for_update` and `install_update` commands MUST return an `Err` string instead of panicking when their shared update state cannot be accessed (e.g. a poisoned mutex).

#### Scenario: Poisoned state

- **WHEN** the pending-update mutex is poisoned
- **THEN** the command returns an error and the app keeps running

### Requirement: Signed updates only

Update artifacts MUST be verified against the minisign public key configured in `tauri.conf.json` before installation. The release workflow MUST fail before publishing when `latest.json` contains an empty signature for any platform, or when any updater artifact's signature does not verify against that public key.

#### Scenario: Unsigned manifest

- **WHEN** the release workflow produces a `latest.json` with an empty signature
- **THEN** the workflow fails before creating the GitHub release

#### Scenario: Signing secret does not match the pubkey

- **WHEN** `TAURI_SIGNING_PRIVATE_KEY` holds a key other than the one matching `tauri.conf.json`'s pubkey
- **THEN** the signature verification step fails, naming both key IDs, and no GitHub release is created

#### Scenario: Manual re-run of a release

- **WHEN** the workflow is dispatched manually with `tag: vX.Y.Z`
- **THEN** every job builds the `vX.Y.Z` ref, and `latest.json` and the GitHub release use version `X.Y.Z` and tag `vX.Y.Z`
