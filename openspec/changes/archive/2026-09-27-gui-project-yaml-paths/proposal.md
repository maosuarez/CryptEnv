# Proposal: GUI Project Path Selection, YAML Creation, and Multi-Path Injection

## Why

When creating a project in the desktop GUI, users currently specify metadata (name, description, tags, template) but cannot select a root filesystem directory to link with the project. As established in the `cli-tui-parity` capability, projects in CryptEnv are tied to a repository root containing `.crypt-env.yaml` (alongside `.gitignore`) and can inject environment files into one or more targets (such as monorepos, microservices, or frontend/backend subdirectories). Allowing users to select a root directory upon project creation in the GUI, automatically generating `.crypt-env.yaml` in that root, configuring multiple injection paths, and choosing where to inject from the GUI delivers seamless parity and flexibility between GUI and CLI workflows.

## What Changes

- **Project Root Directory Selection in GUI**:
  - The project creation modal in the GUI prompts the user for a project root directory (via text input or native folder picker).
  - Upon project creation, the GUI writes the initial `.crypt-env.yaml` into this base directory.
- **Multiple Injection Path Management**:
  - Projects and environments support multiple associated injection paths (e.g. root `.`, `./frontend`, `./backend`, `./apps/api`).
  - The environment paths list in the GUI allows adding and removing multiple target directories or `.env` files.
- **Interactive Injection Target Selection**:
  - When clicking "Inject" on an environment that has multiple configured paths, the GUI presents an interactive modal dialog asking the user which path(s) to inject into (or "All Paths").
  - If only one path is configured, it injects directly into that path (with the existing confirmation if overwriting unmanaged files).

## Capabilities

### Modified Capabilities
- `desktop-ui`: Adds project root path selection and `.crypt-env.yaml` creation to the project creation modal, multi-path management for environment injection targets, and an interactive injection target picker modal when multiple paths are configured.

## Non-Goals

- Modifying the underlying encryption scheme. (The only schema change — nullable `projects.root_path` — is owned by `cli-tui-parity` Decision 8, which this change builds on.)
- Replacing the CLI `init` or `config` commands; this change brings GUI creation to feature parity with the YAML-based project model.

## Security Considerations

- **Path Traversal & Invariant Protection**: Path inputs entered in the GUI are validated and canonicalized to prevent arbitrary file writes outside intended project boundaries.
- **No Plaintext in YAML**: The `.crypt-env.yaml` file written by the GUI contains only project metadata, environments, tags, and injection paths; no secret values are ever written to the YAML file.
- **Overwriting Foreign Files**: Grandfathered protection remains intact: injecting into an unmanaged file creates a `.bak` backup and confirms with the user before overwriting.

## Impact

- Frontend: `src/components/ProjectManager.tsx` and project store.
- Backend / Tauri: Tauri command to write initial `.crypt-env.yaml` file to disk during project creation (`project_create` / `project_save`).
- Localization: Add translation strings for path prompt, YAML creation, and injection target selection modal in `src/locales/`.
