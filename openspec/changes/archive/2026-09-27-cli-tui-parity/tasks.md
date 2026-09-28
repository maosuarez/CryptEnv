# Implementation Tasks: CLI, REST, and TUI Parity & Project Workflow Alignment

## 0. Project Root Path (shared backend, design Decisions 8–9)

- [x] 0.1 Add nullable `projects.root_path` (additive migration), `rootPath` on `Project`/`ProjectInput` (omitted = keep, `""` = clear, must be absolute), persisted by `save_project`.
- [x] 0.2 Resolve relative `environment.paths` against the project root (contained, error when no root) in inject/preview; unit tests.
- [x] 0.3 Shared `.crypt-env.yaml` model + serializer in `src-tauri/src/project/manifest.rs` with unit tests.

## 1. Project Configuration & Scope Resolution

- [x] 1.1 Define Rust data models for `.crypt-env.yaml` with Serde YAML serialization/deserialization (re-exported for the CLI from `commands/scope.rs`) and verify with unit tests.
- [x] 1.2 Implement upward directory search for `.crypt-env.yaml` with graceful fallback and migration notice if legacy `crypt-env.json` is detected.
- [x] 1.3 Verify `.crypt-env.yaml` parser validation and error reporting via `cargo test --bin crypt-env`.

## 2. Project Initialization (`init`)

- [x] 2.1 Implement `crypt-env init [NAME] [--path <PATH>]` subcommand in `src-tauri/src/bin/crypt-env/commands/init.rs` to create (or link) the project in the vault with its root path and generate `.crypt-env.yaml`.
- [x] 2.2 Wire `Cmd::Init` into `main.rs` and handle existing configuration file guard.
- [x] 2.3 Verify `init` command creates expected YAML file structure and vault project record.

## 3. Bidirectional Configuration Sync (`config`)

- [x] 3.1 Implement timestamp comparison logic between `.crypt-env.yaml` mtime and the vault's `max(project.updated, env.updated)`.
- [x] 3.2 Implement pull branch (regenerate YAML, set mtime) and push branch (`POST /projects` + `POST /environments`, vars preserved, WSL path translation) in `commands/config.rs`.
- [x] 3.3 Verify bidirectional sync precedence and edge case handling with unit tests.

## 4. Secret Addition with Collision Prompts (`add`)

- [x] 4.1 Rewrite `commands/add.rs` to check for existing variable keys in the target environment and global scope before saving.
- [x] 4.2 Implement collision warning, confirmation prompt (`[y/N]`), master password prompt, and safe reveal of colliding value.
- [x] 4.3 Verify `add` prevents unintended overwrites and safely reveals colliding secrets only after master password verification.

## 5. Materialization & Template Sync (`fill` & `sync`)

- [x] 5.1 Implement password authentication gating for `commands/fill.rs`, writing each environment's configured targets (via `POST /environments/:id/inject`) and sanitized `.env.example` templates.
- [x] 5.2 Implement password authentication gating for `commands/sync.rs`, parsing `.env.example`, creating template items (`change-me`), and linking/materializing matching global secrets with `--global`.
- [x] 5.3 Verify `fill` and `sync` file generation and password validation using `cargo test`.

## 6. Secure Variable Injection & Search (`inject` & `search`)

- [x] 6.1 Rewrite `commands/inject.rs` to require master password verification and emit shell-safe export strings without terminal log exposure.
- [x] 6.2 Rewrite `commands/search.rs` to require master password verification and support `%substring`/regex patterns and `--global` filtering.
- [x] 6.3 Verify `inject` and `search` security guarantees and output formatting.

## 7. WSL Setup Detection & Legacy Command Removal

- [x] 7.1 Enhance `commands/setup.rs` to auto-detect execution inside WSL vs Windows host, enumerating distros with `wsl -l -q` and prompting when ambiguous.
- [x] 7.2 Delete the `memory`, `list`, `exec`, `cmd`, `project`, `share`, `relay`, `category` and `set` commands entirely (no stubs).
- [x] 7.3 Verify WSL setup resolution and that removed commands are rejected by the parser.

## 8. Comprehensive Diagnostics (`doctor`)

- [x] 8.1 Rewrite `commands/doctor.rs` to run full diagnostic suite: GUI health, vault lock, API, TLS cert validity/expiration, token caches, `.crypt-env.yaml` schema, and WSL integration.
- [x] 8.2 Verify `doctor` output for running/unlocked vault and locked/stopped vault states.

## 9. Interactive TUI Alignment (`tui`)

- [x] 9.1 Rewrite `commands/tui.rs` layout to provide interactive navigation of projects, environments, and masked variables.
- [x] 9.2 Implement modal dialogs for `init`, `config`, `fill`, `sync`, and search inside the TUI.
- [x] 9.3 Implement secure reveal modal (`v`) with password gating and zeroization.
- [x] 9.4 Verify TUI compilation and keyboard navigation with `cargo check --bin crypt-env`.

## 11. Per-Terminal Sliding Sessions (design D4, revised)

- [x] 11.1 Server: replace the single session slot with a bounded session map with sliding expiry (`auto_lock_timeout`); unit tests for coexistence, renewal and expiry.
- [x] 11.2 CLI: terminal identity (`CRYPTENV_TERMINAL_ID`, Unix sid+tty+leader start, Windows console handle), per-terminal token file, stale-file cleanup; gate prompts only without a live session.
- [x] 11.3 TUI: skip password modals while the terminal session is live.
- [x] 11.4 WSL managed launcher exports `CRYPTENV_TERMINAL_ID` via `WSLENV`; test.
- [x] 11.5 Verify end to end: consecutive gated commands prompt once; another terminal prompts; lapse prompts again.

## 10. Documentation & Final Verification

- [x] 10.1 Update public documentation in `docs/index.html`, `docs/cli.md` and `docs/reference.md` reflecting new subcommands (`init`, `config`), updated options, removed/stubbed commands and the project root model.
- [x] 10.2 Run full Rust test suite (`cargo test`) and verify zero compiler warnings.
