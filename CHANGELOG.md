# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [1.1.0] - 2026-10-04

### Added

- **CLI/GUI sync and environment paths** (`cli-gui-sync-and-env-paths`):
  - The open window refreshes when the CLI, TUI or MCP change the vault (payload-free `vault_changed` event, emitted only after an authenticated successful write), plus a titlebar refresh button and `F5` / `Ctrl+R`.
  - `crypt-env add VARNAME` reads the variable from the shell (`$VARNAME` still works). An argument the shell pre-expanded from `$VARNAME` is diagnosed by variable name only; the error no longer echoes the argument.
  - `crypt-env init` targets `./` by default. New projects from `init` and the GUI modal (default initial environment) also get empty `staging` and `production` environments beside `default`; existing projects are untouched.
  - Bulk GUI commands (backup restore, item import, share/relay receive) emit `vault_changed` too.
  - Environment paths may be folders (`./`, `apps/web`) that receive `.env` / `.env.<name>`; explicit `.env*` file paths are unchanged. A rooted environment with no paths defaults to the root folder for session callers (never for the MCP token), with `output_path`-style overwrite gating.

### Fixed

- A relative environment path whose final component is a symlink to a file inside the project root is now refused instead of written through.

- **Vault write integrity** (`vault-db-transactional-integrity`):
  - Saving the category list no longer erases project-category links: it now diffs and upserts in one transaction, removing links only for categories that were actually deleted. Links lost by earlier versions cannot be reconstructed.
  - Saving an environment is atomic (rename, paths, ownership and variables commit together) and rejects duplicate variable keys or paths before writing anything.
  - Unlock is all-or-nothing: a failed migration leaves the vault locked for the GUI, REST and MCP.
  - `PUT /items/:id` holds the vault lock and a transaction across read-merge-write, so concurrent updates no longer overwrite each other.
  - Un-globaling a multi-owner item and the legacy literal-variable migration are atomic.
  - `foreign_keys`, `secure_delete` and WAL are now set on every pooled SQLite connection (5 s busy timeout).

### Removed (Breaking)

