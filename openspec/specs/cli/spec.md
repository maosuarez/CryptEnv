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

### Requirement: Project initialization via init

The CLI SHALL provide a `crypt-env init [NAME] [--path <PATH>] [--yes]` subcommand that provisions a new project in the vault and generates a local configuration file `.crypt-env.yaml`. If `NAME` is omitted, the command SHALL use the folder name of the current working directory. The associated injection path SHALL default to `./` (the project root folder) or the path provided by `--path`. The generated `.crypt-env.yaml` SHALL contain the project name, description, associated injection paths, tags/categories, and environments.

When `init` creates a new project in the vault it SHALL ask for the baseline environments: `default` (marked as default, the unnamed root environment that injects `.env`), `staging` and `production`. `init` MUST NOT add environments to a project that already exists in the vault, and the manifest-driven creation performed by `config` is not affected.

When a vault project with the same name already exists and is bound to a different root, `init` MUST NOT change that project's root or paths. It SHALL fail with the same root-binding error as `config` and suggest `crypt-env config --relink`. When the existing project has no root, adopting the current directory SHALL be treated as a secret-routing change that requires consent (see *Bidirectional project configuration sync via config*): `init` SHALL print the root diff, require a live session, and then ask an interactive `y`/`N` prompt that defaults to *no*, or accept `--yes` in its place. `--yes` answers only that adoption confirmation; it MUST NOT skip vault authentication.

#### Scenario: Init with default parameters in current directory
- **WHEN** user executes `crypt-env init` in directory `/workspace/my-service` without arguments
- **THEN** a project named `my-service` is registered in the vault if it does not exist
- **AND** `.crypt-env.yaml` is written with project name `my-service`, the default environment `default` targeting `./` (which injects `.env`), and the environments `staging` and `production`
- **AND** the vault project's root path is set to `/workspace/my-service` (as seen by the vault host)

#### Scenario: Init with explicit name and relative path
- **WHEN** user executes `crypt-env init backend-api --path ./app`
- **THEN** project `backend-api` is registered in the vault
- **AND** `.crypt-env.yaml` is written with the default environment targeting `app/.env` (relative to the project root)

#### Scenario: Init does not add environments to an existing project
- **WHEN** vault project `backend` already exists, bound to the current directory, and the user runs `crypt-env init`
- **THEN** the project is linked and its environments are exactly what they were before

#### Scenario: Init when config file already exists
- **WHEN** user executes `crypt-env init` in a directory containing `.crypt-env.yaml`
- **THEN** the command warns the user that the project configuration already exists without overwriting it

#### Scenario: Init in an unrelated folder with an existing project name
- **WHEN** vault project `backend` is bound to `/home/u/backend` and the user runs `crypt-env init` in `/tmp/other/backend`
- **THEN** the command fails with the root-binding error and the vault project is unchanged

#### Scenario: Non-interactive init adopting a rootless project with --yes
- **WHEN** vault project `backend` has no root and the user runs `crypt-env init backend --yes` with stdin not a terminal
- **THEN** the command still requires a live session, skips the `y`/`N` prompt, binds the project to the current directory and writes `.crypt-env.yaml`

#### Scenario: Non-interactive init adopting a rootless project without --yes
- **WHEN** vault project `backend` has no root and the user runs `crypt-env init backend` with stdin not a terminal
- **THEN** the command prints the diff and fails with an error that mentions `--yes`, and the vault project is unchanged

### Requirement: Bidirectional project configuration sync via config

The CLI SHALL provide a `crypt-env config [--relink] [--yes]` subcommand that synchronizes metadata (name, description, tags, environments, injection paths) between the local `.crypt-env.yaml` file and the vault database. Precedence SHALL be resolved using last-modified timestamps: if `.crypt-env.yaml` file mtime is newer than the vault project's `updated` timestamp, local configuration SHALL be pushed to the vault; if the vault's `updated` timestamp is newer, local `.crypt-env.yaml` SHALL be updated with vault state.

**Root binding.** A vault project whose root path is set SHALL be bound to that directory. When the directory containing `.crypt-env.yaml` (as seen by the vault host) differs from the bound root, `config` MUST NOT push or pull. Instead it SHALL fail with an error that names the bound root and suggests `crypt-env config --relink`. Root comparison SHALL ignore trailing separators, and SHALL ignore case on Windows hosts. A project with no root SHALL adopt the current directory only through the consent flow below.

