## Context

The frontend copies text with `writeText` from the Tauri clipboard plugin. Store state is managed with Zustand. The backend has `lock_vault` as the single lock path (the auto-lock task, the command, and wipe).

## Decisions

### D1. `resetSecretUi()` in the store
A single function resets `placeholder`, `editTarget`, `menu`, `items`, `cats`, `history`, `revealed*` and share/relay dialog flags. `lock`, `lockedByBackend` and `wipe` call it. The SetupWizard's open flag moves into the store (`wizardOpen`), so the lock can close it. `App.tsx` renders the overlays only when `screen !== 'lock'`.

### D2. Backend clipboard ownership
`clipboard_write_secret(text: SecretString)`:
- **Windows:** `OpenClipboard` → `EmptyClipboard` → `SetClipboardData(CF_UNICODETEXT)` plus registered formats `ExcludeClipboardContentFromMonitorProcessing` (empty), `CanIncludeInClipboardHistory` (DWORD 0) and `CanUploadToCloudClipboard` (DWORD 0) → `CloseClipboard`. Then store `GetClipboardSequenceNumber()`.
- **Other OSes:** write through the plugin's Rust API, and store SHA-256 of the text (not the text).

A `tokio` task sleeps 30 s, then calls `clear_if_ours()`:
- on Windows it clears if the sequence number is unchanged;
- elsewhere it reads the clipboard text, hashes it, compares, and clears on a match.

`lock_vault` calls `clear_if_ours()` as well. Only the latest copy is tracked; a new secret copy replaces the pending one.

*Rejected:* clearing from JS timers. The webview may be suspended or hidden, and the timer would not run on backend-initiated locks without the UI.

### D3. Copy helper split and guard
`src/lib/clipboard.ts` exports `copySecret(text)` (invokes the command) and `copyPlain(text)` (plugin `writeText`, for non-secrets). A test (`vitest` or a node script in `pnpm test`) greps `src/` for imports of `writeText` outside `lib/clipboard.ts` and fails if it finds any.

## Security & Threat Model

- This shrinks the exposure of copied secrets from "indefinite, in history, cloud-synced" to at most 30 seconds, and none after lock.
- Lock now actually hides secrets on screen (shoulder surfing after auto-lock).
- **Residual:** third-party clipboard managers that ignore the exclusion formats; a note in the docs.

## Risks / Trade-offs

- [Linux Wayland clipboard reads may need focus] → `clear_if_ours` falls back to unconditional clear only if the read fails *and* no other copy was observed (best effort). This is documented.
- [Users relying on paste after 30 s] → A toast says "clipboard clears in 30 s".
