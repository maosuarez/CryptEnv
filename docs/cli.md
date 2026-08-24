# 💻 CLI & Interactive TUI Guide

CryptEnv provides a command-line interface (`crypt-env`) and an interactive terminal UI (`crypt-env tui`) built with `ratatui` for terminal-centric workflows, headless servers, and CI/CD pipelines.

---

## 🖥️ Interactive TUI (`crypt-env tui`)

Access and manage your vault directly from the terminal without opening the desktop GUI.

```bash
crypt-env tui
```

### Key Features
- **Master Password Unlock**: Secure masked terminal input.
- **Fuzzy Search**: Real-time filtering with `/`.
- **Item Detail View**: Metadata inspection, copy to clipboard, and reveal modes.
- **Vim-Style Navigation**: `j`/`k` and arrow keys.

### Keybindings

| Key | Action |
|-----|--------|
| `↑` / `↓` or `j` / `k` | Navigate items |
| `/` | Start incremental fuzzy search |
| `Enter` | Open selected item details |
| `v` | Reveal encrypted secret value |
| `c` | Copy selected secret to clipboard |
| `d` | Delete item (with confirmation prompt) |
| `r` | Refresh items from database |
| `?` | Toggle keybindings help screen |
| `q` | Quit and lock vault session |

---

## 🖱️ CLI Command Reference

The `crypt-env` CLI communicates with the running app via the local authenticated REST API (`127.0.0.1:47821`).

### Item Management

#### `crypt-env add [KEY=value] | [--file .env] | [$VARNAME]`
Add a new secret to the vault.
```bash
# Add a key-value secret
crypt-env add "OPENAI_API_KEY=sk-..."

# Import from .env file
crypt-env add --file .env.local

# Add from current shell variable
crypt-env add $DATABASE_URL
```

#### `crypt-env search TERM [--type TYPE] [--category CATEGORY]`
Search items without revealing secret contents.
```bash
crypt-env search api --type key
crypt-env search prod_db --category databases
```

#### `crypt-env set KEY [KEY2 ...]` / `crypt-env inject KEY`
Print shell assignments for direct injection.
```bash
# bash / zsh
eval $(crypt-env set OPENAI_API_KEY)

# PowerShell
crypt-env set OPENAI_API_KEY | Invoke-Expression
```

#### `crypt-env fill TEMPLATE [-o OUTPUT]`
Fill `.env.example` templates with decrypted secrets.
```bash
crypt-env fill .env.example -o .env
```

---

### Command Management

#### `crypt-env memory`
Save a command with placeholders interactively.
```bash
crypt-env memory --name "deploy" \
  --description "Deploy to staging" \
  --command "docker compose -f docker-compose.prod.yml up -d --build"
```

#### `crypt-env exec NAME [--VAR=value ...]`
Execute or resolve a saved command with `{{placeholder}}` variables.
```bash
crypt-env exec deploy --ENV=staging --REGION=us-west-2
```

---

### Secret Sharing Commands

#### LAN Bridge
```bash
# Sender: start listening
crypt-env share send api_key_1 api_key_2

# Receiver: connect with code
crypt-env share receive 123456
```

#### Encrypted Package
```bash
# Export
crypt-env share export api_key_1 api_key_2 -o secrets.vault

# Import
crypt-env share import -f secrets.vault
```

#### Internet Relay
```bash
# Send via Supabase relay
crypt-env share relay send api_key_1 api_key_2

# Receive
crypt-env share relay receive 1234-5678
```

---

### System Diagnostics

```bash
crypt-env doctor
```
Verifies that the CryptEnv daemon/REST API is running, the vault is unlocked, and tokens are valid.