**Consent for secret-routing changes.** A push that would set or change the project's root path, add, remove, or change any environment's target paths, or create an environment with target paths in an existing project, is a *secret-routing change*. Before applying one, `config` MUST:
1. print a diff listing the old and new root and every added and removed path per environment, and mark any absolute path that lies outside the project root;
2. require a live per-terminal CLI session, prompting for the master password when there is none;
3. require explicit confirmation: an interactive `y`/`N` prompt that defaults to *no*, or the `--yes` flag when stdin is not a terminal.

If any of these steps fails or the user declines, the vault SHALL remain unchanged. A push that only changes name, description or categories, or that creates a brand-new project, SHALL NOT require consent. `--relink` SHALL be the only way to move a bound project to a different directory, and it SHALL always be treated as a secret-routing change.

#### Scenario: Vault updated more recently than local file
- **WHEN** user runs `crypt-env config` from the project's bound root and the vault project `updated` timestamp is newer than `.crypt-env.yaml` mtime
- **THEN** `.crypt-env.yaml` is updated with latest project description, tags, and environments from the vault
- **AND** the command reports that local configuration was updated from the vault

#### Scenario: Local file updated more recently than vault
- **WHEN** user edits only the description in `.crypt-env.yaml` at the bound root and runs `crypt-env config`
- **THEN** the description is persisted in the vault without a confirmation prompt
- **AND** the command reports that vault settings were updated from the local configuration

#### Scenario: Local path edit requires consent
- **WHEN** user adds `apps/web/.env` to an environment in `.crypt-env.yaml` at the bound root and runs `crypt-env config`
- **THEN** the CLI prints the path diff, requires a live session, and asks for confirmation
- **AND** the vault is updated only after the user answers `y`

#### Scenario: Cloned repository claims an existing project
- **WHEN** a freshly cloned repository contains `.crypt-env.yaml` naming an existing vault project bound to `/home/u/real-project`, and the user runs `crypt-env config` in the clone
- **THEN** the command fails with an error naming `/home/u/real-project` and suggesting `--relink`
- **AND** the vault project's root and environment paths are unchanged

#### Scenario: Relink declined
- **WHEN** user runs `crypt-env config --relink` in a different checkout and answers `n` at the confirmation
- **THEN** no vault state changes and the command exits with a non-zero status

#### Scenario: Non-interactive path change without --yes
- **WHEN** `crypt-env config` runs with stdin not a terminal and the push is a secret-routing change without `--yes`
- **THEN** the command fails with an error that explains the change needs confirmation, and the vault is unchanged

#### Scenario: Config run with no local config file
- **WHEN** user runs `crypt-env config` in a directory without `.crypt-env.yaml`
- **THEN** the command terminates with an error stating no project configuration file was found and suggests `crypt-env init`

### Requirement: Variable addition with collision prompt and password gating via add

The CLI SHALL provide a `crypt-env add <INPUT>` subcommand that accepts `KEY=value`, `VARNAME` (or `$VARNAME`), or a dotenv file path, adding secrets to the vault under the default environment or the environment specified via `--env <NAME>`. When `INPUT` is a valid environment variable name present in the active shell environment (or escaped as `$VARNAME` / `\$VARNAME`), the key SHALL be the variable name without the leading `$` and the value SHALL be retrieved from the active environment. If `INPUT` cannot be parsed as `KEY=value`, `$VARNAME`, an active environment variable name, or an existing `.env` file, the CLI MUST return an error explaining the expected formats and clarifying that bare `$VAR` invocations may have been expanded by the shell prior to CLI execution. When `INPUT` equals the value of one or more environment variables the error SHALL name those variables (names only) and advise passing the name without `$`. The error MUST NOT contain `INPUT` itself nor any environment variable value, since `INPUT` may be a secret the shell expanded. If `--global` is specified, the item SHALL be created or marked as global. When adding a key that already exists in the target environment or global scope, the CLI MUST warn the user that a collision exists and halt saving. If the user confirms with 'y', the CLI SHALL require a live per-terminal CLI session (prompting for the master password when there is none) and then reveal the colliding value that prevents the addition.

#### Scenario: Adding a new key successfully
- **WHEN** user executes `crypt-env add DB_PORT=5432` and `DB_PORT` does not exist in the active environment
- **THEN** the item is encrypted and saved under the active environment

#### Scenario: Adding variable name without dollar sign
- **WHEN** user executes `crypt-env add PRUEBA_CRYPT` and `PRUEBA_CRYPT` is set in the shell environment
- **THEN** the key `PRUEBA_CRYPT` and its active value are added to the vault target environment

