# 💻 CLI & Interactive TUI Guide

CryptEnv provides a command-line interface (`crypt-env`) and an interactive terminal UI (`crypt-env tui`) built with `ratatui` for terminal-centric workflows. Both talk to the running desktop app through the local authenticated REST API (`127.0.0.1:47821`).

---

## 📁 Projects on disk: `.crypt-env.yaml`

A CryptEnv project is tied to a **root directory** — the directory holding `.crypt-env.yaml`. The CLI finds it by searching from the current directory upward (like `.git`). The file carries **metadata only** (never a secret value) and is safe to commit:

```yaml
# Managed by crypt-env (https://maosuarez.com). Contains NO secret values — safe to commit.
project:
  name: my-service
  description: Backend authentication service
  categories:
  - backend
  environments:
  - name: default
    isDefault: true
    paths:
    - .env
  - name: production
    isDefault: false
    paths:
    - apps/api/.env.production
```

- Environment `paths` are the files an environment is materialized into. **Relative paths** are resolved against the project root (preferred — they work on every machine and across WSL ↔ Windows); absolute paths are used as-is.
- The vault stores the root as the desktop app sees it. When the CLI is a native Linux binary inside WSL and the app runs on Windows, absolute paths are translated automatically (`/home/u/app` ↔ `\\wsl.localhost\<distro>\home\u\app`, `/mnt/c/x` ↔ `C:\x`). Set `CRYPTENV_PATH_TRANSLATION=off` if the app itself runs inside the distro.
- A legacy `crypt-env.json` is still read (with a migration notice) when no `.crypt-env.yaml` exists; run `crypt-env init` to migrate.

---

## 🖱️ Commands

| Command | Password | What it does |
|---------|:--------:|--------------|
| `init [NAME] [--path PATH]` | — | Registers (or links) the project, records this directory as its root, writes `.crypt-env.yaml` |
| `config [--relink] [--yes]` | on path changes | Syncs `.crypt-env.yaml` ⇄ vault; the most recently modified side wins. Changes to where secrets are written need a session and a confirmation |
| `add KEY=value \| $VAR \| FILE [--env NAME] [--global]` | on collision | Adds secrets; an existing key halts the whole addition |
| `fill [--env NAME]` | ✔ | Writes every environment's target files + sanitized `.env.example` |
| `sync [--global] [--env NAME] [--example PATH]` | ✔ | Provisions keys from `.env.example` into the vault |
| `inject KEY... [--env NAME] [--shell SHELL]` | ✔ | Prints shell assignments for `eval` |
| `search [PATTERN] [--global]` | ✔ | Lists variable names and metadata — never values |
| `doctor` | — | End-to-end diagnostics |
| `setup wsl [DISTRO] [--remove]` | — | Configures the CLI for the WSL ↔ Windows split |
| `tui` | login / reveal | Interactive terminal UI |

"Password ✔" commands need a **live session in the current terminal** — like `sudo`. Without one they ask for the master password (`POST /unlock`; the password lives only in a zeroized buffer, and a wrong one aborts with no side effects). The session then lasts the GUI's **auto-lock timeout** (Settings, default 5 min; "Never" still means 5 min for CLI sessions) and **every command renews it**, so a chain of commands asks once. It lapses after that long without use.

**Locking the vault ends every CLI session.** Locking in the GUI (manually, by auto-lock, reset or restore) invalidates all terminals' sessions at once, and a locked vault never renews one. After you unlock again, the next gated command in each terminal asks for the master password.

Sessions are per terminal: each terminal (Unix: session id + tty; Windows: console window, i.e. per Windows Terminal tab) keeps its own token file `<token path>.<hash>` next to `CRYPTENV_TOKEN_PATH`/the default path, so opening another terminal means entering the password there too, and terminals never log each other out. Subshells such as `eval "$(crypt-env inject X)"` count as the same terminal. The binding is enforced by the client (another process running as your user could read the files, as with any cached token); token files unused for a day are deleted (only files named exactly `<token path>.<16 hex>`; anything else next to them is never touched).

