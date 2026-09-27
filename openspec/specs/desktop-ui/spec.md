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