- **Deprecated MCP environment `id` alias** (issue #10): Calls to `crypt_env_inject_environment` and `crypt_env_generate_example_env` using the `id` parameter instead of `environment_id` are no longer accepted. Use `environment_id` in all calls. The deprecation window from 1.0.2 has ended.

### Action Required (Upgrade Notes)

- **Windows Hello / Biometric unlock users** must re-enroll in Settings after upgrading. The encryption key storage mechanism has changed from DPAPI to Hello-derived keys; existing enrollments are automatically removed on upgrade, and a one-time notice will prompt re-enablement.
- **Internet relay users** (Supabase deployment): Apply the security-definer RPC migration (`sql/v2-security-definer.sql`) to your Supabase project. The relay now uses row-level security and PostgreSQL functions instead of direct table access for stricter permission enforcement. See the deployment guide for details.
- **MCP server users**: Tools that can exfiltrate secrets (`crypt_env_reveal_item`, `crypt_env_list_items`, `crypt_env_inject_environment`) now require explicit GUI approval before use. A modal in the app grants or denies access per connected MCP session.

---

## [Unreleased]

---

## [1.0.3] - 2026-09-27

### Added

- **Managed `crypt-env` launcher shim for WSL**:
  - The WSL configuration operation installs a managed launcher (`crypt-env`) inside configured distributions that forwards commands to the Windows-installed CLI binary with transparent exit code propagation, argument forwarding, and stdin/stdout streaming.
  - Automatically isolates WSL-side client environment variables (`CRYPTENV_API_URL`, `CRYPTENV_CERT_PATH`, `CRYPTENV_TOKEN_PATH`) so the launcher targets the host vault correctly.
  - Non-destructive PATH management: appends the launcher directory to `PATH` with lower precedence than any native Linux client.
  - Works with standard WSL2 NAT networking without requiring mirrored networking (`networkingMode=mirrored`).
  - Added launcher status reporting and reconfigure flow to Settings -> WSL Integration.
- **Synced OpenSpec capability specifications**:
  - Main capability specs synced into `openspec/specs/` (`cli`, `desktop-packaging`, `app-updater`, `wsl-integration`).
  - Archived completed changes (`cli-remote-endpoint-config`, `windows-installer-cli-and-wsl-panel`, `startup-update-notice`, `wsl-crypt-env-shim`).
- **Share a whole project via relay** (issue #4) — one send + one receive now carries a project's structure (environments, `isDefault`) *and* the decrypted values of every item they reference, for however many environments are selected, in a single encrypted relay round-trip. Previously the same outcome took an export-template + relay-send-per-environment workaround.
  - **Protocol**: `ProjectBundle`/`EnvironmentBundle`/`ProjectBundleVar` in `share/relay.rs` (`kind: "project"`, `version: 1`, checked on decrypt). Items are deduped by name and hoisted to the bundle root, so an item linked into 3 environments produces one bundled item, not three. Never carries `paths` (machine-specific, dropped) or a `literal` value field.
  - **REST**: `POST /projects/:id/relay/send` (body `{environment_ids}`), `POST /projects/relay/receive` (body `{code, passphrase, project_name_override?}`).
  - **Tauri**: `project_relay_send(project_id, environment_ids)`, `project_relay_receive(code, passphrase, project_name_override?)`.
  - **CLI**: `crypt-env project share --id N --envs a,b [--yes]`, `crypt-env project receive --code X --passphrase Y [--as NAME]`. `--envs` omitted defaults to the default environment only; `--envs all` opts in explicitly.
  - **GUI**: "SHARE PROJECT" / "RECEIVE PROJECT" on the Projects screen, with a per-environment key manifest (KEY → item name, never values) shown before upload and a rename prompt on a name collision.
  - **Receive semantics**: always creates a **new** project — never merges into an existing one. A case-insensitive project-name collision is a hard error (`409 CONFLICT` over REST), retryable with an override name. The whole receive is one transaction (all-or-nothing). Received items are owned by the new project only, never `isGlobal`.
  - **No MCP tool** — deliberately not exposed to MCP; see `docs/reference.md`'s Notes for the reasoning.

### Removed (Breaking)

- **Legacy workspace relay endpoints and MCP tools**, dead since the Projects/Environments migration: they read/wrote the frozen `workspaces`/`workspace_vars` tables that no product surface displays post-migration, so any import through them landed as ownerless, invisible items.
  - REST: `POST /workspaces/:id/relay/send`, `POST /workspaces/relay/receive`.
  - MCP tools: `crypt_env_share_workspace_send`, `crypt_env_share_workspace_receive`.
  - `WorkspaceBundle`/`WorkspaceBundleVar`/`encrypt_workspace`/`decrypt_workspace` deleted from `share/relay.rs`. The underlying `workspaces`/`workspace_vars`/`workspace_paths` tables and their `db` accessors are **kept** — they remain the one-time migration backfill source for pre-migration installs.
  - Not aliased: both endpoints were already documented as legacy/broken, and their one shipped consumer (the MCP server) is updated in the same change.

---

## [1.0.2] - 2026-08-04

### Changed

- **MCP: unified the environment-scope parameter name on `environment_id`** (issue #10). `crypt_env_inject_environment` and `crypt_env_generate_example_env` were the only two MCP tools naming the environment identifier `id` instead of `environment_id`, matching every other environment-scoped tool. This let an LLM caller that inferred the parameter name from the majority pass `environment_id` to these two tools and have it silently ignored — in the case where `project`+`environment` were also present, the resolver would fall through and inject a *different* environment's full decrypted variable set with no error. Both schemas now advertise `environment_id` with the canonical description used by every other scoped tool; the ambiguity error in `crypt_env_inject_env_by_name` now names `environment_id` instead of `id` as well.
  - The bare `id` key is accepted as an unadvertised, deprecated alias for the whole 1.0.x line (`// DEPRECATED(remove in 1.1.0): environment 'id' alias, issue #10`). Calls using `id` still succeed, but the response text appends a fixed deprecation notice so the model is told to switch. Removed in 1.1.0.
  - New in-crate tests (`src-tauri/src/bin/crypt-env-mcp.rs`, `#[cfg(test)] mod tests`) assert every environment-scoped tool declares `environment_id` and not a bare `id`, that the five item/category/workspace tools keep their bare `id`, and that the resolver prefers `environment_id` over the alias.
  - `docs/reference.md` updated to drop the "inconsistent naming" note and describe the resolved state + deprecation window.

## [1.0.1] - 2026-07-28

### Fixed

- **CRITICAL: Windows installer could silently wipe the user's entire PATH environment variable.** The NSIS installer's PATH-safety guard (added in a prior fix) inferred whether the registry value had been truncated by inspecting the *length of the value ReadRegStr returned* — but `ReadRegStr` itself silently truncates any value longer than NSIS's internal string cap with no error signal. A sufficiently long PATH could come back already truncated to something *under* the guard's threshold, pass the check as "safe", and then get written back verbatim — permanently destroying everything past the truncation point. Fixed by querying the registry directly via `RegQueryValueExW` (Win32 API, through NSIS's `System::Call`) to measure the *true* on-disk size of `HKCU\Environment\Path` before any bounded NSIS string variable is involved, so the decision to proceed is based on ground truth instead of a value that may already be mangled. Applies to both the install-time `AddToUserPath` and uninstall-time `RemoveFromUserPath` hooks (`src-tauri/nsis/installer_hooks.nsi`). Verified by compiling the installer script standalone with `makensis`.
- Removed `src-tauri/nsis/path_setup.nsh`, an unreferenced duplicate of the PATH-management logic that was not wired into the build (`tauri.conf.json` only points at `installer_hooks.nsi`) — its divergence from the real file was itself a hazard.

### Known Limitations (Carried Forward)

- This fix is Windows-only; the Linux (`deb`) and macOS (`dmg`) targets do not modify `PATH` programmatically at all (see `docs/building.md`'s manual install instructions), so they were never exposed to this bug class.

---

## [1.0.0] - 2026-07-28

### Changed

- **Major Architecture Refactor: Projects & Environments** — Replaced flat "Workspaces" model with hierarchical Projects containing typed Environments. Every environment variable is now a real vault item (not literal key=value). Projects are now the primary navigation landing page.
  - **Backend**: New tables `projects`, `environments`, `environment_vars`, `item_projects` (many-to-many), `project_categories`. Deleted `workspace/mod.rs`. Item ownership tracked explicitly; global items reusable across projects.
  - **Database**: Added `items.is_global` plaintext column (queryable without decryption). One-time migration `vault::migrate_literal_vars_to_items` converts legacy literal vars to real encrypted items on unlock.
  - **Deletion Logic**: Project deletion is transactional with preview of impact (environments deleted, items deleted vs orphaned). Un-globaling multi-owner items forks them into independent copies.
  - **Frontend**: New `ProjectManager.tsx` screen (project list → detail with environment cards → environment editor). `MainVault.tsx` renamed to `GlobalSecrets.tsx` (filtered to `isGlobal` items only, reachable via footer link). New `src/components/itemFields/` extracted shared per-type field components used by both ProjectManager and EditItem.
  - **Tauri Commands**: New `vault_create_project_item`, `vault_set_item_global`, `vault_get_item_owners`, `project_preview_delete`. Deleted `workspace_*` commands.
  - **CLI**: Replaced `crypt-env workspace` with `crypt-env project` (list/inject/delete/delete-env). Inject now works per-environment.
  - **MCP Server**: Renamed tools: `crypt_env_list_projects`, `crypt_env_inject_environment`, `crypt_env_list_environments_by_name`, `crypt_env_inject_env_by_name`. Legacy `crypt_env_share_workspace_send/receive` unchanged (out of scope, workspace-table-backed).
  - **Types**: `Workspace` → `Project` + `Environment`. `WorkspaceVar` → `EnvironmentVar`. `workspaceStore.ts` → `projectStore.ts`.
  - **Config**: Added `pnpm-workspace.yaml` for monorepo configuration (esbuild disabled).

- **Mandatory Project + Environment Scoping** — All interfaces that operate on vault items now require an explicit project + environment scope. Scope is a filter over linked variables, not access control.
  - **REST API**: Scoped endpoints (`GET`/`POST /items`, `GET /commands`, `POST /fill`, `POST /share/listen`, `POST /share/connect`, `POST /share/import`, `POST /relay/receive`) require either `environment_id` (i64) or both `project` and `environment` (case-insensitive names) as query params. Missing or unresolvable scope returns `422 VALIDATION_ERROR`. `GET`/`PUT`/`DELETE /items/:id` and `POST /items/:id/reveal` remain unscoped by design. New `POST /environments/:id/example` generates placeholder-only env files. `POST /environments/:id/inject` now takes a JSON body `{output_path, output_dir}`.
  - **CLI**: New shared scope resolver (`commands/scope.rs`) applied across all scoped commands. Resolution order per field: CLI flags (`--project`, `--env`) → `crypt-env.json` (searched upward from cwd) → cwd folder name + the project's default environment. Auto-creation of projects is restricted to `add`; every other command errors clearly if the project is missing. `--project`/`--env` added to `add`, `fill`, `inject`, `set`, `search`, `exec`, `list`, `cmd`, `sync`, `share send/receive`, `relay receive`, and `tui`.
  - **TUI**: Top bar shows the resolved project/environment; the item list is scoped to that environment's linked variables.
  - **MCP Server**: 14 tools gained matching `environment_id` / `project` / `environment` parameters in schema and docs, plus new `crypt_env_generate_example_env`.
  - **Docs**: `docs/reference.md` rewritten with the updated endpoint table, the scoping contract, the single-user vault access model, and known limitations.

### Fixed

- **`fill` no longer blanks unmatched keys** — When a key in the target `.env` is not found among the resolved environment's linked variables, the original line is now preserved intact and the key is reported as a warning. Previously unmatched keys were rewritten as `KEY=`, silently destroying existing values in plain (non-example) `.env` files.
- **Project name collision race** — Added a case-insensitive `UNIQUE` index on `projects.name` (`idx_projects_name_nocase`). Concurrent CLI auto-create now resolves deterministically: `POST /projects` returns `409 Conflict` on collision, and the loser re-fetches and reuses the winning project instead of failing. When `add` creates a project it re-reads the actual default environment name rather than assuming a literal.
- **Share/relay key hijack** — Imports from LAN share, internet relay, or `.vault` files no longer overwrite an existing variable link when an incoming item's name collides with a key already linked in the target environment. Collisions are skipped and surfaced via `skipped_keys` in `/share/status` and CLI warnings, preventing a peer from silently repointing the receiver's links with crafted item names.
- **Invisible imported items** — Items imported via `/share/import` and `/relay/receive` with a scope provided are now both owned by the project and linked into the environment (matching `add_item` / `POST /items`). Previously they were owned but never linked, making them invisible to scoped `GET /items` and `crypt_env` list queries. MCP `share_import` and `relay_receive` now build scoped query strings when calling those endpoints.

### Known Limitations (Carried Forward)

- Project export/import templates do not carry resolved secret values (by design)
- Backup/restore does not yet include projects/environments/item_projects tables (pre-existing gap, pre-planned)

---

## [0.1.0] - 2026-04-25

### Added

- **Encrypted local vault** — AES-256-GCM encryption for all sensitive fields, Argon2id for master password hashing
- **5 item types** — API Key, Credential (user+pass+URL), Link, Command (with placeholder resolution), Note
- **Desktop UI** — Industrial aesthetic, dark theme, 5 screens (Lock, Main Vault, Add/Edit Item, Category Manager, Settings)
- **Global hotkey** — Press `Ctrl+Alt+Z` from any app to toggle vault
- **Fuzzy search** — Find items instantly by name or content
- **Editable categories** — Organize vault items with user-defined categories
- **Clipboard integration** — Copy secrets in one click, auto-clear after timeout
- **Export formats** — Generate `.env`, bash `export VAR=val`, PowerShell `$env:VAR = "val"` format
- **Auto-lock timeout** — Vault locks automatically after configurable inactivity (5 min default)
- **CLI binary** (`crypt-env`) — Manage vault from terminal: fill `.env`, set env vars, run commands with placeholders, search items
- **Local REST API** — Axum server at `127.0.0.1:47821` for local integrations (locked by default, requires token)
- **MCP server** (`crypt-env-mcp`) — Stdio-based JSON-RPC 2.0 server for AI agents (Claude Code, Claude Desktop)
  - `vault_list_items` — List items by type/category (no secrets exposed)
  - `vault_get_item` — Get item metadata without secret values
  - `vault_generate_env` — Generate `.env` file (values never in response)
  - `vault_inject_env` — Inject secret as environment variable in client process
  - `vault_add_item` — Add new item to vault
  - `vault_update_settings` — Update app settings (not master password)
  - `vault_list_commands` — List available commands with placeholders
  - `vault_run_command` — Resolve command placeholders
- **Windows NSIS installer** — One-click install with PATH registration
- **Zeroized keys** — Encryption key wiped from memory on lock
- **Timing-safe token comparison** — Prevent brute-force attacks on unlock endpoint
- **Strict Content Security Policy** — Tauri webview hardening
- **Import from password managers** — Import secrets from `.env` files, Bitwarden CSV, and 1Password CSV
- **Encrypted backup & restore** — Export and restore full vault with encryption

### Security Notes

- Master password is never persisted — exists only in memory during active session
- MCP server never returns secret values in plain text — injects directly as environment variables
- REST API is localhost-only (`127.0.0.1:47821`) and requires MCP token authentication
- All secret fields encrypted at rest in SQLite database
- Sensitive data structures use `zeroize` to prevent accidental plaintext leaks

### Known Limitations

- Windows focus (macOS and Linux support in progress)
- Single-user per vault file
- No encrypted cloud backup (intentional — local-first design)

[Unreleased]: https://github.com/maosuarez/crypt-env/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/maosuarez/crypt-env/releases/tag/v0.1.0
