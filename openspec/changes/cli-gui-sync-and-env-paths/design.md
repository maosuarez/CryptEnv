## Context

CryptEnv operates as a local-first desktop application with multiple client interfaces: Desktop GUI (Tauri + React), CLI binary (`crypt-env`), interactive TUI (`crypt-env tui`), and MCP server (`crypt-env-mcp`). All non-GUI clients interact with the vault via the local Axum REST server bound to `127.0.0.1:47821`.

Currently:
1. When mutations are performed via CLI/REST/MCP, the desktop GUI does not react because no mutation events are emitted, and the UI lacks a manual refresh button.
2. In POSIX and Windows shells, running `crypt-env add $VARNAME` causes the shell to expand `$VARNAME` before `crypt-env` executes, passing the secret value as the first argument; `crypt-env` then rejects it with a configuration error. Passing `crypt-env add VARNAME` (without `$`) is also rejected.
3. `crypt-env init` creates `.crypt-env.yaml` without ensuring it is ignored in `.gitignore`, and registers new projects with only a single generic `"default"` environment.
4. Environments require explicit paths and default to nothing. Paths are treated as file targets, whereas the target filename (`.env`, `.env.production`) is already implied by the environment name.

## Goals / Non-Goals

**Goals:**
- Provide real-time UI synchronization via a Tauri `vault_changed` event emitted whenever the REST API or Tauri commands mutate vault items, projects, or environments.
- Provide a manual refresh control and keyboard shortcut (`Ctrl+R` / `F5`) in the Desktop GUI.
- Allow `crypt-env add` to accept `VARNAME` directly from active shell environment variables, accept escaped `\$VARNAME` / `'$VARNAME'`, and provide clear guidance when a shell expansion is detected.
- In `crypt-env init`, automatically detect Git repositories and ensure `.crypt-env.yaml` is listed in `.gitignore`.
- In `crypt-env init`, seed standard baseline environments (`development` [default], `staging`, `production`) when creating a new project.
- Treat configured environment paths as folder directories relative to the project root (defaulting to `./`), automatically computing `<folder>/.env.<name>` (or `<folder>/.env` for default), while preserving full backward compatibility for explicit file paths.

**Non-Goals:**
- Modifying remote sync / internet relay protocols.
- Automated `git commit` or `git push` operations for `.gitignore`.
- Forcing a breaking change on existing stored projects: explicit `.env*` file paths will continue to resolve as direct file targets.

## Decisions

### 1. IPC Event Architecture for Reactive Synchronization

```
┌───────────────────────────────────────────────┐
│ External Clients                              │
│ (CLI / TUI / MCP)                             │
└───────────────────────┬───────────────────────┘
                        │ HTTP (127.0.0.1:47821)
                        ▼
┌───────────────────────────────────────────────┐
│ Axum REST API (src-tauri/src/api)             │
│ (Receives AppHandle)                          │
└───────────────────────┬───────────────────────┘
                        │ app_handle.emit("vault_changed", ())
                        ▼
┌───────────────────────────────────────────────┐
│ Tauri Desktop Window (React Frontend)         │
│ - App.tsx listens to "vault_changed"          │
│ - Triggers projectStore & itemStore refetch   │
│ - Manual Refresh button triggers same refetch │
└───────────────────────────────────────────────┘
```

