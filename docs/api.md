# 🔌 REST API Reference

CryptEnv runs a local HTTPS server on `127.0.0.1:47821` (localhost strictly). This enables integrations with custom tools, scripts, and the CLI/MCP binaries.

---

## 🔒 Authentication & TLS

- **TLS**: Auto-generates a local self-signed certificate on initial launch stored in the app data directory. Use `-k` or `--cacert` with `curl`.
- **Header**: Requests require `X-Vault-Token: <token>` (either a temporary session token from `/unlock` or a static MCP token generated in Settings).
- **Constant-Time Verification**: All token comparisons use `subtle::ConstantTimeEq` to prevent timing attacks.

---

## 📋 Endpoints Overview

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/health` | Server status and vault lock state (unauthenticated) |
| `POST` | `/unlock` | Unlock vault with master password (failed attempts throttled: 1 s doubling to 60 s, `429` + `Retry-After`) |
| `GET` | `/items` | List items (redacted) scoped by `environment_id` or `project` + `environment` |
| `POST` | `/items` | Create and link a vault item into an environment |
| `GET` | `/items/:id` | Get item metadata |
| `PUT` | `/items/:id` | Update item fields (partial server-side merge) |
| `DELETE` | `/items/:id` | Permanently delete an item |
| `POST` | `/items/:id/reveal` | Reveal secret plaintext value (requires `{"confirm": true}`). Session tokens only; the MCP token gets `403 MCP_FORBIDDEN` |
| `POST` | `/fill` | Fill `.env.example` templates from vault |
| `GET` | `/projects` | List all projects and environments |
| `POST` | `/projects` | Create or update a project |
| `POST` | `/environments/:id/inject` | Inject decrypted secrets to configured `.env` file paths |
| `GET` | `/categories` | List categories |
| `POST` | `/categories` | Create category |
| `POST` | `/share/listen` | Start LAN share sender session |
| `POST` | `/share/connect` | Start LAN share receiver session |
| `POST` | `/relay/send` | Send secrets via Supabase relay |
| `POST` | `/relay/receive` | Receive secrets via Supabase relay |
| `POST` | `/exec` | Run a stored command in the app with injected secrets; redacted, bounded output (see `reference.md`) |
| `DELETE` | `/exec/:runId` | Kill a running command |
| `POST` | `/generate-env` | Private `0600` env file for given keys (MCP token: needs approval) |
| `POST` `PUT` `DELETE` | `/mcp-servers` | Edit the Claude MCP config (MCP token: needs approval) |
| `GET` | `/approvals/:id` | Poll an approval; non-secret status only |

For comprehensive parameter definitions, error response schemas, and deep implementation details, see [docs/reference.md](reference.md).