#### Scenario: Adding variable name with escaped dollar sign
- **WHEN** user executes `crypt-env add '$PRUEBA_CRYPT'` or `crypt-env add \$PRUEBA_CRYPT`
- **THEN** the leading `$` is stripped, `PRUEBA_CRYPT` is resolved from the environment, and added to the vault

#### Scenario: Adding from environment variable or dotenv file
- **WHEN** user executes `crypt-env add $AWS_SECRET_ACCESS_KEY` or `crypt-env add .env`
- **THEN** keys and values are parsed from the environment or `.env` and processed with collision checks

#### Scenario: Adding a token the shell already expanded
- **WHEN** user executes unquoted `crypt-env add $PRUEBA_CRYPT` and the shell passes the value of `PRUEBA_CRYPT` as the argument
- **THEN** the CLI terminates with an error that names `PRUEBA_CRYPT` and advises `crypt-env add PRUEBA_CRYPT` without `$`
- **AND** neither the argument nor any variable value appears in the error

#### Scenario: Adding an unrecognized token with shell expansion hint
- **WHEN** user executes `crypt-env add some_raw_value` and `some_raw_value` is not `KEY=value`, not a file, not an environment variable name and not the value of any environment variable
- **THEN** the CLI terminates with an error listing the accepted formats and explaining that if an environment variable was intended, its name should be passed directly without `$` or quoted to avoid shell pre-expansion
- **AND** the argument is not echoed

#### Scenario: Adding a key that collides with an existing secret
- **WHEN** user executes `crypt-env add API_KEY=secret123` and `API_KEY` already exists
- **THEN** the CLI outputs a message stating the key already exists and the action cannot proceed
- **AND** prompts the user whether they want to inspect the colliding value
- **AND** if the user answers 'y', prompts for the master password, verifies credentials, and displays the colliding value

### Requirement: Environment and example file generation via fill

The CLI SHALL provide a `crypt-env fill [--env <NAME>]` subcommand that decrypts vault secrets for the target environment(s) and writes the corresponding `.env` file(s) to the configured project path(s). In addition, `fill` SHALL simultaneously generate sanitized `.env.example` file(s) containing the same variable keys with empty or placeholder values. Each `.env.example` SHALL be written in the same directory as a `.env` file that was actually written for that environment. The `fill` operation MUST require a live per-terminal CLI session (see *Per-terminal password sessions*), prompting for and verifying the master password when there is none, before decrypting secrets or writing `.env` files.

`fill` MUST refuse to run when the current workspace root differs from the vault project's bound root (see *Bidirectional project configuration sync via config*), and SHALL write no files in that case.

#### Scenario: Fill all environments with password prompt
- **WHEN** user executes `crypt-env fill` at the bound root without a live session and enters the correct master password
- **THEN** `.env.<env_name>` (or `.env` for the default environment) and `.env.example` are written to the project path
- **AND** `.env.example` contains all keys with values blank or placeholder strings

#### Scenario: Fill specific environment
- **WHEN** user executes `crypt-env fill --env produccion` and enters the correct master password
- **THEN** only the `.env` (or configured destination path) and `.env.example` for environment `produccion` are generated

#### Scenario: Fill with incorrect password
- **WHEN** user executes `crypt-env fill` without a live session and enters an invalid master password
- **THEN** the operation aborts with an authentication error and writes no files

#### Scenario: Fill from a second checkout
- **WHEN** the project is bound to checkout A and the user runs `crypt-env fill` in checkout B
- **THEN** the command fails with an error naming checkout A and suggesting `crypt-env config --relink`
- **AND** no `.env` or `.env.example` is written in either checkout

### Requirement: Template variable synchronization via sync

The CLI SHALL provide a `crypt-env sync [--global]` subcommand that parses a local `.env.example` file and provisions vault items for any missing keys. Vault items created via `sync` SHALL have empty values or a `"change-me"` placeholder. When `--global` is specified, `sync` SHALL also generate or update `.env`, populating variables whose keys match existing global secrets in the vault while setting unmatched variables to empty or `"change-me"`. This command MUST require a live per-terminal CLI session, prompting for the master password when there is none.

#### Scenario: Sync without --global provisions template secrets
- **WHEN** user runs `crypt-env sync` with a `.env.example` containing `NEW_FEATURE_FLAG` and enters the master password
- **THEN** `NEW_FEATURE_FLAG` is created as an encrypted item in the vault associated with the active environment with value `change-me`

#### Scenario: Sync with --global auto-populates global secrets into .env
- **WHEN** user runs `crypt-env sync --global` where `SHARED_AUTH_URL` exists as a global vault secret
- **THEN** `.env` is written containing `SHARED_AUTH_URL` with its decrypted global value, and non-global keys with `change-me` or empty values

