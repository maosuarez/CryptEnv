## Why

A code review of `bc5f834` (CLI/TUI parity, per-terminal sessions, `.crypt-env.yaml` manifest, project roots) found 8 defects. The worst ones let a cloned repository redirect where decrypted secrets get written, and can hang the TUI. All 8 are in code that already shipped in v1.0.6. They share one root cause: files from a repository are treated as trusted input for writes that carry secrets.

| # | Sev | Where | Defect |
|---|-----|-------|--------|
| 1 | high | `bin/crypt-env/commands/config.rs:78` | `config` pushes any repo's manifest over a same-named vault project. A fresh clone is always "newer", so it rewrites `rootPath` and env paths to attacker targets. |
| 2 | high | `bin/crypt-env/commands/tui.rs:343,354` | TUI `r`/`c`/`i` skip `gate()`. An expired session makes the client call `rpassword` while the terminal is in raw mode, which hangs the TUI (no Enter, no Ctrl+C). |
| 3 | medium | `api/mod.rs:240`, `bin/crypt-env/client.rs:501` | `verify_token` renews the session before checking the lock state. Sessions also survive a lock, so a lock/unlock in the GUI silently revives every terminal's session. |
| 4 | medium | `project/mod.rs:387`, `envfile/mod.rs:233` | Configured paths are only checked lexically, and writes follow symlinks. A repo's `.env -> /tmp/leak` receives plaintext secrets. |
| 5 | medium | `project/manifest.rs:98` | The fixed `.crypt-env.yaml.tmp` is written through `std::fs::write`, which follows symlinks. A planted symlink overwrites any file the user can write. |
| 6 | low | `bin/crypt-env/commands/fill.rs:66` | `.env` goes to the vault root and `.env.example` to the local root. A second checkout gets its secrets written into the other checkout, and the command reports success. |
| 7 | low | `bin/crypt-env/terminal.rs:93` | `prune_stale` deletes any `<base>.*` sibling older than a day, not just per-terminal token files. |
| 8 | low | `project/mod.rs:1026` | `project_write_yaml` does blocking filesystem I/O while holding the global vault mutex. A stalled `\\wsl.localhost` root freezes every GUI and REST call. |

## What Changes

- **Project root binding (#1, #6).** A vault project with a root belongs to that directory. `config` and `fill` refuse to run from a different directory unless the user explicitly relinks the project with `crypt-env config --relink`. **BREAKING** for users who deliberately used one project from two checkouts.
- **Consent for secret-routing changes (#1).** A `config` push that changes an existing project's root or any environment's target paths shows a diff. It needs a live password session and an explicit confirmation (`y`, or `--yes` when not interactive). Changes to name, description and categories push without asking, as they do today. **BREAKING** for scripted `config` runs that change paths.
- **`.env.example` next to the real `.env` (#6).** `fill` derives the `.env.example` directories from the paths the vault actually wrote, not from the local root.
- **No hidden prompts in the TUI (#2).** Every TUI action that reaches the vault goes through the password modal. The client never calls a terminal prompt while the TUI owns the terminal.
- **Sessions die with the lock (#3).** Locking the vault (manually, by auto-lock, or by reset/restore) invalidates every CLI session. A locked vault never renews a session.
- **Symlink-safe, contained env-file writes (#4).** Relative configured paths must resolve, after following symlinks, inside the canonical project root. No env-file write, whether to a relative or an absolute path, may go through a symlink at the final path component.
- **Safe manifest writes (#5).** `.crypt-env.yaml` is written through a uniquely named, exclusively created temp file and then renamed. Planted temp paths are never followed.
- **Precise token pruning (#7).** Only files named `<base>.<16 lowercase hex>` are pruned, and only regular files.
- **Non-blocking manifest export (#8).** The GUI's `project_write_yaml` reads vault state under the lock, then releases it before doing any filesystem I/O on a blocking thread.

## Capabilities

### New Capabilities
- `env-file-writes`: invariants for every filesystem write crypt-env makes into a project tree (`.env` targets, `.env.example`, `.crypt-env.yaml`). Covers containment, symlink handling, atomicity, and not blocking the vault during filesystem I/O.

### Modified Capabilities
- `cli`: *Project initialization via init* refuses to take over a project bound elsewhere. *Bidirectional project configuration sync via config* gains root binding, relink and consent for path changes. *Environment and example file generation via fill* gains the root-binding check and moves `.env.example` next to the real `.env`. *Per-terminal password sessions* makes sessions invalid on vault lock and prunes only token files.
- `tui`: *In-TUI project actions and configuration sync* requires every vault-reaching action (reload, init, config, fill, sync) to authenticate through the modal and never through a terminal prompt.

## Impact

- **Rust backend:** `api/mod.rs` (session store, `verify_token`), `vault/mod.rs` (lock epoch), `project/mod.rs` (`resolve_env_path`, `write_project_manifest`, `project_write_yaml`), `project/manifest.rs` (`write_file`), `envfile/mod.rs` (no-follow writes).
- **CLI/TUI:** `bin/crypt-env/commands/{config,fill,tui,scope}.rs`, `bin/crypt-env/client.rs` (non-interactive mode), `bin/crypt-env/terminal.rs`.
- **Dependencies:** `tempfile` moves from dev-dependencies to dependencies for the atomic manifest write. It is already in the lockfile.
- **REST contract:** no request or response shape changes. The only new observable behavior is that session tokens return 403 after a lock and then 401 once the vault is unlocked again.
- **Docs:** `docs/index.html`, `docs/cli.md` and `docs/reference.md` get the `config --relink` / `--yes` flags, the root-binding error, the path-change confirmation, and the fact that locking ends CLI sessions.
- **Security:** this change closes a path from a cloned repo to plaintext secrets being written to an attacker-chosen location (#1, #4, #5), and a way for a session to outlive the lock (#3). No crypto, key or storage format changes. No database migration is needed; the lock epoch is in-memory only.

## Non-Goals

- Server-side (REST) authorization of `rootPath` or path changes made by other authenticated clients, such as MCP-token holders. That belongs in the whole-codebase audit follow-up.
- Signing or pinning `.crypt-env.yaml` contents.
- Changing the "last modified wins" rule for non-path metadata.
- Windows ACL hardening of written `.env` files. The existing inherited-ACL behavior stays.
- Any GUI UI change beyond `project_write_yaml` no longer blocking the vault.
