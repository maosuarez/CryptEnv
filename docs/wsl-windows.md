# 🪟🐧 WSL ↔ Windows Topology Guide

Run the `crypt-env` CLI / TUI (and `crypt-env-mcp`) inside a WSL distro while the actual vault lives in the **Windows-hosted GUI**. Edit projects and environments from Linux; keep every secret, the master password, and the encryption keys on the Windows side.

This guide covers the one networking prerequisite, how the two clients keep separate data directories, and the two ways to point the WSL client at the Windows vault:

- **GUI route** — Windows installer + **Settings → WSL Integration → Configure** (no Linux build needed). See [GUI route](#gui-route-settings--wsl-integration).
- **Manual route** — `crypt-env setup wsl` from inside the distro. See [Walkthrough](#walkthrough-crypt-env-setup-wsl).

Both routes apply the exact same shell configuration (one shared implementation), and neither ever edits `%UserProfile%\.wslconfig`.

The GUI route also installs a managed `crypt-env` command that runs the Windows `crypt-env.exe` through WSL interop, so it works right away, with no mirrored networking and no Linux build. See [The managed `crypt-env` command](#the-managed-crypt-env-command).

---

## Topology

```
┌───────────────────────────── Windows ─────────────────────────────┐
│  CryptEnv GUI (Tauri)                                              │
│    • SQLite vault + AES-256-GCM  (%APPDATA%\com.maosuarez.cryptenv)│
│    • REST API  ──bind──▶  127.0.0.1:47821  (TLS, pinned self-cert) │
│    • cert.pem + key.pem  (%APPDATA%\...\tls\)                      │
└───────────────────────────────┬──────────────────────────────────-┘
                                │  loopback only — never 0.0.0.0
                                │  (WSL reaches it as 127.0.0.1 via
                                │   mirrored networking or a forwarder)
┌───────────────────────────────┴──────────────────────────────────-┐
│  WSL distro (Ubuntu, …)                                            │
│    • crypt-env  /  crypt-env tui  /  crypt-env-mcp                 │
│    • HTTP client only — no vault, no keys                          │
│    • reads cert.pem across /mnt/c   (references, never copies)     │
│    • own session token in ~/.local/share/com.maosuarez.cryptenv/  │
└───────────────────────────────────────────────────────────────────-┘
```

The client is a pure REST consumer. It needs three things from the Windows side: a reachable `127.0.0.1:47821`, the **public** `cert.pem`, and a place to cache its own session token.

---

## The API bind is unchanged

This setup adds **no** network listener and changes **no** server behavior. The REST API still binds `127.0.0.1:47821` and nothing else; `key.pem` never leaves the Windows profile; TLS trust is still a single pinned certificate with no "accept invalid" path. `CRYPTENV_API_URL` only changes *where the client dials* — reaching the service as `127.0.0.1` is a deployment prerequisite you satisfy with networking config, not a code change.

---

## Networking prerequisite: the client must reach the service as `127.0.0.1`

The pinned certificate is valid for `127.0.0.1` and `localhost` only. The WSL client must therefore connect to a `127.0.0.1:47821` that lands on the Windows loopback.

### Option A — mirrored networking (recommended)

WSL 2 with **mirrored** networking mode shares the host's loopback: `127.0.0.1` inside WSL reaches Windows `127.0.0.1` directly, no forwarder.

`%UserProfile%\.wslconfig` on Windows:

```ini
[wsl2]
networkingMode=mirrored
```

Then `wsl --shutdown` and reopen the distro. Requires Windows 11 22H2+ and a recent WSL.

### Option B — a userspace forwarder (fallback)

On older Windows / WSL, run a forwarder so WSL's `127.0.0.1:47821` is proxied to the Windows loopback.

`netsh portproxy` (run in an elevated Windows shell, uses the WSL vNIC address):

```powershell
netsh interface portproxy add v4tov4 `
  listenaddress=127.0.0.1 listenport=47821 `
  connectaddress=127.0.0.1 connectport=47821
```

or `socat` from inside WSL, forwarding to the Windows host IP (the default route's gateway):

```bash
WIN_HOST=$(ip route show default | awk '{print $3; exit}')
socat TCP-LISTEN:47821,bind=127.0.0.1,fork,reuseaddr TCP:"$WIN_HOST":47821
```

With Option B the TLS SNI/host is still `127.0.0.1`, so the pinned cert keeps validating.

> If the client cannot reach the vault as a loopback address you will see a connection error, or — if you worked around it by pointing `CRYPTENV_API_URL` at a non-loopback host — a stderr warning on every call. Fix the networking instead; do not disable TLS verification (there is no switch for it, by design).

---

## Data-directory separation

The Windows GUI and the WSL client keep **separate** state. Nothing is copied across the boundary except the read-only public certificate.

| Data | Windows (GUI) | WSL (client) |
|------|---------------|--------------|
| Vault DB + encrypted secrets | `%APPDATA%\com.maosuarez.cryptenv\` | — (never present) |
| Master password / derived keys | in-memory while unlocked | — (never present) |
| TLS `cert.pem` (public) | `%APPDATA%\com.maosuarez.cryptenv\tls\cert.pem` | **referenced** at `/mnt/c/Users/<you>/AppData/Roaming/com.maosuarez.cryptenv/tls/cert.pem` via `CRYPTENV_CERT_PATH` |
| TLS `key.pem` (private) | `%APPDATA%\com.maosuarez.cryptenv\tls\key.pem` | — (never read) |
| CLI session token | `%APPDATA%\com.maosuarez.cryptenv\.cli_token` | `~/.local/share/com.maosuarez.cryptenv/.cli_token` (Linux-native, `0600`) |
| MCP token | `%APPDATA%\com.maosuarez.cryptenv\mcp_token` | `~/.local/share/com.maosuarez.cryptenv/mcp_token` |

Keep the WSL session token on the Linux-native filesystem (the default). Putting `CRYPTENV_TOKEN_PATH` on `/mnt/c` means the token is only as protected as the Windows ACLs on that path — a discouraged configuration.

---

## Certificate rotation: reference, never copy

The GUI regenerates its self-signed cert periodically (roughly every 11 months). Point `CRYPTENV_CERT_PATH` at the **live** Windows file over `/mnt/c` so a rotation is picked up automatically:

```
/mnt/c/Users/<you>/AppData/Roaming/com.maosuarez.cryptenv/tls/cert.pem
```

If you copy `cert.pem` into the WSL filesystem instead, the copy goes stale at the next rotation and every request fails the handshake until you re-copy it. `crypt-env setup wsl` always records the `/mnt/c` path, not a copy.

---

## GUI route: Settings → WSL Integration

1. **Install CryptEnv on Windows** with the NSIS installer. It also installs `crypt-env.exe` and `crypt-env-mcp.exe` next to the GUI and adds that folder to your **user** `PATH` (`HKCU\Environment`, no admin). Open a new terminal and `crypt-env --version` prints the same version as the GUI. Uninstalling removes the executables and exactly that `PATH` entry.
2. **Open Settings.** The **WSL INTEGRATION** section appears only on Windows and only when at least one distro is detected (`wsl --list --quiet`). Each distro row shows its default user and whether the cryptenv client is already configured. Use the refresh icon after installing/removing a distro — detection only runs when the section opens or on refresh.
3. **Mirrored networking (native Linux client only).** If `networkingMode=mirrored` is not set under `[wsl2]` in `%UserProfile%\.wslconfig`, the section shows the snippet with a copy button and the `wsl --shutdown` caveat. The managed `crypt-env` command from step 4 does **not** need it; only a native Linux `crypt-env` build does. **The app never writes `.wslconfig`** — applying it restarts every distro and changes networking for all of them, so that stays your call (see [Option A](#option-a--mirrored-networking-recommended)).
4. **Configure.** Click **CONFIGURE** on a distro. The GUI re-checks that the distro still exists, copies a small static helper (`crypt-env-setup`, bundled with the installer) into the distro's `/tmp`, runs it, and deletes it. The helper performs exactly the [`setup wsl`](#walkthrough-crypt-env-setup-wsl) edits — `env.sh`, the marker block, the one-time backup — with `CRYPTENV_CERT_PATH` pointing at the **live** `/mnt/c/.../tls/cert.pem` (never a copy). It also installs the [managed `crypt-env` command](#the-managed-crypt-env-command), pointing at the `crypt-env.exe` next to the running GUI. A report lists the env file written, the rc files that received the block, any backup created, and the `crypt-env` command's outcome. Re-configuring only rewrites `env.sh` and the command. A distro configured before the command existed shows **Configured · no crypt-env command**; click **RECONFIGURE**.
5. **Open a new WSL terminal** and run `crypt-env` there (no `.exe`). With the GUI running and unlocked, `crypt-env search <name>` hits the Windows vault. A native Linux `crypt-env` (e.g. `crypt-env-linux` from the GitHub release) is optional; if it is on your `PATH` it takes precedence over the managed command.
6. **Remove** reverses it: deletes the marker block, `env.sh`, and the managed `crypt-env` command, and leaves every other rc line and the `.cryptenv.bak` backup in place. Removing when nothing is installed is a no-op.

### The managed `crypt-env` command

- **What it is:** a small POSIX script at `~/.local/share/cryptenv/bin/crypt-env` that `exec`s the Windows `crypt-env.exe` belonging to the CryptEnv installation you clicked Configure in. Arguments, stdin/stdout/stderr and the exit status pass through unchanged. It holds only that path: no secrets, token or certificate.
- **Works for any user and install location:** the path is resolved when you click Configure (next to the running GUI) and translated with `wslpath -u`, so custom drives, install folders and `[automount] root` all work. If CryptEnv moves, run **RECONFIGURE**.
- **PATH:** `env.sh` appends `~/.local/share/cryptenv/bin` to the **end** of `PATH`, once. Any `crypt-env` found earlier (a native Linux build in `~/.local/bin`, `~/.cargo/bin`, …) wins.
- **Isolation:** the script unsets `CRYPTENV_API_URL`, `CRYPTENV_CERT_PATH` and `CRYPTENV_TOKEN_PATH` before calling Windows, so the Windows CLI always uses its own endpoint, certificate and token, even if you forward those variables through `WSLENV`.
- **Never clobbers:** the file carries a `# >>> cryptenv managed launcher >>>` marker. Configure and Remove leave any file at that path without the marker untouched, and report it as *skipped*.
- **If the Windows CLI is missing** (for example a dev build without `crypt-env.exe`), Configure still writes `env.sh` and reports the command as *skipped (windows CLI not found)*. If CryptEnv is later uninstalled, `crypt-env` exits `127` with a message naming the missing path.
- **Caveats:**
  - The Windows CLI sees the current directory as `\\wsl.localhost\<distro>\...`. Relative paths work, but absolute Linux paths (`/home/...`) passed as arguments are **not** translated.
  - Each call pays ~50–150 ms of interop start-up.
  - The command needs WSL interop (enabled by default; `[interop] enabled=false` in `/etc/wsl.conf` breaks it).
  - `PATH` comes from `env.sh`, which is sourced from `.bashrc`/`.zshrc`, so non-interactive shells that skip those files must call the full path.

---

## Walkthrough: `crypt-env setup wsl`

Persists `CRYPTENV_API_URL` and `CRYPTENV_CERT_PATH` into your shell startup **non-destructively** — modelled on `conda init` / `rustup`.

What it writes:

- **`~/.config/cryptenv/env.sh`** — a fully-managed file (mode `0644`) containing only the two `export` lines. Rewritten in full on every run; safe to delete.
- **A single marker-delimited block** appended to `~/.bashrc` (and to `~/.zshrc` when that file exists) that sources `env.sh`:

  ```sh
  # >>> cryptenv initialize >>>
  [ -f "$HOME/.config/cryptenv/env.sh" ] && . "$HOME/.config/cryptenv/env.sh"
  # <<< cryptenv initialize <<<
  ```

- **`~/.bashrc.cryptenv.bak`** — a one-time backup, taken only before the first time the block is added.

It never edits, reorders, or pattern-replaces any line outside its own marker block. Re-running rewrites `env.sh` with the current values and leaves the rc files byte-for-byte unchanged.

### Install

```bash
# With mirrored networking, the default URL is correct; supply the cert path
# as seen from WSL:
crypt-env setup wsl \
  --cert-path /mnt/c/Users/<you>/AppData/Roaming/com.maosuarez.cryptenv/tls/cert.pem

# Or, if CRYPTENV_API_URL / CRYPTENV_CERT_PATH are already exported in this
# shell, they are recorded as-is:
crypt-env setup wsl
```

Value precedence per field: **flag → environment → documented WSL default**. The URL defaults to `https://127.0.0.1:47821`. The cert path has no safe default when the Windows profile cannot be located from `APPDATA` / `USERPROFILE`; in that case the command errors and asks for `--cert-path`.

> Tip: to let `setup wsl` derive the cert path automatically, forward the Windows variable into WSL by adding `APPDATA/p` to `WSLENV` in Windows environment settings (append `:APPDATA/p` to any existing value).

Then apply it:

```bash
exec $SHELL -l          # or open a new terminal
crypt-env search <name> # should now hit the Windows vault
```

### Remove

```bash
crypt-env setup wsl --remove
```

Deletes the inclusive marker block from each rc file that has it and removes `~/.config/cryptenv/env.sh`. All other rc content is preserved. It exits `0` even when there is nothing to remove. (`~/.bashrc.cryptenv.bak` is left in place; delete it by hand if you want it gone.)

### Other shells (fish, nu, …)

`setup wsl` only handles bash and zsh. For anything else, source `env.sh` yourself from that shell's startup, e.g. fish:

```fish
test -f ~/.config/cryptenv/env.sh; and bass source ~/.config/cryptenv/env.sh
```

---

## Troubleshooting

| Symptom | Likely cause | Fix |
|---------|--------------|-----|
| `vault is not running` from WSL, GUI is up | `127.0.0.1:47821` in WSL isn't reaching Windows loopback | Enable `networkingMode=mirrored`, or set up Option B |
| `Configuration error: CRYPTENV_API_URL must be an absolute http/https URL` | typo in the exported value | fix `~/.config/cryptenv/env.sh` or re-run `setup wsl` |
| stderr `warning: CRYPTENV_API_URL host '…' is not a loopback address` | pointing at a non-loopback host | revert to `127.0.0.1` and fix networking instead |
| TLS handshake failure after months of working | cert rotated; `CRYPTENV_CERT_PATH` points at a stale copy | point it at the live `/mnt/c/...cert.pem` and re-run `setup wsl` |
| `cannot read TLS certificate at CRYPTENV_CERT_PATH (…)` | wrong path, or `/mnt/c` not mounted | verify the path resolves from WSL (`ls` it) |
