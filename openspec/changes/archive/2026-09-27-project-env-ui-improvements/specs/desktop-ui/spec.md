## ADDED Requirements

### Requirement: Inline Category Creation in Project Modal
The project creation form MUST allow users to create and persist a new category directly within the tag selection interface without navigating away from the project modal. Newly created categories MUST immediately become available, selected for the project, and persisted in the vault database.

#### Scenario: Creating a category inline when no categories exist
- **WHEN** the user creates a project and no categories exist in the vault
- **THEN** the tag selector presents an inline action or input to create a new category
- **AND** entering a category name and saving it persists the category and selects it for the new project without leaving the modal.

#### Scenario: Creating an additional category inline
- **WHEN** the user creates a project and wishes to add a category not present in the existing list
- **THEN** the user can enter the category name directly in the tag selector
- **AND** the category is saved to the vault and added to the project's selected categories.

### Requirement: Customizable Initial Project Environment
When creating a new project, the modal MUST prompt the user for an optional initial environment name. If left blank, the environment name MUST default to "default". If a custom name is supplied, the project MUST be created with the specified initial environment instead of "default".

#### Scenario: Default initial environment when omitted
- **WHEN** the user creates a project without modifying or specifying an initial environment name
- **THEN** the system creates the project with an initial environment named "default".

#### Scenario: Custom initial environment specified
- **WHEN** the user enters a custom environment name (e.g., "production" or "staging") during project creation
- **THEN** the system creates the project with an initial environment using the specified name.

### Requirement: Prominent Environment Variable Key Name Input
In the environment variable addition panel (`AddVarPanel`), the environment variable name (`KEY_NAME`) input field MUST be visually distinctive with a prominent accent/green border, clear labeling, and explanatory text indicating that it defines the exported environment variable name across all item types (secrets, credentials, commands, notes, links).

#### Scenario: Displaying variable key name field
- **WHEN** the user selects "New Item" to add a variable inside an environment
- **THEN** the `KEY_NAME` field is prominently styled with an accent border and clear label indicating it is required
- **AND** its visual priority clearly distinguishes it from the subsequent item-specific payload fields.

### Requirement: Vault Auto-Lock Session Synchronization and Error Handling
The desktop application MUST keep the backend vault session synchronized with active user interactions in the frontend. Active user events (keyboard input, mouse movement, clicks) MUST refresh the backend activity timestamp (`touch()`). If the backend auto-lock fires, the application MUST emit a lock event (`vault_locked`) to prompt the frontend to immediately switch to the lock screen. Any vault operation returning a "vault is locked" error MUST be handled cleanly by notifying the user and redirecting to the lock screen.

#### Scenario: Active user typing prevents premature backend auto-lock
- **WHEN** the user actively interacts with the UI (e.g., typing project or variable details)
- **THEN** user activity notifies the backend to touch its activity timestamp, preventing the backend auto-lock timer from expiring prematurely while the user is actively working.

#### Scenario: Backend lock event transitions frontend UI
- **WHEN** the vault locks in the backend (due to genuine inactivity or explicit lock)
- **THEN** the backend emits a `vault_locked` event
- **AND** the frontend immediately transitions to the lock screen, zeroing sensitive state in memory.

#### Scenario: Vault locked error recovery
- **WHEN** an operation fails with a "vault is locked" response
- **THEN** the frontend catches the error, displays an informative prompt to unlock, and navigates to the lock screen without crashing or leaving stale interactive forms.

### Requirement: Capitalized and Localized Environment Presets
Environment presets available in the environment creation interface (e.g., "production", "local", "test", "staging") MUST be presented with the first letter capitalized (e.g., "Production", "Local", "Test", "Staging") and MUST be localized across supported languages (English, Spanish, Portuguese).

#### Scenario: Displaying environment presets in Spanish
- **WHEN** the user creates a new environment while the active language is Spanish
- **THEN** the environment preset suggestions are displayed with initial capitalization in Spanish (e.g., "Producción", "Local", "Pruebas", "Staging").

#### Scenario: Displaying environment presets in English
- **WHEN** the user creates a new environment while the active language is English
- **THEN** the environment preset suggestions are displayed with initial capitalization (e.g., "Production", "Local", "Test", "Staging").

### Requirement: Strictly Centered Environment View Header
In the project environment view, the environment title in the top header MUST be strictly centered relative to the full viewport width of the panel, regardless of the presence or width of the left-aligned back button.

#### Scenario: Viewing environment details header
- **WHEN** the user navigates into an environment view
- **THEN** the environment title is centered horizontally across the full header area
- **AND** the left-aligned back button does not push or offset the title off-center.
