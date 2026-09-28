## 1. Host config

- [ ] 1.1 Add `hostcfg::merge_json_entry` (strict parse → refuse with a token-free snippet; atomic write; one-time `.bak`; 0600 on create). Verify with unit tests: JSONC input unchanged + error snippet has no token; valid input keeps the other servers; a created file is mode 0600.
- [ ] 1.2 Use it in the MCP config generation in `vault/mod.rs` (and in the backend `/mcp-servers` writer once `mcp-token-capabilities` lands). Show the snippet in the GUI with copy actions. Verify with `cargo test` and `pnpm build`.

## 2. WSL

- [ ] 2.1 Async `WslRunner` with `kill_on_drop`, per-call timeouts, and kill + await on timeout. Verify with a unit test using a fake runner that sleeps (killed at timeout), and `cargo check` on Windows.
- [ ] 2.2 State listing via `wsl -l -v`; probe running distros only; exclude `docker-desktop*`; a per-distro Detect command. Verify with parser unit tests on captured UTF-16LE fixtures (English and Spanish), and a manual check that a stopped distro stays stopped.
- [ ] 2.3 `WSL_OP` serialization with a `Busy` error and the GUI message. Verify with a unit test (the second `try_lock` fails) and a manual double click.

## 3. Docs and verification

- [ ] 3.1 Update `docs/index.html` and `docs/wsl-windows.md` (stopped distros, Detect action, MCP config refusal behavior). Verify by review.
- [ ] 3.2 Run `cargo clippy --all-targets && cargo test`, and a manual Windows run of the WSL panel. Record the results.
