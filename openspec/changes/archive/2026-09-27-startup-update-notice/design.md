## Context

`tauri-plugin-updater` is registered, `check_for_update` stores the found `Update` in `PendingUpdate(Mutex<Option<Update>>)` managed state, and `install_update` takes it and runs `download_and_install`. Only `Settings.tsx` calls them. `App.tsx` already detects the `lock → projects` transition (for the first-run wizard), which is the natural unlock hook.

## Decisions

**D1. Trigger on first unlock, in the frontend.** A `useEffect` on `screen` in `App.tsx` fires when `screen` leaves `'lock'` for the first time, guarded by a flag in a new zustand store (`updateStore`). Both unlock paths (`unlock`, `unlockWithPayload`) end on `screen: 'projects'`, so one hook covers password and biometric.
- *Rejected: check at process start from Rust (`setup`).* Would run while locked and need an event channel to the UI; the user asked for the notice once unlocked, and checking only after unlock keeps a locked app from making network calls.
- *Rejected: check on every unlock.* Auto-lock defaults to 5 min, so this would hit GitHub many times per day for no benefit.

**D2. Dedicated store + component, not the existing toast.** The global toast auto-dismisses after 2.2 s and is `pointer-events-none`, so it can't carry actions. `UpdateNotice` is a fixed, bottom-right card using existing tokens (`bg-raised`, `border-accent-d`, `text-accent`, `font-mono`). State: `{ checked, version, dismissed, installing, installed, error }`.

**D3. Silent failures for the automatic path.** Offline use is normal for a local vault; an error toast on every launch would be noise. Settings' manual check still surfaces errors.

**D4. Reuse the existing commands.** No new Tauri commands or capabilities. `install_update` needs the `Update` stored by the preceding `check_for_update`, which the automatic path provides. If the user also presses the manual check in Settings, it just overwrites the pending update with the same result.

**D5. Unwrap removal.** Map `PoisonError` to `"update state unavailable"` via `map_err`. No typed error enum: both commands already return `Result<_, String>` to the webview; changing the IPC error type is out of scope.

## Security & Threat Model

- **Data exposed**: one HTTPS GET per launch to `github.com/maosuarez/crypt-env/releases/latest/download/latest.json`, revealing IP, user agent and current version. No vault data, secrets or tokens are sent. The request only happens after unlock, so a locked, unattended app stays quiet.
- **Integrity**: `tauri-plugin-updater` verifies the downloaded artifact against the minisign pubkey embedded at build time. A tampered or MITM'd manifest can at most announce a fake version; installing a fake binary fails signature verification. The notice renders the version as text (React escaping), so it is not an injection vector.
- **Key custody**: the private key lives only in the `TAURI_SIGNING_PRIVATE_KEY` GitHub secret (no password). Losing it strands every installed client on its current version, because clients only trust the embedded pubkey. Covered in the release checklist.
- **No new webview capabilities** are granted.

## Cross-platform

- **Windows (NSIS)**: `download_and_install` launches the installer, which closes and relaunches the app. The "restart to apply" message may never be seen; that's fine.
- **macOS / Linux AppImage**: files are replaced in place; the user must restart manually (no `tauri-plugin-process`). `.deb` installs are not covered by `latest.json`; the check finds the Linux AppImage entry, which will fail to install over a `.deb` layout. Accepted: the error is shown in the notice and the user can update through their package manager.

## Risks

- A broken release (empty signatures) would silently prevent updates. Mitigated by the existing workflow check and the checklist's verification step.