#### Scenario: Sync without .env.example present
- **WHEN** user runs `crypt-env sync` in a project directory lacking `.env.example`
- **THEN** the command halts with an error explaining that `.env.example` was not found

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

### Requirement: Variable search and listing via search

The CLI SHALL provide a `crypt-env search [PATTERN] [--global]` subcommand to search and list variables belonging to the project or global vault scope. The command SHALL accept regex or substring patterns (such as `%pattern` or standard regex). The search command MUST require a live per-terminal CLI session, prompting for the master password when there is none, before listing results.

#### Scenario: Search project variables with pattern
- **WHEN** user runs `crypt-env search "%TOKEN"` and authenticates with master password
- **THEN** the CLI lists all project environment variables matching `*TOKEN*` with their key names, environment name, and metadata (without plaintext secret values)

#### Scenario: Search global variables
- **WHEN** user runs `crypt-env search --global` and authenticates with master password
- **THEN** the CLI lists all global vault variables available across projects

### Requirement: Comprehensive diagnostics via doctor

The CLI SHALL provide a `crypt-env doctor` subcommand that performs an end-to-end diagnostic check of the CryptEnv installation and runtime environment. The diagnosis SHALL evaluate:
1. Application process and health endpoint status (`https://127.0.0.1:47821/health`).
2. Vault lock state (locked vs unlocked).
3. TLS certificate validity, path resolution, and expiration.
4. CLI session token existence and file permissions.
5. MCP token configuration and file path.
6. Project configuration validity (`.crypt-env.yaml`).
7. WSL integration status (whether running under WSL or Windows, and distribution readiness).

#### Scenario: Doctor run when vault app is running and config is valid
- **WHEN** user runs `crypt-env doctor` with the GUI open and unlocked in a directory with `.crypt-env.yaml`
- **THEN** doctor reports `[OK]` for App running, Vault unlocked, TLS certificate, Token cache, Project YAML config, and WSL status

#### Scenario: Doctor run when app is closed
- **WHEN** user runs `crypt-env doctor` while CryptEnv GUI is not running
- **THEN** doctor reports `[!!] App running: not running — open crypt-env and try again` and summarizes local file states

### Requirement: Enhanced WSL setup detection and guidance

The CLI SHALL provide `crypt-env setup wsl [DISTRO]` with intelligent environment detection:
1. If invoked inside a WSL distribution, it SHALL detect the current distribution and configure shell startup automatically.
2. If invoked on the Windows host (PowerShell/CMD):
   - If no WSL distributions are found, it SHALL report that no WSL distributions were detected.
   - If distributions are found and no `DISTRO` argument is given, it SHALL list the detected distributions and prompt the user to run `crypt-env setup wsl <distro>`.
   - If a valid `DISTRO` argument is given, it SHALL apply the shell configuration into that distribution.

#### Scenario: Setup WSL run inside WSL
- **WHEN** user runs `crypt-env setup wsl` inside an interactive WSL terminal
- **THEN** the command detects the WSL environment and creates `~/.config/cryptenv/env.sh` and rc marker blocks

#### Scenario: Setup WSL run on Windows without distro argument
- **WHEN** user runs `crypt-env setup wsl` in Windows PowerShell with `Ubuntu` and `Debian` installed
- **THEN** the command lists `Ubuntu` and `Debian` and instructs the user to run `crypt-env setup wsl <distro>`

#### Scenario: Setup WSL run on Windows with explicit distro
- **WHEN** user runs `crypt-env setup wsl Ubuntu` in Windows PowerShell
- **THEN** the command configures `crypt-env` inside the `Ubuntu` distribution

### Requirement: Legacy commands removed

The CLI SHALL NOT provide the subcommands `memory`, `list`, `exec`, `cmd`, `project`, `share`, `relay`, `category` or `set`; invoking any of them SHALL fail with the argument parser's unrecognized-subcommand error. These capabilities remain available in the desktop GUI.

#### Scenario: Running a removed command
- **WHEN** user executes `crypt-env project list` or `crypt-env memory`
- **THEN** the command fails with an unrecognized-subcommand error and a non-zero exit code

### Requirement: Per-terminal password sessions

