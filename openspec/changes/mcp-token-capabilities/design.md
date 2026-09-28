## Context

`verify_token` returns `Result<(), StatusCode>`, so handlers cannot tell who is calling. The MCP binary talks to the REST API with the static token from `settings.mcp_token`. MCP host config writes (`claude_desktop_config.json`, project `.mcp.json`) happen inside the MCP binary itself. The GUI talks to the backend only through Tauri `invoke`.

## Goals / Non-Goals

**Goals:** a closed default for MCP (every route is explicitly classified), human-in-the-loop for exfiltration-capable operations, and no secret material in MCP-visible bytes.

**Non-Goals:** item-level ACLs; multiple MCP tokens.

## Decisions

### D1. `Principal` enum from `verify_token`
`verify_token(...) -> Result<Principal, StatusCode>`, where `Principal::{Session, Mcp}`. It is passed to handlers through an axum extractor (`AuthedPrincipal`) instead of each handler calling `verify_token`. That removes the chance of a route forgetting auth.

A central table `fn mcp_policy(route) -> McpPolicy::{Allow, Deny, Approve}` is enforced by a middleware layer. Unknown routes default to `Deny`. A unit test enumerates the router's routes and asserts that each one has an explicit policy.

*Rejected:*
- Separate router or port for MCP: duplicates the handlers.
- Capability flags inside the token: needs a token format change, and we have only one MCP token.

### D2. Approval store in `ApiState`
`ApprovalStore { entries: Vec<Approval> }`, where `Approval { id: [u8;16] hex, kind, summary: ApprovalSummary, payload: ApprovalPayload, created, state }`. `payload` holds the *request* (item ids, destination), never derived secrets.

The flow:
1. A new approval emits a Tauri event `approval://requested` with `{id, summary}` through the `AppHandle` stored in `ApiState`.
2. The GUI resolves it with `approval_resolve(id, approve)`.
3. The backend executes the operation with the *session* rights of the unlocked vault, stores the non-secret outcome, and emits `approval://result` with the secret outcome (code/passphrase) to the GUI only.

`GET /approvals/:id` (MCP-allowed) returns `{status, meta}`. Expiry is checked lazily and by the auto-lock tick. `lock_vault` clears the store: the lock epoch from `harden-cli-manifest-and-sessions` D6 is reused, and approvals are bound to the epoch.

*Rejected:* blocking the MCP HTTP call until the user answers. That ties up a worker and hits the client timeout; polling is simpler.

### D3. Move MCP host config writes to the backend
New endpoints `POST/PUT/DELETE /mcp-servers` (policy `Approve` for MCP, `Allow` for session). The MCP binary's add/update/delete tools call them instead of writing files. The parse-refusal fix for `.mcp.json` is in `host-config-and-process-hygiene`; this change only relocates the writer.

### D4. Output-path confinement for MCP
For the MCP principal, `output_path` must pass `fsguard::contain_relative` against *some* project's `root_path`. This reuses the helper from `harden-cli-manifest-and-sessions`, which therefore must land first. `overwrite: true` against a `Foreign` target is denied for MCP.

### D5. MCP tool surface
- `share_confirm` and `update_settings` (security keys) are removed from `tools/list`.
- `relay_send` and `share_export` return `{status:"pending_approval", approvalId}`.
- A new `crypt_env_approval_status` tool polls the approval.
- The tool descriptions tell the agent that the human sees the code in the desktop app.

## Security & Threat Model

- **Attacker:** a prompt-injected LLM holding the MCP token.
- **Before:** full vault exfiltration in one call.
- **After:**
  - reveal is denied;
  - exfiltration needs a visible human approval that names the items and destination;
  - secrets appear only in the GUI;
  - persistent config writes are approval-gated.
- **Residual risks:**
  - a user rubber-stamping approvals. Mitigation: the modal shows item names and the destination, and defaults focus to Deny.
  - `inject_env`/`run_command`: see `mcp-command-execution-hardening`.

## Risks / Trade-offs

- [Agents break on the new async flow] → The tool descriptions document polling; this is a breaking change noted in the docs.
- [GUI not running (tray-hidden)] → The approval event also triggers a window show plus an OS notification. If the GUI is closed, the vault is locked anyway.
- [Policy table drift] → Default deny plus a route-enumeration test.

## Migration Plan

**Ordering:** the MCP binary's `inject_env`, `generate_env` and `run_command` currently read values through `/items/:id/reveal`. This change MUST ship in the same release as `mcp-command-execution-hardening`, which moves execution into the backend. Otherwise those tools break. No data migration. Ships with docs updates. Rollback = revert.

## Open Questions

- Should the approval modal allow "approve for 5 minutes" batching? It can be added later without changing the specs' default.
