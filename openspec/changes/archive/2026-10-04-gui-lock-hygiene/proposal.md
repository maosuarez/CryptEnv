## Why

Locking the vault clears the backend key, but secret material stays visible or reachable in the desktop UI and the OS clipboard:
- **CLI-4:** `lock` and `lockedByBackend` reset `items`, `cats`, `history`, `editTarget` and `menu`, but not `placeholder` (`src/store/index.ts:143-152`). The placeholder modal (z-index 9000), which shows the full command text of a command item, stays on top of the lock screen. The SetupWizard lives in local component state (`src/App.tsx:49,88-96`), is rendered whatever the current screen is, and its Copy button (for the MCP token) keeps working while the vault is "locked".
- **CLI-6:** copied secrets are never cleared from the clipboard. There are 13 `writeText` call sites: `ui/CopyBtn.tsx:19`, `rows/SecretRow.tsx:93`, `ShareModal.tsx:503`, `Settings.tsx:742`, `SetupWizard.tsx:125`, `RelayCodeDisplay.tsx:18,23`, and others. Secret values, relay passphrases and the MCP token stay in the clipboard indefinitely, including after lock. On Windows they also land in clipboard history (Win+V) and can be synced to the cloud.

## What Changes

- **Lock clears all secret-bearing UI state.** On lock (user, auto-lock or backend-initiated), the store resets `placeholder` and every other secret-bearing UI state. A single `resetSecretUi()` is used by `lock`, `lockedByBackend` and `wipe`. Modals and wizards that can show secrets render only when `screen !== 'lock'`, and they close on lock.
- **Secret copies go through the backend.** All secret copies (item values, passwords, relay code and passphrase, MCP token, share passphrases) use a new Tauri command, `clipboard_write_secret(text)`:
  - On Windows it also sets the `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory = 0` and `CanUploadToCloudClipboard = 0` formats, so the secret stays out of Win+V history and cloud sync.
  - The backend clears the clipboard after 30 seconds, and immediately on lock, but only if the clipboard still holds what crypt-env put there. That is checked with the clipboard sequence number on Windows, and by comparing a hash on other platforms.
- **Non-secret copies are unchanged**, for example project names and paths.
- A **lint-style test** fails if `writeText` from `@tauri-apps/plugin-clipboard-manager` is imported anywhere outside the non-secret copy helper.

## Capabilities

### New Capabilities
- `gui-lock-hygiene`: what the desktop UI must clear or hide when the vault locks, and how secrets copied to the clipboard are handled.

### Modified Capabilities
<!-- none -->

## Impact

- **Frontend:** `src/store/index.ts`, `src/App.tsx`, `PlaceholderModal`, `SetupWizard`, `ShareModal`, `ReceiveModal`, `RelayCodeDisplay`, `Settings`, `CopyBtn`, `SecretRow`, and a new `src/lib/clipboard.ts` (`copySecret` / `copyPlain`).
- **Backend:** a new `clipboard.rs` with the Tauri command `clipboard_write_secret`, clear timer, and lock hook (`vault::lock_vault` → `clipboard::clear_if_ours`). Windows formats use the `windows` crate `Win32_System_DataExchange` feature.
- **Docs:** a note in the security section of `docs/index.html` (30-second clipboard clear, excluded from history).
- LAN share cancellation on lock is handled in `lan-share-hardening`.

## Non-Goals

- Clearing clipboard managers that have already captured the content before the exclusion formats were honored, or third-party managers that ignore those formats.
- Making the 30-second duration configurable. It can be added later.
