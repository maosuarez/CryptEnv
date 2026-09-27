## Context

See `proposal.md — Why`. Today `src-tauri/src/bin/crypt-env/client.rs` holds `pub const API_BASE: &str = "https://127.0.0.1:47821"` and every command builds URLs with `format!("{API_BASE}/…")`. `tls_cert_path()` probes `APPDATA` → `XDG_DATA_HOME` → `HOME/.local/share`, each joined with `com.maosuarez.cryptenv/tls/cert.pem`; `token_path()` probes `APPDATA` → `HOME/.local/share` for `.cli_token`. The TLS cert is generated with SAN `["127.0.0.1", "localhost"]` (`src-tauri/src/tls/mod.rs`) and the client pins it (`add_root_certificate`, no `danger_accept_invalid_certs`). The REST server binds `const ADDR = "127.0.0.1:47821"` and `cors_guard` only rejects browser `Origin` headers — a non-browser client with no `Origin` is accepted. This same `client.rs` module is compiled into `crypt-env`, the TUI, and is mirrored by `crypt-env-mcp`.

## Goals / Non-Goals

**Goals:**
- Make endpoint, TLS trust anchor, and token path overridable at runtime with zero behavior change when the overrides are absent.
- Keep the override resolution in one place so `crypt-env`, the TUI, and `crypt-env-mcp` all inherit it.
- Give `crypt-env setup wsl` a persistence mechanism that is safe to run repeatedly and safe to reverse, on a shell file the user also hand-edits.

**Non-Goals (design-level):**
- No config-file format. Env vars only; `env.sh` is generated shell, not a parsed format.
- No change to how the *server* resolves its own cert/token/bind.
- No attempt to make the pinned cert valid for a non-`127.0.0.1` host — reaching the service as `127.0.0.1` is a deployment prerequisite, documented, not code.

## Decisions

### D1: Env vars over a config file

`CRYPTENV_API_URL`, `CRYPTENV_CERT_PATH`, `CRYPTENV_TOKEN_PATH`, all optional.

- **Why:** smallest diff; already the idiom in this file (`APPDATA`, `XDG_DATA_HOME`, `HOME` are all env-probed); trivially testable with `std::env::set_var` in unit tests; nothing to parse or version. `setup wsl` gives the persistence a config file would otherwise justify.
- **Alternatives:** (a) `~/.config/cryptenv/cli.toml` parsed by the client — more moving parts, precedence rules, a new dependency surface, and still needs a writer command. (b) Reusing/overloading `APPDATA` on Linux — fragile (`save_token` would attempt `chmod` on a DrvFs path) and semantically wrong.

### D2: `API_BASE` becomes a process-lifetime `LazyLock<String>`

Replace the `const` with `static API_BASE: LazyLock<String>`. Resolution: read `CRYPTENV_API_URL`; if set and non-empty, validate it parses as an absolute `http`/`https` URL (`url::Url` — already transitively present via `reqwest`, else a hand-rolled check) and use it, else use the literal default. Non-loopback host → one `eprintln!` warning. A malformed value → the client exits non-zero before any request; because `LazyLock` can't return a `Result`, the validated value is computed in a small `resolve_api_base() -> Result<String, CliError>` called once from `main`/each command entry and cached, OR the `LazyLock` stores an `enum { Ok(String), Bad(String) }` that command code turns into an error on first use. Prefer the explicit `resolve` + `OnceLock` set from `main` so the error path is normal `Result` flow.

- **Call sites:** ~25 `format!("{API_BASE}/…")` become `format!("{}/…", api_base())` where `api_base()` returns `&'static str`. Mechanical.
- **Why not per-call `env::var`:** repeated syscalls, and the warning would print N times.

### D3: `CRYPTENV_CERT_PATH` short-circuits the probe; missing file is a hard error

In `tls_cert_path()` (and wherever the cert is read), if `CRYPTENV_CERT_PATH` is set: use it verbatim, and if it can't be read, return an error naming the path — do **not** fall through to `APPDATA`/`HOME`. Rationale: an explicit override that silently degrades to a different cert is a footgun (you'd unknowingly trust the wrong file or fail confusingly). Unset → identical to today.

### D4: Best-effort permission hardening on non-Windows

`write_token_file` currently does `fs::write(path, content)?;` then `#[cfg(unix)] set_permissions(path, 0o600)?`. Change the `set_permissions` line so its error is logged/ignored rather than propagated **when the write succeeded**. Keep propagating a genuine write failure. `save_token` already swallows the top-level `Result` (`let _ = …`), but making the intent explicit prevents a future refactor from turning a DrvFs `chmod` failure into a hard error. Windows path unchanged (NTFS ACLs already user-scoped).

### D5: `setup wsl` — marker block + sourced snippet, backup, surgical remove

Model: `conda init` / `rustup`.

