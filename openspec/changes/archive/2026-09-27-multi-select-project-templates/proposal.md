## Why

The "new project" template picker offers only six hardcoded cards (Generic, Node.js, PostgreSQL, MongoDB, Docker, Python), allows exactly one choice, and — as implemented today — does nothing beyond storing the template id string: the listed variables are never created. Real projects combine stacks (e.g. Next.js + PostgreSQL + Redis + OpenAI + Stripe), so a single-choice, decorative picker gives no head start. A searchable, multi-select catalog that actually scaffolds an editable example environment turns project creation into a one-step bootstrap.

## What Changes

- Replace the 6-card `TemplateModal` with a **searchable, multi-select template catalog** (~60 templates) grouped by kind: runtimes & frameworks, databases, caches & queues, AI APIs, cloud & DevOps, auth, payments & messaging, observability. The user can pick **0, 1 or many**.
- Variable names and example values are taken from the conventions documented by the upstream open-source projects (official Docker images, SDK env conventions, framework `.env.example` files) — full catalog and sources in `design.md`.
- Add a **review step** before creation: the merged variables are shown grouped by template, every key and example value is editable, rows can be removed or added, and duplicate keys across templates (e.g. `DATABASE_URL`) are merged into one row.
- The selected templates **prefill the project's categories** (e.g. `Node.js`, `PostgreSQL`, `OpenAI`); the user can edit them before saving. Missing categories are created.
- On confirm, the project is created with its `default` environment **populated**: one project-scoped `secret` vault item per variable, linked into the environment. Empty values are stored as the placeholder `CHANGE_ME`.
- New Tauri command `project_create_from_templates` that performs project + items + environment creation as one all-or-nothing operation (rollback on failure).
- `Project.template` stores the comma-joined list of selected template ids (`generic` when none). `ProjectTemplate` TS union is widened to `string`. Existing projects are unaffected.
- Changing the template of an **existing** project from the project form is removed (it never had any effect); the stored template list is shown read-only.

## Capabilities

### New Capabilities
- `project-templates`: template catalog, search/multi-select picker, review/edit step, merge/dedup rules, and atomic scaffolding of a project with a populated default environment.

### Modified Capabilities
<!-- None: no existing spec in openspec/specs/ covers project creation or templates. -->

## Impact

- **Frontend**: `src/components/ProjectManager.tsx` (`TemplateModal` replaced; new-project flow), new `src/data/projectTemplates.ts` catalog + pure merge helpers (vitest), `src/types/index.ts` (`ProjectTemplate` → `string`), `src/store/projectStore.ts` (new `createFromTemplates`), i18n keys in `src/i18n/locales/{en,es,pt}.json`.
- **Backend**: new `project_create_from_templates` command in `src-tauri/src/vault/mod.rs` (vault orchestrates `project` + `db` + crypto), registered in `lib.rs`. No schema migration (`projects.template` is already `TEXT`).
- **Security**: example values travel over Tauri IPC exactly like the existing `vault_create_project_item` path and are encrypted at rest as normal vault items. Values MUST NOT appear in errors/logs. Catalog is static, bundled data — no network fetch. No REST/MCP/CLI surface is added.
- **Dependencies**: none.

## Non-Goals

- No REST API, MCP or CLI equivalent of template scaffolding (GUI only in this change).
- No user-defined / remote / downloadable template catalogs; the catalog is static and bundled.
- No generation of files on disk (`.env.example`, `docker-compose.yml`); existing inject flow is unchanged.
- No per-environment scaffolding (only the auto-created `default` environment is populated).
- No auto-generation of random secret values (passwords/keys are left as `CHANGE_ME`).
- No change to `.cryptenv-proj` export/import format.
