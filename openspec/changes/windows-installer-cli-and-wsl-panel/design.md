## Context

See `proposal.md — Why`. Depends on `cli-remote-endpoint-config`, which introduces `CRYPTENV_API_URL` / `CRYPTENV_CERT_PATH` / `CRYPTENV_TOKEN_PATH` and the `crypt-env setup wsl` subcommand (managed `~/.config/cryptenv/env.sh`, marker block `# >>> cryptenv initialize >>>` … `# <<< cryptenv initialize <<<` in `~/.bashrc`/`~/.zshrc`, one-time `.cryptenv.bak`, surgical `--remove`).

Current state relevant here:
- `src-tauri/tauri.conf.json` `bundle.targets = ["nsis","dmg","deb","appimage","app"]`, **no** `externalBin` or `resources`; NSIS uses `installMode: "currentUser"` and `installerHooks: "nsis/installer_hooks.nsi"`.
- `crypt-env` and `crypt-env-mcp` are separate `[[bin]]` targets in `src-tauri/Cargo.toml`; `default-run = "tauri-crypt-env"`.
- Tauri commands are registered in `src-tauri/src/lib.rs` via `tauri::generate_handler![…]`, naming `module_action`. `tauri_plugin_os` is already a plugin.
- Frontend: React 19 + Zustand + TanStack Query, all backend calls via `invoke()`; Tailwind-only.

## Goals / Non-Goals

**Goals:**
- Zero-manual-build: the Windows installer delivers `crypt-env`(.exe) + `crypt-env-mcp`(.exe) on `PATH`.
- One button in Settings takes a Windows user from "installed GUI" to "working `crypt-env` inside distro X", showing exactly what it changed.
- Never edit `.wslconfig`; never copy the cert; never do a destructive shell edit (all delegated to `setup wsl`).

**Non-Goals (design-level):**
- No re-implementation of the shell-edit logic in Rust GUI code — it shells out to the installed `crypt-env setup wsl`.
- No background/daemonized watching of WSL state; detection is on-demand (query on section open + manual refresh).
- No support for WSL1-only quirks beyond "it either has a shell and `/mnt/c` or we report an error".

## Decisions

### D1: Sidecars via `bundle.externalBin`, PATH via the NSIS hook

Add to `tauri.conf.json`:
```jsonc
"bundle": {
  "externalBin": ["binaries/crypt-env", "binaries/crypt-env-mcp"]
}
```
Tauri expects target-triple-suffixed files (`crypt-env-x86_64-pc-windows-msvc.exe`); the release build copies the two `[[bin]]` artifacts into `src-tauri/binaries/` before `tauri build`. On non-Windows, the same `externalBin` entries bundle the Unix builds into the `.app`/`.deb`/AppImage — acceptable and arguably nice, but the *spec* only commits to Windows; to keep other bundles byte-identical we gate the entry with a per-target config or a build script that only populates `binaries/` on Windows. **Chosen:** populate `binaries/` only for the Windows target so macOS/Linux bundles are unchanged (satisfies the "other platforms unchanged" scenario without argument).

**Implementation note:** `externalBin` + the helper `resources` entry live in a separate `src-tauri/tauri.windows-bundle.conf.json` merged only by `pnpm tauri:build:windows` (`tauri build --config ...`), after `scripts/prepare-windows-bundle.mjs` stages the files. Putting them in `tauri.conf.json` (or the auto-merged `tauri.windows.conf.json`) would make `tauri-build` require the sidecars for every Windows `cargo build` — including `pnpm tauri dev` and the prep script's own build of `crypt-env` — a circular dependency. The helper crate is a separate workspace member (`src-tauri/crates/crypt-env-setup`) so its musl build never compiles Tauri/WebView dependencies.

`PATH`: extend `nsis/installer_hooks.nsi` — on install, add the install dir to the **user** `PATH` (`currentUser` install) via the registry `HKCU\Environment` + `WM_SETTINGCHANGE` broadcast; on uninstall, remove exactly that entry. Alternative considered: a `crypt-env.cmd` shim in an already-on-PATH dir — rejected, more surface, still needs cleanup.

### D2: A `wsl` backend module, three `#[cfg(windows)]` commands

`src-tauri/src/wsl/mod.rs`, registered in `lib.rs`:
- `wsl_detect() -> Result<WslStatus, String>`
- `wsl_configure_client(distro: String) -> Result<WslActionReport, String>`
- `wsl_remove_client(distro: String) -> Result<WslActionReport, String>`

