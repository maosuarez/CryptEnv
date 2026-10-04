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

#### Scenario: High-contrast form controls and dropdown options in Dark Theme
- **WHEN** the user opens or views any `<select>` dropdown (including Auto-lock Timeout and Language in Settings, or environment import selectors) in Dark Theme
- **THEN** the select trigger and expanded option list MUST render with dark surface backgrounds and high-contrast readable text matching application tokens, and MUST NOT render with unstyled white backgrounds.

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

### Requirement: Cross-Platform Global Shortcut Registration and Lifecycle
The application MUST dynamically register and re-register the global shortcut to toggle application visibility according to user settings at runtime, without requiring an application restart. On application launch, the desktop backend MUST load the persisted hotkey from settings (falling back to a cross-platform default `Ctrl+Alt+Z` on Windows/Linux or `Cmd+Alt+Z` on macOS). When the user saves a new hotkey, the backend MUST unregister the previous shortcut and register the new shortcut immediately. A failure to register either the persisted hotkey or the default at startup MUST NOT prevent the application from starting: the application SHALL start without a global shortcut and SHALL tell the user in the GUI that the hotkey is unavailable and can be changed in Settings.

#### Scenario: Dynamic hotkey re-registration
- **WHEN** the user changes the global hotkey in Settings and saves settings
- **THEN** the backend unregisters the prior shortcut and registers the new shortcut with the operating system without restarting the application
- **AND** pressing the new shortcut immediately toggles the application window visibility.

#### Scenario: Startup hotkey initialization
- **WHEN** the application starts up
- **THEN** the backend reads the configured hotkey from vault settings and registers it as the global shortcut, or registers the platform default if unset.

#### Scenario: Hotkey owned by another application
- **WHEN** the application starts and both the persisted hotkey and the default are already registered by another application
- **THEN** the application starts normally without a global shortcut
- **AND** the GUI shows a notice that the hotkey is unavailable and can be changed in Settings.

#### Scenario: Cross-platform modifier adaptation
- **WHEN** the application is running on macOS
- **THEN** modifier keys adapt to macOS conventions (displaying `Cmd` instead of `Ctrl`), and shortcut parsing accepts macOS Command/Super modifiers.

### Requirement: Safe Hotkey Recording and Validation
The Settings interface MUST provide a safe, validated hotkey recording mechanism. While the hotkey recording mode is active, the active global shortcut MUST NOT trigger window toggle or hide the window when pressed. The hotkey recorder MUST require at least one modifier key (Control/Command or Alt) alongside a non-modifier key, and MUST reject invalid single-key shortcuts. The interface MUST provide an option to reset or revert to the default shortcut.

#### Scenario: Recording the current shortcut without closing
- **WHEN** the user enters hotkey recording mode and presses the existing hotkey
- **THEN** the global window toggle is suppressed, the window remains visible, and the pressed shortcut combination is captured into the draft.

#### Scenario: Validation of modifier keys
- **WHEN** the user presses a single character key without a modifier (e.g. `A` or `1`)
- **THEN** the input is not accepted as a valid shortcut and the interface prompts for a modifier combination.

#### Scenario: Resetting to default hotkey
- **WHEN** the user clicks "Reset to default" on the hotkey configuration
- **THEN** the draft hotkey reverts to the platform default combination (`Ctrl+Alt+Z` / `Cmd+Alt+Z`).

### Requirement: Project Categories Representation and Terminology
Project categorization MUST use the terminology "Categories" / "Categorías" instead of "Tags" throughout all project configuration views, filters, and localized strings. Project categories MUST directly correspond to vault categories.

#### Scenario: Category field display in project editor
- **WHEN** the user views or edits a project's details
- **THEN** the field for selecting categories is labeled "Categories" (or "Categorías" in Spanish / "Categorias" in Portuguese) and manages the project's associated vault categories.

#### Scenario: Category filtering in project list
- **WHEN** the user filters projects by category
- **THEN** the filter control displays category names with their corresponding color indicators and empty state messaging refers to categories.

