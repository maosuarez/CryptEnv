## Why

Several visual and behavioral inconsistencies currently degrade the desktop user experience in CryptEnv:
1. The LockScreen displays outdated and inaccurate branding/version metadata (`vault v2.0.0 · local only`), despite the application being named CryptEnv and versioned dynamically.
2. The Settings screen displays hardcoded, misleading system information (`vault v2.0.0 · tauri 2.0 · rust 1.77`, `storage: ~/.vault/data.enc · argon2id m=65536 t=3`) that does not reflect actual dynamic runtime versions or the true SQLite application storage path.
3. Navigation back buttons (in Settings, Category Manager, and Edit screens) unconditionally route to Global Secrets (`vault`) rather than the primary landing screen (`projects`) or preserving user navigation history.
4. Low contrast in dark mode makes secondary and muted texts difficult to read, and there is no light theme option for users who require higher contrast or prefer light mode.
5. The application UI is hardcoded in English with no internationalization support for Spanish or Portuguese.

Addressing these issues improves brand identity, clarity, accessibility, and accessibility for international users across English, Spanish, and Portuguese.

## What Changes

- **Accurate Branding & Dynamic App Versioning**:
  - Update LockScreen footer to identify the application as **CryptEnv** with the dynamically retrieved application version (e.g. `CryptEnv v1.0.3 · local only`).
  - Verify and keep the accurate cryptographic descriptors: `AES-256-GCM · Argon2id key derivation` (confirmed as the actual cryptographic primitives in `src-tauri/src/crypto/mod.rs`).
  - Replace the misleading Settings metadata block with clear, dynamically populated system information: dynamic app version (`CryptEnv v{version}`), confirmed crypto stack (`AES-256-GCM · Argon2id`), and actual database storage directory.
- **Contextual & Accurate Back Navigation**:
  - Update the navigation architecture in `useVaultStore` to preserve screen history or return contextually to the previous screen (defaulting to the primary `projects` landing page rather than `vault`).
  - Fix back buttons in Settings, Category Manager, and Edit screens to return to the invoking screen or Projects.
- **Light Theme & Enhanced Contrast**:
  - Refine semantic color tokens (`--color-tx2`, `--color-tx3`, `--color-tx4`, borders) to guarantee WCAG-compliant contrast ratios in dark mode.
  - Implement a Light Theme palette with high readability and clean industrial aesthetic.
  - Add theme selection (System / Dark / Light) in Settings and persist user preference.
- **Full Internationalization (i18n)**:
  - Add an i18n management solution supporting three languages: English (`en`), Spanish (`es`), and Portuguese (`pt`).
  - Externalize all user-facing UI strings (LockScreen, Projects & Environments, Global Secrets, Settings, Item Forms, Category Manager, Modals, Toasts) into structured translation dictionaries.
  - Add language selector (English, Español, Português) exclusively configurable from Settings and persisted across sessions.

## Non-Goals

- Modifying underlying cryptographic algorithms, parameters, or database schema.
- Changing MCP server or CLI command line flags/output formats.
- Adding automatic machine translation or dynamic locale downloads at runtime.
- Exposing the local REST API or secrets over non-localhost networks.

## Capabilities

### New Capabilities
- `desktop-ui`: Covers desktop visual presentation, dynamic version display, accurate technical metadata, contextual back navigation, light/dark theming with enhanced contrast, and multilingual internationalization (English, Spanish, Portuguese) managed via Settings.

### Modified Capabilities
<!-- None: existing specs (app-updater, cli, desktop-packaging, wsl-integration) are unaffected at requirement level. -->

## Security Assessment

- **Zero Plaintext Secrets**: Translations and UI theme modifications do not touch encrypted item payloads or secret values.
- **No Secret Leakage**: Version and diagnostic information exposed in Settings and LockScreen will only include static or non-sensitive system info (app version, crypto algorithm names, local app data directory path); secret keys, tokens, and database contents remain strictly protected.
- **Local Persistence**: User preferences (theme and selected language) are stored safely in local storage / app settings without exposing or altering encrypted vault metadata.

## Impact

- **Frontend**:
  - `src/store/index.ts`: screen navigation history / `goBack`, theme state, language state.
  - `src/components/LockScreen.tsx`: dynamic version, branding CryptEnv, i18n strings.
  - `src/components/Settings.tsx`: theme switch, language selector, corrected dynamic app & system info, proper back navigation.
  - `src/components/CategoryManager.tsx`, `src/components/EditItem.tsx`: contextual back navigation, i18n.
  - `src/index.css`: theme variables (light / dark) and improved contrast tokens.
  - `src/i18n/`: locale definitions (`en.json` / `es.json` / `pt.json`) and i18n hook/provider.
- **Backend**:
  - `src-tauri/src/lib.rs` / `vault`: Tauri command or helper to return app version and app data directory if needed (or standard Tauri `@tauri-apps/api/app`).
