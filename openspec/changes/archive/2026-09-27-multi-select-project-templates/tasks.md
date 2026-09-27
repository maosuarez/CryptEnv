## 1. Backend — scaffold command

- [x] 1.1 Add `ProjectFromTemplatesInput { project: ProjectInput, vars: Vec<TemplateVarInput { key, value }> }` (no `Debug` derive exposing values) and pure fn `create_project_from_templates(db, key, input)` in `src-tauri/src/vault/mod.rs` with up-front validation (project name, ≤200 vars, key regex `^[A-Z_][A-Z0-9_]*$`, unique keys); verify `cargo check` passes
- [x] 1.2 Implement the write sequence: `save_project` → one project-scoped secret item per var (`CHANGE_ME` when value empty) → `save_environment` on the auto-created `default` env; on any error call `db.delete_project(project_id)` and return an error naming key/index only; verify `cargo check`
- [x] 1.3 Add Tauri command `project_create_from_templates` (rejects when vault locked, calls `touch()`), register it in `lib.rs`; verify `cargo check`
- [x] 1.4 Unit tests in `vault` tests: success (items created, encrypted, linked, template string stored), empty value → `CHANGE_ME`, invalid key / duplicate key / >200 vars / invalid project name rejected with nothing created, forced mid-way failure leaves no project or items, error strings never contain a value; verify `cd src-tauri && cargo test` passes and `cargo clippy` is clean

## 2. Frontend — catalog and pure helpers

- [x] 2.1 Create `src/data/projectTemplates.ts` with `TemplateDef`/`TemplateVar` types, group constants, and the full catalog from design.md D6; spot-check each template's keys against its upstream docs while transcribing; verify `pnpm tsc --noEmit` (or `pnpm build`) passes
- [x] 2.2 Implement `searchTemplates(query)` (case-insensitive on label/id/group/keywords) and `mergeTemplateVars(ids)` (selection order, first-wins, `alsoIn` list) and `templateCategories(ids)`; verify with vitest
- [x] 2.3 Add `src/data/projectTemplates.test.ts`: unique ids, key regex, no intra-template duplicate keys, ≥50 templates, all required groups and named stacks present, sensitive examples contain no real-looking credential, search `sql` results, merge dedup (`prisma`+`postgres` → one `DATABASE_URL`, Prisma value, `alsoIn: ['postgres']`); verify `pnpm test` passes
- [x] 2.4 Widen `ProjectTemplate` to `string` in `src/types/index.ts`; add `createFromTemplates(input)` to `src/store/projectStore.ts` invoking `project_create_from_templates` then `load()`; verify type check passes

## 3. Frontend — UI

- [x] 3.1 Replace `TemplateModal` with a multi-select picker: autofocused search, grouped sections, toggle cards with var count, selected counter, Clear/Cancel/Continue, empty-state, Esc to cancel, selections preserved across filtering; verify manually in `pnpm dev` (browser) that searching `sql` and multi-selecting works
- [x] 3.2 Add the variables review panel to the new-project form: rows grouped by source template, editable key (normalised to `A-Z0-9_`) and value, sensitive values masked with reveal toggle, "also in" hint, remove row, add custom row, duplicate/empty key highlighting that disables Create; verify manually
- [x] 3.3 Prefill project categories from selected templates; on Create, add missing categories via `saveCats` (palette-cycled colors) then call `createFromTemplates`; open the created project; verify manually that categories appear and the default environment lists the vars
- [x] 3.4 Make the template field read-only for existing projects (remove "change template" button), render comma-joined ids as labels (fallback raw id) in form and project card; verify a legacy `postgres` project still displays correctly
- [x] 3.5 Add i18n keys (picker header, search placeholder, selected count, no results, clear, continue, review panel labels, also-in hint, duplicate/empty key errors, add variable) to `en.json`, `es.json`, `pt.json`; verify `pnpm test` (i18n key-parity test) passes

## 4. Verification

- [x] 4.1 Run `cd src-tauri && cargo test && cargo clippy`, `pnpm test`, and `pnpm build`; all pass
- [ ] 4.2 End-to-end on Windows (`pnpm tauri dev`), also covering the manual UI checks of 3.1–3.4 (only type-check/build/unit tests were run from WSL): create a project with Node.js + PostgreSQL + Redis + OpenAI, edit a value, remove a row, confirm; verify items exist as encrypted project-scoped secrets, the default environment injects a `.env` with the edited values and `CHANGE_ME` placeholders; create a project with zero templates and confirm it behaves like the old Generic
- [x] 4.3 Grep the new Rust code for any logging/formatting of var values and confirm none (security invariant)
