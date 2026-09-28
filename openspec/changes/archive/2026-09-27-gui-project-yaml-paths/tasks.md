# Implementation Tasks: GUI Project Path Selection, YAML Creation, and Multi-Path Injection

> Depends on `cli-tui-parity` task group 0 (`projects.root_path`, relative path resolution, shared manifest serializer).

## 1. Backend & Tauri Commands for YAML Manifest Provisioning

- [x] 1.1 Implement Tauri commands `project_write_yaml` (validate root, refuse to overwrite unless asked, write `.crypt-env.yaml` from vault state) and `project_pick_root_dir` (folder picker) in `src-tauri/src/project/mod.rs`.
- [x] 1.2 Register the commands in `src-tauri/src/lib.rs` and verify with `cargo check`.

## 2. Project Creation Modal UI Updates

- [x] 2.1 Add project root directory input field and folder picker in `ProjectManager.tsx` to the project form (create and edit).
- [x] 2.2 Wire project creation submission to save `rootPath` and invoke `project_write_yaml`, with overwrite/link choice when the file already exists.
- [x] 2.3 Verify in the UI that creating a project generates `.crypt-env.yaml` in the chosen directory.

## 3. Multi-Path Management for Environment Injection

- [x] 3.1 Update the environment edit view in `ProjectManager.tsx` to allow adding, editing, and deleting multiple injection paths (relative to root or absolute).
- [x] 3.2 Verify that multiple paths persist correctly in `environments.paths` and reload on project view.

## 4. Interactive Injection Target Selection Modal

- [x] 4.1 Create `InjectTargetModal` dialog in `ProjectManager.tsx` displaying the configured injection paths with checkboxes and an "All Paths" option.
- [x] 4.2 Update `runInject` so that when `env.paths.length > 1`, `InjectTargetModal` opens to let the user choose which destination path(s) to inject into before executing (`environment_inject` `targets`).
- [x] 4.3 Verify that single-path environments inject directly while multi-path environments open the target selector.

## 5. Localization & Verification

- [x] 5.1 Add localization strings for workspace root input, directory browse, and inject target selector in `src/locales/en.json`, `es.json`, and `pt.json`.
- [x] 5.2 Verify compilation across frontend (`pnpm build`) and backend (`cargo test`).
