## Why

The current project and environment management UI presents friction and visual clutter when organizing secrets across environments:
1. The default environment is labelled "default" and produces `.env.default` instead of being cleanly treated as the canonical, extensionless `.env` file without an explicit name. Furthermore, switching environment types can unintentionally overwrite target files without confirmation, and there is no simple way to duplicate an environment's configuration into another environment type.
2. The environment variable rows display three confusing action icons ('eye', 'x' for unlink, 'trash' for delete everywhere) without delete confirmation, leading to accidental secret destruction.
3. Every secret row displays a redundant uppercase item type badge (`SECRET`), visually overloading the list.
4. Adding a new environment with "+ Add Environment" starts with an empty list instead of preserving the stack templates configured on the project.
5. Vault items within environments cannot be inspected in a full detail view, and cannot be marked as global when created or toggled to global afterwards.

Addressing these issues streamlines environment management, prevents accidental secret deletion or type mutations, and provides clean visual categorization by item type.

## What Changes

- **Extensionless `.env` / Unnamed Default Environment**:
  - The default environment now maps to the canonical `.env` file (without an extension).
  - In environment presets, it is presented as `.env` without requiring a name.
  - If an unnamed `.env` environment exists in a project, it is automatically marked as the default environment (`isDefault`). Only when no unnamed `.env` exists can the user designate another environment as default.
  - Injecting an unnamed environment resolves to `.env` (not `.env.default`).
- **Environment Type Switch Confirmation**:
  - Changing an environment's type/preset in the editor prompts for confirmation via a modal, explaining that the injected target filename will change.
- **Environment Duplication**:
  - Users can duplicate an existing environment into a new environment, selecting a different environment type/preset and copying its variables and path configurations.
- **Simplified Variable Actions with Delete Confirmation**:
  - The unlink ('x') button is removed from environment variable rows.
  - Only the reveal/view ('eye') and delete ('trash') actions remain.
  - The delete action opens a confirmation modal before deleting the vault item.
- **Type Accordions in Environment Variables List**:
  - Variables in an environment are grouped into collapsible accordion sections by variable type (`Secret`, `Credential`, `Link`, `Command`, `Note`).
  - The repetitive type badge on each individual row is removed.
- **Preserve Project Templates on "+ Add Environment"**:
  - Clicking "+ Add Environment" retains the project's stack templates, pre-populating or offering the project's template variables.
- **Variable Detail View & Global Flag Toggle**:
  - Users can click an environment variable to view its full details (metadata, item fields, revealable secret value, owning projects).
  - Variables can be designated as `isGlobal` directly during creation in `AddVarPanel`.
  - Existing environment variables can have their `isGlobal` status toggled directly from the detail view.

## Capabilities

### Modified Capabilities
- `desktop-ui`: Add requirements for unnamed `.env` default handling, environment type change confirmation modal, environment duplication, simplified variable actions with delete confirmation, type-grouped accordions in the environment editor, and variable detail view with `isGlobal` toggle.
- `project-templates`: Add requirement to maintain project stack templates when adding new environments to an existing project.

## Non-Goals
- Modifying the underlying SQLite schema for projects or environments (the existing `name` column in `environments` supports empty string `""` or canonical `.env` mapping).
- Changing the cryptographic primitives or envelope encryption mechanisms.
- Changing MCP server or CLI command syntax beyond respecting the extensionless `.env` environment resolution.

## Security Considerations
- **No Plaintext Leaks**: Variable details opened in the detail modal must respect zeroization and hide plaintext secret values behind visibility toggles.
- **Accidental Deletion Prevention**: Requiring explicit confirmation before deleting vault items prevents irreversible data loss.
- **Safe File Targeting**: Changing environment types changes the filesystem target during inject; requiring confirmation avoids inadvertently clobbering existing target `.env.*` files.

## Impact
- **Frontend**:
  - `src/components/ProjectManager.tsx`: Update `EnvNameField`, add confirmation modal on type change, implement environment duplication, remove unlink action and add delete confirmation modal, implement accordions grouped by `item.type`, add `isGlobal` checkbox to `AddVarPanel`, and add variable detail view.
  - `src/i18n/locales/`: Update translations (`en.json`, `es.json`, `pt.json`) with new strings for modals, duplication, accordions, and confirmation prompts.
- **Backend / Project Logic**:
  - `src-tauri/src/project/mod.rs`: Ensure environment name validation and inject path resolution properly support empty/unnamed `.env` environments (mapping to `.env`).
