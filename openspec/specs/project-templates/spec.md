# Project Templates Specification

## Purpose

Lets users bootstrap a new project by picking any number of stack templates (runtimes, databases, AI APIs, cloud, etc.) from a searchable catalog and turning them into an editable, encrypted default environment in one step.

## Requirements

### Requirement: Template catalog
The application SHALL ship a static, bundled catalog of project templates. Each template MUST have a unique id, a display label, a group, one or more search keywords, a category name, and a list of variables. Each variable MUST have a key matching `^[A-Z_][A-Z0-9_]*$`, an example value (possibly empty), and a flag marking it as sensitive. The catalog MUST contain at least 50 templates spanning at least these groups: runtimes & frameworks, databases, caches & queues, AI APIs, cloud & DevOps, auth, payments & messaging, observability. The catalog MUST include at least PostgreSQL, MySQL, MongoDB, Redis, Node.js, Python, Java/Spring Boot, Docker, OpenAI, Anthropic and Google Gemini. Sensitive variables MUST NOT carry a real credential as example value.

#### Scenario: Catalog integrity
- **WHEN** the catalog is loaded
- **THEN** every template id is unique, every variable key matches `^[A-Z_][A-Z0-9_]*$`, no template has duplicate keys, and the catalog has ≥ 50 templates covering all required groups

#### Scenario: Catalog is offline
- **WHEN** the template picker opens with no network connectivity
- **THEN** the full catalog is available (no network request is made)

### Requirement: Searchable multi-select picker
When the user starts creating a new project, the application SHALL show a template picker with a search field and the catalog grouped by group. The search MUST filter case-insensitively by label, id, group and keywords, and MUST update as the user types. The user MUST be able to select zero, one, or many templates; selected templates MUST remain selected when the search filter hides them, and the picker MUST show the number of selected templates. Selecting zero templates and continuing MUST behave as the former "Generic" template (empty environment).

#### Scenario: Filter by keyword
- **WHEN** the user types `sql` in the search field
- **THEN** only templates whose label, id, group or keywords contain `sql` (e.g. PostgreSQL, MySQL, SQL Server, SQLite) are listed

#### Scenario: No results
- **WHEN** the search matches no template
- **THEN** an empty-state message is shown and existing selections are preserved

#### Scenario: Selection survives filtering
- **WHEN** the user selects Node.js, then searches `redis` and selects Redis, then clears the search
- **THEN** both Node.js and Redis are shown as selected and the counter shows 2

#### Scenario: Continue with no selection
- **WHEN** the user continues with no template selected
- **THEN** the project form opens with no pre-filled variables or categories, and the project is saved with template `generic`

#### Scenario: Cancel
- **WHEN** the user cancels the picker
- **THEN** no project, item, environment or category is created

### Requirement: Variable merge and review
After selecting one or more templates, the application SHALL present a review step listing the merged variables grouped under the template that first introduced them (in selection order). When two or more selected templates define the same key, the key MUST appear exactly once, keeping the first template's example value, and MUST indicate the other templates that share it. In the review step the user MUST be able to edit each key and value, remove any row, and add a custom row. Values of sensitive variables MUST be masked by default with a reveal toggle. Keys MUST be normalised to uppercase `A-Z0-9_`; the user MUST NOT be able to confirm while any key is empty or duplicated.

#### Scenario: Shared key is deduplicated
- **WHEN** the user selects Prisma and PostgreSQL (both defining `DATABASE_URL`)
- **THEN** `DATABASE_URL` appears once, with Prisma's example value, marked as also used by PostgreSQL

#### Scenario: Editing creates a duplicate
- **WHEN** the user renames a key to one that already exists in the list
- **THEN** both rows are flagged and the confirm action is disabled until the duplicate is resolved

#### Scenario: Removing all rows
- **WHEN** the user removes every variable row and confirms
- **THEN** the project is created with an empty default environment

### Requirement: Category prefill from templates
Selected templates SHALL prefill the new project's categories with each template's category name (deduplicated). The user MUST be able to add or remove categories before saving. On save, category names that do not yet exist in the vault MUST be created; existing ones MUST be reused (case-sensitive name match, consistent with existing category handling).

#### Scenario: Categories prefilled
- **WHEN** the user selects Node.js, PostgreSQL and OpenAI
- **THEN** the project form shows categories `Node.js`, `PostgreSQL`, `OpenAI`

#### Scenario: Missing category created
- **WHEN** the project is saved with category `OpenAI` and no such category exists
- **THEN** the category is created and the project is tagged with it

### Requirement: Atomic project scaffolding
Confirming the review step SHALL create, as a single all-or-nothing operation: the project (name, description, categories, template = comma-joined selected template ids in selection order, or `generic` if none), its `default` environment, one project-scoped (non-global) `secret` vault item per variable (item name = key, value = edited value, or `CHANGE_ME` if empty), and the links from the default environment to those items under their keys. The operation MUST require an unlocked vault. The operation MUST reject: an invalid project name (same rules as normal project save), any key not matching `^[A-Z_][A-Z0-9_]*$`, duplicate keys, and more than 200 variables. If any step fails, the operation MUST leave no partial project, environment, or vault item behind.

#### Scenario: Successful scaffold
- **WHEN** the user confirms a project `shop` with Node.js + PostgreSQL selected and 9 variables
- **THEN** project `shop` exists with template `node,postgres`, a default environment containing 9 variables, each linked to a new encrypted project-scoped secret item

#### Scenario: Empty value placeholder
- **WHEN** a variable is confirmed with an empty value
- **THEN** its secret item is stored with value `CHANGE_ME`

#### Scenario: Failure rolls back
- **WHEN** creating the 5th vault item fails
- **THEN** the project and all items created so far are removed and an error is shown

#### Scenario: Locked vault
- **WHEN** the scaffold is requested while the vault is locked
- **THEN** the request is rejected and nothing is created

#### Scenario: Invalid key rejected by backend
- **WHEN** the scaffold request contains key `my-key`
- **THEN** the request is rejected and nothing is created

### Requirement: Scaffolding security invariants
Variable values supplied to scaffolding MUST be encrypted at rest exactly like any other vault item. Error messages and logs produced by scaffolding MUST NOT contain any variable value; they MAY contain keys and counts. Scaffolding MUST NOT be exposed through the REST API, MCP server, or CLI.

#### Scenario: Error does not leak value
- **WHEN** scaffolding fails while processing a variable
- **THEN** the returned error names at most the key and never the value

### Requirement: Existing projects remain compatible
Projects created before this change (single template id such as `node`) MUST continue to load and display. The template of an existing project SHALL be shown read-only; re-applying templates to an existing project is not supported.

#### Scenario: Legacy project display
- **WHEN** the user opens a project whose template is `postgres`
- **THEN** it loads normally and shows `postgres` as its template, with no option to change it