Each has a `#[cfg(not(windows))]` sibling (or a single body with `if !cfg!(windows) { return Err(unsupported) }`) returning a typed `WslError::Unsupported` so the frontend contract is identical everywhere.

**`wsl_detect`:**
- `wsl.exe --list --quiet` → UTF-16LE bytes → decode (`encoding_rs` or manual `u16` LE + `String::from_utf16`), strip NULs/CR. Empty/err ⇒ `available:false`.
- For each distro: default user via `wsl.exe -d <name> -e whoami` (arg vector, not a string); `configured` via `wsl.exe -d <name> -e sh -c 'test -f "$HOME/.config/cryptenv/env.sh" && grep -q "cryptenv initialize" "$HOME/.bashrc" 2>/dev/null'` (exit code only).
- Mirrored: read `%USERPROFILE%\.wslconfig` (INI-ish); look for `networkingMode` = `mirrored` under `[wsl2]`. Missing file ⇒ `false`.
- All `Command` invocations use `.arg()/.args()` — never `sh -c "<interpolated>"` on the Windows side.

**`wsl_configure_client` / `wsl_remove_client`:**
- Validate `distro` ∈ current `wsl --list` set; else `WslError::UnknownDistro`.
- Run the installed CLI **inside** the distro so the shell-edit contract is the single source of truth:
  `wsl.exe -d <distro> -e crypt-env setup wsl [--remove]` — but `crypt-env` here is the *Linux* build, which the user may not have yet. **Chosen:** invoke via `/mnt/c/...` path to the bundled Windows `crypt-env.exe`? No — that's the Windows binary, won't run under Linux. So `wsl_configure_client` needs a Linux `crypt-env`. Options:
  1. Require the user to have built/installed the Linux `crypt-env` first (defeats "turnkey").
  2. Ship a small POSIX `setup` shell script as a Tauri **resource**, push it into the distro (`wsl.exe -d <d> -e sh -s < script`), and have *it* implement the exact same marker-block/backup/`env.sh` logic. This duplicates the contract — rejected as the primary, but see D3.
  3. Cross-compile a tiny `crypt-env-setup` Linux helper (musl static) and bundle it as a resource, `cp` it in and run it. One extra build artifact; no webkit deps; reuses the Rust implementation of the contract by sharing the module.
- **Chosen: option 3** — factor the `setup wsl` file logic from `cli-remote-endpoint-config` into a library function; build it into both `crypt-env` (Linux, if present) and a `x86_64-unknown-linux-musl` `crypt-env-setup` helper bundled with the Windows app as a resource. `wsl_configure_client` copies the helper to `\\wsl$\<distro>\tmp\` (or `wsl.exe -d <d> -e cp /mnt/host/... /tmp/...` via the resource path exposed through `/mnt/c`), `chmod +x`, runs it, captures its JSON action report, deletes it.
- The helper writes `env.sh` with `CRYPTENV_API_URL=https://127.0.0.1:47821` and `CRYPTENV_CERT_PATH=/mnt/c/Users/<winuser>/AppData/Roaming/com.maosuarez.cryptenv/tls/cert.pem`, deriving `<winuser>` from `wslpath` of `%APPDATA%` passed in as an arg.

### D3: The shell-edit contract has exactly one implementation

Both `crypt-env setup wsl` (change 1) and the `crypt-env-setup` musl helper (this change) call the **same** `cryptenv_setup::apply(...) / remove(...)` library function. No second copy of the marker/backup/remove logic. The `wsl-integration` spec deliberately references the `cli` capability rather than restating the mechanics.

### D4: Frontend — a gated Settings subview

- `invoke('wsl_detect')` on section mount via TanStack Query; a Zustand slice holds the selected distro.
- Render the section only when `platform === 'windows'` (from `tauri_plugin_os` / an existing platform value) **and** `status.available`.
- Per-distro row: name, default user, Configured / Not configured, `Configure` / `Remove` button. Mutations call the commands, then invalidate the detect query.
- Mirrored-off banner: static snippet + `wsl --shutdown` caveat, copy-to-clipboard, no apply button.
- Post-action panel echoes `WslActionReport { env_file, env_file_changed, rc_files: [...], backups: [...], marker_added, marker_removed }` (the shared `cryptenv_setup::ActionReport`; `backups` is a list because both `~/.bashrc` and `~/.zshrc` can be backed up in one run).
- Tailwind utility classes only; matches the industrial/utilitarian Settings styling.