- Managed file `~/.config/cryptenv/env.sh` (mode `0644`, created with parent dirs): only `export CRYPTENV_API_URL=…` / `export CRYPTENV_CERT_PATH=…`. Fully rewritten every run.
- rc files: `~/.bashrc` always; `~/.zshrc` only if it already exists; if neither exists, create `~/.bashrc`. Block:
  ```sh
  # >>> cryptenv initialize >>>
  [ -f "$HOME/.config/cryptenv/env.sh" ] && . "$HOME/.config/cryptenv/env.sh"
  # <<< cryptenv initialize <<<
  ```
- Detection: exact string search for the start marker. Present → leave rc untouched, only rewrite `env.sh`. Absent → copy `~/.bashrc` to `~/.bashrc.cryptenv.bak` (only if the `.bak` doesn't already exist), then append the block with a leading blank line.
- `--remove`: read the file, drop the inclusive `[start marker … end marker]` line range, write back only if changed; then `rm -f ~/.config/cryptenv/env.sh`. No-op exits 0.
- Never `sed -i`/regex over user lines. All edits are: whole-file read → block-boundary slice → whole-file write, plus a byte-identical short-circuit when nothing changed.
- Value sourcing: use `CRYPTENV_API_URL` / `CRYPTENV_CERT_PATH` from the environment if present; otherwise fill the documented WSL defaults — URL `https://127.0.0.1:47821`, cert path `"$APPDATA"`-derived Windows path translated to `/mnt/c/...`. When the Windows user directory can't be determined, require `--cert-path` and error out rather than guessing.

### D6: Docs

New guide page (project docs set, cross-linked from `AGENTS.md` CLI section): the topology diagram, the `networkingMode=mirrored` requirement with the `portproxy`/`socat` fallback, the explicit statement that the API bind is unchanged, the data-dir separation table, and cert rotation (reference, never copy).

## Risks / Trade-offs

- **User points `CRYPTENV_API_URL` at a hostile host and types their master password into it.** → Non-loopback stderr warning (D2); docs stress loopback-only; the value is opt-in and explicit, mirroring how `curl`/`kubectl` trust their configured endpoint.
- **`LazyLock` can't express the validation error cleanly.** → Resolve+cache via `OnceLock` set from a `Result`-returning function at command entry (D2), keeping normal error flow; the `LazyLock` sugar is a fallback only if entry-point plumbing proves noisy.
- **`setup wsl` runs against an exotic shell (fish, nu).** → Out of scope; only bash/zsh handled. Document manual `env.sh` sourcing for others; `--remove` still cleans what it wrote.
- **A user's `~/.bashrc` is a symlink or is write-protected.** → Follow the symlink target for read/write as the shell would; on write failure, error out before deleting or half-writing, leaving the `.bak` intact.
- **`env.sh` records a stale `/mnt/c` user path if the Windows profile moves.** → Rare; `setup wsl` re-run regenerates it; the path is visible in the file.
- **Cert override to a copied file goes stale after the ~11-month rotation.** → D3 + docs: point `CRYPTENV_CERT_PATH` at the live `/mnt/c/...` file, never a copy.

## Migration Plan

Purely additive. No migration. Rollback = `crypt-env setup wsl --remove` (reverses shell edits) and unset the env vars; the code defaults restore byte-for-byte the current behavior. No persisted state, no schema, no API contract touched.

## Security & Threat Model

- **Network exposure:** unchanged. The server still binds `127.0.0.1:47821` only. `CRYPTENV_API_URL` changes only where the *client* connects; WSL reaches the Windows loopback via `networkingMode=mirrored` or a user-run forwarder — no new listener, no `0.0.0.0` bind.
- **TLS trust:** unchanged. The client still pins exactly one certificate and has no `danger_accept_invalid_certs` path (D3 keeps it that way even on the override branch). Only the **public** `cert.pem` is ever read across `/mnt/c`; `key.pem` never leaves the Windows profile.
- **Credential flow:** the master password is still sent only to the resolved endpoint over pinned TLS; the non-loopback warning is the tripwire for a misconfigured or malicious `CRYPTENV_API_URL`.
- **Session token at rest:** on WSL the token lives in the Linux-native `~/.local/share/...` by default (own dir, `0600`). If a user sets `CRYPTENV_TOKEN_PATH` onto `/mnt/c`, D4 makes the failed `chmod` non-fatal but the token is then only as protected as the Windows ACLs on that path — documented as a discouraged configuration.
- **Shell-file edits:** always backup-first, marker-scoped, range-delete on removal, never regex over user content (D5). Worst case a user loses the appended block, recoverable from `~/.bashrc.cryptenv.bak`.
- **No secret values** touch logs or error messages; new error strings name env-var names and file paths only.

## Cross-Platform Notes

- The new env vars are read on all platforms but only meaningful where the client and server are split (WSL↔Windows today; a future Linux headless daemon later).
- `setup wsl` is Unix-shell-specific by name and intent; it is a no-op-friendly command a Windows user would never run. It does not gate on `cfg!(unix)` but its file targets (`$HOME/.bashrc`) simply won't exist meaningfully elsewhere.
- macOS: `CRYPTENV_CERT_PATH` / `CRYPTENV_API_URL` work identically; no `/mnt/c` equivalent, so `setup wsl` defaults that can't resolve a Windows profile error out asking for explicit values.