### Requirement: Project Action Bar and Dirty-State Save Visibility
The project settings view MUST place the project "Save" button in the bottom footer action bar, positioned to the left of the "Delete Project" button. The Save button MUST use the high-contrast accent styling (`bg-accent text-[#020504]`). The Save button MUST be visible only when the project name, description, or categories have unsaved changes (`dirty` state), and MUST be hidden when there are no pending changes.

#### Scenario: Project without pending changes
- **WHEN** the user opens an existing project without making edits
- **THEN** the project Save button is not displayed in the action bar.

#### Scenario: Project with modified fields
- **WHEN** the user edits the project name, description, or assigned categories
- **THEN** the Save button appears in the action bar to the left of "Delete Project" styled in high-contrast accent green
- **AND** clicking Save persists the updates to the vault and hides the Save button upon successful save.

### Requirement: Decoupling Scaffolding Templates from Project Detail View
Existing projects MUST NOT display a static "Templates" row or section in their details view. Scaffolding templates MUST serve exclusively during initial project bootstrapping or when adding template-based variable bundles to an environment.

#### Scenario: Viewing existing project details
- **WHEN** the user navigates into an existing project
- **THEN** no "Templates" metadata row or change template action is shown in the project configuration section.

### Requirement: Template Categories Decoupling and Null-Safe Category Management
Selecting templates during project creation MUST NOT automatically inject template category names into the project's categories or create them in the vault's category list. The Category Manager, item lists, and secret badges MUST safely handle items with missing or null category fields without throwing errors or failing to render.

#### Scenario: Creating project with multiple templates does not pollute vault categories
- **WHEN** the user creates a project selecting multiple stack templates (e.g. Node.js, PostgreSQL, Docker)
- **THEN** the project is created with the requested template variables
- **AND** the template category names are NOT automatically added to the project's categories or created as vault categories in the database.

#### Scenario: Category Manager renders safely with null or empty item categories
- **WHEN** the user navigates to the Category Manager and vault items exist with null or empty category fields
- **THEN** the Category Manager renders without uncaught exceptions or blank screens
- **AND** item counts accurately reflect items explicitly assigned to each category.

### Requirement: Canonical Lowercase Environment Normalization with Localized Presentation
The environment selector MUST display human-friendly, localized, and capitalized preset names (such as "Production" / "Producción", "Staging", "Local", "Test") in the user interface, while storing and injecting the canonical environment identifier strictly as lowercase ASCII (e.g., `production`). File injection MUST always target `.env.<canonical_lowercase_name>`.

#### Scenario: Selecting preset environment
- **WHEN** the user creates or edits an environment using the "Production" preset
- **THEN** the UI presents the formatted, localized label "Production" (or "Producción")
- **AND** the environment's stored name and inject target is `.env.production` in lowercase ASCII.

#### Scenario: Custom environment name normalization
- **WHEN** the user specifies a custom environment name
- **THEN** the canonical identifier used for file creation and injection is converted to lowercase alphanumeric ASCII.

### Requirement: Extensionless .env Unnamed Default Environment
The desktop UI MUST represent the canonical `.env` environment as an unnamed environment corresponding to the root `.env` filename without an extension. When an unnamed `.env` environment exists in a project, it MUST be designated as the default environment by default. The option to designate another environment as the default MUST be available only when no unnamed `.env` environment exists in the project.

#### Scenario: Unnamed environment displayed as .env
- **WHEN** the user views or selects the primary default environment in a project
- **THEN** the interface displays `.env` without appending `.default` or any extension
- **AND** the environment name field is blank/unnamed.

#### Scenario: Automatic default prioritization for unnamed environment
- **WHEN** a project contains an unnamed `.env` environment
- **THEN** the system sets that environment as the default environment (`isDefault = true`)
- **AND** the user cannot uncheck or designate another environment as default while the unnamed environment exists.

#### Scenario: Custom default environment selection when no unnamed environment exists
- **WHEN** a project has only named environments (e.g., `production`, `staging`) and no unnamed `.env` environment
- **THEN** the user can designate any of the existing environments as the default environment.

