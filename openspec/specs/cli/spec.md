# CLI Specification

## Purpose

Defines how the `crypt-env` command-line client (and the code it shares with the TUI and MCP server) locates the vault's REST endpoint and TLS trust anchor, stores its session token, guards against non-loopback endpoints, and persists that configuration into a user's shell without destroying existing shell content.

## Requirements

### Requirement: Configurable REST endpoint

The CLI client SHALL resolve the vault's REST base URL from the `CRYPTENV_API_URL` environment variable when it is set and non-empty, and SHALL otherwise use the built-in default `https://127.0.0.1:47821`. The resolved value SHALL be used for every request the client makes in that invocation, and SHALL be resolved once per process so all requests target the same endpoint.

#### Scenario: Default when unset

- **WHEN** `CRYPTENV_API_URL` is unset or empty
- **THEN** the client sends requests to `https://127.0.0.1:47821`

#### Scenario: Override honored

- **WHEN** `CRYPTENV_API_URL` is set to `https://127.0.0.1:47821` via a port-forward, or to another URL the user controls
- **THEN** every request in that invocation targets the configured URL rather than the default

#### Scenario: Non-loopback endpoint warns

- **WHEN** `CRYPTENV_API_URL` is set and its host is not a loopback address (`127.0.0.0/8`, `::1`, or `localhost`)
- **THEN** the client prints a single warning line to stderr stating that the vault is expected to be reachable only over localhost
- **AND** the client still proceeds with the request

#### Scenario: Malformed endpoint fails clearly

- **WHEN** `CRYPTENV_API_URL` is set to a value that is not a valid absolute `http`/`https` URL
- **THEN** the client exits with a non-zero status and an error message naming `CRYPTENV_API_URL`
- **AND** no request is attempted

### Requirement: Configurable TLS trust anchor

The CLI client SHALL load the REST API's TLS certificate from the path in `CRYPTENV_CERT_PATH` when that variable is set and non-empty, checking it before the platform data-directory probing (`APPDATA`, then `XDG_DATA_HOME`, then `HOME/.local/share`). The client SHALL continue to pin the loaded certificate as the sole trust anchor and MUST NOT fall back to accepting invalid or unverified certificates.

#### Scenario: Explicit cert path used

- **WHEN** `CRYPTENV_CERT_PATH` points to a readable PEM file
- **THEN** the client trusts that certificate for the connection and does not consult the platform data directories

#### Scenario: Explicit cert path missing

- **WHEN** `CRYPTENV_CERT_PATH` is set but the file cannot be read
- **THEN** the client reports a clear error identifying the configured path
- **AND** does not silently fall back to the platform data-directory probing

#### Scenario: Unset preserves current behavior

- **WHEN** `CRYPTENV_CERT_PATH` is unset
- **THEN** the client resolves the certificate exactly as it does today (`APPDATA` → `XDG_DATA_HOME` → `HOME/.local/share`, each joined with the app identifier and `tls/cert.pem`)

#### Scenario: No trust bypass

- **WHEN** the configured or probed certificate does not match the server's presented certificate
- **THEN** the connection fails
- **AND** the client never disables certificate verification to complete the request

### Requirement: Session-token storage and permissions

The CLI client SHALL resolve the session-token file path from `CRYPTENV_TOKEN_PATH` when set and non-empty, and SHALL otherwise use the current default location. On non-Windows targets, a failure to restrict the token file's permissions to owner-only SHALL NOT abort the operation, provided the token content was written successfully; the token write itself SHALL still surface an error if it fails.

#### Scenario: Default token location unchanged

- **WHEN** `CRYPTENV_TOKEN_PATH` is unset
- **THEN** the client reads and writes the session token at its current default path

#### Scenario: Permission hardening best-effort off native filesystems

- **WHEN** the token file is written to a path that cannot honor Unix permission bits (for example a Windows-mounted `/mnt/c` path under WSL)
- **THEN** the client keeps the successfully written token and continues
- **AND** does not report the permission-tightening failure as a fatal error

#### Scenario: Token write failure still fails

- **WHEN** the token content cannot be written at all (directory missing, disk full, permission denied on create)
- **THEN** the client surfaces an error rather than proceeding as if a token were saved

### Requirement: Non-destructive shell configuration via `setup wsl`

The CLI SHALL provide a `crypt-env setup wsl` subcommand that persists the endpoint and certificate configuration into the invoking user's shell startup without modifying unrelated shell content. It SHALL write a dedicated, fully-managed file `~/.config/cryptenv/env.sh` containing the `export` statements, and SHALL ensure a single marker-delimited block that sources that file exists in `~/.bashrc` (and in `~/.zshrc` when that file exists). The markers SHALL be `# >>> cryptenv initialize >>>` and `# <<< cryptenv initialize <<<`. Before the first modification of a given rc file the command SHALL create a one-time backup alongside it (for example `~/.bashrc.cryptenv.bak`). The command MUST NOT edit, reorder, or pattern-replace any line outside its own marker block.

#### Scenario: Fresh setup

- **WHEN** `crypt-env setup wsl` runs and no cryptenv marker block is present in `~/.bashrc`
- **THEN** `~/.config/cryptenv/env.sh` is created with the resolved `export CRYPTENV_API_URL=...` and `export CRYPTENV_CERT_PATH=...` lines
- **AND** a marker-delimited block that sources `~/.config/cryptenv/env.sh` is appended to `~/.bashrc`
- **AND** a `~/.bashrc.cryptenv.bak` backup is created
- **AND** every pre-existing line of `~/.bashrc` is preserved unchanged and in order

#### Scenario: Idempotent re-run

- **WHEN** `crypt-env setup wsl` runs and its marker block already exists in `~/.bashrc`
- **THEN** `~/.config/cryptenv/env.sh` is rewritten with the current resolved values
- **AND** `~/.bashrc` is left byte-for-byte unchanged
- **AND** no additional backup is created

#### Scenario: Zsh handled when present

- **WHEN** `~/.zshrc` exists at setup time
- **THEN** the same single marker block is ensured in `~/.zshrc` under the same non-destructive rules

#### Scenario: Removal is surgical

- **WHEN** `crypt-env setup wsl --remove` runs
- **THEN** the marker block (inclusive of both marker lines) is removed from each rc file that contains it
- **AND** `~/.config/cryptenv/env.sh` is deleted
- **AND** all other content of each rc file is preserved unchanged
- **AND** the command succeeds without error when there is nothing to remove

#### Scenario: Values reflect the environment at setup time

- **WHEN** `CRYPTENV_API_URL` and/or `CRYPTENV_CERT_PATH` are set in the environment where `crypt-env setup wsl` runs
- **THEN** `env.sh` records those values
- **AND** when they are unset, `setup wsl` records the documented WSL defaults (loopback URL and the `/mnt/c` Windows cert path) or reports that a required value must be supplied
