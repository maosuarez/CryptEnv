## Why

CryptEnv's settings and project management interfaces have several usability and cross-platform issues that degrade the user experience:
1. Native `<select>` dropdowns (including auto-lock timeout and language) render with unstyled white backgrounds and low contrast in dark mode on desktop webviews.
2. Global shortcut handling is buggy and hardcoded: changes saved in Settings do not take effect at runtime because Tauri does not re-register the shortcut, hotkey capture closes the window if the current shortcut is pressed, there is no validation or reset option, and shortcuts are not adapted for macOS (`Cmd` vs `Ctrl`).
3. In Project Management, the project categories field is ambiguously labeled "tags", the project save button is placed inconsistently in the middle of the form and lacks dirty-state awareness, the "Templates" row remains displayed in existing projects even though templates are one-time seed mechanisms, and the environment presets display technical lowercase names as primary instead of localized, capitalized labels while preserving lowercase `.env.*` filenames under the hood.

Fixing these issues streamlines daily navigation, eliminates confusing visual quirks, and ensures robust cross-platform shortcut control.

## What Changes

- **Dropdown Contrast in Dark Mode**: Ensure all `<select>` and `<option>` elements (e.g., auto-lock timeout, language, import selectors) render with dark background tokens (`bg-surface` / `bg-raised`), proper text contrast (`text-tx`), and clear focus rings in dark mode.
- **Cross-Platform Global Hotkey & Runtime Registration**:
  - Support cross-platform modifiers: support `CommandOrControl` (or `Cmd` on macOS, `Ctrl` on Windows/Linux) and display platform-native keys in the UI.
  - Dynamically re-register the global shortcut via `tauri-plugin-global-shortcut` inside `vault_save_settings` so shortcut changes apply immediately without restarting.
  - Load the stored hotkey from SQLite settings on app startup in `src-tauri/src/lib.rs` instead of hardcoding `Ctrl+Alt+Z`.
  - Safe hotkey capture in UI: prevent the global shortcut from triggering and hiding the window while recording a new shortcut.
  - Hotkey validation & reset: require at least one modifier key (`Ctrl`/`Cmd`, `Alt`) plus a standard key, and provide a "Reset to default" action.
- **Project Categories Terminology**:
  - Rename the project "tags" field and associated UI text across all locales (`en`, `es`, `pt`) to "categories" / "Categorías", explicitly aligning with the vault's category management.
- **Project Settings Action Bar & Dirty State**:
  - Move the project Save button out of the middle form section down to the footer action bar to the left of "Delete Project".
  - Make the Save button visually prominent with green accent styling (`bg-accent text-[#020504]`).
  - Render the Save button conditionally only when project configuration changes have been made (`dirty` state).
- **Decouple Templates from Categories & Ensure Null-Safety**:
  - Stop template selection from automatically injecting template category names into project categories and creating them in the vault's category list.
  - Fix crash in Category Manager and list views by ensuring backend initializes items with empty category lists (`Some(vec![])`) and frontend guards against null/undefined `it.categories` (`(it.categories ?? []).includes(...)`).
- **Decouple Templates from Existing Projects**:
  - Remove the "Templates" metadata row from the existing project detail screen. Templates serve exclusively as initial variable scaffolding or ad-hoc variable injection bundles, not as persistent project attributes.
- **Environment Name Selection & Canonical Lowercase Normalization**:
  - Revamp the environment name selector to prominently show human-friendly, localized, and capitalized labels (e.g., "Production", "Local", "Staging", "Test", or "Custom") while maintaining the canonical value in lowercase ASCII (e.g., `production`) so that generated injection filenames remain strictly lowercase (e.g., `.env.production`).

## Capabilities

### New Capabilities
<!-- None -->

### Modified Capabilities
- `desktop-ui`: Update UI contrast specifications for form selects in dark mode, hotkey recording/registration lifecycle and macOS support, project categories naming, project action bar dirty-state placement, template decoupling in existing project views, and environment naming presentation.

## Non-Goals

- Modifying the underlying SQLite schema or database migrations (the DB table is already `project_categories`).
- Altering vault cryptography, secret storage format, or REST API token authentication.
- Implementing an arbitrary multi-key macro system beyond standard desktop hotkey combinations.

## Impact

- **Frontend**:
  - `src/components/Settings.tsx`: Dropdown styling, cross-platform hotkey capture modal/control, reset hotkey button, hotkey validation.
  - `src/components/ProjectManager.tsx`: Rename tags to categories, move save button to footer next to delete, add dirty-state detection, remove template row in project view, revamp environment name selector.
  - `src/index.css`: Global select/option theme styling rules for dark mode contrast.
  - `src/i18n/locales/{en,es,pt}.json`: Updated strings for categories, hotkey helper text, environment presets, and project actions.
- **Backend**:
  - `src-tauri/src/lib.rs`: Read configured hotkey from SQLite settings on startup, set up initial shortcut.
  - `src-tauri/src/vault/mod.rs`: Update `vault_save_settings` to accept `AppHandle` and dynamically re-register the global shortcut.
