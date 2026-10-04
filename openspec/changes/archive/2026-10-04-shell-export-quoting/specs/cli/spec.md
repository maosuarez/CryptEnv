## MODIFIED Requirements

### Requirement: Secret environment variable injection via inject

The CLI SHALL provide a `crypt-env inject <KEY>` subcommand that injects a secret's value directly into the calling shell environment without displaying the secret value on the screen or in terminal logs. The `inject` command MUST require a live per-terminal CLI session, prompting for the master password when there is none. The emitted assignment MUST follow the *shell-export-quoting* guarantees. Documentation and help text SHALL show the quoted invocation `eval "$(crypt-env inject KEY)"` for POSIX shells and `crypt-env inject KEY | Invoke-Expression` for PowerShell.

#### Scenario: Inject secret variable into terminal session
- **WHEN** user runs `eval "$(crypt-env inject DATABASE_URL)"` and provides the master password
- **THEN** `crypt-env inject` outputs the shell export expression without logging the plaintext secret to stderr or interactive console display
- **AND** the variable `DATABASE_URL` becomes available in the invoking shell with exactly the stored value

#### Scenario: Inject with invalid password
- **WHEN** user runs `crypt-env inject DATABASE_URL` without a live session and enters an incorrect master password
- **THEN** the command exits with an authentication error and outputs no shell assignments

#### Scenario: Inject with an invalid key name
- **WHEN** the stored key is not a valid shell variable name
- **THEN** the command exits with an invalid-key error and outputs no shell assignments
