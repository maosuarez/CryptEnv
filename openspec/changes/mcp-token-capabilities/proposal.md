## Why

The static MCP token is accepted by `verify_token` on every REST route (`api/mod.rs:229-255`). Because of that, anything holding it (an LLM agent, or a prompt injection steering one) can read every secret in plaintext:
- `/items/:id/reveal` returns secret values.
- `/relay/send` and `/share/export` accept any item ids and return the decryption passphrase to the caller (MCP `:2133`, `:1774` → `api/mod.rs:2736`, `:2189`).
- `share_confirm` lets the LLM approve a LAN pairing that no human has compared.
- `add_mcp_server` / `update_mcp_server` let it write persistent commands into MCP host configs (`crypt-env-mcp.rs:2503`).
- `PUT /settings` lets it disable auto-lock.
- `/fill` accepts an arbitrary `output_path` with `overwrite`, which clobbers files like `~/.bashrc`.

This breaks the core rule that "the MCP server never returns secret values". The same critical issue was reported on 2026-08-09 and is still open.

Audit IDs: API-1 (MCP part), API-2, API-4 (MCP part), MCP-2, MCP-8 (confirm part), MCP-10.

## What Changes

- **Principal-aware authentication.** The API distinguishes *session* callers (the user via CLI or GUI password) from the *MCP* token. Every route declares what it allows the MCP principal to do: allow, deny, or require human approval.
- **Denied to MCP (403 `MCP_FORBIDDEN`):**
  - secret reveal;
  - LAN pairing confirmation;
  - changes to security settings (`auto_lock_timeout`, MCP token rotation, biometric);
  - `/fill` and `/example` with an `output_path` outside a registered project root;
  - overwriting a pre-existing file that crypt-env does not manage.
- **GUI approval required for MCP:** `relay_send`, `share_export`, and `add/update/delete_mcp_server`. The call creates a pending request, and the desktop app shows a modal describing it. Only after the user approves does the backend perform the action. The relay code, passphrase and export passphrase are shown **only in the GUI**, never returned to the MCP caller.
- **Backend performs MCP config writes.** Writing MCP host configs moves from the MCP binary into the backend, so the approval gate cannot be bypassed.
- **BREAKING for agents:**
  - these tools now return `pending_approval` and then a result without secrets;
  - `share_confirm` and security-setting updates are gone from MCP;
  - reveal via the MCP token returns 403.

## Capabilities

### New Capabilities
- `mcp-access-control`: which REST operations the MCP token may perform, the human-approval flow, and the guarantee that no secret or decryption material reaches an MCP caller.

### Modified Capabilities
<!-- none -->

## Impact

- **Backend:** `api/mod.rs` (`verify_token` returns a principal; per-route policy; new `/approvals` routes); new `api/approvals.rs`; `vault/mod.rs` (MCP config write moves here).
- **MCP server:** `bin/crypt-env-mcp.rs`, with tool changes for relay_send, share_export, share_confirm, update_settings and the mcp_server tools.
- **GUI:** new approval modal (listener on a Tauri event), and new Tauri commands `approval_list` and `approval_resolve`.
- **Docs:** the MCP tools section in `docs/index.html` and `docs/reference.md`.
- **Security:** closes the MCP→plaintext exfiltration path. No crypto or storage changes. Pending approvals are held in memory only and cleared on lock.

## Non-Goals

- Per-tool scoping of *which* items MCP may list or inject (existing scope params stay).
- Command execution hardening (`run_command`, `generate_env`): that is covered by `mcp-command-execution-hardening`.
- LAN pairing cryptography: covered by `lan-share-hardening`.
- Multiple MCP tokens or token scopes per agent.
