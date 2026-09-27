## 1. Dropdown Styling and Dark Mode Contrast

- [x] 1.1 Add global CSS rules in `src/index.css` for `<select>` and `<option>` elements in dark theme (`:root:not([data-theme="light"]) select option`) to guarantee dark backgrounds (`var(--color-surface)`) and high-contrast text (`var(--color-tx)`). Verify contrast in dark mode.
- [x] 1.2 Audit and apply consistent styling classes (`bg-raised text-tx border-bd2`) to all `<select>` dropdowns in `src/components/Settings.tsx` (Auto-lock Timeout, Language) and `src/components/ProjectManager.tsx` (import var select). Verify that dropdowns render cleanly with no unstyled white flashes.

## 2. Backend Dynamic Global Shortcut Registration & Cross-Platform Support

- [x] 2.1 In `src-tauri/src/lib.rs`, load the saved hotkey from SQLite settings on startup (defaulting to platform-specific default `Ctrl+Alt+Z` / `Cmd+Alt+Z`), and implement a helper function to register the global shortcut listener. Verify with `cargo check`.
- [x] 2.2 Add an in-memory hotkey pause toggle and Tauri command (`vault_pause_hotkey`) to temporarily suppress the window toggle while the user is capturing a new key combination.
- [x] 2.3 Update `vault_save_settings` in `src-tauri/src/vault/mod.rs` to accept `app: tauri::AppHandle`, unregister the previous global shortcut, register the updated shortcut at runtime, and persist the setting. Verify with `cargo test`.

## 3. Frontend Hotkey Recording, Validation & Cross-Platform UI

- [x] 3.1 In `src/components/Settings.tsx`, call `vault_pause_hotkey` when entering recording mode (`capturing === true`) and resume on completion or blur, preventing the window from closing when pressing the active shortcut.
- [x] 3.2 Add cross-platform modifier detection (displaying `Cmd` on macOS vs `Ctrl` on Windows/Linux), require at least one modifier key (`Ctrl`/`Cmd`, `Alt`) for valid combinations, and add a "Reset to default" button.

## 4. Project Categories Terminology & Layout Cleanup

- [x] 4.1 Update `src/i18n/locales/en.json`, `es.json`, and `pt.json` to rename all project "tags" strings to "categories" / "Categorías" / "Categorias".
- [x] 4.2 In `src/components/ProjectManager.tsx`, update the project field label, hints, and filter popover to use category terminology.
- [x] 4.3 In `src/components/ProjectManager.tsx`, remove the static "Template" row and change-template button from the existing project view (`!isCreatingProj`).
- [x] 4.4 In `src/components/ProjectManager.tsx`, remove automatic prefill of template categories into `projCategories` upon selecting templates, ensuring templates do not auto-create vault categories.
- [x] 4.5 In `src-tauri/src/vault/mod.rs` (`populate_default_environment`), initialize item categories as `categories: Some(vec![])` instead of `None`.
- [x] 4.6 In `src/components/CategoryManager.tsx`, `src/components/GlobalSecrets.tsx`, and `src/components/rows/SecretRow.tsx`, add defensive fallbacks `(it.categories ?? [])` to prevent uncaught null reference exceptions when computing category counts or rendering badges.

## 5. Project Action Bar & Dirty-State Save Visibility

- [x] 5.1 In `src/components/ProjectManager.tsx`, relocate the project Save button from the form body to the footer action bar, positioned to the left of the "Delete Project" button, styled in green accent (`bg-accent text-[#020504]`).
- [x] 5.2 Implement dirty-state tracking comparing `projName`, `projDescription`, and `projCategories` against `selectedProject`. Render the Save button only when changes are pending, and hide it when clean.

## 6. Environment Selector UX & Lowercase Normalization

- [x] 6.1 In `src/components/ProjectManager.tsx`, replace the `<datalist>` input with a clean preset selector showing formatted, capitalized, localized labels ("Production", "Staging", "Local", "Test", "Custom...").
- [x] 6.2 Enforce canonical lowercase ASCII normalization for the stored environment name and ensure `.env.<name>` injection targets strictly use the lowercase name.

## 7. Verification & Build

- [x] 7.1 Run `cd src-tauri && cargo check && cargo test` to verify backend compilation and unit tests.
- [x] 7.2 Run `pnpm test` and `pnpm build` to verify frontend tests and production bundle compilation.