### Requirement: Environment Type Change Confirmation Modal
When an environment is already saved, changing its type or preset in the environment editor MUST NOT take effect immediately. The application MUST display a confirmation modal notifying the user that changing the environment type alters the target inject filename (e.g. `.env` vs `.env.production`). The change MUST take effect only after the user explicitly confirms in the modal.

#### Scenario: Triggering confirmation modal on environment type change
- **WHEN** the user selects a different preset or enters a custom name for an existing environment
- **THEN** a modal appears asking for confirmation, explaining the impact on the target filename
- **AND** if cancelled, the environment type reverts to its previous value without saving.

#### Scenario: Confirming environment type change
- **WHEN** the user confirms the type change in the confirmation modal
- **THEN** the new environment type/name is applied to the editor and marks the environment dirty for saving.

### Requirement: Environment Duplication
The environment management interface MUST provide an action to duplicate an existing environment into a new environment under the same project. The user MUST be prompted to select a target environment type/preset that does not already exist in the project. The duplicated environment MUST copy all variables and item links from the source environment. Configured paths MUST NOT be copied verbatim (two environments would inject into the same file): a path whose filename is the source environment's own file (`.env` or `.env.<name>`) MUST be retargeted to the target environment's file in the same folder, and any other path MUST be omitted.

#### Scenario: Duplicating an environment to a new type
- **WHEN** the user clicks the duplicate environment action on an environment
- **THEN** a modal or prompt allows selecting the target environment type/preset from unused presets in the project
- **AND** upon confirmation, a new environment is created containing identical variable links
- **AND** each source path named after the source environment file points at the target environment file in the same folder (e.g. `C:\app\.env.local` → `C:\app\.env.production`), while other paths are not copied.

### Requirement: Simplified Variable Actions and Delete Confirmation
In the environment variable listing, the row actions MUST provide only two buttons: Reveal/Hide value ('eye') and 'trash'. A separate Unlink ('x') button MUST NOT be displayed. For a project-local item, 'trash' deletes the vault item and MUST first open a confirmation modal. For a global item, 'trash' MUST only unlink it from the current environment (the vault item is kept); global items are deleted from the Global Secrets view.

#### Scenario: Variable row action controls
- **WHEN** the user views a variable row in an environment
- **THEN** only the reveal/hide toggle and the delete (trash) button are displayed
- **AND** the unlink ('close' / 'x') button is not present.

#### Scenario: Confirming variable deletion
- **WHEN** the user clicks the delete button on a project-local variable row
- **THEN** a confirmation modal is displayed indicating the item will be deleted
- **AND** the item is deleted and removed from the environment only after explicit confirmation.

#### Scenario: Unlinking a global variable
- **WHEN** the user clicks the trash button on a variable whose item is global
- **THEN** the variable is removed from the current environment only
- **AND** the global vault item and its links in other environments are unchanged.

### Requirement: Type-Grouped Accordions in Environment Variables
The environment variables list MUST group variables into collapsible accordion sections categorized by item type (`Secret`, `Credential`, `Link`, `Command`, `Note`). Individual variable rows within each accordion MUST NOT render repetitive type badges. Each accordion section MUST display the count of variables within that category and allow expanding or collapsing the section.

#### Scenario: Variables categorized by type in accordions
- **WHEN** the user views variables in an environment
- **THEN** variables are organized into separate accordion sections for Secret, Credential, Link, Command, and Note
- **AND** each row displays its key and masked/revealed value without an uppercase type badge.

#### Scenario: Collapsing and expanding type sections
- **WHEN** the user clicks an accordion category header
- **THEN** the section toggles between expanded and collapsed states, with its state preserved during the view session.

### Requirement: Variable Detail View and Global Status Toggle
The environment editor MUST allow users to view the full details of any variable. Furthermore, the UI MUST allow marking a variable as global (`isGlobal`) both during its initial creation in `AddVarPanel` and when inspecting an existing variable in the detail view.

