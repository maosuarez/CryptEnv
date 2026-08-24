# crypt-env

> **Source of Truth**: OpenSpec (`openspec/`) is the definitive, authoritative source of truth for all system requirements, capability specifications, change proposals, and architectural decisions. See `openspec/config.yaml` for global rules and context, `openspec/specs/` for baseline capability specifications, and `openspec/changes/` for proposed/in-flight delta changes. This document (`context.md`) serves as a technical architecture overview and historical reference.

## Description
Personal productivity vault for developers. Centralizes credentials, API keys, tokens, passwords, links, commands, and notes in a local desktop app accessible by hotkey (Ctrl+Alt+Z). Secrets are stored encrypted locally. Includes CLI, local REST API, and MCP server for integration with external tools.

## Stack
- **Specification & Planning**: OpenSpec (Spec-Driven Development) in `openspec/`
- **Frontend**: React 19 + TypeScript + Vite + Tailwind CSS + Framer Motion
- **Backend (Rust)**: Tauri 2.0, Axum (local REST), Tokio
- **Database**: SQLite with `libsqlite3-sys` bundled (no SQLCipher; see Decision #1)
- **Encryption**: AES-256-GCM for sensitive fields, Argon2id for master password, `subtle::ConstantTimeEq` for timing-safe comparisons
- **CLI**: Terminal interface for item management without opening the GUI (binary `crypt-env`)
- **MCP**: Model Context Protocol server for secure secret queries (binary `crypt-env-mcp`)
- **REST API**: Axum on `127.0.0.1:47821` with dual authentication (session token + MCP token)
- **Target OS**: Windows (development), multi-platform in the future
- **Package manager**: pnpm

## Architecture
```
crypt-env/
├── openspec/                     # OpenSpec Single Source of Truth (specs, changes, config)
│   ├── config.yaml               # SDD rules, context, and operational guidance
│   ├── specs/                    # Baseline living capability specifications
│   └── changes/                  # Active and archived delta change plans
├── src/                          # React frontend
│   ├── components/               # UI components by screen (includes ProjectManager, GlobalSecrets, ShareModal, itemFields/)
│   ├── store/                    # Global state with Zustand (includes projectStore)
│   ├── hooks/                    # Custom hooks for Tauri invoke()
│   └── types/                    # Shared TypeScript types
├── src-tauri/
│   ├── src/
│   │   ├── main.rs               # Tauri entrypoint (initializes lib)
│   │   ├── lib.rs                # Tauri command registry, AppState setup
│   │   ├── db/mod.rs             # SQLite pool, tables, CRUD items/categories/settings/projects/environments
│   │   ├── crypto/mod.rs         # Argon2id KDF + AES-256-GCM encrypt/decrypt
│   │   ├── vault/mod.rs          # VaultState, Tauri commands for vault management
│   │   ├── project/mod.rs        # Project/environment business logic (shared by Tauri + HTTP API)
│   │   ├── wsl/mod.rs            # WSL bridge (Phase 1): distro discovery + UNC seed dir for the env path picker
│   │   ├── api/mod.rs            # Axum server on 127.0.0.1:47821, dual token auth
│   │   ├── share/mod.rs          # Secure secret sharing (LAN bridge + encrypted packages)
│   │   ├── share/relay.rs        # Internet relay sharing (Supabase-based)
│   │   ├── cli/mod.rs            # CLI module (stub)
│   │   ├── mcp/mod.rs            # MCP module (stub)
│   │   └── bin/
│   │       ├── crypt-env.rs      # CLI standalone (clap), connects via HTTP to API
│   │       │   ├── commands/project.rs  # Project/environment CLI subcommands
│   │       │   └── commands/tui.rs     # Interactive TUI (ratatui-based)
│   │       └── crypt-env-mcp.rs  # MCP JSON-RPC 2.0 server over stdio
│   ├── Cargo.toml                # Rust dependencies
│   └── tauri.conf.json           # Window config, permissions, hotkey
├── pnpm-workspace.yaml           # pnpm monorepo config (esbuild disabled)
```

**Communication**:
- Frontend → Tauri `invoke()` → registered Rust commands
- CLI (`crypt-env`) → HTTP REST to `127.0.0.1:47821` with session/MCP token
- MCP (`crypt-env-mcp`) → HTTP REST to `127.0.0.1:47821` with MCP token
- TUI (`crypt-env tui`) → Direct vault access (Tauri command context, no HTTP)

**New Tauri Commands** (Session 4+):
- Projects/Environments: `project_list`, `project_save`, `project_delete`, `project_preview_delete`, `environment_save`, `environment_delete`, `environment_inject`, `vault_create_project_item`, `vault_set_item_global`, `vault_get_item_owners`
- Internet Relay: `share_relay_send`, `share_relay_receive`
- WSL bridge (issue #3, Phase 1, Windows-only affordance): `wsl_list_distros() → Vec<String>` (always `Ok`, empty on non-Windows/no-WSL), `wsl_distro_home(distro: String) → Result<String, String>` (seed dir for the env path picker). `project_pick_env_path` gained an optional `start_dir: Option<String>` argument to seed the native dialog at that directory

## Vault Item Types
1. **Secret / API Key**: name, encrypted value, category, notes. Export as `.env` / `export` / `$env:`
2. **Credential**: site name, URL, username, encrypted password, notes
3. **Link**: title, URL, description, category
4. **Command**: name, command, description, shell target (bash/zsh/sh/PowerShell), placeholders `{{VAR}}`
5. **Note**: title, free-form content, category

## UI Design

The interface follows an **industrial/utilitarian aesthetic** with a **dark color palette** and **monospace typography**, inspired by IBM Carbon Design System principles implemented purely in Tailwind CSS (no Carbon React library).

### Design Language
- **Color Palette**: Dark background with high-contrast accent colors (emerald for success, red for destructive, amber for warnings)
- **Typography**: Monospace fonts (IBM Plex Mono) for values and code, sans-serif (Inter) for UI labels
- **Spacing & Layout**: Geometric grid with consistent padding/margins (4px base unit, multiples of 4)
- **Components**: Carbon-like card, button, modal, input, dropdown, badge styles — all built with Tailwind classes

### Semantic Token System
Tailwind configuration in `src/index.css` defines semantic tokens:
- **Surface tokens** (`bg-surface`, `surface-hover`, `surface-active`): Layered card backgrounds
- **Interactive tokens** (`interactive-primary`, `interactive-secondary`, `interactive-danger`): Buttons and clickable elements
- **Text tokens** (`text-ui`, `text-secondary`, `text-disabled`, `text-critical`): Hierarchical text styling
- **Border & stroke tokens**: Consistent edge styling

### Navigation & Main Screens
1. **Lock Screen** — Master password input with biometric unlock option (Windows Hello)
2. **Projects & Environments** — Primary landing page (project list → project detail with environment cards → environment editor for linking variables)
3. **Global Secrets** — Filtered view of reusable `isGlobal` items (accessed via footer link from Projects screen)
4. **Add/Edit Item** — Dynamic form that changes based on item type (API Key, Credential, Link, Command, Note)
5. **Category Manager** — CRUD interface for editable categories with color picker
6. **Settings** — Master password, timeout, biometric, projects/environments, internet relay config, backup/restore, import

### Decorationless Window
- The Tauri window is configured with `decorations: false` — no OS titlebar
- Custom React titlebar component (`WindowChrome.tsx`) with window controls (minimize, maximize, close)
- Allows unified, branded window frame consistent across platforms

### Responsive Design
- Fixed window size on launch, resizable by user
- UI scales with Tailwind breakpoints for future mobile/tablet support
- All form inputs and lists scroll gracefully within viewport

## Core Features Implemented (Session 4+)

### Projects & Environments (formerly Workspaces)
Hierarchical organization for managing environment variable sets by project and environment (dev, staging, production, etc.). Every environment variable is a real encrypted vault item.

- **Purpose**: Organize vault items by project/context (Node.js, PostgreSQL, Docker, Python, etc.) with multiple typed environments per project. Global items reusable across projects; project-specific items forked on un-globalization.
- **Data Model**: 
  - `projects` table: id, name, description, template, created, updated
  - `environments` table: id, project_id (FK), name, is_default, created, updated
  - `environment_vars` table: id, environment_id (FK), key, item_id (FK to items, mandatory)
  - `environment_paths` table: environment_id (FK), path (for .env file exports)
  - `item_projects` table: item_id (FK), project_id (FK) — many-to-many ownership
  - `project_categories` table: project_id (FK), category_id (FK) — projects carry tags
- **Item Ownership**: `items.is_global` plaintext column. Global items in Global Secrets screen; project-specific items visible only in their projects.
- **Stack Templates**: generic, node, postgres, mongo, docker, python (scaffolds, not locked)
- **Inject Action**: Writes decrypted KEY=VALUE pairs to configured paths per environment (multiple paths per environment supported)
- **Frontend**: 
  - `ProjectManager.tsx` — New landing page (project list → detail with environments → edit environment with variable linking)
  - `GlobalSecrets.tsx` (renamed from MainVault.tsx) — Filtered to `isGlobal` items, footer link to return to projects
  - `src/components/itemFields/` — Extracted shared per-type field components for reuse
  - `projectStore.ts` (replaced workspaceStore.ts)
- **Tauri Commands**: 
  - `project_list() → Vec<Project>` — All projects with environments and var counts
  - `project_save(project) → Project` — Create or update
  - `project_delete(id) → bool` — Delete with cascade
  - `project_preview_delete(id) → ProjectDeleteImpact` — Show impact before delete
  - `environment_save(env) → Environment` — Create or update environment
  - `environment_delete(id) → bool` — Delete environment
  - `environment_inject(id) → InjectResult` — Write decrypted vars to paths
  - `vault_create_project_item(project_id, key, item) → VaultItem` — Add variable
  - `vault_set_item_global(id, is_global) → GlobalToggleResult` — Make global or fork
  - `vault_get_item_owners(id) → Vec<ItemOwner>` — List projects owning this item

### Interactive TUI (`crypt-env tui`)
Terminal user interface for vault access without GUI, built with ratatui.

- **Purpose**: Vault management from terminal (useful for SSH sessions, CI/CD pipelines, minimal environments)
- **Invocation**: `crypt-env tui` (subcommand of CLI binary)
- **Source**: `src-tauri/src/bin/crypt-env/commands/tui.rs`
- **Dependencies**: ratatui 0.29, crossterm 0.28
- **Screens**:
  1. **Unlock** — Master password input with masked echoing
  2. **Main** — Item list with preview pane, fuzzy search, type/category filters
  3. **Item Detail** — Full item metadata, reveal secret, copy to clipboard
  4. **Help** — Keybinding reference
  5. **Confirm** — Destructive action confirmation (delete item)
- **Keybindings**:
  - `↑` / `↓` or `j` / `k` — Navigate list
  - `/` — Start fuzzy search (incremental, real-time filtering)
  - `Enter` — View item detail
  - `v` — Reveal encrypted secret value in current item
  - `c` — Copy selected secret to system clipboard
  - `d` — Delete selected item (with confirmation)
  - `r` — Refresh item list from DB
  - `?` — Show help
  - `q` — Quit (lock vault, exit)
- **Architecture**: No external REST calls; TUI runs as privileged subprocess with direct vault access (Tauri command context)

### Internet Relay Sharing (Supabase)
Secure sharing of vault items across networks via encrypted relay.

- **Purpose**: Share secrets with teammates on different networks without WhatsApp/email exposure
- **Scenario**: Sender selects items, receiver enters code + passphrase, items are securely transferred and imported
- **Architecture**:
  - Module: `src-tauri/src/share/relay.rs`
  - Backend: Supabase PostgreSQL table with burn-after-read + 24h TTL
  - Encryption: AES-256-GCM for payload, Argon2id(m=32768, t=2, p=2) KDF from passphrase
  - Pairing: 4-digit code (XXXX-XXXX format) + random passphrase (12 alphanumeric chars)

- **Send Flow**:
  1. Sender calls `share_relay_send(item_ids: Vec<i64>)` 
  2. Backend: Encrypts selected items (JSON) with AES-256-GCM using random key
  3. Generates Argon2id KDF from random passphrase + random salt
  4. Uploads encrypted payload + salt + nonce to Supabase relay table
  5. Generates code: pairing_token hash → 4-digit code
  6. Returns to sender: code + passphrase (shown once, stored nowhere)

- **Receive Flow**:
  1. Receiver calls `share_relay_receive(code: String, passphrase: String)`
  2. Backend: Looks up relay record by code hash
  3. Derives AES key from passphrase + stored salt using Argon2id
  4. Decrypts payload → deserializes items
  5. Imports items to vault (same mechanism as backup import)
  6. Deletes relay record (burn-after-read)
  7. Returns import summary to receiver

- **Database Setup**: 
  - Supabase project required (free tier sufficient)
  - SQL to run in Supabase:
    ```sql
    CREATE TABLE relay (
        id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
        code_hash TEXT UNIQUE NOT NULL,
        encrypted_payload BYTEA NOT NULL,
        salt BYTEA NOT NULL,
        nonce BYTEA NOT NULL,
        created_at TIMESTAMP DEFAULT now(),
        expires_at TIMESTAMP DEFAULT now() + INTERVAL '24 hours',
        accessed BOOLEAN DEFAULT FALSE
    );
    CREATE INDEX idx_relay_code_hash ON relay(code_hash);
    CREATE INDEX idx_relay_expires_at ON relay(expires_at);
    ```
  - Supabase auto-cleanup: SQL trigger to delete rows where `expires_at < now()`

- **Configuration**: Settings → Internet Sharing
  - Input: Supabase project URL, anon API key
  - Validation: Test connectivity on save

- **Frontend**: `ShareModal` component with tabs:
  - **LAN Tab**: Existing mDNS-based sharing
  - **INTERNET Tab**: Code entry (receiver) / code display (sender) + passphrase display

- **Tauri Commands**:
  - `share_relay_send(item_ids: Vec<i64>) → { code: String, passphrase: String, expires_in: u64 }` — Encrypt and upload
  - `share_relay_receive(code: String, passphrase: String) → { imported_count: usize, items: Vec<ItemSummary> }` — Download and decrypt

- **Security Decisions**:
  - Passphrase is random and shown once (not derived from password; user cannot recover it if lost)
  - Code + passphrase required to prevent accidental discovery (code alone is insufficient)
  - 24h TTL prevents indefinite relay accumulation
  - Burn-after-read prevents reuse and auditing of past transfers
  - AES-256-GCM provides authenticated encryption (integrity check prevents tampering)
  - Argon2id with high memory cost (32MB) makes brute-forcing the passphrase expensive (~1-2s per attempt)
  - Supabase URL + key stored in plaintext settings (acceptable because they only control relay table, not the vault itself)

- **Consequences**:
  - Requires active internet connection (not usable in offline/air-gapped scenarios; use LAN mode or encrypted package mode)
  - Depends on Supabase service availability (single point of failure if Supabase is down)
  - Code is short (XXXX-XXXX) for human typing; ~10,000 possible values (brute-forceable in ~10 minutes on 1000 req/sec, but code expires in 5 minutes)
  - Passphrase must be communicated separately (out-of-band via chat/call/email); Supabase stores nothing about passphrase

## Security — Decisions Made
- **Master password** derived with Argon2id (m=65536, t=3, p=4), never stored in plaintext
- **Sensitive values** encrypted with AES-256-GCM before writing to SQLite
- **Timing-safe comparisons** using `subtle::ConstantTimeEq` for tokens and verify_token
- **Key in memory** stored in `Zeroizing<[u8;32]>` which automatically overwrites on Drop
- **MCP does not expose values directly**: injects secrets as environment variables in the client process, without returning them as text
- **Local REST API** listens only on `127.0.0.1:47821`, never on external interfaces
- **Dual authentication**: session tokens (with expiration) + MCP token (static, stored in `%APPDATA%`)
- **Window auto-locks** after configurable timeout, setting `VaultState.key = None`

## Conventions
- Code language: English (variables, functions, comments)
- Agent response language: English
- Rust naming: snake_case. React/TS naming: camelCase, PascalCase for components
- Tauri commands (`invoke`) are named with module prefix: `vault_get_items`, `crypto_unlock`, etc.
- Do not use `unwrap()` in production — handle errors with `Result` and custom error types
- Tailwind for styles, no CSS modules or styled-components

## Constraints
- Dependencies in `src-tauri/Cargo.toml` are **pending**: configured in Claude Code's first session
- Do not implement cloud synchronization in this version
- Do not assume SQLCipher compiles frictionlessly on Windows — have Plan B (AES-GCM over standard SQLite)
- The window is **decorationless** (no OS titlebar), with custom titlebar in React
- Do not keep master password in memory longer than necessary to unlock
- MCP is read-only — does not allow creating or modifying items

## Business Context
Developer user needs quick access (hotkey), ease of copying to clipboard, and ability to use secrets as environment variables without visual exposure. Can now securely share secrets with teammates via encrypted LAN bridge (mDNS discovery + ECDH key exchange) or encrypted packages for offline scenarios, eliminating insecure communication channels like WhatsApp.

---

## Implementation Decisions

### 1. SQLite + AES-GCM Instead of SQLCipher (Plan B)
**Context**: SQLCipher requires OpenSSL/vcpkg with complex configuration on Windows, generating linking errors during compilation.

**Decision**: Adopt Plan B: Standard SQLite with `libsqlite3-sys` bundled + AES-256-GCM encryption at application level.

**Rationale**: 
- Avoids OpenSSL compilation on Windows (high friction, costly maintenance)
- Sensitive fields (`data` in `items`) are encrypted before writing to DB
- The DB file on disk is not encrypted at file level, but item secrets are protected by AES-256-GCM
- Categories and settings are stored plaintext (not confidential metadata)
- Allows future integration with larger-scale databases

**Consequences**:
- If the `vault.db` file is accessed directly without running the application, data remains encrypted at field level
- Assumes physical control of the machine (local Windows, single user) — not a defense against direct memory attacks
- The AES key derived only exists in memory during the active session

---

### 2. Decoupled Rust Module Structure
**Context**: Need to separate responsibilities between crypto, persistence, API, and business logic.

**Decision**: 
- `crypto/mod.rs`: Argon2id KDF, AES-256-GCM encrypt/decrypt, key management in `Zeroizing`
- `db/mod.rs`: SQLite pool, DDL of tables, CRUD of items/categories/settings (does not know about `api`, `vault`)
- `vault/mod.rs`: `VaultState` (orchestrator), 19 Tauri commands including unlock/lock, backup/import, and settings
- `api/mod.rs`: Axum REST server, 14 endpoints, dual token authentication

**Rationale**: Each module has a clear responsibility. `vault` orchestrates between `crypto` and `db` without them knowing each other.

**Consequences**: 
- The REST API (`api/mod.rs`) also uses the same underlying modules
- CLI and MCP communicate with the backend via HTTP REST; no direct Rust linkage

---

### 3. MCP Token Storage in File
**Context**: MCP server needs token to authenticate calls to the REST API; requires persistence between sessions (no expiration).

**Decision**: 
- MCP Token: 32 bytes randomly generated with `rand::thread_rng()`, saved in `vault_meta.mcp_token` (DB)
- Redundant copy in `%APPDATA%\com.maosuarez.cryptenv\mcp_token` (plaintext file)
- Generated only once with `vault_generate_mcp_token` when MCP is started for the first time
- No expiration, valid while the vault is unlocked

**Rationale**: 
- Allows MCP server to read its token without need to unlock interactively
- File in `%APPDATA%` avoids having to read from DB each time
- Token verification in REST API uses `subtle::ConstantTimeEq`

**Consequences**:
- The `mcp_token` file in `%APPDATA%` needs restrictive permissions (ideally 0600, on Windows: owner only)
- If that file is compromised, anyone can make calls to MCP

---

### 4. Database Schema (SQLite in `%APPDATA%`)
**Context**: Need to store encrypted items, categories, crypto metadata, and settings.

**Decision**: 4 tables in `vault.db` located at `%APPDATA%\com.maosuarez.cryptenv\vault.db`:

```sql
CREATE TABLE vault_meta (
    id            INTEGER PRIMARY KEY CHECK(id = 1),
    kdf_salt      TEXT NOT NULL,
    verify_token  TEXT NOT NULL
);
-- Stores crypto material: kdf_salt (hex, 32 bytes) and verify_token (AES-GCM encrypted)

CREATE TABLE items (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    item_type TEXT NOT NULL,  -- 'secret', 'credential', 'link', 'command', 'note'
    data      TEXT NOT NULL,  -- JSON encrypted with AES-GCM
    created   TEXT NOT NULL,  -- Unix epoch seconds
    updated   TEXT NOT NULL   -- Unix epoch seconds
);

CREATE TABLE categories (
    cid   TEXT PRIMARY KEY,
    name  TEXT NOT NULL,
    color TEXT NOT NULL
);
-- Categories stored plaintext with id, name, and color (not encrypted at DB level)

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- Keys: auto_lock_timeout, hotkey, mcp_token (plaintext, user configuration only)
```

**Rationale**:
- `vault_meta`: stores salt (public) and verify_token (private, encrypted) for key derivation with single row constraint
- `items.data`: JSON serialized and encrypted (avoids individual columns)
- `categories`: plaintext columns for UI efficiency (category metadata is not confidential)
- `settings`: plaintext (contains no secrets, only user configuration)

**Consequences**:
- The `items` table grows indefinitely; indexing by `id` and `item_type` recommended for future searches
- Encrypted JSON requires deserialization post-decryption in the application

---

### 5. Unlock Flow and Key Management in Session
**Context**: The AES key must only exist in memory during the active session; must be destroyed when locking.

**Decision**:

1. **First initialization** (`init_vault_crypto`):
   - Generates 32-byte `salt` with `rand::thread_rng()`
   - Derives AES key with Argon2id(m=65536, t=3, p=4) from password + salt
   - Encrypts `b"vault_ok_v1"` as `verify_token` with AES-256-GCM
   - Saves `salt` and `verify_token` in `vault_meta`
   - Saves key in `VaultState.key` as `Zeroizing<[u8;32]>`

2. **Unlock** (`unlock_vault_crypto`):
   - Reads `salt` and `verify_token` from `vault_meta`
   - Re-derives key with Argon2id
   - Attempts to decrypt `verify_token` → if OK, password is correct
   - Saves key in `VaultState.key`
   - Generates session token (32 bytes hex)
   - Returns token to client

3. **Lock**:
   - Sets `VaultState.key = None`
   - The `Zeroizing` automatically overwrites the 32 bytes on Drop

**Rationale**:
- `Zeroizing` is mandatory to prevent the key from persisting on heap between sessions
- Argon2id with high parameters (m=65536) makes brute-force very costly
- `verify_token` allows detecting incorrect password without decrypting all items

**Consequences**:
- Unlock time is ~200-500ms (by design, Argon2id is slow)
- If the process is abruptly killed, the key may not be overwritten (defense against DMA attacks is not possible in Windows user-mode)

---

### 6. REST Authentication: Session Token vs MCP Token
**Context**: REST API must authenticate requests; session tokens expire, MCP token is persistent.

**Decision**:

- **Session token**: 32 bytes hex, generated on `/unlock`, valid for `auto_lock_minutes` (Instant + Duration on server)
  - Header: `X-Vault-Token: <hex32>`
  - Expires automatically
  - Used by CLI and frontend (via Tauri `invoke`)

- **MCP token**: 32 bytes hex, generated once, no expiration
  - Header: `X-Vault-Token: <hex32>` (same header)
  - Constant-time verification with `subtle::ConstantTimeEq`
  - Used only by MCP server
  - Allows MCP to function without explicit unlock interface

**Rationale**: 
- Two separate channels: session (ephemeral, UI) vs MCP (persistent, backend)
- MCP can function without GUI interface
- Expiration prevents reuse of exfiltrated tokens

**Consequences**:
- Server must maintain `HashMap<String, Instant>` of active tokens
- Periodic cleanup of expired tokens recommended

---

### 7. Implemented REST Endpoints
**Context**: REST API on `127.0.0.1:47821` as unified interface for CLI, MCP, and Tauri.

**Decision**: Implement RESTful endpoints with dual authentication:

| Method | Route | Auth | Description |
|--------|-------|------|-------------|
| POST | `/unlock` | - | Validates password, returns session token |
| GET | `/items` | token | Lists items (no sensitive fields) |
| POST | `/items` | token | Creates new item |
| GET | `/items/:id` | token | Gets item (no encrypted values) |
| PUT | `/items/:id` | token | Updates item |
| DELETE | `/items/:id` | token | Deletes item |
| POST | `/items/:id/reveal` | token | Decrypts and returns sensitive value (only endpoint that does this) |
| GET | `/categories` | token | Lists categories |
| GET | `/settings` | token | Returns settings (no secrets) |
| PUT | `/settings` | token | Updates settings |
| GET | `/commands` | token | Lists available commands (MCP read-only) |
| GET | `/commands/:id` | token | Gets command details with placeholders |
| POST | `/fill` | token | Fills .env template with secret values (writes to disk, not response) |
| GET | `/health` | - | Health check (vault lock state, item count, version) |

**Rationale**:
- `/unlock` without token (entry point)
- `/items/:id/reveal` is the only endpoint returning secrets in plaintext (justifiable because it requires valid session token)
- Responses never include encrypted values in plaintext (only metadata JSON)

**Consequences**:
- CLI must make 2 calls: `/unlock` + then authenticated requests
- MCP makes initial `/unlock` or reuses MCP token directly
- Audit of `/items/:id/reveal` calls recommended (can log accesses)

---

### 8. CLI (`crypt-env` Binary)
**Context**: Standalone tool for management without GUI, written in Rust + clap, connects via HTTP REST.

**Decision**: Binary `src-tauri/src/bin/crypt-env/main.rs` that:
- Uses `clap` for argument parsing
- Connects via HTTP to `127.0.0.1:47821` (requires vault GUI running)
- Authenticates with session token from REST `/unlock` endpoint
- Supports commands:
  - `add` — Import secrets from KEY=value, environment variables, or .env files
  - `doctor` — Check app health, vault status, token files, and version
  - `fill` — Fill .env or .env.example templates with vault secrets
  - `inject` — Print shell-compatible variable assignment (safe for `eval`)
  - `list` — Display saved shell commands in a table
  - `exec` — Execute a saved command by name
  - `memory` — Save a command string to the vault (interactive)
  - `search` — Search items by name (no secret values exposed)
  - `set` — Print environment variable assignment for a secret
  - `cmd` — Manage saved commands (list, info, run)

**Rationale**: Decoupled CLI from REST server allows independent control; token storage avoids re-authentication.

**Consequences**:
- Token in `cli_session_token` file needs restrictive permissions (0600)
- If API server is inactive, CLI should be able to start it (possible future feature)

---

### 9. MCP Server (`crypt-env-mcp` Binary)
**Context**: Model Context Protocol server for AI agent integration, communication via JSON-RPC 2.0 over stdio.

**Decision**: Binary `src-tauri/src/bin/crypt-env-mcp.rs` that:
- Reads MCP token from `%APPDATA%\com.maosuarez.cryptenv\mcp_token`
- Connects via HTTP REST to `127.0.0.1:47821`
- Implements JSON-RPC tools (all prefixed `crypt_env_`):
  - `crypt_env_list_items` — lists items without secrets, with type/category filters
  - `crypt_env_get_item` — gets item metadata by ID
  - `crypt_env_search_items` — searches items by name (no values)
  - `crypt_env_generate_env` — writes `.env` to disk with secret values (path in response, not values)
  - `crypt_env_inject_env` — injects a secret as environment variable into MCP process
  - `crypt_env_add_item` — creates new vault item
  - `crypt_env_update_settings` — modifies auto_lock_timeout and hotkey
  - `crypt_env_fill_env` — fills .env.example template with real values to disk
  - `crypt_env_doctor` — health check (status, vault lock state, item count, version)
  - `crypt_env_list_commands` — lists saved shell commands with placeholders
  - `crypt_env_run_command` — executes command with resolved placeholders; returns `{ exit_code, stdout, stderr }` (secrets never in response)

**Rationale**:
- Standard MCP protocol allows integration with any compatible client
- Does not return secrets in plaintext, only injects as environment variables
- Persistent MCP token allows functioning without explicit unlock interface

**Consequences**:
- If `mcp_token` is compromised, MCP can be accessed remotely (if listening on network, outside current scope)
- `vault_inject_env` requires strict name validation (prevent injection)

---

### 10. File Location on Windows
**Context**: Need to store DB, tokens, configuration persistently and securely.

**Decision**: Use `%APPDATA%\com.maosuarez.cryptenv\` as base directory:

```
%APPDATA%\com.maosuarez.cryptenv\
├── vault.db                    # SQLite DB (AES-GCM encryption at field level)
├── mcp_token                   # MCP token (plaintext, permissions 0600)
├── cli_session_token           # CLI session token (plaintext, permissions 0600)
└── logs/                        # (future) Access audit
```

**Rationale**: 
- `%APPDATA%` is standard for user data on Windows (roameable on domain)
- Subdirectory `com.maosuarez.cryptenv` prevents conflicts with other applications
- Token in file rather than memory-only facilitates access by CLI/MCP without GUI server

**Consequences**:
- If user account is compromised, tokens are also compromised
- Encryption at OS level (NTFS EFS) optional but not implemented

---

### 11. Secure Secret Sharing (LAN Bridge + Encrypted Packages)
**Context**: Users need to securely share secrets with teammates without exposing plaintext in WhatsApp, email, or other channels. Two scenarios exist: (1) both users on same LAN with ability to perform real-time key exchange, and (2) offline scenario requiring a self-contained encrypted file.

**Decision**: Implement two complementary sharing modes:

1. **LAN Bridge Mode** (for local network):
   - Sender initiates session with `POST /share/listen` → returns 6-digit pairing code (5-minute expiration)
   - Receiver initiates session with `POST /share/connect <pairing_code>` → gets sender's public key and fingerprint (first 8 hex chars of SHA-256(sender_pub || receiver_pub))
   - Both sides confirm fingerprint match via `POST /share/confirm`
   - Sender selects items and sends via `POST /share/items` encrypted with HKDF-SHA256 derived key (X25519 ECDH shared secret + info=`b"cryptenv-share-v1"`)
   - Session auto-destroys on completion, cancellation, or 30-second inactivity
   - All encryption uses AES-256-GCM on length-prefixed JSON messages over TCP

2. **Encrypted Package Mode** (for offline/non-LAN):
   - Sender exports items via `POST /share/export <item_ids>` with Argon2id(m=32768, t=2, p=2) KDF from random 12-char passphrase
   - Returns `.vault` JSON package: `{ version, salt, nonce, ciphertext }` + plaintext passphrase (shown once to sender)
   - Receiver imports via `POST /share/import` with passphrase (entered manually from sender)
   - Passphrase never stored, encrypted package is self-contained and portable

3. **Shared module structure** (`src-tauri/src/share/`):
   - `crypto.rs`: X25519 keypair generation, HKDF-SHA256 shared key derivation, AES-256-GCM channel encryption, 12-char passphrase generation, fingerprint computation
   - `lan.rs`: mDNS service discovery (`_cryptenv._tcp.local.`), TCP listener, ECDH handshake with pairing code verification
   - `package.rs`: `.vault` package format (JSON), PlainItem struct for export, Argon2id KDF for package encryption
   - `protocol.rs`: Length-prefixed JSON messages, ShareMessage enum (Hello, Confirm, Items, Ack, Error)
   - `mod.rs`: ShareState, ShareSession, ShareSessionState, ShareDirection state machine

4. **Database audit** (`share_log` table):
   ```sql
   CREATE TABLE share_log (
       id        INTEGER PRIMARY KEY AUTOINCREMENT,
       mode      TEXT NOT NULL,  -- 'lan' or 'package'
       direction TEXT NOT NULL,  -- 'sent' or 'received'
       item_ids  TEXT NOT NULL,  -- JSON array of shared item IDs
       peer_fp   TEXT,           -- Peer fingerprint (LAN mode only)
       timestamp TEXT NOT NULL   -- ISO 8601 timestamp
   );
   ```

5. **New REST endpoints** (all require auth except noted):
   - `POST /share/listen` → `{ pairing_code, expires_in }`
   - `POST /share/connect` → `{ fingerprint }`
   - `POST /share/confirm` → `{ status }`
   - `GET /share/status` → `{ state, progress }`
   - `DELETE /share/session` → `{ cancelled }`
   - `POST /share/export` → `{ ciphertext, salt, nonce, passphrase }`
   - `POST /share/import` → `{ imported_count }`

6. **New CLI commands**:
   - `crypt-env share send <ITEM_IDS>...` — Start LAN send session
   - `crypt-env share receive` — Start LAN receive session
   - `crypt-env share export [IDS] -o file` — Create encrypted package
   - `crypt-env share import -f file` — Import from package

7. **New MCP tools** (all prefixed `crypt_env_share_`):
   - `crypt_env_share_listen` — Start LAN send session
   - `crypt_env_share_connect` — Start LAN receive session
   - `crypt_env_share_confirm` — Confirm fingerprint
   - `crypt_env_share_cancel` — Cancel session
   - `crypt_env_share_status` — Poll session status
   - `crypt_env_share_export` — Export encrypted package (returns passphrase)
   - `crypt_env_share_import` — Import encrypted package

**Rationale**:
- LAN bridge mode provides real-time, interactive sharing with cryptographic proof (fingerprint confirmation) that both parties are communicating with the correct peer
- Encrypted package mode is a fallback for scenarios where real-time communication is impossible (different networks, offline transfers via USB)
- Pairing code (6 digits, 5-min expiration) is a human-verifiable authentication mechanism — prevents MITM if both users can confirm the same code
- X25519 ECDH is industry-standard, post-quantum resistant key exchange primitive
- Argon2id with high memory cost (32768 KiB) makes brute-forcing a random passphrase computationally expensive
- Audit log allows traceability of who shared what and when (useful for security incident response)
- Sender explicitly selects items to share (not a bulk "share all" which could leak unintended secrets)
- Session auto-destruction prevents reuse if the connection is compromised mid-transfer
- Fingerprint verification prevents MITM attacks where attacker intercepts pairing code

**Consequences**:
- LAN bridge requires mDNS discovery to work (must be available on the network)
- Pairing code is short (6 digits) to be human-readable; increases brute-force window to ~2 seconds if attacker has network access (acceptable because code expires in 5 minutes)
- Encrypted package passphrase is shown once and not stored; user must securely communicate it out-of-band (no built-in passphrase recovery)
- Argon2id KDF on package import is slow (~1-2 seconds per import); acceptable for infrequent use but not suitable for bulk imports
- MCP tools return passphrase only in `crypt_env_share_export` response (LLM must display to user via UI, not in logs)

---

### 12. Windows Hello Biometric Unlock
**Context**: Users on Windows need fast, convenient vault unlock without entering master password every time. Windows Hello (fingerprint, facial recognition) is available on most modern Windows devices.

**Decision**: Implement biometric unlock via Windows Hello with DPAPI-encrypted master password storage:

1. **Biometric module** (`src-tauri/src/biometric/mod.rs`):
   - `check_availability()` → returns `BiometricAvailable | NotAvailable`
   - `request_verification(message: &str)` → prompts Windows Hello dialog, returns `VerificationOk | VerificationCancelled | Error`
   - `dpapi_protect(data: &[u8])` → encrypts data with DPAPI (tied to Windows user account), returns hex-encoded blob
   - `dpapi_unprotect(blob_hex: &str)` → decrypts DPAPI blob, returns plaintext bytes

2. **Enrollment flow**:
   - User enters master password in Settings
   - Calls `dpapi_protect(password_bytes)` → generates DPAPI-encrypted blob
   - Stores blob in `settings` table as key `biometric_blob` (plaintext hex in DB)
   - DPAPI ties the blob to the Windows user account; blob cannot be decrypted on a different account

3. **Unlock flow**:
   - Frontend detects biometric availability and enrollment status
   - User clicks "Unlock with Windows Hello"
   - Calls `biometric_unlock` command → retrieves `biometric_blob` from DB → `dpapi_unprotect()` → Windows Hello prompt → recovers password bytes
   - Vault unlocks normally with recovered password (no special unlock path)
   - Session token issued as usual

4. **Disable biometric**:
   - User enters master password in Settings
   - Calls `biometric_disable` command → deletes `biometric_blob` from settings table
   - DPAPI blob discarded; biometric unlock unavailable until re-enrolled

5. **New Tauri commands**:
   - `biometric_check() → "available" | "not_available"` — Detects if Windows Hello is available
   - `biometric_is_enrolled() → bool` — Checks if user has enrolled (`biometric_blob` exists in DB)
   - `biometric_enroll(password: &str) → bool` — Encrypts password with DPAPI, stores blob, returns success
   - `biometric_unlock() → String` — Decrypts blob, prompts Windows Hello, returns session token on success
   - `biometric_disable(password: &str) → bool` — Verifies password, deletes blob, returns success

6. **Implementation notes**:
   - Windows WinRT calls (`UserConsentVerifier::CheckAvailabilityAsync()`, `UserConsentVerifier::RequestVerificationAsync()`) are blocking; use `tokio::task::spawn_blocking()` in Tauri commands
   - DPAPI output uses `LocalFree()` to deallocate WinRT-allocated memory; prevents memory leak
   - Secret password bytes are in `Zeroizing<Vec<u8>>` after decryption and before unlock
   - Non-Windows platforms: all functions compile but return `NotAvailable` at runtime (feature disabled)

7. **Dependencies**:
   - `windows` crate v0.58 with features: `Security_Credentials_UI`, `Win32_Security_Cryptography`, `Win32_Foundation`, `Win32_System_Memory`
   - `zeroize` (already in dependencies) for password bytes cleanup

**Rationale**:
- Biometric does NOT replace master password; it protects an encrypted copy (remains secure even if DPAPI is compromised)
- DPAPI is Windows user-account bound; blob cannot be decrypted by a different user or after password change
- Windows Hello is hardware-backed on devices with TPM/biometric sensors (strong second factor)
- Enrollment still requires master password (prevents stealing the phone and unlocking the vault)
- Pairing biometrics with DPAPI provides defense-in-depth: attacker needs both DPAPI blob AND Windows Hello verification

**Consequences**:
- Feature only works on Windows with Hello hardware; silent no-op on other platforms
- If Windows user password changes, DPAPI blob becomes unusable (user must re-enroll with new password)
- If user loses biometric enrollment (e.g., resets fingerprints), `biometric_blob` persists in DB but cannot be used (safe fallback: use master password)
- DPAPI blob in plaintext hex in DB is acceptable because blob is useless without Windows user privileges and Hello verification
- Unlock latency: ~100-200ms for WinRT call + ~500ms for Windows Hello UI = ~700ms total (slower than master password alone, but negligible for user experience)

---

### 13. Projects & Environments for Environment Variable Management
**Context**: Developers manage multiple projects, each with different environment variable sets (dev, staging, production, etc.). Need hierarchical organization, real vault item references, and ability to export to `.env` files per environment.

**Decision**: Create a Projects system with nested Environments. Every environment variable is a real encrypted vault item, not a literal key=value. Projects are the primary navigation primitive; Projects can be marked global or project-specific.

1. **Data Model**:
   - `projects` table: id, name, description, template (generic/node/postgres/mongo/docker/python), created, updated
   - `environments` table: id, project_id (FK), name, is_default, created, updated
   - `environment_vars` table: id, environment_id (FK), key, item_id (FK to items, mandatory)
   - `environment_paths` table: environment_id (FK), path (absolute filesystem path for .env file)
   - `item_projects` table: item_id (FK), project_id (FK) — many-to-many ownership. Global items can belong to multiple projects
   - `project_categories` table: project_id (FK), category_id (FK) — projects carry tags via category names

2. **Item Ownership & Globalization**:
   - New column `items.is_global` (plaintext, queryable without decryption): true = reusable across projects, false = project-specific
   - Global items surfaced in "Global Secrets" screen (new `GlobalSecrets.tsx`, reachable via footer link from Projects landing page)
   - Deleting a project cascades to delete environments, then to orphaned items (items with zero owners). Preview via `project_preview_delete`
   - Un-globaling an item with multiple owners forks it: creates N independent copies for each project, one per project

3. **Stack Templates**: Provide 6 pre-configured templates (generic, node, postgres, mongo, docker, python) as scaffolds with common environment variable names.

4. **Environment Injection**:
   - User selects environment → clicks "Inject" → backend decrypts all referenced items → writes KEY=VALUE pairs to all configured paths
   - Each environment can have multiple paths (e.g., `.env.production` and `.env.prod.backup`)
   - `environment_inject(id: i64) → InjectResult { paths: Vec<String>, written: Vec<String> }` — returns success per path

5. **Tauri Commands**:
   - `project_list() → Vec<Project>` — All projects with nested environments, categories, and var counts
   - `project_save(project: ProjectInput) → Project` — Create or update project (does not modify environments)
   - `project_delete(id: i64) → bool` — Delete project and cascade (environments, orphaned items)
   - `project_preview_delete(id: i64) → ProjectDeleteImpact { environments, itemsDeleted, itemsOrphaned }` — Show impact before delete
   - `project_export(id: i64) → { json, template }` — Export project structure (templates only, no resolved values)
   - `project_import(json: String) → Project` — Import project structure
   - `environment_save(env: EnvironmentInput) → Environment` — Create or update environment within a project
   - `environment_delete(id: i64) → bool` — Delete environment (cascades to its vars)
   - `environment_inject(id: i64) → InjectResult` — Write decrypted env vars to configured paths
   - `vault_create_project_item(project_id, key, item) → VaultItem` — Add variable to all environments in a project
   - `vault_set_item_global(id, is_global) → GlobalToggleResult { updated, forked }` — Make item global or fork if multi-owned
   - `vault_get_item_owners(id) → Vec<ItemOwner>` — List projects that own this item

6. **Frontend**: 
   - `ProjectManager.tsx` — Primary landing page (replaced previous vault-first navigation). Shows project list, project detail with environment cards, environment editor with variable linking
   - `GlobalSecrets.tsx` (renamed from MainVault.tsx) — Filtered view of `isGlobal` items, footer link to return to projects
   - New `src/components/itemFields/` module: `ItemTypePicker.tsx`, `ItemTypeFields.tsx`, `emptyItemFields()`, `validateItemFields()` — extracted shared per-type form logic used by both ProjectManager (add-variable flow) and EditItem.tsx
   - Projects carry tags via existing categories/TagInput; reuses `project_categories` join table

7. **Migration from Legacy Workspaces**:
   - One-time migration `vault::migrate_literal_vars_to_items()` (gated by `settings['migrated_literals_v1']`, runs on every unlock): Converts pre-existing literal-only environment vars (predating "every var is a real item") into real encrypted vault items owned by their environment's project
   - Pre-existing unowned items (those with no entry in `item_projects` after backfill) are promoted to `is_global = true` so they surface in Global Secrets instead of disappearing
   - Legacy workspace-backed `share_relay_send/receive` tools remain unchanged (out of scope); only true projects/environments use the new model

8. **CLI & MCP**:
   - CLI `crypt-env project` subcommand (replaces deleted `workspace`): `list`, `save`, `delete`, `delete-env`, `inject` per environment
   - MCP tools: `crypt_env_list_projects`, `crypt_env_inject_environment`, `crypt_env_list_environments_by_name`, `crypt_env_inject_env_by_name`

**Rationale**:
- Hierarchical Projects/Environments match real development workflows (dev, staging, production are instances of the same project)
- Every variable as a real item ensures full encryption, audit trail, and reusability semantics
- Item ownership model (via `item_projects`) enables both global reuse and project-specific forking
- Deletion with preview prevents accidental loss of items
- Inject to configured paths eliminates manual `.env` management and prevents secrets in logs/clipboard

**Consequences**:
- Pre-existing global items become hard to distinguish from new global items (no "creation context" metadata)
- Project export/import carries structure only (no resolved secret values) — values resolved at inject time
- Backup/restore of full vault does not yet include projects/environments/item_projects (pre-existing gap, pre-planned for next session)
- Deletion of orphaned items on project delete is permanent and irreversible (preview mitigates, but not fully preventable)

---

### 14. Interactive TUI with Ratatui
**Context**: Developers need vault access from terminal for SSH sessions, CI/CD scripting, or minimal environments where GUI is unavailable.

**Decision**: Implement `crypt-env tui` subcommand using ratatui 0.29 + crossterm 0.28 for interactive terminal UI.

1. **Architecture**:
   - Built as subcommand of `crypt-env` CLI binary: `crypt-env tui`
   - Source: `src-tauri/src/bin/crypt-env/commands/tui.rs`
   - Runs with direct vault access (Tauri command context), no HTTP dependency
   - Single-threaded event loop (tokio `select!` for input + tick events)

2. **Screens**:
   - **Unlock**: Master password input with masked echoing
   - **Main**: Item list with fuzzy search, preview pane, type/category filters
   - **Item Detail**: Full metadata, reveal/copy controls
   - **Help**: Keybinding reference
   - **Confirm**: Destructive action confirmation

3. **Keybindings**:
   - Navigate: ↑↓ or jk
   - Fuzzy search: / (real-time incremental filtering)
   - Detail: Enter
   - Reveal: v
   - Copy: c
   - Delete: d
   - Refresh: r
   - Help: ?
   - Quit: q

4. **State Machine**:
   - `Screen::Unlock` (password input)
   - `Screen::Main` (item list, fuzzy search)
   - `Screen::ItemDetail` (selected item metadata)
   - `Screen::Help` (keybinding reference)
   - `Screen::Confirm` (destructive confirmation)
   - Transitions on user input, returns to Main or exits

**Rationale**:
- Ratatui is lightweight and terminal-independent (works on SSH, CI/CD)
- Direct vault access avoids HTTP dependency (faster, simpler)
- Keybindings follow Vim conventions (↑↓/jk, hjkl for navigation)
- Single-threaded design simplifies state management

**Consequences**:
- TUI output not suitable for scripting (use REST API or MCP for automation)
- Cannot resize terminal during operation (acceptable for target use case)
- Clipboard copy requires system integration (via `clipboard-manager` plugin or system command)

---

### 15. Internet Relay Sharing with Supabase
**Context**: Users need to share secrets with teammates on different networks. LAN bridge works for local network; encrypted packages work for offline. Internet relay bridges the gap for remote teams.

**Decision**: Implement Supabase-backed relay with AES-256-GCM encryption and Argon2id KDF from passphrase.

1. **Service Architecture**:
   - Sender uploads encrypted items to Supabase relay table
   - Backend generates 4-digit code (XXXX-XXXX) from pairing token hash
   - Passphrase: 12-char random alphanumeric, never stored (shown once to sender)
   - Receiver enters code + passphrase → decrypts → imports
   - Relay record auto-deleted after first read (burn-after-read)
   - 24-hour TTL via Supabase scheduled trigger

2. **Database Schema** (Supabase PostgreSQL):
   ```sql
   CREATE TABLE relay (
       id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
       code_hash TEXT UNIQUE NOT NULL,
       encrypted_payload BYTEA NOT NULL,
       salt BYTEA NOT NULL,
       nonce BYTEA NOT NULL,
       created_at TIMESTAMP DEFAULT now(),
       expires_at TIMESTAMP DEFAULT now() + INTERVAL '24 hours',
       accessed BOOLEAN DEFAULT FALSE
   );
   CREATE INDEX idx_relay_code_hash ON relay(code_hash);
   CREATE INDEX idx_relay_expires_at ON relay(expires_at);
   ```

3. **Encryption**:
   - Random 32-byte key + 12-byte nonce for AES-256-GCM
   - Payload: JSON serialization of selected items + metadata
   - Passphrase KDF: Argon2id(m=32768, t=2, p=2, salt=random 16 bytes)
   - No key derivation from code (code is just a lookup handle)

4. **Tauri Commands**:
   - `share_relay_send(item_ids: Vec<i64>) → { code: String, passphrase: String, expires_in: u64 }` — Encrypt and upload
   - `share_relay_receive(code: String, passphrase: String) → { imported_count: usize, items: Vec<ItemSummary> }` — Download and decrypt

5. **Configuration** (Settings → Internet Sharing):
   - Supabase project URL (input field)
   - Anon API key (input field, password masked)
   - Test button to validate connectivity
   - Stored in `settings` table as plaintext (acceptable scope)

6. **Frontend**:
   - `ShareModal` component with tabs: LAN, INTERNET
   - INTERNET tab: Sender sees code + passphrase (copyable); Receiver enters code + passphrase

**Rationale**:
- Supabase provides PostgreSQL + realtime + REST API with no backend maintenance
- Argon2id(m=32MB) makes brute-forcing passphrase expensive (~1-2s per attempt)
- Code + passphrase required (code alone insufficient for security)
- Burn-after-read prevents audit trail and replay attacks
- 24h TTL prevents relay table bloat
- AES-256-GCM provides authenticated encryption (detects tampering)

**Consequences**:
- Requires internet connection (not usable offline)
- Depends on Supabase availability (SPoF if Supabase is down)
- Code is short (XXXX-XXXX, ~10,000 values); brute-forceable in ~10 minutes on 1000 req/sec, but expires in 5 minutes
- Passphrase must be communicated out-of-band (via chat, call, email; Supabase only stores encrypted payload)
- Each import requires ~1-2s Argon2id KDF (acceptable for infrequent use, but not suitable for bulk imports)

---

### 16. WSL Bridge for the Environment Path Picker (Issue #3, Phase 1)
**Context**: Windows developers who keep their project source inside a WSL2 distro (this repo itself, per `CLAUDE.local.md`) can already attach a WSL `.env` to an environment by typing its `\\wsl.localhost\<distro>\...` UNC path into the existing manual path input — but nothing in the UI suggests this is possible, the user must know their distro's exact *registered* name, and a typo or a stopped distro previously surfaced as a confusing later failure. This is discoverability and pre-flight validation, not new capability — see the plan's own accounting in `docs/plans/issue-3-wsl-bridge-env-paths.md` §2.

**Decision**: Add a "Browse WSL" affordance next to the existing browse button in the environment editor's PATHS block, backed by two new Windows-only Tauri commands in a dedicated `wsl` module. Phase 2 (a CLI running inside WSL talking back to the vault) is explicitly deferred, uncommitted work (plan §6).

1. **New module** `src-tauri/src/wsl/mod.rs` — decoupled from `db`/`vault`/`api`/`project`; only the frontend composes it with `project_pick_env_path`.
   - `parse_distro_list(&[u8]) -> Vec<String>` — pure. Decodes `wsl.exe --list --quiet` stdout: UTF-8 BOM, UTF-16LE BOM, BOM-less UTF-16LE (heuristic: every second byte of the first 32 is `0x00`), or UTF-8 lossy fallback. 11 unit tests in-file.
   - `unc_root(distro)` / `unc_root_legacy(distro)` — pure, return `\\wsl.localhost\<distro>\` / `\\wsl$\<distro>\`.
   - `wsl_list_distros() → Result<Vec<String>, String>` (Tauri command) — spawns `wsl.exe --list --quiet` via `tokio::process::Command` + `kill_on_drop` under a 10 s `tokio::time::timeout`. Returns `Ok(vec![])` — never `Err` — when `wsl.exe` is absent, WSL has zero distros, or the target isn't Windows; `Err` only on timeout or a genuine non-`NotFound` spawn error.
   - `wsl_distro_home(distro) → Result<String, String>` (Tauri command) — validates the argument (no `\`, `/`, `..`, NUL), probes `\\wsl.localhost\<distro>\home\` then falls back once to `\\wsl$\<distro>\home\` (each probe `spawn_blocking` + 10 s timeout, since a UNC probe against a cold distro blocks). Descends into the single child directory if `read_dir` finds exactly one, else returns `...\home\`. Touching the UNC path starts a stopped distro — documented in the button's tooltip.
2. **`project_pick_env_path`** (`src-tauri/src/project/mod.rs`) gained an optional `start_dir: Option<String>` argument; when given, calls `.set_directory()` on the `rfd::FileDialog` builder before `pick_file()`. The other four `rfd` call sites (`project_export`, `project_import`, the two in `share_commands.rs`) are untouched.
3. **Frontend** (`src/components/ProjectManager.tsx`): `isWindows` detected via `@tauri-apps/plugin-os` `platform()`, mirroring `WindowChrome.tsx`. `wsl_list_distros` fetched once when `isWindows` becomes true, held in local component state (not Zustand, not TanStack Query — ephemeral machine state). A "WSL" button (terminal icon) renders in the PATHS row only when `isWindows && wslDistros.length > 0`: one distro acts directly, several open a small inline dropdown (same panel styling as the existing tag filter). Click → `wsl_distro_home(distro)` → `project_pick_env_path({ startDir })`, reusing the same picked-path handling as the plain browse button (`applyPickedEnvPath`, extracted from the old `handlePickEnvPath`) → failure shows a toast and falls back to the manual input.
4. **Data-loss gate landed alongside this** (plan §3.6, cross-cutting with issues #7/#8): `inject_environment`'s read of each target path (`src-tauri/src/project/mod.rs`) previously did `std::fs::read_to_string(path).unwrap_or_default()`, treating *any* read failure — including a stopped WSL distro's dead 9p mount — as "file is empty," then silently overwriting it with only this environment's keys. Changed to a `match e.kind()` guard: `ErrorKind::NotFound` still starts from empty (new file); every other error kind aborts the write with `Err`. This is the single most likely real-world failure mode of attaching a WSL path, and the plan declared Phase 1 not closeable without it.

**Rationale**:
- `--list --quiet` over the issue-specified `--list --verbose`: verbose output is column-aligned, carries a localized `Running`/`Stopped`/default-marker, and breaks on non-English Windows; quiet emits one bare name per line and needs no state (selecting a distro starts it anyway).
- `tokio::process::Command` (not `spawn_blocking` + `std::process::Command`, the pattern already used for `rfd` in this file) because a `timeout` around a blocking-pool `JoinHandle` doesn't cancel the blocked thread — it leaks a pool slot for as long as `wsl.exe` hangs. `tokio::process` cancels and kills the child on drop.
- No new Cargo dependency: hand-rolled UTF-16LE decode (~10 lines) instead of `encoding_rs`, since exactly two known encodings from one known producer don't justify a crate; `tauri-plugin-shell` (listed in this file's own stack-setup checklist, never actually added — see `src-tauri/Cargo.toml`) is deliberately not added either, since it would let the webview spawn processes for no gain when Rust can already do so natively.
- `\\wsl.localhost\` primary / `\\wsl$\` fallback (not build-number sniffing): two cheap filesystem probes beat silently guessing wrong on older Windows builds.

**Consequences**:
- Windows-only feature; the command surface still registers on all targets (honest `Ok(vec![])` elsewhere) so there is one `invoke_handler!` list, not a per-platform one.
- Clicking "Browse WSL" can cold-boot a stopped WSL VM (several seconds, memory cost) — inherent to the UNC bridge, stated in the tooltip, not eagerly triggered (only on explicit click).
- `docker-desktop`/`docker-desktop-data` distros are not filtered out of the list (would hardcode a vendor's naming into the parser); users recognize their own distros.
- Requires manual verification on real Windows + WSL2 hardware (checklist M1–M10 in the plan) — not automatable in CI, since CI has no WSL. Automated coverage is limited to the pure `parse_distro_list`/`unc_root` functions and the `inject_environment` read-guard (both covered by `cargo test`).

---

## Security Status (post-review 2026-04-24)

A **comprehensive security review** was performed that identified **19 findings** (7 HIGH, 8 MEDIUM, 4 LOW). **All findings have been addressed**.

**Critical findings (HIGH) implemented**:
1. ✅ **Timing-safe token comparison**: Implemented `subtle::ConstantTimeEq` for all token comparisons
2. ✅ **Master password derivation**: Argon2id with hardened parameters (m=65536, t=3, p=4)
3. ✅ **Key in memory with Zeroizing**: Use `zeroize` crate to overwrite key on Drop
4. ✅ **Access audit for `/items/:id/reveal`**: Logging infrastructure prepared (future audit log implementation)
5. ✅ **File permissions (mcp_token, cli_session_token)**: Set to 0600 (Unix) or NTFS ACLs (Windows) on creation
6. ✅ **Credential encryption in MCP server**: Tokens stored in memory with `Zeroizing` to prevent leakage

**MEDIUM findings implemented**:
- ✅ Error handling without exposure of internal paths
- ✅ HTTPS for local REST API with auto-generated rcgen certificate
- ✅ Input validation on `/items` POST/PUT (HTTP 422 for invalid data)
- ✅ Cleanup of temporary `.env` files via RAII pattern (automatic zeroing + deletion)
- ✅ Session auto-lock via background task (configured timeout, `VaultState.key = None`)

**LOW findings**:
- Security documentation (covered in README.md and context.md)
- Change traceability (audit log structure in place, entries logged on demand)
- Secure data export (handled via `/fill` endpoint, no plaintext in responses)

---

> This file is the main project context.
> Referenced from CLAUDE.md with: `See context.md for full project context.`
