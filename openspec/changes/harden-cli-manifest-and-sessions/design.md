## Context

See proposal.md for the 8 findings. The relevant current state:

- `config::execute` (CLI) decides push or pull purely by mtime and matches projects by name. `push()` unconditionally sends `rootPath` and every environment's `paths` through `POST /projects` and `POST /environments`.
- The REST session store (`api::SessionStore`) lives in `ApiState`, separately from the vault's `SharedState`. `vault::lock_vault` sets `key = None` but has no handle on the session store, so sessions survive a lock.
- `project::resolve_env_path` is a pure string function. `envfile::write_with_mode` opens with `create + truncate`, which follows symlinks. `fsguard::resolve_within` already canonicalizes, but only for single-component file names under `output_dir`.
- The CLI client functions (`authenticated_get`, `get_auth_token`, the `save_*` helpers) fall back to `rpassword::prompt_password` on 401. The TUI calls several of them directly.
- `tempfile` is already a dev-dependency.

## Goals / Non-Goals

**Goals:** close all 8 findings at their root cause. Make the behavior visible to the user only where consent is the point (path and root changes). Keep the REST request and response shapes unchanged.

**Non-Goals:** server-side authorization of path changes. The API trusts any authenticated client, and the MCP-token capability split is tracked by the whole-codebase audit. No change to how GUI-initiated path edits work, since those come from the vault owner directly.

## Decisions

### D1. Root binding and consent are enforced in the CLI, keyed on the vault project's root (#1, #6)

A new helper, `scope::check_root_binding(ws_root, &Project) -> Result<Binding, CliError>`, compares `paths::to_host(ws_root)` with `project.root_path`. It normalizes both through `validate_root_path` and compares case-insensitively when the host path is a Windows path (drive letter or UNC). It returns `Bound`, `Unbound`, or `Mismatch(bound_root)`. `config` (both directions) and `fill` call it first. `Mismatch` becomes a `CliError::Config` naming the bound root and suggesting `config --relink`.

`config` push computes a `RoutingDiff { root: Option<(Option<String>, String)>, envs: Vec<(name, added, removed)> }` between the manifest and the vault project. Absolute paths outside the root are flagged by reusing `manifest::relativize`. A non-empty diff (or `--relink`) triggers the consent flow:
1. Print the diff to stderr.
2. Call `client::ensure_session()` (password prompt when there is no session).
3. Confirm: `y/N` on a TTY stdin; otherwise require `--yes`.

Only after that are the `save_*` calls made. Creating a brand-new project (`find_project` → `None`) stays consent-free, because the new project holds no secrets yet.

*Alternatives rejected:*
- **Enforce in the REST API:** stronger, but the API cannot tell a user's own edit from a manifest-driven one. It also needs the MCP capability model first, and it would change the API contract. That is deferred to the audit follow-up.
- **Track "trusted manifest" hashes per directory:** more state and a new storage format, and it still needs a consent step. Root binding reuses data we already have.
- **Silently pull instead of push on mismatch:** would write vault paths into a foreign checkout's file, which surprises the user and leaks the vault's layout.

The TUI's `c` action uses the same `RoutingDiff`. A non-empty diff opens a `Modal::Confirm { diff, then: Gated::ConfigPush }` that goes through `gate()`. `Mismatch` is shown as an error with no relink option: relinking stays a deliberate shell command.

### D2. `.env.example` placement follows the actual writes (#6)

`fill::execute` builds the example directories from `InjectResult.paths` (host paths), mapped back with `paths::to_local`, instead of from `local_target(ws.root, configured)`. Together with D1, the example always lands next to a `.env` that was just written. The example itself is written through the D4 no-follow writer.

### D3. The TUI never lets the client prompt (#2)

The client gets a process-wide `static NON_INTERACTIVE: AtomicBool`, set by `tui::run` on entry and cleared on exit, including through the terminal-restore guard. Every `rpassword::prompt_password` call site in `client.rs` goes through one `prompt_password()` helper. When the flag is set, that helper returns a new `CliError::SessionRequired` instead of prompting. In the TUI, `r`, `c` and `i` route through `gate()` like `f` and `s` do. `app.error()` maps `SessionRequired` to opening the password modal for the pending action, which covers a session expiring mid-action.

*Alternative rejected:* leaving raw mode around each prompt. That is fragile (interleaves with rendering) and still lets the prompt garble the alternate screen.

### D4. Contained, no-follow env-file writes (#4)

- **Containment.** `resolve_env_path` stays lexical and pure. A new `fsguard::contain_relative(root, rel) -> Result<PathBuf, ContainmentError>` canonicalizes the root and walks up to the deepest existing ancestor of the joined path. It canonicalizes that ancestor and requires `starts_with(real_root)`, then returns `real_ancestor.join(remaining components)`. `resolve_paths` in `project/mod.rs` calls it for relative configured paths (absolute paths stay grandfathered for location).
- **No-follow open.** `envfile::write_with_mode` and the `.env.example` writer use a shared `envfile::open_nofollow(path, mode)`.
  - Unix: `OpenOptions::custom_flags(libc::O_NOFOLLOW)`, which returns `ELOOP` on a symlink and is mapped to a new `EnvFileError::Symlink(path)`.
  - Windows: `symlink_metadata` immediately before the open. A symlink or `FILE_ATTRIBUTE_REPARSE_POINT` is refused. This leaves a residual TOCTOU window on Windows, which is accepted (see Risks).
  - `inspect` also uses `symlink_metadata` and reports symlinks as a new `Target::Symlink`, so the preview and the GUI confirm dialog can show them. The `.bak` copy is never made from a symlink.
