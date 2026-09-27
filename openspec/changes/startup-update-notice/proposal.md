## Why

The updater plumbing works (`check_for_update` / `install_update` in `src-tauri/src/lib.rs`, signed `latest.json` in `release.yml`), but updates are only discovered when the user opens Settings and presses the check button. In practice nobody does, so installed copies drift behind. The two updater commands also call `Mutex::lock().unwrap()`, which the project rules forbid in production code.

## What Changes

- After the **first successful unlock of each app launch**, the frontend calls `check_for_update` once in the background.
- When a newer version exists, a persistent in-app notice appears. It shows the version and offers **INSTALL** (calls `install_update`) and **LATER** (hides the notice for the rest of this launch).
- Update-check failures (offline, GitHub unreachable, invalid manifest) are silent. The notice never appears on the lock screen.
- Replace `lock().unwrap()` in `check_for_update` / `install_update` with `Result`-based error mapping.
- Release workflow: verify every updater signature against the `tauri.conf.json` pubkey before publishing (`scripts/verify-updater-sig.mjs`), and make manual runs build and publish the requested tag instead of the dispatch branch.
- Add a release checklist (`docs/release.md`) covering signing secrets, version bump, tagging and end-to-end update verification.
- Ships with the next deployed version (1.0.2).

## Capabilities

### New Capabilities
- `app-updater`: how the desktop app discovers, announces and installs signed updates.

### Modified Capabilities
<!-- None: no existing living specs. -->

## Non-Goals

- A setting to disable automatic checks, or a configurable check interval (follow-up if requested).
- Persisting "skip this version" across launches.
- Periodic re-checks while the app stays open.
- Automatic relaunch after install on macOS/Linux (would require `tauri-plugin-process`, a new webview capability).
- Renaming the existing commands to the `module_action` convention (would break the Settings call sites; separate cleanup).
- Update checks from the CLI/TUI/MCP binaries.

## Impact

- **Code**: `src-tauri/src/lib.rs` (unwrap removal), `src/store/updateStore.ts` (new), `src/components/ui/UpdateNotice.tsx` (new), `src/App.tsx` (trigger + mount).
- **CI**: `.github/workflows/release.yml`, new `scripts/verify-updater-sig.mjs`.
- **Docs**: new `docs/release.md`.
- **Security**: no secret data involved. The check sends one HTTPS GET to the GitHub releases endpoint already configured in `tauri.conf.json`; it now happens automatically after unlock instead of on demand, which reveals the user's IP and app version to GitHub once per launch. Update payloads remain verified against the minisign pubkey by `tauri-plugin-updater`. Details in `design.md`.
