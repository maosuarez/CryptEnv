# gui-lock-hygiene Specification

## Purpose
Defines what the desktop UI must remove or hide when the vault locks, and how secrets copied to the OS clipboard are limited in lifetime and spread.

## Requirements

### Requirement: Lock removes secret-bearing UI

When the vault locks for any reason (the user, auto-lock, the backend, or a wipe), every UI element that displays or can copy secret material SHALL close, and its state SHALL be cleared, before or at the moment the lock screen appears. This includes command placeholders, the setup wizard's MCP token, reveal views, and share or relay dialogs. None of these elements SHALL be rendered while the lock screen is shown.

#### Scenario: Auto-lock with placeholder modal open
- **WHEN** a command placeholder modal is open and the vault auto-locks
- **THEN** only the lock screen is visible, and after unlock the modal is not restored

#### Scenario: Setup wizard open at lock
- **WHEN** the setup wizard is showing the MCP token and the vault locks
- **THEN** the wizard closes, and its copy action is unavailable

### Requirement: Secret clipboard lifetime

A secret copied to the clipboard by crypt-env SHALL be removed from the clipboard 30 seconds after it was copied, and immediately when the vault locks, provided the clipboard still holds the content crypt-env placed there. Content copied later by the user or by another application MUST NOT be cleared.

#### Scenario: Timer expiry
- **WHEN** the user copies a secret value and waits 30 seconds
- **THEN** the clipboard no longer contains the value

#### Scenario: User copied something else
- **WHEN** the user copies a secret and then, within 30 seconds, copies unrelated text in another app
- **THEN** the unrelated text remains in the clipboard after 30 seconds

#### Scenario: Lock clears
- **WHEN** the user copies a secret and the vault locks 5 seconds later
- **THEN** the clipboard is cleared at lock

### Requirement: Secrets excluded from clipboard history and sync

On Windows, secrets copied by crypt-env SHALL be marked so that clipboard history and cloud clipboard sync exclude them.

#### Scenario: Win+V
- **WHEN** the user copies a secret and opens Windows clipboard history
- **THEN** the secret is not listed
