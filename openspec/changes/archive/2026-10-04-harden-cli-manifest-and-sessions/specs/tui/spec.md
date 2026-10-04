## MODIFIED Requirements

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