### `crypt-env init [NAME] [--path PATH]`
```bash
cd ~/code/my-service
crypt-env init                       # project "my-service", default env → .env
crypt-env init backend-api --path ./app   # default env → app/.env
```
`NAME` defaults to the legacy `crypt-env.json` project, else the folder name. If a project with that name already exists and has no root, it is linked to this directory only after the same confirmation `config` uses (diff, session, `y/N`). If it is already bound to another directory, `init` fails with the bound root and suggests `config --relink`; the project is not touched. If `.crypt-env.yaml` already exists, `init` warns and changes nothing.

### `crypt-env config`
Compares the file's modification time with the vault's last change (project or any environment). File newer → the vault is updated (description, categories, environments, paths; missing categories are created). Vault newer → the file is rewritten. Vault environments missing from the file are **kept** (they may hold secrets) and reported — delete them from the GUI. A project renamed in the file is treated as a new project.

**Root binding.** A project with a root is bound to that directory. Running `config` (or `fill`) from any other directory, for example a fresh clone of the repository, fails with `project 'X' is bound to <root>, not this directory` and changes nothing. `crypt-env config --relink` is the only way to move a project to another directory; it always asks for confirmation.

**Consent for path changes.** A push that sets or changes the root, or adds, removes or changes any environment's target paths, decides where decrypted secrets get written. `config` prints a diff (absolute paths outside the project root are marked), requires a live session (asks for the master password if there is none) and asks `y/N`, default no. When stdin is not a terminal, pass `--yes`; without it the command fails and the vault is unchanged. Changes to name, description or categories, and creating a brand-new project, need no confirmation. In the TUI, `c` shows the same diff in a confirmation modal and a root mismatch is shown as an error (no relink from inside the TUI).

### `crypt-env add`
```bash
crypt-env add OPENAI_API_KEY=sk-...          # literal
crypt-env add '$DATABASE_URL'                # from this shell's environment
crypt-env add .env.local                     # every entry of a dotenv file
crypt-env add SHARED_URL=https://x --global  # reusable across projects
crypt-env add STRIPE_KEY=sk_live --env production
```
If any key already exists in the target environment (or among global items with `--global`), nothing is added: the CLI prints `Error: Key 'K' already exists in environment 'E'. Addition aborted.` and offers to show the colliding value — only after `y` **and** the master password.

### `crypt-env fill [--env NAME]`
Materializes each environment (or only `--env`) into its configured paths — written by the app, so WSL repos work from Windows — and writes/extends a `.env.example` (keys only) next to every target. A pre-existing file not created by crypt-env is backed up to `<file>.bak` first. Environments without paths are skipped with a hint. `.env.example` is written next to each `.env` that was actually written. `fill` refuses to run from a directory other than the project's bound root (see `config --relink`). A target that is a symlink is refused (`TARGET_SYMLINK`, nothing written), and a relative path that leaves the project root through a symlinked directory is rejected; configure the real file instead.

### `crypt-env sync [--global]`
Reads `.env.example` (current directory, then the project root, or `--example`) and creates every key missing from the environment as a vault item with the value `change-me`. With `--global`, keys matching a global secret are **linked** to it instead, and the environment is then written to its `.env` target(s).

### `crypt-env inject KEY...`
```bash
eval "$(crypt-env inject DATABASE_URL API_KEY)"                # bash / zsh
crypt-env inject DATABASE_URL --shell pwsh | Invoke-Expression  # PowerShell
```
The password prompt goes to the terminal; only the single-quoted assignments go to stdout. Every key is resolved before anything is printed, so a missing key never produces a half-applied `eval`. Keys are looked up in the environment, then among global items.

