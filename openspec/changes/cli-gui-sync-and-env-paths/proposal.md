## Why

When working across CLI, TUI, MCP, and the desktop GUI, developers encounter four workflow friction points:
1. Changes made through external tools (`crypt-env add`, REST API, MCP, or TUI) do not reflect in the open GUI without restarting or navigating away, and there is no manual refresh trigger.
2. In shells like Bash and Zsh, running `crypt-env add $VARNAME` causes the shell to expand `$VARNAME` to its value before passing it to the CLI, resulting in confusing errors like `'HolaSecreto' is not KEY=value, $VARNAME, or an existing .env file`. Furthermore, passing `crypt-env add VARNAME` (without `$`) is currently rejected.
3. New projects are created with only a single generic `default` environment rather than a baseline set (`default`, `staging`, `production`).
4. Environments currently require explicit file paths with no sensible defaults. The file name itself is already determined by the environment convention (`.env`, `.env.production`), but users are forced to specify full file paths rather than relative folder locations (defaulting to `./`), with awkward ergonomics for projects needing multiple directory targets.

Resolving these issues bridges the interaction between the CLI and GUI and aligns environment configuration with standard developer expectations.

## What Changes

- **Reactive GUI Synchronization & Manual Refresh**:
  - The Tauri backend emits a payload-free `vault_changed` event after an authenticated, successful REST write (CLI, TUI, MCP) and after the bulk GUI commands (backup restore, item import, share/relay receive). Ordinary single-item GUI edits do not emit.
  - The desktop GUI listens for `vault_changed` and automatically refetches project, environment, and item stores.
  - A manual refresh button and keyboard shortcut (`Ctrl+R` / `F5`) are added to the GUI titlebar and project header.
- **Improved CLI `add` for Environment Variables**:
  - `crypt-env add <VARNAME>` resolves the active shell variable `VARNAME` from the environment and adds it as `VARNAME=<value>`.
  - Escaped syntax like `crypt-env add '$VARNAME'` or `crypt-env add \$VARNAME` strips the leading `$` and extracts the environment variable.
  - If a bare input cannot be parsed and matches no file or `KEY=value`, the CLI checks if the value matches an active environment variable value in `std::env::vars()` or provides a descriptive hint explaining that the shell may have expanded `$VARNAME` before `crypt-env` ran, advising the use of `VARNAME` without `$`.
- **Baseline Environments for New Projects**:
  - New projects created by `crypt-env init` or the GUI project modal (with no custom initial environment) get `default` (default, root, injects `.env`), `staging` and `production`. Existing projects, manifest-driven creation by `config`, and projects with a custom initial environment are not given extra environments.
- **Relative Folder Paths & Default `./` for Environments**:
  - Environment target paths represent directory folders relative to the project root, defaulting to `./` when unspecified.
  - When injecting or generating preview targets, the destination file name is automatically computed using the environment's filename convention (e.g. `<folder>/.env` for default/root environments, `<folder>/.env.<name>` for named environments such as `production`).
  - Existing explicit file targets (e.g. paths explicitly naming `.env*`) remain supported for backward compatibility.
  - Environments can configure multiple relative folder paths (e.g., `["./", "apps/api", "apps/web"]`).
  - An environment of a rooted project with no paths defaults to the root folder for session callers (CLI/GUI) with `output_path`-style overwrite gating; the MCP principal does not get this default.

## Non-Goals

- Remote synchronization over internet relays for live GUI push (this change focuses on local process synchronization between CLI/REST/MCP and the local desktop GUI).
- Changing `.gitignore`: `.crypt-env.yaml` stays safe to commit and `init` does not touch Git files (dropped by user decision).
- Removing support for explicit legacy `.env` file paths (existing projects with full file paths will continue to inject to those paths).

## Capabilities

### Modified Capabilities
- `cli`: Update `crypt-env add` to support `VARNAME` directly from the environment, provide shell-expansion guidance, update `crypt-env init` to seed baseline environments (`default`, `staging`, `production`), and resolve relative directory paths for environments defaulting to `./`.
- `project-templates`: scaffolding also creates the baseline environments.
- `desktop-ui`: Add live auto-refresh on backend `vault_changed` events, provide a manual refresh button and shortcut in the interface, and update the environment path configuration to default to `./` folder paths while displaying the derived `.env.<name>` target.

## Impact

- **Backend / Tauri (`src-tauri`)**:
  - Emit Tauri event `vault_changed` from a route layer on the REST data-changing endpoints (`api/changes.rs`) and from the bulk GUI commands (`vault_import_backup`, `vault_import_backup_data`, `vault_import_items`, `share_import_file`, `share_relay_receive`, `project_relay_receive`).
  - `save_project` accepts an opt-in `seedBaseline` flag that creates `staging` and `production` beside `default` for a new project.
  - Path resolution logic in `src-tauri/src/project/mod.rs` to treat directory paths as containing `environment_filename(env.name)`.
- **CLI (`src-tauri/src/bin/crypt-env`)**:
  - `commands/add.rs`: parse bare `VARNAME` without `$`, strip `$`, inspect active env vars, and provide shell expansion tips on collision/not-found.
  - `commands/init.rs`: request the baseline environments for new projects and target `./` by default.
- **Frontend (`src/`)**:
  - Listen for `vault_changed` in `App.tsx` and invalidate TanStack Query / Zustand stores.
  - Add manual refresh button in `WindowChrome.tsx` / `ProjectManager.tsx`.
  - Update environment path inputs and helpers in `ProjectManager.tsx` to reflect folder paths and default `./`.
- **Public Documentation (`docs/index.html`, `docs/cli.md`)**:
  - Update documentation to reflect `crypt-env add VARNAME`, and folder-based environment paths.
