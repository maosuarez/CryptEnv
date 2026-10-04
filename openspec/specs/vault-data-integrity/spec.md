# vault-data-integrity Specification

## Purpose
Defines the atomicity and consistency guarantees of vault write operations. A failed operation leaves the vault exactly as it was before, and unrelated data is never altered as a side effect.

## Requirements

### Requirement: Category edits preserve unrelated links

Saving the category list MUST only add, update or remove the categories that actually changed. Project–category and item–category associations of categories that still exist after the save MUST be preserved. The save SHALL be atomic.

#### Scenario: Rename one category
- **WHEN** a user renames category `Backend` to `API` in the GUI while three projects are tagged with `Frontend`
- **THEN** the three projects remain tagged with `Frontend`

#### Scenario: Failure mid-save
- **WHEN** a category save fails partway
- **THEN** the category list and all associations are unchanged

### Requirement: Atomic environment save with input validation

Saving an environment (name, target paths, variables, ownership changes) MUST be atomic: either every change is applied, or none is. Input containing the same variable key twice (case-sensitive) or the same path twice SHALL be rejected with a validation error before anything is written.

#### Scenario: Duplicate key
- **WHEN** an environment save contains `API_KEY` twice
- **THEN** the save fails with a duplicate-key error, and the environment's existing variables are unchanged

### Requirement: Unlock is all-or-nothing

The vault MUST NOT be usable by any interface (GUI, REST, MCP) unless the unlock operation fully succeeds. If any step of unlocking fails after the password has been verified, the vault SHALL remain locked.

#### Scenario: Migration failure during unlock
- **WHEN** the password is correct but the data migration during unlock fails
- **THEN** the unlock returns an error, and a REST request with a valid MCP token receives 403 (locked)

### Requirement: Item updates are serialized

Concurrent updates to the same item from different interfaces SHALL be serialized. Each update SHALL merge into the item's latest stored state, never into a stale copy.

#### Scenario: GUI and CLI update different fields
- **WHEN** the GUI changes an item's value while the CLI changes its categories at the same time
- **THEN** the stored item has both the new value and the new categories

### Requirement: Atomic fork and migration

Converting an item between global and project-scoped (forking), and migrating legacy literal variables into items, MUST each be atomic. A failure MUST NOT leave duplicate or ownerless items behind.

#### Scenario: Fork failure
- **WHEN** forking a global item for one project fails after creating the copy
- **THEN** no copy exists, and the original item and its links are unchanged

### Requirement: Storage settings on every connection

Foreign-key enforcement and secure deletion (overwriting freed pages) MUST be active on every database connection for the whole application lifetime, including connections opened after the pool recycles idle ones.

#### Scenario: After idle recycling
- **WHEN** the app has been idle for more than 30 minutes, and an item is then deleted
- **THEN** the delete runs on a connection with secure deletion and foreign keys enabled