### `crypt-env search [PATTERN] [--global]`
```bash
crypt-env search            # every variable of the project, all environments
crypt-env search %TOKEN     # substring (%x, x%, %x%)
crypt-env search '^DB_'     # case-insensitive regex
crypt-env search --global   # global vault items (works outside a project)
```

### `crypt-env doctor`
Reports: app/health endpoint, vault lock state, MCP token, TLS certificate (path + days to expiry), CLI session token (and whether its file is readable by others), MCP token file, `.crypt-env.yaml` validity, and WSL status (inside a distro: configured or not; on Windows: installed distributions).

### `crypt-env setup wsl [DISTRO]`
- **Inside WSL**: detects the distribution and writes `~/.config/cryptenv/env.sh` plus the rc marker blocks (non-destructive; `--remove` reverses it).
- **On Windows**: lists installed distributions; with none it says so, without `DISTRO` it lists them and asks you to run `crypt-env setup wsl <distro>`, with `DISTRO` it configures that distribution through the bundled helper.

See the [WSL ↔ Windows guide](wsl-windows.md).

### Removed commands
`memory`, `list`, `exec`, `cmd`, `project`, `share`, `relay`, `category` and `set` no longer exist (the parser reports an unrecognized subcommand). Sharing, saved commands, categories and project deletion remain in the desktop GUI; use `inject` instead of `set`.

---

## 🖥️ Interactive TUI (`crypt-env tui`)

```bash
crypt-env tui
```
Three panes — projects · environments (plus a *global items* entry) · variables with values masked. It opens on the project of the current directory's `.crypt-env.yaml`.

| Key | Action |
|-----|--------|
| `←`/`→`, `h`/`l`, `Tab` | Switch pane |
| `↑`/`↓` or `j`/`k` | Move selection |
| `/` | Filter variables (regex, or `%substring`) |
| `v` | Reveal the selected value — asks for the master password unless this terminal has a live session; the value is wiped when the popup closes |
| `i` | `init` the current directory |
| `c` | `config` sync (a path change opens a confirmation modal) |
| `f` | `fill` (session / master password) |
| `s` / `S` | `sync` / `sync --global` (session / master password) |
| `d` | `doctor` |
| `r` | Reload |
| `?` | Help |
| `q` / `Ctrl+C` | Quit (Ctrl+C works in every state, including password prompts) |

---

## ⚙️ Client configuration (environment variables)

All are optional. The CLI, `crypt-env tui` and `crypt-env-mcp` read the first three; they exist mainly for split setups (see the [WSL ↔ Windows guide](wsl-windows.md)).

| Variable | Default | Effect |
|----------|---------|--------|
| `CRYPTENV_API_URL` | `https://127.0.0.1:47821` | REST base URL. Must be an absolute `http`/`https` URL — a malformed value aborts before any request. A non-loopback host prints a one-line stderr warning, then proceeds. |
| `CRYPTENV_CERT_PATH` | probe `APPDATA` → `XDG_DATA_HOME` → `HOME/.local/share`, each joined with `com.maosuarez.cryptenv/tls/cert.pem` | TLS certificate PEM, used verbatim when set (never falls back, never disables verification). |
| `CRYPTENV_TOKEN_PATH` | `%APPDATA%\com.maosuarez.cryptenv\.cli_token` (Windows) or `~/.local/share/com.maosuarez.cryptenv/.cli_token` | Session-token file. If its permissions cannot be tightened (e.g. a `/mnt/c` path under WSL), the token is kept and the command continues. |
| `CRYPTENV_PATH_TRANSLATION` | on inside WSL | `off` disables WSL ↔ Windows path translation (app running inside the distro). |
| `CRYPTENV_TERMINAL_ID` | detected | Overrides the terminal identity used for per-terminal sessions. The WSL managed launcher sets it (distro + session id + tty) and forwards it to the Windows CLI via `WSLENV`. |

Use `crypt-env setup wsl` to persist `CRYPTENV_API_URL` and `CRYPTENV_CERT_PATH` into your shell startup.
