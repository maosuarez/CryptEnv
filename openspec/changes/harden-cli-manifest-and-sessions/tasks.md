## 1. Backend: sessions die with the lock (#3, D6)

- [ ] 1.1 Add `epoch: u64` to `VaultState`, and bump it wherever `key` is set or cleared: unlock (GUI and `/unlock`), `lock_vault`, auto-lock, reset, restore. Verify with `cargo check`, and with a grep showing every `key = ` assignment in `vault/mod.rs`, `api/mod.rs` and `lib.rs` has a matching bump.
- [ ] 1.2 Store the issuing epoch on each `Session`. Make `SessionStore::touch` take the current epoch and drop stale-epoch entries while keeping the constant-time scan. Verify with unit tests in `api/tests/units.rs`: a stale-epoch token is not renewed and is removed, and a same-epoch token is renewed.
- [ ] 1.3 Reorder `verify_token`: read `key.is_some()` and `epoch` under the vault lock, return 403 before touching sessions, then touch. Verify with an API test: session issued → lock → request gets 403 and the expiry is unchanged → unlock → the same token gets 401.

## 2. Backend: contained, no-follow env-file writes (#4, #5, D4, D5)

- [ ] 2.1 Add `fsguard::contain_relative(root, rel)` (canonical root, deepest existing ancestor canonicalized, `starts_with` check). Verify with unit tests covering a symlinked directory escaping the root (unix, `#[cfg(unix)]`), a normal nested path, a not-yet-existing nested directory, and `..`.
- [ ] 2.2 Call `contain_relative` for relative configured paths in `project::resolve_paths`. Verify with `cargo test resolve_env_path` plus a new test showing that a `config -> /tmp/x` symlink directory fails injection and writes nothing.
- [ ] 2.3 Add `envfile::open_nofollow` (unix `O_NOFOLLOW` → `EnvFileError::Symlink`; windows pre-open `symlink_metadata` / reparse-point refusal) and use it in `write_with_mode`. Make `inspect` use `symlink_metadata` and return `Target::Symlink`. Never create a `.bak` from a symlink. Verify with envfile unit tests: symlinked `.env` → `Symlink` error, the link and its target are both unchanged, no `.bak`.
- [ ] 2.4 Map `EnvFileError::Symlink` in `api::err_envfile` (409 with a stable code `TARGET_SYMLINK`) and in the inject preview and GUI confirm list. Verify with `cargo check` and an API test for `/fill` against a symlink target.
- [ ] 2.5 Replace the `target.exists()` step 6 in `fsguard::resolve_within` with a `symlink_metadata`-based check that also rejects dangling symlinks. Verify with a test in `tests/path_containment.rs`: a dangling symlink to outside the base is rejected.
- [ ] 2.6 Move `tempfile` to `[dependencies]` and rewrite `manifest::write_file` with `NamedTempFile::new_in(dir)` + `write_all` + `sync_all` + `persist`. Verify with tests: a planted `.crypt-env.yaml.tmp -> victim` leaves the victim unchanged and writes the manifest; a failure leaves the previous manifest intact and no temp file behind.

## 3. Backend: non-blocking manifest export (#8, D8)

- [ ] 3.1 Split `write_project_manifest` into a locked load step and a `spawn_blocking` fs step. Make `project_write_yaml` drop the `MutexGuard` before the fs step. Verify with `cargo check`, the existing manifest tests, and by reading `project_write_yaml` to confirm no `std::fs` call happens while the guard is held.
- [ ] 3.2 Audit the rest of `project/mod.rs` for `std::fs` calls made under the vault lock outside the inject path, and fix any that exist the same way (or record "none found" in this task). Verify with a grep-based review noted in the task.

## 4. CLI: TUI never prompts on the raw terminal (#2, D3)

- [ ] 4.1 Add `client::NON_INTERACTIVE` plus a single `prompt_password()` helper that returns `CliError::SessionRequired` when the flag is set. Route every `rpassword::prompt_password` call site in `client.rs` through it. Verify with `grep -n rpassword src-tauri/src/bin/crypt-env/client.rs` showing only the helper, plus a unit test for the flag.
- [ ] 4.2 In `tui.rs`, set and clear the flag around the event loop (including the panic and restore path). Route `r`, `c` and `i` confirm through `gate()` with new `Gated::Reload`, `Gated::Config` and `Gated::Init { name }`. Map `SessionRequired` to opening the password modal for the pending action. Pass the full `KeyEvent` to `handle_key`: Ctrl+C quits in every state (including modals), and other Control combinations are ignored rather than treated as plain letters. Verify with `cargo test` for the TUI reducer (key → modal) and a manual check: expired session + `r` shows the modal, and Ctrl+C / `q` exit.

## 5. CLI: root binding and consent (#1, #6, D1, D2)

- [ ] 5.1 Add `scope::check_root_binding` (normalized comparison, case-insensitive for Windows-shaped host paths) and `CliError` text for `Mismatch` naming the bound root and `--relink`. Verify with unit tests for bound, unbound, mismatch, trailing separator, and Windows case.
- [ ] 5.2 Add `RoutingDiff` computation (root change, per-environment added and removed paths, outside-root flag) as a pure function in `config.rs`. Verify with unit tests: metadata-only → empty diff; added path → non-empty; new environment with paths → non-empty; brand-new project → consent not required.
- [ ] 5.3 Wire `config` to use the binding check first (both directions), then run the consent flow for a non-empty diff or `--relink`: print the diff, `ensure_session()`, TTY `y/N` or `--yes`. Add the `--relink` and `--yes` flags to `ConfigArgs`. Verify with tests for `execute` using a mocked confirm or client seam: declined → no `save_*` calls; mismatch without `--relink` → error, no calls.
- [ ] 5.4 TUI `c`: on a non-empty diff open `Modal::Confirm` with the diff, apply through `gate()` on `y`, and show mismatch as an error. Verify with a TUI reducer test and a manual run.
- [ ] 5.5 `init` (CLI and TUI `i`): run the binding check against an existing same-name project; mismatch → error; unbound existing project → consent flow. Verify with unit tests: an existing bound project in another directory is unchanged.
- [ ] 5.6 `fill`: call the binding check before any write, and derive `.env.example` directories from `InjectResult.paths` mapped with `paths::to_local`. Write the example through the no-follow opener. Verify with unit tests: mismatch → error and no files; example directories equal the parents of the written paths.

## 6. CLI: precise pruning (#7, D7)

- [ ] 6.1 Restrict `prune_stale` to `<stem>.<16 lowercase hex>` regular files. Verify by extending `prune_removes_only_old_sibling_token_files` with old `<stem>.bak`, `<stem>.json`, and a symlink named `<stem>.<16hex>`, all of which must survive.

## 7. Docs and verification

- [ ] 7.1 Update `docs/index.html`, `docs/cli.md` and `docs/reference.md`: the `config --relink` / `--yes` flags, the root-binding error, the path-change consent, that locking ends CLI sessions, and the symlink-target refusal (`TARGET_SYMLINK`). Verify by reading the rendered sections and checking the `.ver` pill is unchanged unless a version bump happens.
- [ ] 7.2 Run `cd src-tauri && cargo check && cargo clippy --all-targets && cargo test` with `CARGO_TARGET_DIR` set per CLAUDE.local.md. All must pass.
- [ ] 7.3 Manual Windows verification: `pnpm tauri dev`; lock/unlock in the GUI invalidates a WSL terminal's session; `project_write_yaml` with a stopped-distro UNC root doesn't freeze other GUI actions; `crypt-env.exe config` on a Windows path with different case binds correctly. Record the results in this task.
