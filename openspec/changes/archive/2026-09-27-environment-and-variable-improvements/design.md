## Context

CryptEnv organizes secrets hierarchically: Projects contain Environments, which link to encrypted Vault Items as Environment Variables.

Currently:
- The default environment has name `"default"`, leading to `.env.default` filenames during inject instead of canonical `.env`.
- Changing an environment preset in the editor immediately changes the target file name without confirmation.
- There is no mechanism to clone or duplicate an environment's configuration into another environment type.
- Each variable row renders three action icons: reveal ('eye'), unlink ('x'), and delete ('trash') without confirmation, creating cognitive load and risk of accidental data loss.
- Every variable row displays a redundant item type badge (`SECRET`, `CREDENTIAL`), cluttering the UI.
- "+ Add Environment" creates an empty environment, ignoring the stack templates chosen when the project was created.
- Variables cannot be inspected in a detailed view within the environment editor, and cannot be designated or toggled as global during or after creation.

## Goals / Non-Goals

**Goals:**
- Canonicalize `.env` as the unnamed default environment that maps to the extensionless `.env` file.
- Automatically prioritize the unnamed `.env` as the project default, restricting custom default selection to when no unnamed `.env` exists.
- Guard environment type mutations with an explicit confirmation modal.
- Provide a seamless environment duplication feature with target type selection.
- Streamline variable actions: remove unlink, keep reveal and delete, and protect deletion with a confirmation modal.
- Categorize variables into clean, collapsible accordion sections by item type without row-level type badges.
- Maintain project stack templates on "+ Add Environment" to pre-populate required variables.
- Allow viewing variable details in a modal/drawer and toggling `isGlobal` both during creation and post-creation.

**Non-Goals:**
- Changing database table structures or cryptographic algorithms.
- Changing CLI/MCP protocols beyond supporting the extensionless `.env` environment resolution.

## Decisions

### 1. Canonical Extensionless `.env` (Unnamed Environment)
- **Decision**: In the UI, the default environment is represented with an empty name `""`, displayed as `.env` without extension or the label "default".
- **Backend Resolution**: In `src-tauri/src/project/mod.rs`, `validate_environment_name` will accept an empty string `""` specifically for the root `.env` environment. In directory-based inject resolution, an environment with `name == ""` or `name == "default"` resolves to `.env` (rather than `.env.default`).
- **Default Selection Rule**: If an environment with `name == ""` (or legacy `"default"`) exists in a project, it is strictly assigned `is_default = true`, and the default checkbox is locked/hidden. If and only if no unnamed `.env` environment exists in the project, the default selector/checkbox is available on the named environments.
- **Alternatives Considered**: Storing a magic string like `"__root__"` in SQLite. *Rejected* because an empty string `""` naturally maps to `.env` without suffix and is fully supported by SQLite unique constraints `UNIQUE(project_id, name)`.

### 2. Environment Type Change Confirmation Modal
- **Decision**: When an existing environment's type/preset is changed in the editor, capture the event before updating state and show `ConfirmEnvTypeChangeModal`.
- **Modal Content**: Highlights the change in target inject file (e.g., from `.env` to `.env.production`) and warns that existing scripts or workflows relying on the old file name will be affected.
- **Alternatives Considered**: Inline warning message below the dropdown. *Rejected* because changing the environment type mutates the inject target and can lead to unintended file overwrites if saved unintentionally.

### 3. Environment Duplication
- **Decision**: Provide a "Duplicate" button in the environment header. Opening it presents a modal listing unused presets (`.env`, `production`, `staging`, `test`, `local`, `custom`). Upon selecting a target type, a new environment is created with cloned `vars` and saved immediately. Paths are retargeted, not cloned: a path whose filename is the source's own `.env[.<name>]` is rewritten to the target's filename in the same folder; other paths are dropped, so two environments never inject into the same file.
- **Rationale**: Developers often create a staging or production environment with the same variables as local/.env, with only values differing. Duplicating allows rapid configuration.

### 4. Simplified Variable Row Actions & Deletion Modal
- **Decision**: Remove the 'x' (unlink) icon entirely from `VarRow`. Retain only:
  1. Reveal/Mask toggle ('eye' / 'eyeOff')
  2. Delete ('trash')
- **Global items**: For an `isGlobal` item the trash button only unlinks the variable from this environment (the item may be shared by other environments/projects); global items are deleted from the Global Secrets view. Two buttons remain on every row.
- **Confirmation Modal**: For project-local items, clicking delete opens `ConfirmDeleteVarModal` detailing that the item will be permanently removed from the vault and unlinked from the environment.
- **Rationale**: Having both unlink and delete side-by-side with subtle icon differences was confusing and led to unintended deletions. Deletion must always be deliberate and confirmed.

### 5. Type-Grouped Accordions
- **Decision**: Group the environment's variables by `item.type` (`secret`, `credential`, `link`, `command`, `note`, and any missing items).
- Each group renders as a collapsible accordion section:
  - Header: localized type label, variable count (e.g. `Secrets (4)`), and expand/collapse chevron.
  - Body: list of variable rows belonging to that type, without individual type badges.
- **Rationale**: Eliminates repetitive visual noise while instantly conveying the type of each secret through section grouping.

### 6. Preserving Project Templates on "+ Add Environment"
- **Decision**: In `handleNewEnvironment`, read `selectedProject.template` (e.g. `"nextjs,supabase"`). If the project has templates configured, look up their template definitions and offer to pre-fill the new environment with those template variable keys.
- **Rationale**: Ensures variable consistency across environments without forcing manual re-entry of the stack's variable names.

### 7. Variable Detail View and Global Status Toggle
- **Decision**:
  - In `AddVarPanel`: add an `isGlobal` toggle checkbox. When checked, `createProjectItem` receives `isGlobal: true`.
  - In `VarRow`: clicking the variable row opens a `VarDetailModal` showing full decrypted fields, metadata, and an interactive `isGlobal` toggle switch that invokes `toggleGlobal(item.id, newStatus)`.
- **Rationale**: Solves the limitation where secrets created inside a project could never be shared globally or viewed in full detail.

## Security & Threat Model

- **Memory Safety & Zeroization**: The `VarDetailModal` decrypts item fields only when requested, keeps decrypted values in React component state, and discards them on unmount.
- **Deletion Safety**: Deleting a vault item permanently deletes its ciphertext from SQLite and cascades to all links. The mandatory confirmation modal prevents accidental data loss.
- **Path Traversal & Injection Safety**: Unnamed `.env` maps strictly to `.env` resolved via `fsguard::resolve_within`, preventing any directory traversal.

## Database & API Contract Modifications

- **SQLite Schema**: No schema migrations needed. The `environments` table already stores `name TEXT NOT NULL`, which accepts `""`.
- **Backend Rust (`project/mod.rs`)**:
  - `validate_environment_name`: Allow empty string `""` as a valid name specifically representing `.env`.
  - `resolve_environment`: Handle `""` and legacy `"default"` interchangeably for backward compatibility.
  - Injections for `""` resolve filename to `.env`.

## Risks / Trade-offs

- **[Risk] Existing projects have environments named "default"**
  - → *Mitigation*: Map `"default"` and `""` to the same canonical `.env` in backend logic, and display `"default"` as `.env` in the UI to ensure transparent backward compatibility.
- **[Risk] Duplicate environment fails if all presets are already used**
  - → *Mitigation*: The duplication modal always allows custom environment names in addition to standard presets.
