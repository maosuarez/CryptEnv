# CLI Specification Delta

## ADDED Requirements

### Requirement: Project initialization via init

The CLI SHALL provide a `crypt-env init [NAME] [--path <PATH>]` subcommand that provisions a new project in the vault and generates a local configuration file `.crypt-env.yaml`. If `NAME` is omitted, the command SHALL use the folder name of the current working directory. The associated injection path SHALL default to the directory where `init` is run or the path provided by `--path`. The generated `.crypt-env.yaml` SHALL contain the project name, description, associated injection paths, tags/categories, and environments.

#### Scenario: Init with default parameters in current directory
- **WHEN** user executes `crypt-env init` in directory `/workspace/my-service` without arguments
- **THEN** a project named `my-service` is registered in the vault if it does not exist
- **AND** `.crypt-env.yaml` is written with project name `my-service` and default environment `default` targeting `.env`
- **AND** the vault project's root path is set to `/workspace/my-service` (as seen by the vault host)

#### Scenario: Init with explicit name and relative path
- **WHEN** user executes `crypt-env init backend-api --path ./app`
- **THEN** project `backend-api` is registered in the vault
- **AND** `.crypt-env.yaml` is written with the default environment targeting `app/.env` (relative to the project root)

#### Scenario: Init when config file already exists
- **WHEN** user executes `crypt-env init` in a directory containing `.crypt-env.yaml`
- **THEN** the command warns the user that the project configuration already exists without overwriting it

### Requirement: Bidirectional project configuration sync via config

The CLI SHALL provide a `crypt-env config` subcommand that synchronizes metadata (name, description, tags, environments, injection paths) between the local `.crypt-env.yaml` file and the vault database. Precedence SHALL be resolved using last-modified timestamps: if `.crypt-env.yaml` file mtime is newer than the vault project's `updated` timestamp, local configuration SHALL be pushed to the vault; if the vault's `updated` timestamp is newer, local `.crypt-env.yaml` SHALL be updated with vault state.

#### Scenario: Vault updated more recently than local file
- **WHEN** user runs `crypt-env config` and the vault project `updated` timestamp is newer than `.crypt-env.yaml` mtime
- **THEN** `.crypt-env.yaml` is updated with latest project description, tags, and environments from the vault
- **AND** the command reports that local configuration was updated from the vault

#### Scenario: Local file updated more recently than vault
- **WHEN** user edits `.crypt-env.yaml` (e.g. modifies description or environments) and runs `crypt-env config`
- **THEN** the updated settings are sent to the REST API and persisted in the vault database
- **AND** the command reports that vault settings were updated from the local configuration

#### Scenario: Config run with no local config file
- **WHEN** user runs `crypt-env config` in a directory without `.crypt-env.yaml`
- **THEN** the command terminates with an error stating no project configuration file was found and suggests `crypt-env init`

### Requirement: Variable addition with collision prompt and password gating via add

The CLI SHALL provide a `crypt-env add KEY=value | $VAR | .env` subcommand that adds secrets to the vault under the default environment or the environment specified via `--env <NAME>`. If `--global` is specified, the item SHALL be created or marked as global. When adding a key that already exists in the target environment or global scope, the CLI MUST warn the user that a collision exists and halt saving. If the user confirms with 'y', the CLI SHALL require a live per-terminal CLI session (prompting for the master password when there is none) and then reveal the colliding value that prevents the addition.

#### Scenario: Adding a new key successfully
- **WHEN** user executes `crypt-env add DB_PORT=5432` and `DB_PORT` does not exist in the active environment
- **THEN** the item is encrypted and saved under the active environment

#### Scenario: Adding a key that collides with an existing secret
- **WHEN** user executes `crypt-env add API_KEY=secret123` and `API_KEY` already exists
- **THEN** the CLI outputs a message stating the key already exists and the action cannot proceed
- **AND** prompts the user whether they want to inspect the colliding value
- **AND** if the user answers 'y', prompts for the master password, verifies credentials, and displays the colliding value

#### Scenario: Adding from environment variable or dotenv file
- **WHEN** user executes `crypt-env add $AWS_SECRET_ACCESS_KEY` or `crypt-env add .env`
- **THEN** keys and values are parsed from the environment or `.env` and processed with collision checks

### Requirement: Environment and example file generation via fill

The CLI SHALL provide a `crypt-env fill [--env <NAME>]` subcommand that decrypts vault secrets for the target environment(s) and writes the corresponding `.env` file(s) to the configured project path(s). In addition, `fill` SHALL simultaneously generate sanitized `.env.example` file(s) containing the same variable keys with empty or placeholder values. The `fill` operation MUST require a live per-terminal CLI session (see *Per-terminal password sessions*), prompting for and verifying the master password when there is none, before decrypting secrets or writing `.env` files.

#### Scenario: Fill all environments with password prompt
- **WHEN** user executes `crypt-env fill` without a live session and enters the correct master password
- **THEN** `.env.<env_name>` (or `.env` for the default environment) and `.env.example` are written to the project path
- **AND** `.env.example` contains all keys with values blank or placeholder strings

#### Scenario: Fill specific environment
- **WHEN** user executes `crypt-env fill --env produccion` and enters the correct master password
- **THEN** only the `.env` (or configured destination path) and `.env.example` for environment `produccion` are generated

#### Scenario: Fill with incorrect password
- **WHEN** user executes `crypt-env fill` without a live session and enters an invalid master password
- **THEN** the operation aborts with an authentication error and writes no files

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

The CLI SHALL provide a `crypt-env inject <KEY>` subcommand that injects a secret's value directly into the calling shell environment without displaying the secret value on the screen or in terminal logs. The `inject` command MUST require a live per-terminal CLI session, prompting for the master password when there is none.

#### Scenario: Inject secret variable into terminal session
- **WHEN** user runs `eval $(crypt-env inject DATABASE_URL)` and provides the master password
- **THEN** `crypt-env inject` outputs the shell export expression without logging the plaintext secret to stderr or interactive console display
- **AND** the variable `DATABASE_URL` becomes available in the invoking shell

#### Scenario: Inject with invalid password
- **WHEN** user runs `crypt-env inject DATABASE_URL` without a live session and enters an incorrect master password
- **THEN** the command exits with an authentication error and outputs no shell assignments

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

Password-gated CLI actions (`fill`, `sync`, `inject`, `search`, the `add` collision reveal, and TUI reveal/fill/sync) SHALL prompt for the master password only when the current terminal holds no live CLI session. A successful password entry SHALL create a session bound to that terminal whose lifetime is the GUI's auto-lock timeout (`auto_lock_timeout`, default 5 minutes). Every authenticated use of the session within its lifetime SHALL renew it for a full timeout. Another terminal SHALL NOT reuse the session and SHALL prompt for its own. Sessions from different terminals SHALL coexist without invalidating each other.

#### Scenario: Consecutive gated commands in one terminal
- **WHEN** user runs `crypt-env fill`, enters the password, and runs `crypt-env inject API_KEY` two minutes later in the same terminal
- **THEN** `inject` does not prompt for the password and the session is renewed for another timeout

#### Scenario: Session lapses
- **WHEN** more than the timeout elapses since the last command in that terminal
- **THEN** the next gated command prompts for the master password again

#### Scenario: Different terminal
- **WHEN** user authenticated in terminal A and runs `crypt-env search` in terminal B
- **THEN** terminal B prompts for the master password, and terminal A's session stays valid
