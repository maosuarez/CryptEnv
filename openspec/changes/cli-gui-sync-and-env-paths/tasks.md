## 1. Backend Synchronization & Event Bus

- [ ] 1.1 Update `src-tauri/src/api/mod.rs` to accept `tauri::AppHandle` and emit `"vault_changed"` on successful POST/PUT/DELETE mutations (`items`, `projects`, `environments`), verifying compilation with `cargo check`.
- [ ] 1.2 Update Tauri mutation commands in `src-tauri/src/lib.rs` and project commands to emit `"vault_changed"` when mutations occur, verifying with `cargo check`.

## 2. Desktop GUI Reactive Auto-Refresh & Manual Controls

- [ ] 2.1 Update `src/App.tsx` to listen for `"vault_changed"` events and trigger query invalidation and store refreshes (`projectStore`, `itemStore`), verifying with `pnpm build`.
- [ ] 2.2 Add a manual refresh button in `src/components/WindowChrome.tsx` and register `F5` / `Ctrl+R` / `Cmd+R` keyboard shortcut for immediate manual refresh, verifying with `pnpm build`.

## 3. CLI Variable Addition & Shell Expansion Diagnostics

- [ ] 3.1 Update `parse_input` in `src-tauri/src/bin/crypt-env/commands/add.rs` to accept `VARNAME` directly (without `$`) from `std::env::var`, and support stripped `$` prefixes (`'$VARNAME'` / `\$VARNAME`), verifying with unit tests in `add.rs`.
- [ ] 3.2 Add shell-expansion detection in `src-tauri/src/bin/crypt-env/commands/add.rs` that checks `std::env::vars()` when an unparseable token is provided, printing a descriptive diagnostic message, verifying with `cargo test --bin crypt-env`.

## 4. CLI Init Gitignore Management & Baseline Environments

- [ ] 4.1 Update `src-tauri/src/bin/crypt-env/commands/init.rs` to detect Git repositories and ensure `.crypt-env.yaml` is listed in `.gitignore` (creating or appending as needed), verifying with unit tests in `init.rs`.
- [ ] 4.2 Update project initialization in `commands/init.rs` and `src-tauri/src/project/mod.rs` to seed standard baseline environments (`development` [default], `staging`, `production`) when creating a new project, verifying with `cargo test`.

## 5. Folder-Based Environment Paths & Default `./`

- [ ] 5.1 Update environment path resolution in `src-tauri/src/project/mod.rs` to treat relative folder paths (defaulting to `./` when empty) as directory targets containing `environment_filename(&env.name)`, while maintaining backward compatibility for explicit `.env*` files, verifying with `cargo test --lib project`.
- [ ] 5.2 Update `src/components/ProjectManager.tsx` to default empty environment paths to `./`, display the resolved filename preview for directory targets, and allow configuring multiple folder paths, verifying with `pnpm build`.

## 6. Documentation and Integration Verification

- [ ] 6.1 Update public documentation in `docs/index.html` and `docs/cli.md` reflecting `crypt-env add VARNAME`, `.gitignore` handling in `init`, and folder-based environment targets.
- [ ] 6.2 Execute full test suites (`cargo test` across backend and CLI, and `pnpm build`) to verify all components compile cleanly.