- The `fsguard::resolve_within` step-6 `exists()` check (which follows symlinks) is replaced by `symlink_metadata`, because the same no-follow rule applies to `output_dir` targets. This is also audit finding A-3.

### D5. Manifest write via `tempfile::NamedTempFile::new_in(dir)` (#5)

`manifest::write_file` creates the temp file with `NamedTempFile::new_in(dir)`, which uses a random name and `O_EXCL` / `CREATE_NEW`. It then does `write_all`, `sync_all`, and `persist(target)`. On error, the temp file is removed on drop. `tempfile` moves to `[dependencies]`; it is already resolved in `Cargo.lock`, so no new crate is added. `persist` uses `rename` / `MoveFileEx(REPLACE_EXISTING)`, which replaces a symlink at `.crypt-env.yaml` without following it.

### D6. Lock epoch invalidates sessions (#3)

`VaultState` gets `pub epoch: u64`. It is incremented in every place that sets `key` (unlock, `lock_vault`, reset, restore). Each `Session` records the epoch at which it was issued.

`verify_token` order:
1. lock `vault` → if `key.is_none()` return 403 *without touching sessions*, and read `epoch`; release.
2. `sessions.touch(provided, now, epoch)`. A match only counts if `e.epoch == epoch`. Entries with a stale epoch are dropped during the scan (the scan stays constant-time over the entries).
3. MCP-token fallback, unchanged.

After an unlock, old tokens therefore get 401. The client clears the token and prompts, which is the existing behavior. `session_alive`'s 403 → `VaultLocked` mapping stays.

*Alternative rejected:* giving `vault::lock_vault` a handle on the session store. That breaks the "vault doesn't know about api" boundary; the epoch keeps api dependent on vault only. The lock order is unchanged: `vault` is always taken and released before `sessions`, never nested.

### D7. Precise pruning (#7)

`prune_stale` matches `name == format!("{stem}.{hex}")` where `hex` is exactly 16 chars of `[0-9a-f]`. It checks `entry.file_type()?.is_file()`, which does not follow symlinks, before removing.

### D8. `project_write_yaml` off the lock and off the runtime (#8)

`write_project_manifest` is split into:
- `load_manifest_source(db, id) -> (Project, root)`, which runs under the vault lock;
- a pure-fs `write_manifest_at(root, &Project, overwrite)`, which runs in `tokio::task::spawn_blocking` after the `MutexGuard` is dropped.

This matches `project_check_root`. Any other `std::fs` call made under the vault lock in `project/mod.rs` gets the same treatment if the apply phase finds one; the inject path is out of scope.

## Security & Threat Model

| Threat | Before | After |
|---|---|---|
| Hostile repo manifest names a victim project | Push rewrites root and paths, and the next fill writes secrets to attacker paths | Root mismatch fails; any path change needs a password plus explicit `y` |
| Hostile repo `.env` symlink or symlinked directory | Secrets written through the link | Contained resolution plus no-follow open; the write is refused |
| Hostile `.crypt-env.yaml.tmp` symlink | Arbitrary user file overwritten | Random `O_EXCL` temp file; planted names are never opened |
| Session outlives lock | Old session revives after the next unlock | Epoch mismatch → 401 |
| TUI hang (availability) | Unkillable TUI | No raw-terminal prompts; modal only |
| Over-eager pruning (integrity) | Arbitrary `<base>.*` deleted | Only `<base>.<16hex>` regular files |

No secret value is added to any error message. New errors carry paths and project names only. There are no crypto or key changes. The epoch is a non-secret counter.

## Cross-platform

- `O_NOFOLLOW` is Unix-only. The Windows path uses a pre-open `symlink_metadata` check, which is weaker (see Risks).
- Root comparison ignores case for Windows-shaped host paths. WSL paths are compared after `to_host`.
- `NamedTempFile::persist` works on Windows including over UNC `\\wsl.localhost` paths (MoveFileEx).

## Risks / Trade-offs

- [Windows TOCTOU between the symlink check and open] → Creating a symlink on Windows needs admin rights or Developer Mode, which narrows the attacker. A follow-up could use `FILE_FLAG_OPEN_REPARSE_POINT` through `windows` crate `CreateFileW`.
- [BREAKING: two checkouts sharing one project now need `--relink` each time they switch] → The error message names the command. Documented in `docs/cli.md`.
- [BREAKING: scripted `config` with path changes now needs `--yes` and a live session] → Documented. Non-path pushes are unaffected.
- [Users who deliberately symlink `.env` into a shared location (for example, a dotfiles setup) will get a refusal] → The error explains why. They can configure the real absolute path instead.
- [Locking now forces every terminal to re-enter the password after the next unlock] → This is the intended security property, and it is documented.

## Migration Plan

No DB migration. Existing projects without a root become `Unbound`: the first path-changing `config` push needs consent and then binds them. Rollback means reverting the commit; nothing persistent changes format.
