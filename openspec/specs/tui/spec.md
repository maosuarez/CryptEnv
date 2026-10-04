# Terminal User Interface (TUI) Specification

## Purpose

Defines the terminal user interface (TUI) for CryptEnv, providing interactive, keyboard-driven vault project management, environment switching, secret inspection, and synchronization.

## Requirements

### Requirement: Interactive project and environment navigation

The TUI SHALL provide a full-screen, keyboard-navigable terminal interface (invoked via `crypt-env tui`) allowing the user to select and browse through vault projects, inspect environments, and view assigned environment variable keys.

#### Scenario: Navigating projects and environments
- **WHEN** user launches `crypt-env tui`
- **THEN** the interface renders an interactive view of active projects and their environments
- **AND** the user can navigate using arrow keys or `j`/`k` to select projects and switch environments

### Requirement: In-TUI project actions and configuration sync

The TUI SHALL support the core project actions accessible in the CLI: project initialization (`init`), configuration synchronization with the vault (`config`), materializing `.env` files (`fill`), and syncing template keys (`sync`), plus reloading vault data (`r`).

Every TUI action that sends an authenticated request to the vault, including reload, init, config, fill, sync and reveal, MUST first check for a live session. When there is none, it MUST collect the master password through the TUI's modal dialog. While the TUI owns the terminal, no code path SHALL read a password or confirmation from the terminal outside the TUI's own event loop. If a session expires in the middle of an action, the TUI SHALL show the password modal instead of prompting on the raw terminal. Ctrl+C and `q` SHALL remain able to exit the TUI in every state (including modals); Ctrl-modified keys MUST NOT trigger the plain-letter action (for example, Ctrl+C MUST NOT run `config`).

A `config` action that is a secret-routing change (see the CLI *Bidirectional project configuration sync via config* requirement) SHALL show the diff in a TUI modal and apply it only after an explicit confirmation there. A root-binding mismatch SHALL be shown as an error with the bound root, and SHALL NOT offer to relink from inside the TUI.

#### Scenario: Running fill or sync from the TUI
- **WHEN** user triggers the fill or sync action inside the TUI
- **THEN** the TUI prompts for master password credentials within a modal dialog unless the terminal already holds a live CLI session
- **AND** upon successful authentication, executes the action and displays the status summary

#### Scenario: Reload after the session expired
- **WHEN** the session has expired and the user presses `r`, `c` or confirms `i`
- **THEN** the TUI opens the password modal, and after successful authentication runs the requested action
- **AND** the terminal stays responsive: Enter submits the modal, and Ctrl+C or `q` exits the TUI

#### Scenario: Config path change from the TUI
- **WHEN** user presses `c` and the local manifest adds a new environment path
- **THEN** the TUI shows the path diff in a confirmation modal and changes the vault only if the user confirms

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

### Requirement: Responsive event loop

The TUI SHALL keep responding to input while it redraws, with no per-frame filesystem traversal and no repeated compilation of unchanged filters. Every network request made from the TUI SHALL time out within 5 seconds, and SHALL show a status while it is pending. A slow or unreachable backend MUST NOT freeze the TUI for longer than that timeout.

#### Scenario: Backend unreachable
- **WHEN** the backend stops responding and the user presses `r`
- **THEN** within 5 seconds the TUI shows a connection error and accepts input again

#### Scenario: Workspace on a slow mount
- **WHEN** the TUI runs in a directory under `/mnt/c`
- **THEN** idle redraws do not access the filesystem

### Requirement: Terminal always restored

On every exit path, whether a normal quit, an error during startup after raw mode was enabled, an error during operation, or a panic, the TUI SHALL leave the alternate screen and disable raw mode.

#### Scenario: Startup failure after raw mode
- **WHEN** creating the terminal backend fails after raw mode was enabled
- **THEN** the TUI exits with an error, and the shell is in normal (cooked) mode
