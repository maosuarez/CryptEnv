# 🤖 Model Context Protocol (MCP) Integration

CryptEnv exposes a **stdio-based MCP server** (`crypt-env-mcp`, JSON-RPC 2.0) that AI coding assistants (e.g., Claude Code, Claude Desktop, Cursor) use to interact with your vault securely without exposing plaintext secret values.

---

## 🔒 Security Principles

- **Zero Plaintext in Context**: Secret values are **never returned** in JSON-RPC responses or logs. They are injected directly as environment variables or written securely into files.
- **Token Authentication**: MCP commands authenticate with the local REST API using an MCP token generated in CryptEnv Settings.
- **Stdio Subprocess**: The server runs as a local subprocess over standard I/O (no open network listening ports).
- **Restricted token**: The MCP token cannot reveal values, confirm pairings, change security settings, or write outside a project root. Exfiltration-capable operations (relay send, share export, `generate_env`, MCP host config edits) wait for **your approval in the desktop app**; relay codes and passphrases are shown only there.
- **Commands run in the app**: `crypt_env_run_command` executes inside CryptEnv with a cleared environment, strict `{{param}}` characters (`[A-Za-z0-9._/:@=+,-]`), redacted output, a 120 s limit and process-tree kill.

---

## ⚙️ Configuration

### Claude Desktop

`crypt-env-mcp` is a **stdio subprocess** launched directly by Claude Desktop. It is not an HTTP server and must not be configured with a `url` field.

Add the following to your `%APPDATA%\Claude\claude_desktop_config.json` (Windows) or `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS):

```json
{
  "mcpServers": {
    "cryptenv": {
      "command": "C:\\full\\path\\to\\crypt-env-mcp.exe"
    }
  }
}
```

If built from source:
```
src-tauri\target\release\crypt-env-mcp.exe
```

### Prerequisites
1. **Generate Token**: Open CryptEnv → **Settings** → **Integrations** → **Generate MCP Token**.
2. **Unlock Vault**: Launch CryptEnv and unlock your vault. `crypt-env-mcp` communicates with the local REST API at `127.0.0.1:47821`.
   The setup wizard can also write the `cryptenv` entry into `~/.mcp.json` for you. Only that entry changes (other servers are kept), the write is atomic with a one-time `.bak`, and a new file is `0600` on Unix. A file that is not strict JSON (comments, trailing commas) is left untouched and the wizard shows the entry to add by hand, with a token placeholder.
3. **Restart Client**: Restart Claude Desktop or your MCP client after updating the configuration.

---

## 🛠️ Available MCP Tools

| Tool | Input | Output | Description |
|------|-------|--------|-------------|
| `crypt_env_doctor` | — | Health status | Check app connectivity and vault lock state |
| `crypt_env_list_items` | `type`, `category` (optional) | Item metadata (redacted) | List secrets filtered by type and/or category |
| `crypt_env_get_item` | `item_id` | Item metadata | Get item details without exposing the secret value |
| `crypt_env_search_items` | `query`, `type`, `category` (optional) | Matching metadata | Search vault without revealing secret contents |
| `crypt_env_add_item` | `name`, `type`, `value`, `category`, `notes` | Confirmation | Add a new secret to the vault |
| `crypt_env_generate_env` | `keys`, scope | `pending_approval` + `approvalId` | After you approve in the app, writes a private (`0600`) `.env` and reports its path via `crypt_env_approval_status`. Deleted after 10 minutes, on lock and at exit |
| `crypt_env_inject_env` | `key`, scope | Confirmation | Selects a secret to inject into later `crypt_env_run_command` calls (key name only; the MCP process never holds the value) |
| `crypt_env_fill_env` | `template`, `output_path` or `output_dir`, scope | Stats | Fill a template and write it inside a registered project root (never returned inline; foreign files are not overwritten) |
| `crypt_env_update_settings` | `hotkey` | Confirmation | Change the hotkey. Security settings (auto-lock, etc.) are user-only |
| `crypt_env_list_commands` | — | Command list with placeholders | List all saved commands in the vault |
| `crypt_env_run_command` | `name`, `params`, scope | `{ exit_code, timed_out, stdout, stderr }` | Run a saved command in the app; secrets from `crypt_env_inject_env` are in its environment, output is redacted |
| `crypt_env_list_categories` | — | Category list | List all categories (id, name, color, description) |
| `crypt_env_create_category` | `name`, `color`, `description` | Category object | Create a new category |
| `crypt_env_update_category` | `id`, `name`, `color`, `description` | Category object | Update category fields |
| `crypt_env_delete_category` | `id` | `{ deleted: true }` | Delete a category |
| `crypt_env_update_item` | `id`, fields to update | `{ updated_at }` | Partial update of item metadata |
| `crypt_env_delete_item` | `id` | `{ deleted: true }` | Delete a vault item |
| `crypt_env_share_listen` | `item_ids: [...]` | `{ pairing_code, expires_in }` | Start LAN send session |
| `crypt_env_share_connect` | `pairing_code` | `{ fingerprint }` | Start LAN receive session |
| `crypt_env_share_cancel` | — | `{ cancelled }` | Cancel active share session |
| `crypt_env_share_status` | — | `{ state, progress }` | Poll share session status |
| `crypt_env_share_export` | `items`, `output_path` | `pending_approval` + `approvalId` | You approve in the app, which shows the passphrase (never returned to the agent) |
| `crypt_env_approval_status` | `id` | Status + non-secret metadata | Poll a request waiting for your approval (`pending`, `approved`, `denied`, `expired`; expires after 120 s) |
| `crypt_env_share_import` | `package, passphrase` | `{ imported_count }` | Import from encrypted package |

---

## 💡 Typical Agent Workflows

### 1. Safe Deployment Execution
```
AI: "I will deploy the staging database. Let me inspect available credentials..."
→ Calls: crypt_env_list_items(type="credential", category="databases")
← Returns: [staging_pg_db (credential), staging_redis (credential)]

AI: "Injecting secrets into subprocess environment..."
→ Calls: crypt_env_inject_env(items=["staging_pg_db", "staging_redis"])
← Injects credentials directly into subprocess process environment

AI: "Running deployment migration script..."
→ Subprocess executes successfully.
← Plaintext secrets were never passed into the AI context window.
```

### 2. Peer Secret Sharing via Agent
```
User: "Share the staging Stripe API key with Alice"

AI: "Initiating LAN share session..."
→ Calls: crypt_env_share_listen(item_ids=["stripe_staging_key"])
← Returns: { pairing_code: "492015", expires_in: 300 }

AI: "Pairing code 492015 generated. Ask Alice to open CryptEnv → Receive and enter 492015"
[Alice connects]

AI: "Compare the fingerprint shown in CryptEnv with Alice's screen and confirm it in the app."
[You confirm the fingerprint in the desktop app: the agent cannot confirm it]
AI: "Item transferred securely."
```
