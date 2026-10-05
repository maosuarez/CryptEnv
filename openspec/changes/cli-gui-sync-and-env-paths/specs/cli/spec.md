## MODIFIED Requirements

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

## ADDED Requirements

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
