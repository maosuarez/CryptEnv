## Context

CryptEnv uses Tauri 2.0 with a Rust backend and a React 19 frontend. The UI follows an industrial Carbon-inspired dark aesthetic by default, with optional light theme support. Global shortcuts are handled via `tauri-plugin-global-shortcut`.

Currently:
- The OS global shortcut is registered once at application setup with a hardcoded `"Ctrl+Alt+Z"` in `src-tauri/src/lib.rs`. Saving a new hotkey in Settings only updates a SQLite row; it never calls `tauri-plugin-global-shortcut` to apply the change, leaving the OS listener stale.
- While the hotkey capture button in `Settings.tsx` is focused, pressing the active shortcut triggers Tauri's OS-level listener first, hiding the window before the key combination can be captured.
- Native `<select>` and `<option>` elements rely on browser/OS defaults, which on desktop Webviews (Chromium/WebView2) render popup option menus with bright white backgrounds when running in dark theme.
- In `ProjectManager.tsx`, the project's categories field is labeled "Tags", causing confusion with item labels. The project save button is placed inside an unrelated "Template" metadata row in the middle of the form instead of the bottom action bar.
- Existing projects display the template they were initialized from, even though templates are one-time seed configurations.
- The environment preset input uses an HTML `<datalist>` where lowercase technical names (`production`) take visual precedence over user-facing capitalized/localized names.

## Goals / Non-Goals

**Goals:**
- Provide dark-mode contrast for all dropdown `<select>` and `<option>` elements in Settings and Project views.
- Implement dynamic global shortcut re-registration in Tauri at runtime so hotkey changes apply immediately.
- Load the persisted hotkey on startup and adapt modifier labels for macOS (`Cmd` vs `Ctrl`).
- Provide safe shortcut recording that pauses or avoids triggering window toggles during capture, validates modifiers, and includes a "Reset to default" action.
- Rename project "tags" to "categories" across UI and localization dictionaries.
- Relocate the project Save button to the footer action bar (to the left of "Delete Project"), style it in green accent, and show it only when project fields are dirty.
- Remove the static "Template" row from existing project views.
- Present environment presets with human-friendly capitalized/localized display while enforcing lowercase ASCII for stored names and `.env.<name>` injection targets.

**Non-Goals:**
- Modifying the underlying SQLite schema or database migrations (the DB table is already `project_categories`).
- Altering vault cryptography or secret storage format.
- Creating a global macro recorder for non-standard key sequences.

## Decisions

### 1. Dropdown Dark Mode Contrast via CSS and Tokenized Classes
**Decision**: Configure global `<select>` and `<option>` rules in `src/index.css` for dark mode (`:root:not([data-theme="light"]) select option`), explicitly setting `background-color: var(--color-surface)` and `color: var(--color-tx)`. Ensure all select elements throughout `Settings.tsx` and `ProjectManager.tsx` use Tailwind classes `bg-raised text-tx border-bd2` with consistent padding and outline styling.
*Alternative considered*: Replacing native selects with heavy custom dropdown popovers. *Rationale*: Native selects are accessible and lightweight; applying explicit option styles in Webview2/WebKit resolves the contrast bug without introducing outside-click listener overhead or z-index clashing.

### 2. Runtime Global Shortcut Re-registration via Tauri Plugin
**Decision**:
1. In `src-tauri/src/lib.rs`, extract shortcut registration into a helper function `register_app_hotkey(app: &tauri::AppHandle, hotkey: &str) -> Result<(), String>`. At startup, query SQLite for the configured hotkey (defaulting to `Ctrl+Alt+Z` or macOS `Cmd+Alt+Z`).
2. Expose Tauri commands or extend `vault_save_settings` to accept `app: tauri::AppHandle`. When `hotkey` is changed, unregister the old shortcut and register the new one.
3. Expose `vault_pause_hotkey(app: tauri::AppHandle, paused: bool)` or maintain an atomic pause flag so that when the frontend enters hotkey recording mode, the global shortcut toggle does not fire and close the window.
*Alternative considered*: Requiring app restart for hotkey changes. *Rationale*: Poor user experience; Tauri 2 `GlobalShortcutExt` supports runtime `register` and `unregister`.

