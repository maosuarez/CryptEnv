# mcp-command-execution Specification

## Purpose
Defines how crypt-env runs user-stored commands for MCP callers with injected secrets. The requirement is that secrets reach the child process, but never the MCP caller, the MCP process, or a shared temporary location.

## Requirements

### Requirement: Secrets are injected by the backend, never returned to MCP

Running a stored command on behalf of the MCP principal MUST happen in the crypt-env backend. The backend SHALL resolve the requested secret values and pass them only to the child process's environment. The MCP server process MUST NOT receive, hold, or set secret values in its own environment. Selecting keys to inject SHALL record key names only.

#### Scenario: Inject then run
- **WHEN** an agent calls `inject_env` with `DB_PASSWORD` and then `run_command` for a stored command
- **THEN** the child process sees `DB_PASSWORD` in its environment
- **AND** the MCP server's own environment does not contain `DB_PASSWORD`

### Requirement: Child environment isolation

The child process environment MUST be built from an empty environment plus a fixed baseline (for example `PATH`, `HOME`/`USERPROFILE`, `SystemRoot`, `TEMP`, `LANG`) plus the explicitly injected keys. Any other variable of the backend process MUST NOT be inherited.

#### Scenario: Unrelated backend variables absent
- **WHEN** the backend process has `AWS_SECRET_ACCESS_KEY` in its own environment and a command is run without injecting it
- **THEN** the child does not see `AWS_SECRET_ACCESS_KEY`

### Requirement: Strict parameter substitution

Each value substituted into a stored command's `{{name}}` placeholder MUST match `^[A-Za-z0-9._/:@=+,-]{0,256}$`. A non-matching value SHALL cause the call to fail with an error naming the parameter, and no process SHALL start.

#### Scenario: Shell metacharacters rejected
- **WHEN** `run_command` is called with `params: {p: "; printenv DB_PASSWORD"}`
- **THEN** the call fails with an invalid-parameter error and no process starts

### Requirement: Output redaction and bounds

Before returning output to the MCP caller, the backend MUST replace every occurrence of each injected secret value, and of its standard base64 and lowercase-hex encodings, with `[REDACTED:<KEY>]`. Captured stdout and stderr SHALL each be limited to 64 KiB; further output SHALL be discarded. The returned text SHALL be at most 2000 characters per stream, truncated on a character boundary. Truncation MUST NOT cause a failure for any byte sequence.

#### Scenario: Command prints an injected secret
- **WHEN** a stored command runs `printenv DB_PASSWORD` with `DB_PASSWORD` injected
- **THEN** the returned stdout contains `[REDACTED:DB_PASSWORD]` and not the value

#### Scenario: Multi-byte output at the truncation boundary
- **WHEN** a command prints 3000 characters of accented text
- **THEN** the call succeeds with at most 2000 characters and a truncation marker

### Requirement: Process lifetime bounds

A child SHALL have stdin connected to null. A child MUST be terminated, together with every descendant, when it exceeds 120 seconds of wall-clock time, or when the MCP connection that requested it ends. The call SHALL report a timeout distinctly from a non-zero exit.

#### Scenario: Hanging command
- **WHEN** a stored command runs `tail -f` and spawns children
- **THEN** after 120 seconds the whole process tree is killed, and the call returns a timeout result
- **AND** the MCP server keeps processing other requests while the command runs

### Requirement: Private temporary secret files

Any file with plaintext secrets that is created for an MCP request MUST be written exclusively at mode 0600 (or a user-only ACL on Windows) inside a per-user private directory (mode 0700), never in a shared temporary directory. It SHALL be deleted after 10 minutes, when the vault locks, or at application exit, whichever comes first. Leftovers SHALL be swept at startup. Generating such a file for the MCP principal SHALL require human approval.

#### Scenario: Lock deletes generated files
- **WHEN** an approved `generate_env` wrote a file and the vault then locks
- **THEN** the file no longer exists