#### Scenario: Marking a variable as global on creation
- **WHEN** the user opens the `AddVarPanel` to add a new variable
- **THEN** an `isGlobal` checkbox or toggle is available
- **AND** when checked, the newly created item is saved with `isGlobal = true`.

#### Scenario: Inspecting an existing variable in detail view
- **WHEN** the user clicks a variable row or a detail trigger in the environment list
- **THEN** a detail modal or panel opens showing full item metadata, fields, and owning projects
- **AND** the user can toggle the item's `isGlobal` status directly from this view.

### Requirement: Project Root Directory and YAML File Provisioning in GUI

The project creation modal in the GUI SHALL require the user to specify a project root filesystem directory (via a directory path input and/or native folder picker). Upon successful project creation, the GUI backend SHALL write a `.crypt-env.yaml` file into that specified root directory containing the project name, description, categories/tags, and initial environment definitions.

#### Scenario: Creating a project with a valid root path
- **WHEN** user fills in the project creation form, specifies root directory `/workspace/web-app`, and confirms creation
- **THEN** the project is created in the vault
- **AND** a valid `.crypt-env.yaml` file is written to `/workspace/web-app/.crypt-env.yaml`
- **AND** the root path `/workspace/web-app` is associated with the project's base paths

#### Scenario: Specifying an invalid or unwriteable directory
- **WHEN** user specifies a directory where the process lacks write permissions or that does not exist
- **THEN** the GUI displays an inline validation error and prevents project creation until corrected

### Requirement: Multi-Path Injection Configuration in Project Environments

The GUI environment settings SHALL allow configuring multiple injection paths for an environment. Users SHALL be able to add, edit, and remove multiple relative or absolute file or directory paths where environment variables can be injected.

#### Scenario: Adding multiple injection paths to an environment
- **WHEN** user edits an environment and adds `./apps/web/.env` and `./apps/api/.env`
- **THEN** both paths are saved in the environment's `paths` array in the vault database

### Requirement: Interactive Injection Target Selection Modal

When a user initiates an environment injection from the GUI (via the project card quick-inject or the environment editor), if the environment has multiple configured injection paths, the GUI SHALL display an interactive modal dialog prompting the user to select which specific path to inject into, or to inject into "All configured paths". If the environment has only a single configured path, the GUI SHALL inject into that path directly without presenting the selection modal.

#### Scenario: Injecting with multiple configured paths
- **WHEN** user clicks "Inject" on an environment with 2 or more configured paths
- **THEN** the GUI opens an "Inject Target Selection" modal listing each configured path with radio buttons or checkboxes and an "All Paths" option
- **AND** confirming the selection injects secrets only into the selected destination(s)

#### Scenario: Injecting with a single configured path
- **WHEN** user clicks "Inject" on an environment with exactly 1 configured path
- **THEN** the GUI proceeds directly with the injection workflow without prompting for target path selection

### Requirement: Unlock Failure Feedback
The lock screen MUST distinguish unlock failures. A wrong master password SHALL show the existing shake and "incorrect password" message. A throttled attempt SHALL show a distinct, non-shake message with a live countdown of the remaining wait and SHALL disable both the password and biometric submit actions until the countdown reaches 0. An unlock aborted by a concurrent lock, wipe, restore or unlock SHALL show a neutral "try again" message. The desktop commands MUST signal throttled and aborted with stable machine-readable codes, not by the GUI parsing English text; the REST API response shape MUST NOT change.

#### Scenario: Throttled unlock shows a countdown
- **WHEN** the user submits a password while the unlock throttle is active
- **THEN** the lock screen shows the remaining seconds counting down, without the wrong-password shake
- **AND** the submit buttons are disabled until the countdown reaches 0.

#### Scenario: Wrong password keeps the shake
- **WHEN** the user submits a wrong password and the attempt is not throttled
- **THEN** the lock screen shakes and shows "incorrect password".

#### Scenario: Aborted unlock is neutral
- **WHEN** an unlock is aborted because the vault changed during key derivation
- **THEN** the lock screen shows a neutral "try again" message, not the wrong-password message.
