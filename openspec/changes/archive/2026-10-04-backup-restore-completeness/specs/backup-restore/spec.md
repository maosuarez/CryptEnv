## Purpose

Defines what a vault backup contains and the guarantees of restoring one. A backup preserves the user's full organisation, a restore never destroys data on failure, and only the vault owner can replace a vault.

## ADDED Requirements

### Requirement: Complete backups

A backup MUST contain every item (as stored ciphertext), every category, every project (name, description, template, root path, categories), every environment (name, default flag, target paths), every environment variable binding, and item ownership. Organisational metadata MUST be encrypted with the vault key inside the backup. The MCP token and biometric enrollment data MUST NOT be included.

#### Scenario: Round trip
- **WHEN** a user exports a backup, resets the vault, and restores the backup with the same master password
- **THEN** all projects, environments, paths, variable bindings, categories and items are identical to before

#### Scenario: Metadata not readable without the password
- **WHEN** someone opens a v2 backup file without the master password
- **THEN** project names, paths and variable keys are not readable

### Requirement: Atomic restore

A restore MUST either fully succeed, or leave the current vault exactly as it was. The previous vault data SHALL be retained on disk until the next successful unlock after the restore.

#### Scenario: Failure during restore
- **WHEN** a restore fails partway (for example, the disk becomes full)
- **THEN** the current vault still opens with the current password and contains all of its data

### Requirement: Restore ends sessions holding the old key

When the vault key changes (a replace restore installing a different vault, or a re-key), REST sessions and any in-progress LAN share session MUST end, and the share session's key copies MUST be dropped.

#### Scenario: Replace restore during a share session
- **WHEN** a LAN share session is active and a replace restore installs a vault with a different key
- **THEN** the share session is cancelled and holds no vault key

### Requirement: Restore authorization

Replacing the vault from a backup MUST require the vault to be unlocked, and the backup's master password to be verified. Merging a backup into the vault MUST require the vault to be unlocked.

#### Scenario: Locked vault
- **WHEN** a replace-restore is attempted while the vault is locked
- **THEN** the restore is refused, and nothing changes

### Requirement: Restored and imported items are owned

A non-global item created by restoring a legacy backup, or by an import without a target project, MUST be assigned to a project so that it is never reported as an orphan. That project SHALL be created automatically, named `Restored <date>` or `Imported <date>`.

#### Scenario: Legacy backup restore
- **WHEN** a v1 backup with 5 non-global items is restored
- **THEN** a project `Restored <date>` owns the 5 items, and none of them is offered for pruning

### Requirement: Legacy compatibility

v1 backups SHALL remain restorable. The restore result SHALL state that project structure was not part of the backup.

#### Scenario: v1 file
- **WHEN** the user restores a v1 `.cenvbak`
- **THEN** items and categories are restored, and the summary notes that projects were not included
