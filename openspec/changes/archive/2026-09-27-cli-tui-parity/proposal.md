# Proposal: CLI, REST, and TUI Parity & Project Workflow Alignment

## Why

Developers working from the terminal often need to initialize, configure, and synchronize local project secrets without context-switching to the desktop GUI. The current CLI has disjointed subcommands (`memory`, `cmd`, `project`, `list`) with inconsistent project scope models (previously relying on a minimal `crypt-env.json`), lacks a unified project descriptor (`.crypt-env.yaml`) that mirrors GUI projects, does not support bidirectional configuration synchronization with last-modified conflict resolution, and lacks consistent password-gated actions and interactive collision resolution on additions.

By modernizing the CLI and interactive TUI (`crypt-env tui`), we provide developer workflows that match GUI capabilities directly from the command line while cleanly deprecating out-of-scope legacy commands.

## What Changes

- **Project Initialization (`crypt-env init [NAME] [--path <PATH>]`)**:
  - Automatically creates a vault project (using the current directory name if `NAME` is omitted) and associates the target path for variable injection (defaulting to current directory or `--path`).
  - Generates a human-readable `.crypt-env.yaml` containing project name, description, associated paths, tags/categories, and environments.
- **Bidirectional Configuration Sync (`crypt-env config`)**:
  - Synchronizes project metadata, environments, paths, and tags between the local `.crypt-env.yaml` and the CryptEnv GUI / database.
  - Implements last-modified-wins precedence by comparing `.crypt-env.yaml` file mtime with project `updated` timestamp from the vault.
- **Enhanced Variable Addition (`crypt-env add KEY=value | $VAR | .env`)**:
  - Adds secrets to the default environment or an explicit `--env <NAME>`.
  - Supports `--global` flag for cross-project secrets.
  - Detects collisions: when a key already exists, warns the user, halts addition, asks for the master password, and upon user confirmation ('y') displays the colliding value.
- **Environment Materialization (`crypt-env fill [--env <NAME>]`)**:
  - Generates `.env` files for all environments (or a specific `--env`) using encrypted secrets.
  - Simultaneously creates sanitized `.env.example` templates suitable for Git commit.
  - Password-protected: requires master password authentication before writing plaintext values to disk.
- **Template Synchronization (`crypt-env sync [--global]`)**:
  - Reads `.env.example` templates and provisions vault items with empty or `change-me` values for unassigned keys.
  - With `--global`, automatically populates matching global vault secrets into the target `.env`, leaving unmatched keys empty or `change-me`.
  - Password-protected action.
- **Secure Shell Injection (`crypt-env inject <KEY>`)**:
  - Injects specific secret variables into the invoking shell session without echoing secret values to the terminal screen.
  - Requires master password authentication.
- **Search & Inspection (`crypt-env search [PATTERN] [--global]`)**:
  - Lists and searches project or global variables matching regex or substring patterns.
  - Password-protected action.
- **WSL Setup Detection & Guidance (`crypt-env setup wsl [DISTRO]`)**:
  - When executed inside WSL: automatically identifies the running distribution and configures shell initialization.
  - When executed from Windows host: detects installed WSL distributions; if none found, prints an informative message; if multiple exist or none specified, lists available distros and instructs the user to run `crypt-env setup wsl <distro>`.
- **Comprehensive Diagnostics (`crypt-env doctor`)**:
  - Comprehensive health check: GUI application process status, vault lock status, local REST API connectivity, TLS certificate validity and paths, CLI session token status, MCP token status, `.crypt-env.yaml` schema/path validity, and WSL integration status.
- **Interactive TUI Modernization (`crypt-env tui`)**:
  - Full interactive terminal interface reflecting the same project workflows (`init`, `config`, `fill`, `sync`, `search`, secret navigation, and details) built on `ratatui`.
- **Removed Subcommands**:
  - Deletes `memory`, `list`, `exec`, `cmd`, legacy `project`, `share`, `relay`, `category` and `set` from the CLI entirely (the GUI keeps these capabilities).
- **Per-Terminal Password Sessions**:
  - Password-gated commands prompt only when the current terminal has no live CLI session; a session lasts `auto_lock_timeout` (default 5 min) and is renewed on every use, like `sudo`.

## Capabilities

### New Capabilities
- `tui`: Interactive terminal user interface for project browsing, environment switching, secret inspection, syncing, and diagnosing without leaving the terminal.

### Modified Capabilities
- `cli`: Realigns CLI subcommands to provide direct project/environment parity (`init`, `config`, `add`, `fill`, `sync`, `inject`, `search`, `setup wsl`, `doctor`) with YAML-based project files (`.crypt-env.yaml`), collision prompts, password verification on secret operations, and removing legacy/unsupported subcommands.

## Non-Goals

- Implementing remote or cloud synchronization beyond the existing local REST API and internet relay.
- Implementing standalone command execution or saved shell snippets in this change (`memory`, `cmd`, `exec` remain explicitly unimplemented).
- Replacing the desktop GUI; the GUI remains the primary administrative console.
- Storing unencrypted secret values in `.crypt-env.yaml` (only project/environment metadata, paths, and tags are stored in YAML).

## Security Considerations

- **Master Password Zeroization**: Password-gated operations (`fill`, `sync`, `inject`, `search`, collision reveal) require a live per-terminal session, prompting for the master password when there is none; passwords are verified by `POST /unlock` and zeroized immediately after use. Sessions slide on use and expire after `auto_lock_timeout`.
- **Safe Variable Injection**: `crypt-env inject` must never print secret values directly to stdout/stderr in unquoted or exposed form that could be captured by terminal logs or scrollback; it must output safe shell evaluations or pipeable assignments.
- **Zero Plaintext Secrets in Config Files**: `.crypt-env.yaml` MUST NOT contain decrypted secret values. It only describes project schema, environments, tags, and injection file paths.
- **Accidental Overwrite Protection**: `fill` creates backups when overwriting pre-existing foreign files. Collision detection in `add` prevents unintended overwrites of existing environment variables.

## Impact

- CLI binary `crypt-env` (under `src-tauri/src/bin/crypt-env/`).
- Local REST API in `src-tauri/src/api/` (supporting any new endpoints or parameters required by `config` synchronization, timestamp queries, or collision queries).
- Shared project configuration parsing (transitioning `crypt-env.json` to `.crypt-env.yaml` with backward compatibility during discovery).
- Public documentation in `docs/index.html` and `docs/cli.md` updated to match the new CLI/TUI command contracts.
