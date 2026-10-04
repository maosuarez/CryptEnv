## MODIFIED Requirements

### Requirement: Session-token storage and permissions

The CLI client SHALL resolve the session-token file path from `CRYPTENV_TOKEN_PATH` when set and non-empty, and SHALL otherwise use the current default location. A token file MUST be created with owner-only permissions from the moment it exists: it must never be readable by other users, even briefly. It SHALL replace any previous token at the same path atomically. When the client creates the token directory, it SHALL create it with owner-only permissions. On non-Windows targets, a failure to apply owner-only permissions on a filesystem that cannot honor them SHALL NOT abort the operation, provided the token content was written successfully; the token write itself SHALL still surface an error if it fails.

#### Scenario: Default token location unchanged

- **WHEN** `CRYPTENV_TOKEN_PATH` is unset
- **THEN** the client reads and writes the session token at its current default path

#### Scenario: Token never world-readable on native filesystems

- **WHEN** the client writes a token on a native Linux filesystem with umask `022`
- **THEN** at no point does a file containing the token exist with group or other read permission

#### Scenario: Permission hardening best-effort off native filesystems

- **WHEN** the token file is written to a path that cannot honor Unix permission bits (for example a Windows-mounted `/mnt/c` path under WSL)
- **THEN** the client keeps the successfully written token and continues
- **AND** does not report the permission-tightening failure as a fatal error

#### Scenario: Token write failure still fails

- **WHEN** the token content cannot be written at all (directory missing, disk full, permission denied on create)
- **THEN** the client surfaces an error rather than proceeding as if a token were saved

## ADDED Requirements

### Requirement: Terminal identity is not reusable

The identity used to bind a CLI session to a terminal MUST include a value that differs between two terminals opened one after another, even if the operating system reuses the handle or session number. On Windows it SHALL include the console host process's creation time. A process without a console SHALL NOT share a session with any other process. In the WSL launcher, the identity SHALL include the session leader's start time.

#### Scenario: Recycled console handle
- **WHEN** terminal A authenticates and is closed, and a new terminal B receives the same console window handle
- **THEN** terminal B prompts for the master password

#### Scenario: Console-less processes
- **WHEN** two detached processes without a console run gated commands
- **THEN** each one needs its own authentication

### Requirement: Session token retained on transient errors

The CLI SHALL delete a cached session token only when the server rejects it as unauthenticated (401). A server error (5xx), a throttling response (429), or a network failure SHALL leave the token in place, and SHALL be reported as an error.

#### Scenario: Backend restarting
- **WHEN** the backend answers 503 during a gated command
- **THEN** the command fails with a server error, and the next command after the backend recovers does not prompt for the password

### Requirement: add reports failures in its exit status

`crypt-env add` SHALL exit with a non-zero status when any key fails to be added. It SHALL print, by key name only, which keys were added and which failed.

#### Scenario: Partial failure
- **WHEN** `crypt-env add .env` adds 3 keys and 1 key's request fails
- **THEN** the command lists the 3 added keys and the 1 failed key, and exits non-zero
