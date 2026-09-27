## 1. Backend Diagnostics & System Info Command

- [x] 1.1 Implement `app_get_system_info` Tauri command in `src-tauri/src/vault/mod.rs` and register it in `src-tauri/src/lib.rs` returning dynamic application version, data directory, and OS info. Verify with `cd src-tauri && cargo check`.
- [x] 1.2 Add Rust unit test in `src-tauri` validating that `app_get_system_info` properly returns non-empty version and valid app directory paths. Verify with `cd src-tauri && cargo test`.

## 2. Navigation History & Contextual Back Buttons

- [x] 2.1 Update `src/store/index.ts` to add navigation history tracking (`history: Screen[]`), update `go(screen)` to push history, and add `goBack()` that falls back to `'projects'`. Verify with unit test or frontend store test.
- [x] 2.2 Update `src/components/Settings.tsx` to use `goBack()` instead of hardcoded `go('vault')`. Verify that navigating from Projects to Settings and clicking Back returns to Projects.
- [x] 2.3 Update `src/components/CategoryManager.tsx` and `src/components/EditItem.tsx` to use `goBack()` instead of hardcoded `go('vault')`. Verify that clicking Back returns to the preceding screen.

## 3. Dynamic Versioning & Diagnostic Text Accuracy

- [x] 3.1 Update `src/components/LockScreen.tsx` footer to fetch and display dynamic CryptEnv version (e.g. `CryptEnv v{version} · local only`) and preserve accurate `AES-256-GCM · Argon2id key derivation`. Verify via component rendering and `pnpm build`.
- [x] 3.2 Update `src/components/Settings.tsx` footer to replace the static outdated info (`vault v2.0.0`, `rust 1.77`, `~/.vault/data.enc`) with dynamic CryptEnv version, actual application database directory path, and confirmed cryptographic specifications. Verify info displays accurately.

## 4. Theming, Contrast Enhancement & Light Mode

- [x] 4.1 Update `src/index.css` to enhance text contrast for `--color-tx3` and `--color-tx4` in Dark Theme, and define `[data-theme="light"]` semantic token overrides for backgrounds, surfaces, borders, and typography. Verify styles compile with Vite.
- [x] 4.2 Create theme state management in `src/store/themeStore.ts` (or `src/store/index.ts`) supporting `'dark'` and `'light'` modes, persisting to `localStorage`, and setting `data-theme` on document root. Verify theme toggling in browser console or tests.
- [x] 4.3 Add Theme selector in `src/components/Settings.tsx` allowing users to toggle between Dark and Light mode. Verify that switching theme updates the UI immediately and persists on reload.

## 5. Internationalization (i18n) Infrastructure & Translations

- [x] 5.1 Implement lightweight type-safe i18n module in `src/i18n/` with typed dictionaries, language store (`cryptenv_language`), and `useTranslation` hook supporting English (`en`), Spanish (`es`), and Portuguese (`pt`). Verify interpolation and key resolution with unit tests.
- [x] 5.2 Add Language selector in `src/components/Settings.tsx` (English, Español, Português) and verify changing language persists across app reloads.
- [x] 5.3 Externalize all user-facing strings into `src/i18n/locales/en.json`, `src/i18n/locales/es.json`, and `src/i18n/locales/pt.json` for:
  - `LockScreen.tsx` and `WindowChrome.tsx`
  - `Settings.tsx` (all sections, confirmations, dialogs)
  - `ProjectManager.tsx`, environment editor, injection modals
  - `GlobalSecrets.tsx` and item row context menus
  - `EditItem.tsx` and `CategoryManager.tsx`
  - Modal dialogues (`BackupModal`, `ImportModal`, `ReceiveModal`, `ShareModal`, `SetupWizard`, `UpdateNotice`)
  Verify that switching language immediately updates text across all views without requiring an app restart.

## 6. End-to-End Verification & Quality Gates

- [x] 6.1 Run full frontend tests and type checks (`pnpm test` and `pnpm build`) to ensure all TypeScript types, i18n keys, and UI components pass without errors.
- [x] 6.2 Run full Rust compilation and tests (`cd src-tauri && cargo check && cargo test`) to verify backend integrity.