- **Decision**: Pass `AppHandle` to `api::start_server`. When any write endpoint in `src-tauri/src/api/mod.rs` (`POST/PUT/DELETE /items`, `POST /projects`, `POST /environments`, `DELETE /projects`, etc.) successfully commits, emit `"vault_changed"`.
- **Frontend Handling**: In `src/App.tsx`, listen to `"vault_changed"` using Tauri's `listen()`. Debounce rapid consecutive triggers (100ms) and invalidate TanStack Query caches, re-fetching `projectStore.fetchProjects()` and `itemStore.fetchItems()`.
- **Manual Refresh**: Add a refresh button in `WindowChrome.tsx` and register a global keydown handler for `F5` / `Ctrl+R` (`Cmd+R` on macOS) that calls the refresh method.
- **Alternatives Considered**: Polling the database from frontend on a timer (rejected due to wasted CPU and SQLite locks) or SSE/WebSockets (rejected as Tauri's native `AppHandle::emit` already provides direct zero-overhead IPC).

### 2. CLI `add` Parsing and Shell-Expansion Protection

- **Decision**:
  1. If `input.starts_with('$')`: strip `$` and read `std::env::var(stripped_name)`.
  2. If `input` contains `=`: parse as literal `KEY=value`.
  3. If `Path::new(input).is_file()`: parse as dotenv file.
  4. If `input` is a valid environment variable name (`[A-Za-z_][A-Za-z0-9_]*`): check `std::env::var(input)`. If found, use `key = input`, `value = val`.
  5. Fallback diagnostic: If `input` does not match any of the above, inspect `std::env::vars()` to see if any variable currently holds `input` as its value. If found, print a targeted error explaining that `$VARNAME` was pre-expanded by the shell and instructing the user to run `crypt-env add <VARNAME>` without `$`. If not found, print a clear usage message detailing the valid formats and the shell expansion caveat.
- **Alternatives Considered**: Attempting to hook into shell history or raw command line (rejected because `argv` receives pre-expanded tokens by POSIX shell specification; inspecting `std::env::vars()` safely detects pre-expansion without fragile platform hacks).

### 3. Git Repository Detection and `.gitignore` Maintenance in `init`

- **Decision**:
  - In `commands/init.rs`, check if `dir.join(".git").exists()` or if any parent directory contains `.git`.
  - If a Git repository is detected:
    - Path to `.gitignore` is `dir.join(".gitignore")`.
    - If `.gitignore` does not exist, create it with `.crypt-env.yaml\n`.
    - If `.gitignore` exists, read lines. If `.crypt-env.yaml` is not present as a separate line or entry, append `\n.crypt-env.yaml\n`.
- **Project Seeding**:
  - When creating a new project during `init` (or when `client::save_project` creates a new project), create three baseline environments:
    - `development`: `is_default: true`, `paths: ["./"]`
    - `staging`: `is_default: false`, `paths: ["./"]`
    - `production`: `is_default: false`, `paths: ["./"]`
- **Alternatives Considered**: Leaving `.crypt-env.yaml` in git by default (rejected per user request and because manifest may contain paths and project IDs that developers may prefer isolated).

### 4. Folder-Based Environment Paths & Default `./`

- **Decision**:
  - In `src-tauri/src/project/mod.rs` (and `resolve_env_path`):
    - If configured `paths` is empty, treat as `["./"]`.
    - For each path in `paths`:
      - If the path ends in a file starting with `.env` (e.g. `.env`, `.env.test`, `apps/api/.env`), preserve direct file resolution for backward compatibility.
      - Otherwise, treat the path as a folder directory relative to project root (e.g. `./`, `apps/web`), and append `environment_filename(&env.name)` (`.env` for root/default environments, `.env.<name>` for named environments).
  - In GUI `ProjectManager.tsx`:
    - Display folder targets with their evaluated `.env` filename preview (e.g. `./ → .env.production`).
    - If no paths are configured, automatically default to `./`.
- **Alternatives Considered**: Strict breaking migration converting all old paths to directories (rejected: would break existing setups with custom `.env.local` or existing configurations).

## Security & Threat Model

- **IPC Security**: The `vault_changed` event carries NO secret payload; it is an empty signal (`()`) indicating that vault state has changed. The frontend fetches data via existing authenticated Tauri invoke handlers.
- **REST API Emission**: Emitting `vault_changed` from Axum occurs only after authentication tokens have been validated using constant-time comparison and after the database write succeeds.
- **Environment Variable Reading**: When `crypt-env add VARNAME` reads `std::env::var(VARNAME)`, it reads only the specified variable from the current process environment and zeros/drops buffers according to existing vault memory hygiene rules.
- **Gitignore Safety**: Modifying `.gitignore` is additive only (appends `.crypt-env.yaml`), never deleting or modifying existing rules.

## Risks / Trade-offs

- **[Risk] Multiple rapid CLI invocations could flood Tauri events**
  → *Mitigation*: The frontend debounces `vault_changed` handlers by 100ms so a batch of CLI adds triggers a single UI re-render.
- **[Risk] Ambiguity between directory path and file path**
  → *Mitigation*: Any path ending in `.env*` is treated as a file; any path ending in `/` or naming a directory without `.env*` is resolved as a directory and appended with `environment_filename(name)`.
- **[Risk] Existing projects without `.gitignore` in non-root subdirectories**
  → *Mitigation*: Check `.git` existence at current directory and climb upwards to identify the repo boundary if necessary.
