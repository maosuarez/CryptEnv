## 1. Backend Core Logic & Extensionless .env Support

- [x] 1.1 Update `validate_environment_name` in `src-tauri/src/project/mod.rs` to allow empty string `""` as a valid environment name representing the canonical root `.env` file, and update inject path resolution so an empty environment name outputs `.env` without an extension. Verify with `cargo test`.
- [x] 1.2 Add unit tests in `src-tauri/src/project/mod.rs` verifying that an unnamed environment resolves correctly to `.env` and passes validation. Verify with `cargo test`.

## 2. Environment Naming, Type Change Confirmation & Duplication

- [x] 2.1 Update `EnvNameField` and environment state in `src/components/ProjectManager.tsx` so the default option represents the extensionless `.env` (empty string name) without displaying the label "default". Verify that `.env` renders cleanly in the UI.
- [x] 2.2 Implement default environment selection rules in `ProjectManager.tsx`: if an unnamed `.env` environment exists in the project, it is automatically marked as default; allow manual default selection only if no unnamed `.env` environment exists.
- [x] 2.3 Implement `ConfirmEnvTypeChangeModal` in `ProjectManager.tsx` triggered when changing an existing environment's type, requiring explicit confirmation before applying the change.
- [x] 2.4 Implement Environment Duplication in `ProjectManager.tsx` allowing users to duplicate an environment and all its variables into a different available environment type.
- [x] 2.5 Update the "+ Add Environment" action in `ProjectManager.tsx` to preserve the project's stack templates and pre-populate template variables for the new environment.

## 3. Simplified Variable Actions & Type Accordions

- [x] 3.1 Update `VarRow` in `src/components/ProjectManager.tsx`: remove the unlink ('x') button, keeping only the reveal ('eye') and delete ('trash') buttons.
- [x] 3.2 Implement a delete confirmation modal when clicking the trash button in `VarRow`, ensuring items are deleted only after user confirmation.
- [x] 3.3 Implement collapsible accordion sections in `ProjectManager.tsx` grouped by item type (`Secret`, `Credential`, `Link`, `Command`, `Note`), removing the repetitive uppercase type badge from individual variable rows.

## 4. Variable Detail View & Global Toggle

- [x] 4.1 Update `AddVarPanel` in `src/components/ProjectManager.tsx` to include an `isGlobal` toggle checkbox, passing the flag to `createProjectItem`.
- [x] 4.2 Implement `VarDetailModal` in `src/components/ProjectManager.tsx` allowing users to inspect an environment variable's full details (fields, metadata, decrypted values) and toggle `isGlobal` for existing items via `toggleGlobal`.

## 5. Localization & Verification

- [x] 5.1 Add translation strings for all new modals, labels, and actions in `src/i18n/locales/en.json`, `src/i18n/locales/es.json`, and `src/i18n/locales/pt.json`.
- [x] 5.2 Run `cargo check` and `cargo test` in `src-tauri/`, and run frontend build/test suites (`pnpm build` or `pnpm test`) to verify everything compiles and passes.
