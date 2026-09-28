# Desktop UI Specification Delta

## ADDED Requirements

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