### D5: Detection freshness

Detect on section open + explicit "Refresh". Configure/Remove validate `distro` against a *fresh* `wsl --list` inside the command (not the cached frontend list) to close the TOCTOU window between UI render and action.

## Risks / Trade-offs

- **Extra build artifact (`crypt-env-setup` musl helper) + release plumbing.** → It's a tiny static bin sharing the library crate; CI adds one `cargo build --target x86_64-unknown-linux-musl -p crypt-env-setup` step. Keeps "one contract implementation" (D3) which is worth it.
- **`wsl.exe` output encoding/format varies by Windows build (UTF-16LE, occasional BOM, localized headers with `--list` without `--quiet`).** → Always use `--quiet`; decode UTF-16LE defensively; treat any parse failure as "could not detect" (typed error, non-fatal UI), never panic.
- **User selects a distro, uninstalls it, then clicks Configure.** → D5 re-validation inside the command; `UnknownDistro` error, no changes.
- **Helper push path:** writing into `\\wsl$\<distro>\...` vs `wsl.exe -e cp` from a `/mnt/c` source — the `/mnt/c` route is more robust across WSL versions and mount configs. → Use `/mnt/c` source path derived from the app's resource dir; fall back to `\\wsl$` only if `/mnt/c` isn't mounted, else error with guidance.
- **PATH broadcast not picked up by already-open shells.** → Expected Windows behavior; the UI report says "open a new terminal". Uninstall reversal removes only the exact entry we added (match on the full install-dir string).
- **`.wslconfig` already has a `[wsl2]` section with other keys.** → We only *read* it; guidance tells the user to add the key under their existing `[wsl2]`. No write, no risk.
- **Antivirus / SmartScreen flags an app that spawns `wsl.exe`.** → Unavoidable for the feature; documented; the spawn only happens on explicit user action in Settings.

## Migration Plan

Additive. New installs get the sidecars automatically; existing installs get them on the next update. Rollback: revert the `tauri.conf.json`/NSIS-hook changes (sidecars stop shipping) and remove the `wsl` module + commands + Settings subview; nothing persisted on the Windows side. Inside a distro, `wsl_remove_client` (or `crypt-env setup wsl --remove`) fully reverses the shell configuration; the `.cryptenv.bak` backup remains for manual recovery.

## Security & Threat Model

- **No new network listener.** The REST API still binds `127.0.0.1:47821` only. This change adds no server surface; it only makes a client reachable and configured.
- **External process execution.** `wsl.exe` is spawned only on explicit user action, always with an argument vector — the user-chosen distro name is a discrete `.arg()`, never concatenated into `sh -c`. The distro name is additionally validated against the live `wsl --list` set before any mutating call.
- **Files written inside the distro** are limited to `~/.config/cryptenv/env.sh`, the marker block in `~/.bashrc`/`~/.zshrc`, and the one-time `.cryptenv.bak` — all via the single shared `cryptenv_setup` contract (D3), which is backup-first, marker-scoped, and range-delete on removal. The pushed helper binary is removed after it runs.
- **TLS trust unchanged.** The generated `env.sh` points `CRYPTENV_CERT_PATH` at the **live** public `cert.pem` over `/mnt/c` (read-only from the distro's view); `key.pem` never leaves `%APPDATA%`. No cert copy, so rotation (~11 months, or 30 days pre-expiry) is picked up automatically.
- **No secret values** are read, logged, or passed as arguments anywhere in this flow. `WslActionReport` contains only file paths and booleans.
- **`.wslconfig` is never written** — mirrored networking is guidance only, because the change requires a full `wsl --shutdown` and affects networking for every distro.
- **Privilege:** NSIS `currentUser` install; the `PATH` edit is `HKCU\Environment` only — no admin, no machine-wide change.

## Cross-Platform Notes

- The `wsl` commands compile on all targets but the non-Windows bodies immediately return `Unsupported`; the Settings section is not rendered off Windows. This keeps the `invoke()` surface identical for the frontend.
- macOS/Linux installers are explicitly unchanged (D1: `binaries/` populated only for the Windows target).
- The musl helper is a Linux artifact bundled *inside the Windows app* as a resource; it is never executed on Windows, only pushed into and run within a WSL distro.

## Open Questions

- Whether to also expose `crypt-env-mcp` wiring (Claude Desktop / MCP client config) from the same panel, or leave that to a later change. Does not affect this change's specs or tasks — deferrable.
