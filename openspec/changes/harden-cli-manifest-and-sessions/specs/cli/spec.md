## MODIFIED Requirements

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

### Requirement: Project initialization via init

The CLI SHALL provide a `crypt-env init [NAME] [--path <PATH>]` subcommand that provisions a new project in the vault and generates a local configuration file `.crypt-env.yaml`. If `NAME` is omitted, the command SHALL use the folder name of the current working directory. The associated injection path SHALL default to the directory where `init` is run or the path provided by `--path`. The generated `.crypt-env.yaml` SHALL contain the project name, description, associated injection paths, tags/categories, and environments.

When a vault project with the same name already exists and is bound to a different root, `init` MUST NOT change that project's root or paths. It SHALL fail with the same root-binding error as `config` and suggest `crypt-env config --relink`. When the existing project has no root, adopting the current directory SHALL be treated as a secret-routing change that requires consent (see *Bidirectional project configuration sync via config*).

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

#### Scenario: Init in an unrelated folder with an existing project name
- **WHEN** vault project `backend` is bound to `/home/u/backend` and the user runs `crypt-env init` in `/tmp/other/backend`
- **THEN** the command fails with the root-binding error and the vault project is unchanged

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
