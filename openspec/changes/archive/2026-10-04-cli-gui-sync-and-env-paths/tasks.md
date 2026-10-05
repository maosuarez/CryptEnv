## 1. Backend Synchronization & Event Bus

- [x] 1.1 Update `src-tauri/src/api/mod.rs` to accept `tauri::AppHandle` and emit `"vault_changed"` on successful POST/PUT/DELETE mutations (`items`, `projects`, `environments`), verifying compilation with `cargo check`.
- [x] 1.2 Emit `"vault_changed"` (empty payload) after success from the bulk GUI commands only (`vault_import_backup`, `vault_import_backup_data`, `vault_import_items`, `share_import_file`, `share_relay_receive`, `project_relay_receive`) via `api::changes::emit_vault_changed`; ordinary single-item GUI mutations do not emit. Verified with a mock-runtime event test and `cargo check`.

## 2. Desktop GUI Reactive Auto-Refresh & Manual Controls

- [x] 2.1 Update `src/App.tsx` to listen for `"vault_changed"` events and trigger query invalidation and store refreshes (`projectStore`, `itemStore`), verifying with `pnpm build`.
- [x] 2.2 Add a manual refresh button in `src/components/WindowChrome.tsx` and register `F5` / `Ctrl+R` / `Cmd+R` keyboard shortcut for immediate manual refresh, verifying with `pnpm build`.

## 3. CLI Variable Addition & Shell Expansion Diagnostics

- [x] 3.1 Update `parse_input` in `src-tauri/src/bin/crypt-env/commands/add.rs` to accept `VARNAME` directly (without `$`) from `std::env::var`, and support stripped `$` prefixes (`'$VARNAME'` / `\$VARNAME`), verifying with unit tests in `add.rs`.
- [x] 3.2 Add shell-expansion detection in `src-tauri/src/bin/crypt-env/commands/add.rs` that checks `std::env::vars()` when an unparseable token is provided, printing a descriptive diagnostic message, verifying with `cargo test --bin crypt-env`.

## 4. Baseline Environments for New Projects

- [ ] 4.1 ~~Ensure `.crypt-env.yaml` is listed in `.gitignore` during `init`~~ DROPPED by user decision (the manifest stays safe to commit). Implemented in 9d8334b and reverted in 20d9784.
- [x] 4.2 Seed the baseline environments `default` (default), `staging`, `production` for new projects created by CLI `init` and the GUI project modal through the opt-in `ProjectInput.seedBaseline` in `project::save_project` (new project, default initial environment only; existing projects, `config` manifest creation and custom initial environments unaffected), verifying with `cargo test`, `vitest` and the `init`/template tests.

## 5. Folder-Based Environment Paths & Default `./`

- [x] 5.1 Update environment path resolution in `src-tauri/src/project/mod.rs` to treat relative folder paths (defaulting to `./` when empty, for session callers only — never MCP) as directory targets containing `environment_filename(&env.name)`, while maintaining backward compatibility for explicit `.env*` files, verifying with `cargo test --lib project`.
- [x] 5.2 Update `src/components/ProjectManager.tsx` to default empty environment paths to `./`, display the resolved filename preview for directory targets, and allow configuring multiple folder paths, verifying with `pnpm build`.

## 6. Documentation and Integration Verification

- [x] 6.1 Update public documentation in `docs/index.html` and `docs/cli.md` reflecting `crypt-env add VARNAME`, and folder-based environment targets.
- [x] 6.2 Execute full test suites (`cargo test` across backend and CLI, and `pnpm build`) to verify all components compile cleanly.
  - Run from WSL: `cargo check --all-targets`, `cargo test --workspace`, `cargo clippy --all-targets`, `tsc --noEmit`, `vitest run`, `vite build` (scratch outDir). Manual `pnpm tauri dev` checks (live refresh from a real `crypt-env add`, F5/Ctrl+R, titlebar button, folder-path preview) are NOT done.