Password-gated CLI actions (`fill`, `sync`, `inject`, `search`, the `add` collision reveal, secret-routing `config` pushes, and TUI reveal/fill/sync) SHALL prompt for the master password only when the current terminal holds no live CLI session. A successful password entry SHALL create a session bound to that terminal whose lifetime is the GUI's auto-lock timeout (`auto_lock_timeout`, default 5 minutes). Every authenticated use of the session within its lifetime SHALL renew it for a full timeout. Another terminal SHALL NOT reuse the session and SHALL prompt for its own. Sessions from different terminals SHALL coexist without invalidating each other.

Locking the vault, whether manually, by auto-lock, or by reset or backup restore, MUST invalidate every CLI session at once. A request that arrives while the vault is locked MUST NOT renew any session. After a later unlock, a pre-lock session token MUST be rejected as unauthenticated (401), so the terminal prompts for the password again.

Stale per-terminal session files SHALL be pruned only if their name is exactly the configured token path followed by `.` and 16 lowercase hexadecimal characters, and they are regular files. No other file SHALL ever be deleted by pruning.

#### Scenario: Consecutive gated commands in one terminal
- **WHEN** user runs `crypt-env fill`, enters the password, and runs `crypt-env inject API_KEY` two minutes later in the same terminal
- **THEN** `inject` does not prompt for the password and the session is renewed for another timeout

#### Scenario: Session lapses
- **WHEN** more than the timeout elapses since the last command in that terminal
- **THEN** the next gated command prompts for the master password again

#### Scenario: Different terminal
- **WHEN** user authenticated in terminal A and runs `crypt-env search` in terminal B
- **THEN** terminal B prompts for the master password, and terminal A's session stays valid

#### Scenario: Vault locked while a terminal keeps retrying
- **WHEN** the GUI auto-locks and the user repeatedly runs `crypt-env fill` in a terminal holding a session
- **THEN** each attempt reports that the vault is locked and no attempt extends the session
- **AND** after the user unlocks the GUI, the next gated command in that terminal prompts for the master password

#### Scenario: Pruning leaves unrelated files alone
- **WHEN** `CRYPTENV_TOKEN_PATH` is `~/secrets/cli` and the directory also holds `cli.bak` and `cli.json`, both older than one day
- **THEN** pruning deletes only stale `cli.<16-hex>` files and leaves `cli.bak` and `cli.json` untouched

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

### Requirement: Folder-based environment injection targets

An environment's configured relative `paths` SHALL be either an env file or a folder under the project root. A relative path whose last component starts or ends with `.env` (for example `.env`, `apps/api/.env.local`, `prod.env`) SHALL be that file. Any other relative path SHALL be a folder that receives the environment's file: `.env` for the root/`default` environment and `.env.<name>` otherwise (`./` → `.env.production`, `apps/web` → `apps/web/.env.production`). A regular file already existing at a non-`.env*` relative path SHALL keep its meaning of "that file". Absolute paths SHALL always be files, as before. An environment MAY list several folders. Folder targets MUST go through the same project-root containment, symlink refusal, overwrite gating and atomic write as file targets; a target whose final component is a symlink MUST be refused even when the link points inside the project root.

When an environment of a project that has a root has no configured paths, and the caller supplies no `output_path`/`output_dir` and no target subset, injection by a **session** caller (CLI or GUI) SHALL default to the project root folder (`./`). That default target is not owner-consented: an existing file not created by crypt-env MUST be refused rather than written through, like an `output_path`. The MCP principal MUST NOT receive this default and still requires a configured path.

#### Scenario: Folder targets receive the environment file
- **WHEN** environment `production` has paths `./` and `apps/web`, and `apps/web` exists
- **THEN** injecting writes `<root>/.env.production` and `<root>/apps/web/.env.production`

#### Scenario: Explicit env file paths are unchanged
- **WHEN** an environment has the path `apps/api/.env.custom` or `prod.env`
- **THEN** injecting writes exactly that file

#### Scenario: Folder target through a symlinked directory
- **WHEN** a relative folder path leaves the project root through a symlinked directory
- **THEN** the injection is refused and nothing is written outside the root

#### Scenario: Final component is a symlink inside the root
- **WHEN** the resolved target file is a symlink to another file inside the project root
- **THEN** the injection is refused and the link target is not modified

#### Scenario: Session caller defaults to the project root
- **WHEN** a session caller injects an environment with no paths in a project with a root, and `<root>/.env.production` does not exist
- **THEN** the file is created
- **AND** if it exists and was not created by crypt-env the injection is refused with `TARGET_EXISTS`

#### Scenario: MCP does not get the root default
- **WHEN** the MCP token injects an environment with no configured paths and supplies no output path
- **THEN** the request fails with the "no paths configured" validation error and nothing is written
