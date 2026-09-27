## MODIFIED Requirements

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

## ADDED Requirements

### Requirement: Cross-Platform Global Shortcut Registration and Lifecycle
The application MUST dynamically register and re-register the global shortcut to toggle application visibility according to user settings at runtime, without requiring an application restart. On application launch, the desktop backend MUST load the persisted hotkey from settings (falling back to a cross-platform default `Ctrl+Alt+Z` on Windows/Linux or `Cmd+Alt+Z` on macOS). When the user saves a new hotkey, the backend MUST unregister the previous shortcut and register the new shortcut immediately.

#### Scenario: Dynamic hotkey re-registration
- **WHEN** the user changes the global hotkey in Settings and saves settings
- **THEN** the backend unregisters the prior shortcut and registers the new shortcut with the operating system without restarting the application
- **AND** pressing the new shortcut immediately toggles the application window visibility.

#### Scenario: Startup hotkey initialization
- **WHEN** the application starts up
- **THEN** the backend reads the configured hotkey from vault settings and registers it as the global shortcut, or registers the platform default if unset.

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
