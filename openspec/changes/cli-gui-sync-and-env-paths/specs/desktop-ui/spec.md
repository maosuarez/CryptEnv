## ADDED Requirements

### Requirement: Reactive Desktop GUI Synchronization on Vault Changes

The desktop application MUST automatically synchronize its interface state whenever vault data (items, projects, environments, categories) is modified by another client. The backend MUST emit a payload-free `vault_changed` event over the Tauri event channel (a) after an authenticated REST API request on a data-changing route has succeeded (additions, updates, deletions and imports made through the CLI, TUI or MCP) and (b) after a bulk GUI command has succeeded: backup restore, password-manager import, share-package import and relay receive (item and project). The event MUST NOT be emitted for failed, unauthenticated, policy-denied or pending-approval requests, and MUST NOT be emitted for ordinary single-item or single-environment GUI mutations, whose results the invoking window already applies. The desktop frontend MUST listen for `vault_changed` events, collapse bursts, and trigger a refetch of active query data and Zustand stores.

#### Scenario: Vault item added via CLI while GUI is open
- **WHEN** a secret or environment variable is added via `crypt-env add` or REST API while the desktop application is running
- **THEN** the backend emits `vault_changed`
- **AND** the GUI receives the event and automatically reloads project and item data without requiring application restart or manual page navigation

#### Scenario: Environment or project modified via CLI
- **WHEN** an environment or project is updated via CLI `crypt-env config`
- **THEN** the GUI receives `vault_changed` and updates the active project view to display the latest configuration

#### Scenario: Rejected or pending request does not signal
- **WHEN** a REST write fails, is rejected for authentication or MCP policy, or answers `202` pending approval
- **THEN** no `vault_changed` event is emitted

#### Scenario: Bulk import from the GUI signals
- **WHEN** a backup restore, an import of items or a relay receive completes successfully from the GUI
- **THEN** `vault_changed` is emitted with no payload

#### Scenario: Single-item GUI edit does not signal
- **WHEN** the user saves one item or one environment from the GUI
- **THEN** no `vault_changed` event is emitted

### Requirement: Manual GUI Refresh Trigger

The desktop application MUST provide a user-accessible manual refresh control in the titlebar / header area, as well as a standard keyboard shortcut (`Ctrl+R` or `F5`). Triggering the refresh MUST invalidate all cached project, environment, and item stores and fetch current state from the local backend.

#### Scenario: Clicking manual refresh button
- **WHEN** user clicks the refresh icon button in the desktop interface
- **THEN** an active refresh animation or indicator is displayed
- **AND** the interface re-queries all projects, environments, and items from the backend

#### Scenario: Pressing refresh keyboard shortcut
- **WHEN** user presses `Ctrl+R` (or `Cmd+R` on macOS, or `F5`) while the desktop window is focused
- **THEN** the application prevents default browser reload behavior and triggers the internal vault data refresh

## MODIFIED Requirements

### Requirement: Multi-Path Injection Configuration in Project Environments

The GUI environment settings SHALL allow configuring multiple injection paths for an environment. Configured paths SHALL represent directory folders relative to the project root (defaulting to `./`) or explicit file paths. The system SHALL automatically derive the target `.env` filename based on the environment name (e.g. `<folder>/.env` for default environments, `<folder>/.env.<name>` for named environments) while preserving explicit file paths. Users SHALL be able to add, edit, and remove multiple relative folder paths where environment variables can be injected.

#### Scenario: Adding multiple injection paths to an environment
- **WHEN** user edits an environment and adds `./apps/web/.env` and `./apps/api/.env`
- **THEN** both paths are saved in the environment's `paths` array in the vault database

#### Scenario: Defaulting to root folder for new environment
- **WHEN** user creates a new environment in a project without specifying custom paths
- **THEN** the environment is initialized with `./` as its default target directory
- **AND** the GUI indicates that variables will be injected into `<root>/.env.<name>` (or `<root>/.env` if default)

#### Scenario: Adding multiple relative folder paths
- **WHEN** user edits an environment and configures folder paths `./apps/web` and `./apps/api`
- **THEN** both folder paths are saved in the environment's `paths` array
- **AND** injecting into those targets generates `./apps/web/.env.<name>` and `./apps/api/.env.<name>` respectively

#### Scenario: Backward compatibility with explicit file paths
- **WHEN** an environment contains an existing path ending with a `.env*` filename (e.g. `config/.env.local`)
- **THEN** the system injects directly into that explicit file path without appending an extra `.env` suffix

### Requirement: Customizable Initial Project Environment
When creating a new project, the modal MUST prompt the user for an optional initial environment name. If left blank, the environment name MUST default to "default" and the project MUST also be created with the baseline environments `staging` and `production` (empty, targeting `./` when the project has a root). If a custom name is supplied, the project MUST be created with the specified initial environment instead of "default" and with no baseline environments added. Existing projects MUST NOT gain environments.

#### Scenario: Default initial environment when omitted
- **WHEN** the user creates a project without modifying or specifying an initial environment name
- **THEN** the system creates the project with an initial environment named "default"
- **AND** also creates the environments "staging" and "production"

#### Scenario: Custom initial environment specified
- **WHEN** the user enters a custom environment name (e.g., "production" or "staging") during project creation
- **THEN** the system creates the project with an initial environment using the specified name
- **AND** no baseline environments are added
