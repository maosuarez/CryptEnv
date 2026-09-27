## Why

The `crypt-env` CLI, TUI, and MCP server are pure HTTP clients of the local REST API, whose base URL (`https://127.0.0.1:47821`) and TLS-cert location are effectively hardcoded. On a headless box — most concretely a WSL distro whose vault actually lives in the Windows-hosted GUI — there is no supported way to point the client at that running vault, and no non-destructive way to persist the pointer in the user's shell. This blocks the "edit projects in WSL, keep secrets in the Windows vault" workflow entirely.

## What Changes

- Introduce three optional environment variables read by the CLI/TUI/MCP client layer, each defaulting to today's exact behavior:
  - `CRYPTENV_API_URL` — overrides the REST base URL.
  - `CRYPTENV_CERT_PATH` — absolute path to the API's TLS cert PEM, checked before the existing `APPDATA` / `XDG_DATA_HOME` / `HOME` probing.
  - `CRYPTENV_TOKEN_PATH` — absolute path to the session-token file; default unchanged (WSL keeps its own token under `~/.local/share`).
- Emit a one-line stderr warning when `CRYPTENV_API_URL`'s host is not a loopback address, restating that the vault is expected to be reachable only over localhost.
- Stop propagating a `set_permissions(0o600)` failure on the session-token file on non-Windows targets (DrvFs / `/mnt/c` paths cannot honor Unix modes); the token write itself still must succeed or error.
- Add a `crypt-env setup wsl` subcommand that writes shell configuration **non-destructively**: a fully-owned, rewritable `~/.config/cryptenv/env.sh` plus a single idempotent marker-delimited block appended to `~/.bashrc` (and `~/.zshrc` when present), with a one-time `~/.bashrc.cryptenv.bak` backup. `--remove` deletes only the marker block and `env.sh`. The command never rewrites or pattern-edits user content.
- Add a documentation guide for the WSL ↔ Windows topology: the networking prerequisite (the client must reach the service as `127.0.0.1` — `networkingMode=mirrored`, or a `portproxy`/`socat` fallback), the data-directory separation between the Windows and WSL clients, and referencing (never copying) the Windows cert so it survives rotation.

## Capabilities

### New Capabilities
- `cli`: Behavior of the `crypt-env` command-line client — how it resolves the vault's REST endpoint, TLS trust anchor, and session-token storage, how it warns on non-loopback endpoints, and how `crypt-env setup wsl` persists that configuration into a user's shell without destroying existing shell content.

### Modified Capabilities
<!-- None: no existing specs. -->

## Non-Goals

- Binding or exposing the REST API on any address other than `127.0.0.1:47821`. The server side is untouched by this change.
- Changing TLS trust: the client keeps pinning the loaded cert with no `danger_accept_invalid_certs` fallback.
- Copying the TLS cert into the WSL filesystem, or otherwise duplicating vault data across the boundary.
- Cross-distro or multi-machine synchronization of the CLI configuration.
- Any GUI, installer, or Windows-side automation for this setup (that is a separate follow-up change).
- A vault-initialization path for the CLI (first-run master-password setup still requires the GUI).

## Impact

- **Code**: `src-tauri/src/bin/crypt-env/client.rs` (endpoint + cert + token path resolution, token-file permission handling), `src-tauri/src/bin/crypt-env/main.rs` and `src-tauri/src/bin/crypt-env/commands/` (new `setup` subcommand), shared client code used by `crypt-env-mcp` and the TUI.
- **Behavior**: New env vars are additive and default to current behavior; no breaking change. New stderr warning path.
- **Filesystem**: `crypt-env setup wsl` creates `~/.config/cryptenv/env.sh`, appends a marked block to shell rc files, writes `~/.bashrc.cryptenv.bak`.
- **Docs**: New topology guide under the project's documentation set; cross-link from `AGENTS.md` CLI reference.
- **Security surface**: No new network listener; the only new outbound trust decision is the user-supplied `CRYPTENV_API_URL`, mitigated by the non-loopback warning. Full review in `design.md`.
