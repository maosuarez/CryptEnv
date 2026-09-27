# Desktop UI Specification

## Purpose

Defines the requirements for the CryptEnv desktop user interface presentation, including accurate branding and dynamic versioning, contextual back navigation, light/dark theme contrast, and multilingual internationalization.

## Requirements

### Requirement: Accurate Branding and Dynamic Version Display
The LockScreen footer and desktop interfaces MUST explicitly identify the application as "CryptEnv" and MUST display the dynamically retrieved application package version rather than a static or hardcoded version string. The cryptographic algorithm indicators MUST accurately state "AES-256-GCM · Argon2id key derivation".

#### Scenario: LockScreen version display
- **WHEN** the user opens or views the LockScreen
- **THEN** the screen displays the product name "CryptEnv" with the dynamic package version (e.g., `CryptEnv v1.0.3 · local only`) and the verified cryptographic technologies `AES-256-GCM · Argon2id key derivation`.

#### Scenario: Dynamic version retrieval fallback
- **WHEN** the application retrieves its package version at runtime
- **THEN** the system resolves the version from runtime metadata, falling back gracefully to the bundle version if dynamic retrieval fails.

### Requirement: System and Storage Diagnostic Clarity in Settings
The Settings screen MUST display accurate, dynamic technical diagnostics replacing obsolete hardcoded strings. The diagnostic block MUST display the actual dynamic CryptEnv version, the verified cryptography stack (`AES-256-GCM · Argon2id`), and the accurate local storage location corresponding to the application data directory.

#### Scenario: Viewing system info in Settings
- **WHEN** the user navigates to the Settings screen
- **THEN** the diagnostic summary displays `CryptEnv v{version}`, active cryptographic primitives, and the accurate local storage path where the encrypted database is maintained.

#### Scenario: Diagnostics expose no secret material
- **WHEN** the frontend requests system diagnostics (available while the vault is locked)
- **THEN** the response contains only the app version, application data directory, database path, and OS/architecture
- **AND** it MUST NOT include master passwords, derived keys, tokens, or vault contents

### Requirement: Contextual Back Navigation
Navigation back controls on secondary screens (including Settings, Category Manager, and Item Edit/Create screens) MUST NOT unconditionally route to Global Secrets. Back navigation MUST return the user contextually to their previous screen, or default to the primary Projects screen if no prior history exists.

#### Scenario: Back from Settings opened from Projects
- **WHEN** the user navigates to Settings from the Projects view and clicks "Back"
- **THEN** the application transitions back to the Projects view.

#### Scenario: Back from Settings opened from Global Secrets
- **WHEN** the user navigates to Settings from the Global Secrets view and clicks "Back"
- **THEN** the application transitions back to the Global Secrets view.

#### Scenario: Back from Category Manager or Edit Screen
- **WHEN** the user clicks "Back" on Category Manager or Item Edit view
- **THEN** the application returns to the preceding view from which the user navigated.

#### Scenario: No prior history
- **WHEN** the user clicks "Back" on a secondary screen with no recorded navigation history
- **THEN** the application transitions to the Projects view

#### Scenario: Lock screen is never a back target
- **WHEN** the recorded history contains the Lock screen, or the vault is locked or unlocked
- **THEN** back navigation MUST NOT return to the Lock screen, and history is cleared on lock and unlock

### Requirement: Light Theme and Enhanced Contrast
The application MUST support both a Dark Theme and a Light Theme ("modo claro"), selectable by the user in Settings. All text tokens, borders, and input controls in both themes MUST provide clear visual contrast and adhere to accessible readability standards.

#### Scenario: Switching to Light Theme
- **WHEN** the user selects the Light Theme in Settings
- **THEN** the entire application interface updates its palette to light backgrounds with dark high-contrast typography and borders, persisting the theme preference across application restarts.

#### Scenario: Enhanced text contrast in Dark Theme
- **WHEN** the user uses Dark Theme
- **THEN** secondary and muted text elements (such as timestamps, tags, labels, and hints) retain sufficient contrast against dark background surfaces to remain easily readable.

### Requirement: Multi-language Support and Settings Localization
The application interface MUST support internationalization (i18n) for English (`en`), Spanish (`es`), and Portuguese (`pt`). The active language MUST be selectable exclusively via a dedicated language control in Settings, and all application strings across all screens, modals, and notifications MUST reflect the selected language immediately.

#### Scenario: Changing language in Settings
- **WHEN** the user changes the language setting to Spanish or Portuguese in Settings
- **THEN** all user-facing labels, buttons, headers, modals, and toast messages immediately switch to the selected language and persist the choice across sessions.

#### Scenario: Initial language initialization
- **WHEN** the application starts for the first time or without saved language preferences
- **THEN** the application initializes with a default language (English or system locale if matching English/Spanish/Portuguese) and loads the corresponding translations.

#### Scenario: Invalid or missing saved language
- **WHEN** the saved language preference is absent or not one of `en`, `es`, `pt`
- **THEN** the application ignores it and applies the initial-language rule

#### Scenario: Missing translation key
- **WHEN** a translation key is missing in the active language at runtime
- **THEN** the English string is shown, and every locale MUST define the same keys as English (enforced at compile time and by tests)


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
