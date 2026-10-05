## 1. Backend Synchronization & Event Bus

- [x] 1.1 Update `src-tauri/src/api/mod.rs` to accept `tauri::AppHandle` and emit `"vault_changed"` on successful POST/PUT/DELETE mutations (`items`, `projects`, `environments`), verifying compilation with `cargo check`.
- [ ] 1.2 Update Tauri mutation commands in `src-tauri/src/lib.rs` and project commands to emit `"vault_changed"` when mutations occur, verifying with `cargo check`.
  - NOT DONE (needs a decision): GUI-originated commands already update the window that invoked them, so emitting would only cause a redundant full refetch (`vault_list` decrypts every item). Only REST writes (CLI/TUI/MCP) are external. Decide: drop 1.2 and reword the desktop-ui requirement to REST-originated changes, or emit from selected bulk commands (`vault_import_*`, `share_*_receive`, `project_relay_receive`).

## 2. Desktop GUI Reactive Auto-Refresh & Manual Controls

- [x] 2.1 Update `src/App.tsx` to listen for `"vault_changed"` events and trigger query invalidation and store refreshes (`projectStore`, `itemStore`), verifying with `pnpm build`.
- [x] 2.2 Add a manual refresh button in `src/components/WindowChrome.tsx` and register `F5` / `Ctrl+R` / `Cmd+R` keyboard shortcut for immediate manual refresh, verifying with `pnpm build`.

## 3. CLI Variable Addition & Shell Expansion Diagnostics

- [x] 3.1 Update `parse_input` in `src-tauri/src/bin/crypt-env/commands/add.rs` to accept `VARNAME` directly (without `$`) from `std::env::var`, and support stripped `$` prefixes (`'$VARNAME'` / `\$VARNAME`), verifying with unit tests in `add.rs`.
- [x] 3.2 Add shell-expansion detection in `src-tauri/src/bin/crypt-env/commands/add.rs` that checks `std::env::vars()` when an unparseable token is provided, printing a descriptive diagnostic message, verifying with `cargo test --bin crypt-env`.

## 4. CLI Init Gitignore Management & Baseline Environments

- [x] 4.1 Update `src-tauri/src/bin/crypt-env/commands/init.rs` to detect Git repositories and ensure `.crypt-env.yaml` is listed in `.gitignore` (creating or appending as needed), verifying with unit tests in `init.rs`.
- [ ] 4.2 Update project initialization in `commands/init.rs` and `src-tauri/src/project/mod.rs` to seed standard baseline environments (`development` [default], `staging`, `production`) when creating a new project, verifying with `cargo test`.
  - BLOCKED on a spec conflict (see report): `development` is not a root environment, so as the default it injects `.env.development`, not `.env`; living specs `cli` (init → `default` targeting `.env`), `desktop-ui` (initial environment `default`) and `project-templates` (`default` environment) contradict it, as does this change's own `init --path ./app` → `app/.env` scenario.

## 5. Folder-Based Environment Paths & Default `./`

- [x] 5.1 Update environment path resolution in `src-tauri/src/project/mod.rs` to treat relative folder paths (defaulting to `./` when empty) as directory targets containing `environment_filename(&env.name)`, while maintaining backward compatibility for explicit `.env*` files, verifying with `cargo test --lib project`.
- [x] 5.2 Update `src/components/ProjectManager.tsx` to default empty environment paths to `./`, display the resolved filename preview for directory targets, and allow configuring multiple folder paths, verifying with `pnpm build`.

## 6. Documentation and Integration Verification

- [x] 6.1 Update public documentation in `docs/index.html` and `docs/cli.md` reflecting `crypt-env add VARNAME`, `.gitignore` handling in `init`, and folder-based environment targets.
- [x] 6.2 Execute full test suites (`cargo test` across backend and CLI, and `pnpm build`) to verify all components compile cleanly.
  - Run from WSL: `cargo check --all-targets`, `cargo test --workspace`, `cargo clippy --all-targets`, `tsc --noEmit`, `vitest run`, `vite build` (scratch outDir). Manual `pnpm tauri dev` checks (live refresh from a real `crypt-env add`, F5/Ctrl+R, titlebar button, folder-path preview) are NOT done.