### 3. macOS vs Windows/Linux Modifier Adaptation
**Decision**: In `Settings.tsx`, check platform via `@tauri-apps/plugin-os` or Tauri `platform()`. On macOS, display `Cmd` instead of `Ctrl`. Accept `CommandOrControl` or convert `Cmd` into `Command` for Tauri's shortcut string parser.
*Alternative considered*: Hardcoding `Ctrl` everywhere. *Rationale*: macOS users do not have a primary `Ctrl` workflow; standard macOS applications use Command (⌘).

### 4. Safe Hotkey Recording & Validation
**Decision**:
- While recording (`capturing === true`), invoke `vault_pause_hotkey(true)` on start and `vault_pause_hotkey(false)` on completion/cancel/blur.
- Validate that the captured combination includes at least one modifier key (`Ctrl`, `Alt`, or `Meta`/`Cmd`) before saving. Single character keys without modifiers (e.g. `A`, `1`) will be ignored with helper text.
- Add a "Reset to default" button that reverts the draft to `Ctrl+Alt+Z` (or `Cmd+Alt+Z`).

### 5. Project Form Layout & Dirty-State Management
**Decision**:
- Remove the "Template" row from existing project view (`!isCreatingProj`).
- Rename all project "tag" labels and placeholders to "Categories" / "Categorías" in `en.json`, `es.json`, `pt.json`, and `ProjectManager.tsx`.
- Move the Save button to the bottom footer action bar:
  ```tsx
  <div className="flex items-center gap-2">
    <div className="flex-1" />
    {isDirty && (
      <button onClick={handleSaveProject} className="bg-accent text-[#020504] font-bold ...">
        {t('common.save')}
      </button>
    )}
    <button onClick={() => setConfirmDelProj(true)} className="...">
      {t('projects.deleteProject')}
    </button>
  </div>
  ```
- Track dirty state via:
  ```tsx
  const isDirty = projName.trim() !== selectedProject.name ||
    projDescription.trim() !== (selectedProject.description ?? '') ||
    !setsEqual(new Set(projCategories), new Set(selectedProject.categories));
  ```

### 6. Environment Name Presentation vs Canonical Value
**Decision**:
- Model presets with display labels: `production` -> "Production" / "Producción", `staging` -> "Staging", `local` -> "Local", `test` -> "Test".
- Allow picking a preset or typing a custom name.
- Regardless of display casing or localization, normalize the canonical environment `name` to lowercase ASCII before saving (`name.toLowerCase().trim()`).
- Injections strictly target `.env.<canonical_name>`.

### 7. Template Category Decoupling & Null-Safe Item Categories
**Decision**:
- In `ProjectManager.tsx`, stop injecting `templateCategories(ids)` into `projCategories`. Templates contribute variables and defaults only; user categories are curated explicitly by the user.
- In `src-tauri/src/vault/mod.rs` (`populate_default_environment`), initialize item categories as `categories: Some(vec![])` instead of `categories: None`.
- In `CategoryManager.tsx`, `GlobalSecrets.tsx`, and `SecretRow.tsx`, defensively access categories with `(it.categories ?? [])` before invoking array methods (`includes`, `some`, `join`).
*Alternative considered*: Auto-creating a category per template. *Rationale*: Selecting 10 templates flooded the user's category list with 10 tech tags that users did not ask for, while null values triggered uncaught type errors in React rendering.

## Security & Threat Model

- **No Secrets in IPC**: Hotkey updates, category renames, and environment naming carry non-sensitive metadata only.
- **Shortcut Hijacking Prevention**: Hotkey registration requires valid modifiers so the app does not intercept plain typing keys globally across the OS.
- **Path Traversal Guard**: Environment names continue to be validated against `validate_environment_name` (`^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`) preventing path traversal outside the project directory.

## Risks / Trade-offs

- **[Risk]** Hotkey conflict with other OS applications when registering a user-defined shortcut.
  → **Mitigation**: Return a clear descriptive error from `vault_save_settings` if `app.global_shortcut().register()` fails, display a toast notification in the frontend, and allow reverting to the default hotkey.
- **[Risk]** Pausing hotkey during capture could leave hotkey unregistered if browser crashes during recording.
  → **Mitigation**: Use an in-memory boolean flag in Rust (`HOTKEY_PAUSED`) checked inside the shortcut event callback rather than unregistering, ensuring safety across state disruptions.
