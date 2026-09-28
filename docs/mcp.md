# 🤖 Model Context Protocol (MCP) Integration

CryptEnv exposes a **stdio-based MCP server** (`crypt-env-mcp`, JSON-RPC 2.0) that AI coding assistants (e.g., Claude Code, Claude Desktop, Cursor) use to interact with your vault securely without exposing plaintext secret values.

---

## 🔒 Security Principles

- **Zero Plaintext in Context**: Secret values are **never returned** in JSON-RPC responses or logs. They are injected directly as environment variables or written securely into files.
- **Token Authentication**: MCP commands authenticate with the local REST API using an MCP token generated in CryptEnv Settings.
- **Stdio Subprocess**: The server runs as a local subprocess over standard I/O (no open network listening ports).

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
| `crypt_env_generate_env` | `items: [KEY, ...]` | Shell export statements | Generate `.env` syntax for keys (injected, values redacted in output) |
| `crypt_env_inject_env` | `items: [KEY, ...]` | Shell assignment code | Inject secrets directly as environment variables in the current process |
| `crypt_env_fill_env` | `template_path`, `output_path` | File path confirmation | Fill a `.env.example` template with vault secrets and save to `.env` |
| `crypt_env_update_settings` | `timeout`, `theme`, etc. | Updated settings | Modify app settings |
| `crypt_env_list_commands` | — | Command list with placeholders | List all saved commands in the vault |
| `crypt_env_run_command` | `command_name`, `variables: {VAR=value, ...}` | `{ exit_code, stdout, stderr }` | Execute command with resolved `{{placeholder}}` variables; secrets never in response |
| `crypt_env_list_categories` | — | Category list | List all categories (id, name, color, description) |
| `crypt_env_create_category` | `name`, `color`, `description` | Category object | Create a new category |
| `crypt_env_update_category` | `id`, `name`, `color`, `description` | Category object | Update category fields |
| `crypt_env_delete_category` | `id` | `{ deleted: true }` | Delete a category |
| `crypt_env_update_item` | `id`, fields to update | `{ updated_at }` | Partial update of item metadata |
| `crypt_env_delete_item` | `id` | `{ deleted: true }` | Delete a vault item |
| `crypt_env_share_listen` | `item_ids: [...]` | `{ pairing_code, expires_in }` | Start LAN send session |
| `crypt_env_share_connect` | `pairing_code` | `{ fingerprint }` | Start LAN receive session |
| `crypt_env_share_confirm` | `confirmed: bool` | `{ status }` | Confirm fingerprint match |
| `crypt_env_share_cancel` | — | `{ cancelled }` | Cancel active share session |
| `crypt_env_share_status` | — | `{ state, progress }` | Poll share session status |
| `crypt_env_share_export` | `item_ids: [...]` | `{ ciphertext, salt, nonce, passphrase }` | Export encrypted package |
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

AI: "Confirming fingerprint match..."
→ Calls: crypt_env_share_confirm(confirmed=true)
← Returns: { status: "confirmed" }
AI: "Item transferred securely."
```
