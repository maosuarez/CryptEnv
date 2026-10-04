## Purpose

Defines responsiveness and startup guarantees for the desktop backend. One slow or blocked operation must not freeze the other interfaces or prevent auto-lock, and a recoverable problem must not prevent the app from starting.

## ADDED Requirements

### Requirement: Vault lock not held across user interaction or key derivation

The vault's shared lock MUST NOT be held while waiting for user input (dialogs, prompts) or while running a password-based key derivation. Key derivation MUST run off the async runtime's worker threads.

#### Scenario: Export dialog left open
- **WHEN** the user opens the project export save dialog and leaves it open past the auto-lock timeout
- **THEN** the vault auto-locks on schedule, and REST requests keep responding while the dialog is open

#### Scenario: Unlock in progress
- **WHEN** a CLI unlock is deriving the key
- **THEN** GUI commands that don't need the key, and the REST health endpoints, respond without waiting for the derivation

### Requirement: Fair unlock throttling

Unlock throttling SHALL count only failed password attempts. It SHALL apply an increasing delay: 1 second doubling to at most 60 seconds between allowed attempts after a failure. A successful unlock SHALL reset the throttle. Requests rejected before password verification (malformed, vault not initialized) SHALL NOT count.

#### Scenario: Local spam of malformed requests
- **WHEN** a local process sends 100 malformed `/unlock` requests
- **THEN** a subsequent valid unlock from the CLI with the correct password succeeds immediately

#### Scenario: Password guessing
- **WHEN** 5 wrong passwords are sent in a row
- **THEN** the next attempt is refused until at least 16 seconds have passed since the last failure

### Requirement: Bounded auto-lock timeout

`auto_lock_timeout` MUST be either 0 (never auto-lock) or an integer from 1 to 1440 minutes. Other values SHALL be rejected with a validation error, and the stored value SHALL remain unchanged. No duration computed from this setting may overflow or panic.

#### Scenario: Huge value
- **WHEN** a client sets `auto_lock_timeout` to `9223372036854775807`
- **THEN** the request is rejected, and the next unlock and CLI session work normally

### Requirement: Safe vault wipe

Resetting the vault MUST NOT leave the application without a usable database. If the existing database file can't be moved aside, the reset SHALL fail and the current vault SHALL remain open and usable. Old database files, including write-ahead-log files, SHALL be overwritten and deleted. If deletion fails, it SHALL be retried at every startup until it succeeds.

#### Scenario: File held by another process
- **WHEN** the user resets the vault while an antivirus scanner holds `vault.db` open, so it can't be renamed
- **THEN** the reset reports an error, and the current vault stays usable in this session and at the next launch

### Requirement: Startup recovery from an unopenable database

If the vault database can't be opened at startup, the application MUST NOT terminate. It SHALL show a recovery screen with two options: move the database aside under a timestamped name and start with an empty vault, or quit. Moving aside SHALL NOT delete the original file.

#### Scenario: Corrupt file
- **WHEN** `vault.db` is corrupt at launch
- **THEN** the app shows the recovery screen, and choosing "move aside" renames the file to `vault.db.corrupt-<timestamp>` and opens a fresh vault
