## 1. Endpoint resolution

- [x] 1.1 Add `resolve_api_base() -> Result<String, CliError>` in `client.rs`: read `CRYPTENV_API_URL`, use built-in default when unset/empty, validate set values parse as an absolute `http`/`https` URL (error naming the var otherwise). Verify with unit tests covering unset, valid override, and malformed value.
- [x] 1.2 Cache the resolved base in a `OnceLock<String>` and expose `api_base() -> &'static str`; call `resolve_api_base()` once at each command entry so the error surfaces as normal `Result` flow. Verify `cargo check` passes.
- [x] 1.3 Replace every `format!("{API_BASE}/…")` call site (client.rs + `commands/*.rs`) with `api_base()`; remove the old `const API_BASE`. Verify `cargo check` and `grep -r API_BASE` shows no stale const.
- [x] 1.4 Emit one stderr warning from `resolve_api_base()` when the host is not loopback (`127.0.0.0/8`, `::1`, `localhost`); still return the URL. Verify a unit test asserts the warning fires once for a non-loopback host and never for loopback.

## 2. TLS trust anchor

- [x] 2.1 In the cert-path resolution, short-circuit to `CRYPTENV_CERT_PATH` when set/non-empty; on unreadable file return a `CliError` naming the path (no fallback to `APPDATA`/`XDG_DATA_HOME`/`HOME`). Verify unit tests for: valid override used, missing override errors, unset preserves the existing probe order.
- [x] 2.2 Confirm the pinned-cert client build still rejects a mismatched/invalid cert with the override branch active and no `danger_accept_invalid_certs` anywhere. Verify with `grep -r danger_accept_invalid_certs` (no hits) and a test that a wrong cert fails the handshake.

## 3. Session token storage

- [x] 3.1 Add `CRYPTENV_TOKEN_PATH` support to `token_path()` (set/non-empty wins, else current default). Verify unit tests for override and default.
- [x] 3.2 Make `write_token_file` treat a non-Windows `set_permissions(0o600)` failure as non-fatal when the content write succeeded; keep propagating a real write failure. Verify a unit test simulating a chmod-refusing path keeps the token and returns `Ok`, and a test where the write itself fails returns `Err`.

## 4. `crypt-env setup wsl` subcommand

- [x] 4.1 Add the `Setup { Wsl { remove: bool, cert_path: Option<PathBuf>, api_url: Option<String> } }` subcommand to `main.rs` and a `commands/setup.rs` module. Verify `crypt-env setup wsl --help` renders.
- [x] 4.2 Implement `env.sh` generation: create `~/.config/cryptenv/` and write `env.sh` (mode 0644) with the resolved `export` lines; values come from flags → environment → documented WSL defaults, erroring if the Windows cert path can't be derived and no `--cert-path` given. Verify a unit test writes and re-writes `env.sh` deterministically.
- [x] 4.3 Implement the marker-block manager: whole-file read → detect `# >>> cryptenv initialize >>>` → if absent, back up once to `<rc>.cryptenv.bak` and append the block; if present, leave the rc byte-identical. Apply to `~/.bashrc` (create if neither rc exists) and `~/.zshrc` (only if it already exists). Verify unit tests: fresh append preserves prior content + order, re-run leaves the file unchanged and creates no second backup, zsh handled only when present.
- [x] 4.4 Implement `--remove`: delete the inclusive marker line range from each rc that has it, `rm -f` the `env.sh`, preserve all other lines, exit 0 when nothing to remove. Verify unit tests for surgical removal and the no-op case.
- [x] 4.5 Guard against symlinked/write-protected rc files: operate on the link target; on write failure error out before deleting or partially writing. Verify a unit test with a read-only target returns `Err` and leaves the backup and original intact.

## 5. Shared consumers

- [x] 5.1 Confirm `crypt-env-mcp` picks up the same endpoint/cert resolution (shared module or mirrored change). Verify `cargo check -p` for the mcp binary and a smoke test with `CRYPTENV_API_URL` set.
- [ ] 5.2 Run the TUI path (`crypt-env tui`) against an overridden `CRYPTENV_API_URL` in a local manual check; verify it connects and lists items.

## 6. Documentation

- [x] 6.1 Add the WSL ↔ Windows topology guide (docs set): networking prerequisite (`networkingMode=mirrored` + `portproxy`/`socat` fallback), explicit "API bind unchanged, 127.0.0.1 only" note, data-dir separation table, cert-rotation "reference not copy" note, and a `crypt-env setup wsl` walkthrough. Verify the page renders and is linked from `AGENTS.md`'s CLI section.
- [x] 6.2 Document the three env vars (`CRYPTENV_API_URL`, `CRYPTENV_CERT_PATH`, `CRYPTENV_TOKEN_PATH`) with defaults and the non-loopback warning in the CLI reference. Verify the reference lists all three.

## 7. Verification

- [x] 7.1 `cd src-tauri && cargo test` green, including the new unit tests for endpoint, cert, token, and `setup wsl` file manipulation.
- [x] 7.2 `cd src-tauri && cargo clippy --all-targets` clean (no `unwrap`/`expect` in the new production paths).
- [ ] 7.3 End-to-end manual check on WSL against a running Windows GUI: `crypt-env setup wsl`, open a new shell, `crypt-env search <name>` succeeds; `crypt-env setup wsl --remove` restores the shell and `~/.bashrc` matches the pre-setup content except for expected whitespace.
- [x] 7.4 `openspec validate cli-remote-endpoint-config --strict` passes.
