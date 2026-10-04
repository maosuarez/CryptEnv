## ADDED Requirements

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
