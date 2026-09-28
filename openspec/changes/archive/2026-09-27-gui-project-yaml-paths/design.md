# Technical Design: GUI Project Path Selection, YAML Creation, and Multi-Path Injection

## Context

The `cli-tui-parity` capability establishes `.crypt-env.yaml` as the canonical local project manifest for CryptEnv workspaces. In the desktop GUI, projects are created without choosing a workspace root on disk, and environment injection defaults to all configured paths at once. Developers managing multi-tier architectures (e.g., monorepos with frontend and backend subdirectories) need to establish the workspace root directly upon creation, generate `.crypt-env.yaml` locally, and interactively select which destination path(s) to inject into.

See [proposal.md](proposal.md) for background and motivation.

## Goals / Non-Goals

**Goals:**
- Provide a directory picker/input in the GUI's project creation flow to specify the workspace root.
- Automatically write `.crypt-env.yaml` in the specified workspace root when a project is created from the GUI.
- Allow configuring multiple injection target paths per environment in the GUI.
- Introduce an interactive modal dialog when clicking "Inject" if an environment has multiple configured destination paths, asking the user where to inject.

**Non-Goals:**
- Storing decrypted secrets inside `.crypt-env.yaml`.
- Adding schema beyond `projects.root_path` (introduced by `cli-tui-parity`, Decision 8). `environments.paths` already stores a JSON array of strings.

## Architecture & UI Flow

```mermaid
flowchart TD
    subgraph GUI ["Desktop GUI (React)"]
        CreateModal["New Project Modal"]
        RootInput["Workspace Root Path Input / Picker"]
        EnvEditor["Environment Paths Editor (Multiple Paths)"]
        InjectBtn["Inject Button"]
        TargetModal["Inject Target Selection Modal"]
    end

    subgraph Backend ["Tauri Backend (Rust)"]
        CmdCreate["project_create_yaml / project_save"]
        CmdInject["environment_inject"]
    end

    subgraph Filesystem ["Host Filesystem"]
        YamlFile[".crypt-env.yaml (in Workspace Root)"]
        EnvTarget1["Path 1 (.env in ./frontend)"]
        EnvTarget2["Path 2 (.env in ./backend)"]
    end

    CreateModal --> RootInput
    RootInput -->|Tauri invoke| CmdCreate
    CmdCreate -->|write| YamlFile
    EnvEditor -->|save paths array| CmdCreate
    InjectBtn -->|if paths.length > 1| TargetModal
    TargetModal -->|selected path(s)| CmdInject
    CmdInject --> EnvTarget1
    CmdInject --> EnvTarget2
```

## Decisions

### 1. Workspace Root Selection in Project Creation Form
- **Decision**: Add a "Workspace Root Directory" field to `handleCreateProject` in `ProjectManager.tsx`:
  - Includes a text input with directory validation.
  - Includes a "Browse" button using Tauri's `@tauri-apps/plugin-dialog` or folder selector.
  - Defaults to current user directory or an empty prompt with validation.
- **Rationale**: Connects the GUI project to the actual local repository immediately.

### 2. Auto-Generating `.crypt-env.yaml` on Project Creation
- **Decision**: Expose a Tauri command `project_init_yaml` (or integrate into `project_save`):
  - Formats the project metadata and initial environment into standard YAML.
  - Writes `.crypt-env.yaml` into the root path provided.
  - If a `.crypt-env.yaml` already exists at that path, it prompts the user to overwrite or link without overwriting.
- **Rationale**: Keeps full parity with CLI `crypt-env init`.

### 3. Multiple Injection Paths per Environment
- **Decision**: The existing `Environment.paths` JSON column in SQLite supports `Vec<String>`. The environment edit view in `ProjectManager.tsx` will display a dynamic list of injection target paths with "Add Path" and "Remove Path" buttons.
- **Rationale**: Monorepos and microservice repos often require injecting variables into multiple subpackages (e.g. `./frontend/.env` and `./backend/.env`).

### 4. Interactive Injection Target Selection
- **Decision**:
  - When clicking "Inject" on an environment:
    - If `env.paths.length === 1`: Proceed directly to inject preview/execution.
    - If `env.paths.length > 1`: Display `InjectTargetModal` asking:
      `"Where do you want to inject environment variables?"`
      - Radio/checkbox options:
        - `[x] All configured paths`
        - `( ) ./apps/web/.env`
        - `( ) ./apps/api/.env`
      - "Inject" button triggers injection targeted to the selected subset.
- **Rationale**: Gives developers precise control over which subsystem receives secrets during active development.

### 5. Path Model (resolved 2026-09-27, shared with `cli-tui-parity` Decisions 8–9)
- The workspace root chosen at creation is stored in `projects.root_path` (as the GUI host sees it, e.g. `\\wsl.localhost\Ubuntu\home\u\app` for a WSL repo) and is editable later from the project form.
- Environment paths may be relative to the root (`.env`, `apps/api/.env`) or absolute. Paths picked with the file dialog that fall inside the root are stored relative; outside it they stay absolute.
- A new project's initial environment targets `.env` when a root is set, so it is injectable immediately.
- `.crypt-env.yaml` is generated by the backend from vault state (`project_write_yaml`) using the same serializer as the CLI; if it already exists the user chooses **Overwrite** or **Link without overwriting**.
- Injection into a subset: `environment_inject` gains an optional `targets` list (a subset of the environment's configured paths); unknown targets are rejected.

## Security & Threat Model

- **Safe Path Normalization**: All paths received from the frontend are normalized and canonicalized in Rust before write operations to prevent directory traversal exploits.
- **Zero Plaintext Secrets in Manifest**: The generated `.crypt-env.yaml` strictly contains project and environment metadata; it never contains secret values.
- **Protection for Pre-existing Files**: The existing grandfathered foreign file check (`environment_inject_preview`) remains active: if an injection target was not generated by CryptEnv, a `.bak` backup is made and user confirmation is required.

## Risks & Trade-offs

- **[Risk] Path inaccessible or permission denied on Windows/Linux**
  → *Mitigation*: Tauri command catches I/O errors and surfaces user-friendly toast messages in the GUI.
- **[Risk] User enters a relative path in GUI**
  → *Mitigation*: Resolve relative paths against the project root directory selected during creation.
