## 1. Backend: Vault Session Touch, Auto-Lock Event, and Initial Environment

- [x] 1.1 Implement `vault_touch` command in `src-tauri/src/vault/mod.rs` to refresh `s.last_activity` when the vault is unlocked, and register it in `src-tauri/src/lib.rs`. Verify with `cargo check` in `src-tauri`.
- [x] 1.2 Emit `vault_locked` event to all webviews when the background auto-lock timer fires in `src-tauri/src/lib.rs`. Verify with `cargo check` in `src-tauri`.
- [x] 1.3 Add optional `initial_environment` field to `ProjectInput` in `src-tauri/src/project/mod.rs` and update `save_project` to validate and use it when creating a new project. Verify with `cargo test project` in `src-tauri`.

## 2. Frontend Session Synchronization & Error Recovery

- [x] 2.1 Update `src/hooks/useAutoLock.ts` to throttle calls to `vault_touch` on user interaction events (`mousemove`, `keydown`, `mousedown`, `touchstart`), keeping the backend activity timestamp fresh. Verify with `pnpm tsc --noEmit`.
- [x] 2.2 Add event listener for `vault_locked` in `src/App.tsx` (or `src/store/index.ts`) that immediately resets in-memory vault state and sets `screen: 'lock'`. Verify with `pnpm tsc --noEmit`.
- [x] 2.3 Add error handling in `src/components/ProjectManager.tsx` for `"vault is locked"` errors, presenting an informative toast and navigating to the lock screen. Verify with `pnpm tsc --noEmit`.

## 3. TagInput & Inline Category Creation

- [x] 3.1 Extend `src/components/ui/TagInput.tsx` to support inline creation of new categories with a name input and add trigger. Verify component renders and emits creation callback.
- [x] 3.2 Wire inline category creation in `src/components/ProjectManager.tsx`, saving new categories to the vault via `vault_save_categories` and immediately selecting them for the project. Verify by checking category persistence in `useVaultStore`.

## 4. Customizable Initial Environment & Capitalized Presets

- [x] 4.1 Update `src/store/projectStore.ts` and `src/components/ProjectManager.tsx` to include an initial environment name input (defaulting to "default") in the project creation form. Verify with `pnpm tsc --noEmit`.
- [x] 4.2 Show capitalized, localized labels for `ENV_PRESETS` ("Production", "Local", "Test", "Staging"; values stay lowercase — see design §5) in `src/components/ProjectManager.tsx` and add localized preset strings to `src/i18n/locales/en.json`, `es.json`, and `pt.json`. Verify presets render with initial capitalization across locales.

## 5. Prominent Key Name Styling & Header Alignment

- [x] 5.1 Restyle `KEY_NAME` in `AddVarPanel` with high-contrast accent green borders (`border-accent/60 focus-within:border-accent ring-1 ring-accent/20`), an explicit label badge, and clear placeholder indicating it is the required environment variable export name. Verify with `pnpm tsc --noEmit`.
- [x] 5.2 Update the environment detail header in `src/components/ProjectManager.tsx` using absolute centering (`absolute inset-0 flex items-center justify-center pointer-events-none`) to strictly center the environment title across the full header area. Verify header alignment layout with varying back button widths.

## 6. Verification & Test Suite

- [x] 6.1 Run `cargo test` in `src-tauri` to ensure all Rust backend tests pass.
- [x] 6.2 Run `pnpm test` and `pnpm tsc --noEmit` to verify all frontend tests and type checks pass.
