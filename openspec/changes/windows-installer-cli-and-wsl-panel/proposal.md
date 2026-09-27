## Why

After `cli-remote-endpoint-config` lands, the WSL ↔ Windows workflow still requires the user to build the CLI themselves, hand-write env vars, know that `networkingMode=mirrored` is needed, and locate the Windows cert path by hand. The Windows NSIS installer ships only the GUI today (`tauri.conf.json` has no `externalBin`/`resources`). This change makes the setup turnkey: the installer provides the CLI, and the GUI detects WSL and configures the client for a chosen distro with one click — with no destructive edits and no understanding gaps.

## What Changes

- Bundle `crypt-env.exe` and `crypt-env-mcp.exe` with the Windows installer via `externalBin` in `tauri.conf.json`, and place them on the user's `PATH` (NSIS install step). The GUI-only bundle behavior on macOS/Linux is unchanged.
- Add a Settings section **"WSL Integration"**, rendered only when the app runs on Windows and only when at least one WSL distro is detected.
- Add three Windows-only Tauri commands (`module_action` naming, `#[cfg(windows)]` plus a runtime guard that returns a typed error elsewhere):
  - `wsl_detect` — reports whether WSL is available, the list of distros with their default Linux user and whether the cryptenv client block is already installed, and whether `networkingMode=mirrored` is configured.
  - `wsl_configure_client` — for a named distro, runs the **same** non-destructive shell configuration defined by `cli-remote-endpoint-config` (`crypt-env setup wsl`) inside that distro, and reports exactly what it created or changed.
  - `wsl_remove_client` — reverses it for a named distro.
- Detect mirrored networking and, when it is off, show the `.wslconfig` snippet and the rationale (requires `wsl --shutdown`, affects all WSL networking). The app MUST NOT edit `.wslconfig` automatically.
- The configuration references the live Windows cert path over `/mnt/c` and never copies the cert, so it survives certificate rotation.
- Settings flow: distro dropdown + per-distro status → "Configure" → a report of the actions taken (created `env.sh`, N lines added to `~/.bashrc` between markers, backup path) → "Remove".

## Capabilities

### New Capabilities
- `desktop-packaging`: What the platform installers deliver alongside the GUI — specifically that the Windows installer also installs the `crypt-env` and `crypt-env-mcp` executables and makes them invokable from a shell.
- `wsl-integration`: The Windows-only GUI capability that detects WSL, reports its readiness for the split-client workflow, and configures or removes the `crypt-env` client inside a chosen distro non-destructively, delegating the actual shell edits to the `cli` capability's `setup wsl` contract.

### Modified Capabilities
<!-- None. `cli-remote-endpoint-config` owns the `cli` capability and the setup-wsl contract this change consumes. -->

## Non-Goals

- Re-specifying the shell-edit mechanics (marker block, backup, `env.sh`, `--remove`) — that contract lives in the `cli` capability from `cli-remote-endpoint-config` and is reused verbatim.
- Editing `%USERPROFILE%\.wslconfig` automatically, or restarting WSL for the user.
- Binding or exposing the REST API on anything other than `127.0.0.1:47821`.
- Copying the TLS cert into the distro, or syncing configuration across distros/machines.
- Any non-Windows behavior for the WSL panel (it is hidden and its commands are inert off Windows).
- Installing or bootstrapping WSL itself.

## Impact

- **Build/packaging**: `src-tauri/tauri.conf.json` (`bundle.externalBin`), NSIS installer hook (`nsis/installer_hooks.nsi` / `installer_hooks.nsi`) for `PATH`; `crypt-env` / `crypt-env-mcp` become bundled sidecars and must be built for the Windows target in CI/release.
- **Backend**: new `wsl` module + three commands registered in `src-tauri/src/lib.rs`; invokes `wsl.exe` via argument vectors (never string-interpolated), decodes its UTF-16LE output, reads `%USERPROFILE%\.wslconfig`. Embeds/writes the `setup wsl` invocation.
- **Frontend**: new Settings subview (Zustand slice + TanStack Query calls via `invoke()`), Windows + WSL-detected gating, Tailwind-only styling.
- **Dependency on** `cli-remote-endpoint-config`: consumes its env-var names and `crypt-env setup wsl` semantics; should be implemented and archived after it.
- **Security surface**: launches an external process (`wsl.exe`) with a user-selected distro name, and reads/writes files inside the distro. No new network listener. Full review in `design.md`.
