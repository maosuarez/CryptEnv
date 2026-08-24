# 🔐 CryptEnv

> **A local-first, encrypted productivity vault for developers — accessible instantly via hotkey (`Ctrl+Alt+Z`). Safely integrate with AI coding agents (Claude, Cursor) via MCP to inject secrets without exposing plaintext credentials.**

![License](https://img.shields.io/badge/license-MIT-green)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue)
![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202.0-orange)
![Spec-Driven](https://img.shields.io/badge/specs-OpenSpec-purple)
![Status](https://img.shields.io/badge/status-active%20development-yellow)

---

## 🎯 Why CryptEnv?

Modern development requires managing dozens of API keys, database credentials, and environment variables across multiple stacks. Sharing secrets over chat or storing them in plaintext `.env` files is a major security risk. Furthermore, when automating workflows with AI coding assistants (Claude Code, Cursor, MCP clients), sharing API keys in prompts exposes them to logs and external context windows.

**CryptEnv solves this by providing:**
1. **Zero Plaintext Leaks**: All secrets are encrypted at rest with **AES-256-GCM** and **Argon2id**. Master keys live only in volatile memory during active sessions.
2. **AI Agent Integration via MCP**: The built-in **Model Context Protocol (MCP)** server injects environment variables directly into subprocesses without returning plaintext secrets to the LLM.
3. **Projects & Environments**: Group variables by project (`dev`, `staging`, `prod`) and inject them directly into your local `.env` files with a single click or command.
4. **Multiple Interfaces**: Use the sleek desktop GUI, the full-featured **`crypt-env tui`** terminal interface, the CLI, or the local REST API.

---

## ✨ Key Features

- ⚡ **Instant Access**: Press `Ctrl+Alt+Z` from anywhere to summon the vault.
- 🗂️ **5 Item Types**: Secrets / API Keys, Credentials (user/pass/URL), Links, Parameterized Commands (`{{VAR}}`), and Markdown Notes.
- 📦 **Projects & Typed Environments**: Organize environment variables by project and stack; auto-inject decrypted variables into target `.env` paths.
- 🖐️ **Windows Hello Biometric Unlock**: Quick unlock using fingerprint, face recognition, or PIN on Windows.
- 💻 **Interactive TUI**: Terminal UI (`crypt-env tui` built with Ratatui) for terminal-centric and headless workflows.
- 🤖 **Secure MCP Server**: stdio-based integration for Claude and other AI coding assistants.
- 🔗 **3 Sharing Modes**: Real-time LAN Bridge (mDNS + ECDH), portable encrypted `.vault` packages, and ephemeral Internet Relay (burn-after-read Supabase table).
- 💾 **Backup & Migration**: Encrypted `.cenvbak` backups, plus import from `.env`, Bitwarden, and 1Password CSVs.

---

## 🖥️ Preview

![CryptEnv Preview](/docs/images/imagen_readme.png "CryptEnv Interface")

---

## 🚀 Quick Start

### Prerequisites
- [Rust](https://rustup.rs/) (stable toolchain; MSVC on Windows)
- [Node.js](https://nodejs.org/) LTS + [pnpm](https://pnpm.io/)
- **Windows**: Microsoft C++ Build Tools + WebView2 Runtime
- **Linux**: `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`

### Run in Development

```bash
git clone https://github.com/maosuarez/crypt-env.git
cd crypt-env
pnpm install
pnpm tauri dev
```

### Build Production Binaries

```bash
pnpm tauri build
```
Produces platform installers in `src-tauri/target/release/bundle/`. See [docs/building.md](docs/building.md) for full distribution steps.

---

## 📚 Documentation & Guides

| Guide | Description |
|---|---|
| 🤖 **[MCP Integration](docs/mcp.md)** | Configure Claude Desktop, available tools, and AI agent workflows |
| 💻 **[CLI & TUI Guide](docs/cli.md)** | Terminal usage, `crypt-env tui` keybindings, and command runner |
| 🔌 **[REST API Reference](docs/api.md)** | Local HTTPS REST API endpoints (`127.0.0.1:47821`), token auth, and schema |
| 🔗 **[Secret Sharing](docs/sharing.md)** | LAN bridge, offline encrypted `.vault` packages, and Supabase relay setup |
| 🏗️ **[Build & Distribution](docs/building.md)** | Cross-platform build instructions, NSIS installers, and packaging |
| 📐 **[OpenSpec SDD](openspec/)** | Spec-Driven Development specifications and active change proposals |

---

## 🔒 Security Architecture

- **Encryption**: AES-256-GCM authenticated encryption for sensitive database fields.
- **Key Derivation**: Argon2id for master password hashing and key derivation.
- **Volatile Storage**: Encryption keys exist only in RAM during unlocked sessions and are zeroized on drop or lock.
- **Local Isolation**: REST API binds strictly to `127.0.0.1:47821` with token validation using constant-time comparison (`subtle::ConstantTimeEq`).
- **No Cloud Dependency**: Local-first SQLite database. Cloud sharing (Internet Relay) is strictly opt-in, client-side encrypted, and burn-after-read.

For reporting vulnerabilities, see [SECURITY.md](SECURITY.md).

---

## 📐 Spec-Driven Development (SDD)

This project strictly follows **Spec-Driven Development** with [OpenSpec](openspec/). All features, architectural decisions, and requirement changes originate in `openspec/` before implementation. See [AGENTS.md](AGENTS.md) and [CONTRIBUTING.md](CONTRIBUTING.md) for agent and contributor workflows.

---

## 🗂️ Project Structure

```
crypt-env/
├── openspec/               # OpenSpec Single Source of Truth (specs, changes, config.yaml)
│   ├── config.yaml         # SDD rules, quality gates, and operational guidance
│   ├── specs/              # Living baseline capability specifications
│   └── changes/            # Active and archived delta change plans
├── docs/                   # Dedicated documentation (MCP, CLI, API, Sharing, Building)
├── src/                    # React 19 + TypeScript frontend
│   ├── components/         # UI components (WindowChrome, ProjectManager, GlobalSecrets...)
│   ├── store/              # Zustand global state
│   ├── hooks/              # Tauri invoke() wrappers
│   └── types/              # TypeScript types
├── src-tauri/              # Rust backend (Tauri 2.0)
│   ├── src/
│   │   ├── bin/            # CLI (crypt-env) & MCP (crypt-env-mcp) binaries
│   │   ├── crypto/         # AES-256-GCM + Argon2id encryption logic
│   │   ├── db/             # SQLite connection pool and migrations
│   │   ├── vault/          # Vault orchestration and business logic
│   │   ├── project/        # Project & environment management logic
│   │   └── api/            # Axum local HTTPS REST server
│   ├── Cargo.toml
│   └── tauri.conf.json
├── AGENTS.md               # Agent & developer rules and SDD workflows
├── context.md              # Technical reference and architectural context
├── CONTRIBUTING.md         # Contribution guidelines
├── SECURITY.md             # Security policy and disclosure
└── README.md
```

---

## 📄 License

MIT © [Mao Suárez](https://github.com/maosuarez)
