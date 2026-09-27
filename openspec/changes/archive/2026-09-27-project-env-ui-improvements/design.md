# Design: Project & Environment UI Improvements & Session Synchronization

## Context

The CryptEnv desktop UI is built with React 19, TypeScript, Tailwind CSS, and Tauri 2.0.
The backend maintains an in-memory `SharedState` storing the derived vault key in volatile memory (`Option<Zeroizing<[u8; 32]>>`).
When users manage projects and environments:
- Category selection is restricted to pre-existing categories.
- Projects are automatically created with a single environment hardcoded as `"default"`.
- The `KEY_NAME` input in `AddVarPanel` blends visually with general item fields, obscuring that it specifies the environment variable key.
- A background Tokio task in `src-tauri/src/lib.rs` checks `s.last_activity` every 30 seconds and calls `lock_vault` when idle for `auto_lock_timeout` minutes. However, frontend DOM events (typing, mouse moves) only reset a frontend timer and do not communicate with the backend. Additionally, when `lock_vault` runs in the background, no Tauri event is emitted, leaving the frontend in an inconsistent "unlocked" state until an encrypted vault command fails with `"vault is locked"`.
- Environment presets are lowercase, and the environment header title is visually offset due to asymmetric flexbox spacing.

## Goals / Non-Goals

**Goals:**
- Provide seamless inline category creation inside the project creation modal.
- Support user-defined initial environment names during project creation (defaulting to `"default"`).
- Make the environment variable key name field visually prominent and self-explanatory.
- Synchronize frontend user activity with backend auto-lock timer and listen for backend lock events.
- Present environment presets with initial capitalization and localized names across English, Spanish, and Portuguese.
- Accurately center the environment header title across the entire header width.

**Non-Goals:**
- Removing or bypassing auto-lock security invariants.
- Migrating existing project environment names in database tables.
- Replacing the standalone Category Manager view.

## Decisions

### 1. Inline Category Creation via TagInput
- **Decision**: Extend `TagInput` to support an optional inline creation mode or button that lets the user enter a category name, select a preset color, and save it via `vault_save_categories`.
- **Rationale**: Keeps users in flow when creating projects. Eliminates the need to abandon the modal, navigate to settings/categories, and start over.
- **Alternatives Considered**: Opening a separate CategoryManager modal over the project modal. Rejected because nested modals increase visual clutter and disrupt context.

### 2. Initial Environment Parameter in `ProjectInput`
- **Decision**: Add `initial_environment: Option<String>` to `ProjectInput` in Rust (`src-tauri/src/project/mod.rs`) and `ProjectStore` in TypeScript. In `save_project`, if `is_new`, use `input.initial_environment.as_deref().unwrap_or("default")`.
- **Rationale**: Backward compatible (defaults to `"default"` if omitted), validated against environment naming rules (`validate_environment_name`), and avoids creating a dummy `"default"` environment only to rename it.

### 3. Visual Prominence for `KEY_NAME`
- **Decision**: In `AddVarPanel` (mode 'new'), wrap the `KEY_NAME` input in a dedicated high-visibility card styled with `border-accent/60 focus-within:border-accent ring-1 ring-accent/20` and an explicit label badge.
- **Rationale**: In CryptEnv, all items attached to an environment represent environment variables (exported as `KEY_NAME=value`). Users need immediate clarity that this field is the shell/env variable name, distinct from item title or credentials.

### 4. Frontend-Backend Auto-Lock Synchronization
- **Decision**:
  1. Add a Tauri command `vault_touch` in Rust that updates `s.last_activity` if `s.key.is_some()`.
  2. In `useAutoLock`, throttle user activity events to invoke `vault_touch` (e.g., at most once every 15 seconds).
  3. In `src-tauri/src/lib.rs`, when the auto-lock loop executes `lock_vault`, emit a Tauri event `vault_locked`.
  4. In `App.tsx` / `useVaultStore`, listen for `vault_locked` to immediately set `screen: 'lock'` and reset items in memory.
  5. In `AddVarPanel` and project operations, catch `"vault is locked"` errors and transition cleanly to the lock screen with a user notification.
- **Rationale**: Prevents unexpected `"vault is locked"` errors while a user is actively typing in the app, while preserving the strict volatile memory zeroization guarantees when the user is truly idle.

### 5. Localized and Capitalized Environment Presets
- **Decision**: Keep the canonical preset *values* lowercase (`production`, `local`, `test`, `staging`) and show capitalized, localized *labels* (`Production` / `Producción` / `Produção`, …) from `en.json`, `es.json`, `pt.json` via `<option value label>`.
- **Rationale**: An environment name becomes the `.env.<name>` filename on inject (`output_dir`), and frameworks (Next.js, Vite, dotenv) expect `.env.production` on case-sensitive filesystems (Linux/WSL); env names are also ASCII-only, so a localized value like `Producción` would be rejected. Labels satisfy the display requirement without changing stored names.
- **Rejected**: capitalized canonical values (`Production`) — would write `.env.Production`, silently ignored by common loaders on Linux.

### 6. Absolute Header Centering
- **Decision**: Position the environment title in `ProjectManager` using `absolute inset-0 flex items-center justify-center pointer-events-none` with side padding to prevent overlapping the back button.
- **Rationale**: Unlike `flex-1 text-center` which only centers within the remaining space after subtracting the back button's width, absolute centering ensures mathematical centering across the full width of the header bar.

## Security & Threat Model

- **Memory Security**: No secret keys are stored in DOM or transferred across the IPC during `vault_touch` or `vault_locked` events.
- **IPC Safety**: `vault_touch` is a lightweight, authenticated in-process Tauri command that only updates a monotonic `std::time::Instant`. It does not accept parameters or return secrets.
- **Auto-Lock Invariant**: The backend auto-lock remains the authoritative security boundary. If the user stops interacting with the machine, both backend and frontend timers expire, wiping keys from volatile memory.

## Risks / Trade-offs

- **[Risk] High-frequency IPC calls from DOM events**:
  *Mitigation*: Throttle `vault_touch` invocations in `useAutoLock` to a maximum rate of once per 15-30 seconds.
- **[Risk] Custom initial environment name validation**:
  *Mitigation*: Re-use existing `validate_environment_name` and `ensure_no_case_collision` logic in Rust backend to reject invalid characters or collisions.
