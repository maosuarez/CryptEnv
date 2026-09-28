## Purpose

Defines the terminal user interface (TUI) for CryptEnv, providing interactive, keyboard-driven vault project management, environment switching, secret inspection, and synchronization.

## ADDED Requirements

### Requirement: Interactive project and environment navigation

The TUI SHALL provide a full-screen, keyboard-navigable terminal interface (invoked via `crypt-env tui`) allowing the user to select and browse through vault projects, inspect environments, and view assigned environment variable keys.

#### Scenario: Navigating projects and environments
- **WHEN** user launches `crypt-env tui`
- **THEN** the interface renders an interactive view of active projects and their environments
- **AND** the user can navigate using arrow keys or `j`/`k` to select projects and switch environments

### Requirement: In-TUI project actions and configuration sync

The TUI SHALL support the core project actions accessible in the CLI: project initialization (`init`), configuration synchronization with the vault (`config`), materializing `.env` files (`fill`), and syncing template keys (`sync`).

#### Scenario: Running fill or sync from the TUI
- **WHEN** user triggers the fill or sync action inside the TUI
- **THEN** the TUI prompts for master password credentials within a modal dialog unless the terminal already holds a live CLI session
- **AND** upon successful authentication, executes the action and displays the status summary

### Requirement: Secure secret revelation and inspection in TUI

The TUI SHALL display secret variables with values masked by default. The TUI SHALL allow temporary unmasking or copying of a secret value only after the user requests revelation and provides master password authentication.

#### Scenario: Revealing a secret value
- **WHEN** user highlights a secret and presses the reveal key (`v`)
- **THEN** the TUI prompts for the master password if the terminal has no live CLI session
- **AND** displays the secret value in a temporary popup modal, with zeroization upon dismissal

### Requirement: In-TUI variable search and filtering

The TUI SHALL provide interactive searching and filtering of environment variables across the current project and global scope using `/` fuzzy/substring filtering.

#### Scenario: Filtering variables in active project
- **WHEN** user presses `/` in the variable list and types a query
- **THEN** the TUI filters the displayed items in real time, matching variable keys
