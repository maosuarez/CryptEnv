# CRYPTENV — Encrypted Local Secrets Manager

> **Source of Truth**: OpenSpec (`openspec/`) is the definitive, authoritative source of truth for all requirements, system specifications, architectural decisions, and tasks in this project. See `openspec/config.yaml` for project context and artifact rules, and `openspec/specs/` for living capability specifications.

---

## Agent Role
You are a senior software engineer working on a Tauri 2.0 desktop application on Windows. Your stack is Rust (backend) + React + TypeScript (frontend). You prioritize security, clean code, and justified decisions. You always respond in English.

---

## Spec-Driven Development (SDD) — OpenSpec as Source of Truth

All development, feature design, behavior changes, and non-trivial refactoring in this repository follow **Spec-Driven Development** with OpenSpec.

### Core Structure:
- **`openspec/config.yaml`**: Contains global project context, quality gates, per-artifact rules (`proposal`, `specs`, `design`, `tasks`), and operational guidance.
- **`openspec/specs/<capability>/spec.md`**: Living capability specifications defining what the system MUST do (using RFC 2119 keywords: `MUST`, `MUST NOT`, `SHOULD`).
- **`openspec/changes/<change-name>/`**: Active or archived delta changes containing:
  - `proposal.md` (what & why, scope, non-goals)
  - `specs/<capability>/spec.md` (delta requirement changes)
  - `design.md` (architecture decisions, security review, trade-offs)
  - `tasks.md` (verifiable implementation steps)

### Standard Agent Workflow:
1. **Explore (`openspec-explore`)**: Think through problems, clarify ambiguity, and investigate the codebase without modifying code.
2. **Propose (`openspec-propose` / `openspec new change <name>`)**: Generate planning artifacts (`proposal.md`, `specs/`, `design.md`, `tasks.md`). *Do NOT implement code during the planning phase.*
3. **Apply (`openspec-apply-change`)**: Implement the tasks strictly according to the approved change plan, validating compilation (`cargo check`) and test suites after each step.
4. **Sync / Archive (`openspec-archive-change` / `openspec-sync-specs`)**: Sync verified delta specifications into living capability specs in `openspec/specs/` and archive completed changes.

---

## Work Rules

### General & Architecture
- OpenSpec is the source of truth; never implement unsolicited features or changes outside the agreed change plan.
- Never make architectural decisions without first explaining the options and trade-offs in `design.md` or chat.
- If a dependency or pattern could cause problems on Windows/Linux/macOS, highlight it before proceeding.
- Maintain documentation integrity: preserve all existing comments and docstrings unless explicitly changing that functionality.

### Security (Critical Mandates)
- **Zero Plaintext Secrets**: Secret values **must never** appear in logs, error messages, or API responses in plaintext.
- **Volatile Memory Only**: The master password and derived encryption keys exist in memory only during active unlocked sessions and are zeroized on drop / lock.
- **MCP Security**: The MCP server **never returns secret values**; it injects them directly as environment variables.
- **Localhost REST**: The local REST API binds strictly to `127.0.0.1:47821` and requires token authentication with constant-time comparison (`subtle::ConstantTimeEq`).

### Rust (Backend)
- Error handling with `Result` and strongly typed custom errors — `unwrap()` and `expect()` are forbidden in production code.
- Decoupled modules: `db` does not know about `api`, `vault` orchestrates both.
- Tauri commands are registered in `lib.rs` with naming: `module_action` (e.g., `vault_get_items`, `project_save`).

### Frontend (React / TypeScript)
- Communication with Rust exclusively via Tauri `invoke()` — never `fetch()` to localhost from React.
- Global state with Zustand, async queries with TanStack Query.
- Pure Tailwind CSS utility classes and semantic tokens in `src/index.css` — no CSS modules or inline styles.
- Decorationless window: custom React titlebar (`WindowChrome.tsx`) with window controls.

---

## Useful Commands

```powershell
# Development
pnpm tauri dev

# Production build
pnpm tauri build

# Frontend only
pnpm dev

# Check Rust compilation
cd src-tauri && cargo check

# Run Rust tests
cd src-tauri && cargo test

# OpenSpec operations
openspec doctor
openspec list --json
openspec new change <name>
openspec status --change <name>
```

---

## UI Design
Industrial/utilitarian aesthetic with dark palette and technical typography (Carbon-inspired design language). Navigation is footer-based (text links over icon rail per user preference). Main screens:
1. **Lock Screen** — Master password entry + biometric unlock option (Windows Hello)
2. **Projects & Environments** — Primary landing page (project list → project detail with environments → environment editor for variable linking)
3. **Global Secrets** — Filtered view of reusable `isGlobal` items (footer link back to projects)
4. **Add/Edit Item** — Dynamic form by item type (secret, credential, link, command, note)
5. **Category Manager** — CRUD of editable categories
6. **Settings** — Master password, timeout, biometric, projects/environments management, internet relay config, backup/restore, import

---

## Core Features Reference

### 1. Projects & Environments
Hierarchical organization for environment variables. Projects contain multiple typed Environments; every environment variable is a real encrypted vault item.
- `src-tauri/src/project/mod.rs` — Project/environment business logic
- `src-tauri/src/db/mod.rs` — SQLite tables: `projects`, `environments`, `environment_vars`, `item_projects`, `project_categories`
- `items.is_global` plaintext column tracks reusability across projects; `item_projects` tracks ownership

### 2. CLI & Interactive TUI (`crypt-env`, `crypt-env tui`)
Terminal user interface for vault management without opening the GUI.
- Source: `src-tauri/src/bin/crypt-env/commands/tui.rs` (ratatui 0.29 + crossterm 0.28)
- Keybindings: `↑`/`↓`/`j`/`k` (navigate), `/` (fuzzy search), `Enter` (detail), `v` (reveal), `c` (copy), `d` (delete), `?` (help), `q` (quit).
- Client config: `CRYPTENV_API_URL`, `CRYPTENV_CERT_PATH`, `CRYPTENV_TOKEN_PATH` env vars (endpoint / TLS anchor / token path) resolved in `src-tauri/src/bin/crypt-env/client.rs`; `crypt-env setup wsl` (`commands/setup.rs`) persists them into the shell. See [`docs/cli.md`](docs/cli.md) and the WSL ↔ Windows topology guide [`docs/wsl-windows.md`](docs/wsl-windows.md).

### 3. Internet Relay Sharing
Secure ephemeral secret sharing via Supabase table (AES-256-GCM + Argon2id passphrases + burn-after-read + 24-hour TTL).
- `src-tauri/src/share/relay.rs` — Relay protocol implementation
- Tauri commands: `share_relay_send`, `share_relay_receive`
