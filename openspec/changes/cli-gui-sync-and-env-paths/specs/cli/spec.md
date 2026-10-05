## MODIFIED Requirements

### Requirement: Project initialization via init

The CLI SHALL provide a `crypt-env init [NAME] [--path <PATH>] [--yes]` subcommand that provisions a new project in the vault and generates a local configuration file `.crypt-env.yaml`. If `NAME` is omitted, the command SHALL use the folder name of the current working directory. The associated injection path SHALL default to `./` (relative folder) or the path provided by `--path`. The generated `.crypt-env.yaml` SHALL contain the project name, description, associated injection paths, tags/categories, and environments.

When a vault project with the same name already exists and is bound to a different root, `init` MUST NOT change that project's root or paths. It SHALL fail with the same root-binding error as `config` and suggest `crypt-env config --relink`. When the existing project has no root, adopting the current directory SHALL be treated as a secret-routing change that requires consent (see *Bidirectional project configuration sync via config*): `init` SHALL print the root diff, require a live session, and then ask an interactive `y`/`N` prompt that defaults to *no*, or accept `--yes` in its place. `--yes` answers only that adoption confirmation; it MUST NOT skip vault authentication.

When `init` is executed inside a Git repository (or worktree), the CLI SHALL ensure that `.crypt-env.yaml` is listed in the root `.gitignore`, creating the file if missing or appending the entry if not already present. When creating a new project in the vault without explicit pre-existing environments, the CLI SHALL initialize standard baseline environments: `development` (marked as default), `staging`, and `production`.

#### Scenario: Init with default parameters in current directory
- **WHEN** user executes `crypt-env init` in directory `/workspace/my-service` without arguments
- **THEN** a project named `my-service` is registered in the vault if it does not exist
- **AND** `.crypt-env.yaml` is written with project name `my-service`, injection path `./`, and baseline environments `development` (default), `staging` and `production`
- **AND** the vault project's root path is set to `/workspace/my-service` (as seen by the vault host)

#### Scenario: Init inside a git repository updates .gitignore
- **WHEN** user executes `crypt-env init` inside a Git repository (or worktree) whose root `.gitignore` does not list `.crypt-env.yaml`
- **THEN** `.crypt-env.yaml` is appended to the root `.gitignore`, creating the file if it is missing
- **AND** running `init` again does not add a duplicate entry

#### Scenario: Init with explicit name and relative path
- **WHEN** user executes `crypt-env init backend-api --path ./app`
- **THEN** project `backend-api` is registered in the vault
- **AND** `.crypt-env.yaml` is written with the default environment targeting `app/.env` (relative to the project root)

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

### Requirement: Variable addition with collision prompt and password gating via add

The CLI SHALL provide a `crypt-env add <INPUT>` subcommand that accepts `KEY=value`, `VARNAME` (or `$VARNAME`), or a dotenv file path, adding secrets to the vault under the default environment or the environment specified via `--env <NAME>`. When `INPUT` is a valid environment variable name present in the active shell environment (or escaped as `$VARNAME` / `\$VARNAME`), the key SHALL be the variable name without the leading `$` and the value SHALL be retrieved from the active environment. If `INPUT` cannot be parsed as `KEY=value`, `$VARNAME`, an active environment variable name, or an existing `.env` file, the CLI MUST return an error explaining the expected formats and clarifying that bare `$VAR` invocations may have been expanded by the shell prior to CLI execution. If `--global` is specified, the item SHALL be created or marked as global. When adding a key that already exists in the target environment or global scope, the CLI MUST warn the user that a collision exists and halt saving. If the user confirms with 'y', the CLI SHALL require a live per-terminal CLI session (prompting for the master password when there is none) and then reveal the colliding value that prevents the addition.

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

#### Scenario: Adding an unrecognized token with shell expansion hint
- **WHEN** user executes `crypt-env add some_raw_value` and `some_raw_value` is not `KEY=value`, not a file, and not found in environment variables
- **THEN** the CLI terminates with an error explaining that if an environment variable was intended, its name should be passed directly without `$` or quoted to avoid shell pre-expansion

#### Scenario: Adding a key that collides with an existing secret
- **WHEN** user executes `crypt-env add API_KEY=secret123` and `API_KEY` already exists
- **THEN** the CLI outputs a message stating the key already exists and the action cannot proceed
- **AND** prompts the user whether they want to inspect the colliding value
- **AND** if the user answers 'y', prompts for the master password, verifies credentials, and displays the colliding value
