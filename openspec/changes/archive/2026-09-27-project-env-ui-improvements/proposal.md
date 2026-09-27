# Proposal: Project & Environment UI Improvements & Session Synchronization

## Why

Users creating projects and environments encounter several UX frictions and an unexpected session lock error:
1. When creating a project, tag selection requires existing categories; if none exist, users must navigate away to Category Manager and lose their progress.
2. The initial environment created with a project is hardcoded to "default" without giving the user the option to name or configure it upfront.
3. In the environment variable creation form, the `KEY_NAME` input is visually indistinct and ambiguous alongside item-specific fields, confusing users as to what must be filled.
4. Users actively working in the UI suddenly receive a "vault is locked" error when adding environment variables because the Rust backend auto-lock timer runs independently of frontend user interactions and does not notify the frontend when locking.
5. In the new environment dialog, environment presets are displayed entirely lowercase instead of capitalized across supported languages.
6. In the environment view header, the title is misaligned and off-center due to the presence of the left-aligned navigation button.

## What Changes

- **Inline Category Creation in Project Modal**: Enable users to quickly add a new category directly within `TagInput` / Project creation modal without leaving the project wizard, persisting it to the database and selecting it immediately.
- **Customizable Initial Environment**: Allow users to specify the initial environment name during project creation, defaulting to "default" when left blank.
- **Visual Distinction for Variable `KEY_NAME`**: Restyle the `KEY_NAME` input with prominent green/accent borders (`border-accent`), clear labeling, and helper text emphasizing that it is the required environment variable export name for all item types.
- **Vault Auto-Lock Session Synchronization & Error Recovery**:
  - Keep the backend vault session alive by updating activity timestamp (`touch()`) on active user interactions from the frontend.
  - Emit a Tauri event (`vault_locked`) when the backend auto-lock fires, enabling the frontend to immediately transition to the lock screen rather than remaining in an inconsistent state.
  - If a vault crypto operation ever returns "vault is locked", handle it gracefully in the frontend by notifying the user and navigating to the lock screen.
- **Capitalized Environment Presets**: Format environment type presets with initial capitalization (e.g., "Production", "Local", "Test", "Staging") and provide localized preset labels in English, Spanish, and Portuguese.
- **Strictly Centered Environment Header Title**: Use an absolute-centering layout in the environment detail header to keep the environment name centrally aligned across the full available width regardless of the left back button's width.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `desktop-ui`: Extends the desktop UI specification with requirements for inline category creation in tag inputs, customizable initial project environments, visual clarity for environment variable key inputs, frontend-backend vault lock synchronization, capitalized/localized environment presets, and centered environment view headers.

## Non-Goals
- Altering the underlying AES-256-GCM / Argon2id encryption schema or database storage tables.
- Modifying CLI or TUI environment commands beyond the shared Rust backend logic.
- Redesigning the full Category Manager screen (existing CRUD in Settings/CategoryManager remains intact).

## Security Implications
- Synchronizing user activity via a lightweight `vault_touch` Tauri command prevents premature auto-locking while the user is actively typing, without exposing any secret keys or plaintext data.
- Emitting `vault_locked` from the backend when auto-lock triggers ensures volatile memory keys zeroized by backend timers are promptly reflected in the frontend UI, preventing stale UI states.
- Zero plaintext secrets are ever exposed in logs, events, or errors.

## Impact
- **Frontend**: `src/components/ProjectManager.tsx`, `src/components/ui/TagInput.tsx`, `src/hooks/useAutoLock.ts`, `src/store/index.ts`, `src/store/projectStore.ts`, `src/i18n/locales/*.json`.
- **Backend**: `src-tauri/src/lib.rs`, `src-tauri/src/vault/mod.rs`, `src-tauri/src/project/mod.rs`.
- **APIs**: New Tauri command `vault_touch` and Tauri event `vault_locked`. Updated `ProjectInput` to optionally accept an `initial_environment` name.
