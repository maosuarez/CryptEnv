## ADDED Requirements

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
