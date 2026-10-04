## 1. Host config

- [x] 1.1 Add `hostcfg::merge_json_entry` (strict parse → refuse with a token-free snippet; atomic write; one-time `.bak`; 0600 on create). Verify with unit tests: JSONC input unchanged + error snippet has no token; valid input keeps the other servers; a created file is mode 0600.
- [x] 1.2 Use it in the MCP config generation in `vault/mod.rs` (and in the backend `/mcp-servers` writer once `mcp-token-capabilities` lands). Show the snippet in the GUI with copy actions. Verify with `cargo test` and `pnpm build`. Done: `app_generate_mcp_config` and the backend `/mcp-servers` writer (`api/mcp_servers.rs`, which had a fixed `.tmp` name and no `.bak`) both go through `hostcfg`; the setup wizard shows the refusal snippet with a copy button. Verified with `cargo test`, `tsc --noEmit` and `vitest` (not `pnpm build`).

## 2. WSL

- [ ] 2.1 Async `WslRunner` with `kill_on_drop`, per-call timeouts, and kill + await on timeout. Verify with a unit test using a fake runner that sleeps (killed at timeout), and `cargo check` on Windows. IMPLEMENTED (async `WslRunner`, `run_process` with kill_on_drop + kill/await on timeout, 30 s/120 s budgets; unit tests kill/reap a real `sleep` child on Unix; the CLI `setup wsl` runner was moved onto it). Left unchecked: the `cargo check` on Windows (`SystemRunner` is `cfg(windows)` and could not be compiled here).
- [ ] 2.2 State listing via `wsl -l -v`; probe running distros only; exclude `docker-desktop*`; a per-distro Detect command. Verify with parser unit tests on captured UTF-16LE fixtures (English and Spanish), and a manual check that a stopped distro stays stopped. IMPLEMENTED (`wsl -l -v` parser by header column offsets, EN/ES fixtures, `docker-desktop*` excluded, `wsl_detect_distro` command + GUI Stopped row). Left unchecked: the manual Windows check that a stopped distro stays stopped; fixtures are synthetic UTF-16LE, not captured from a real host.
- [ ] 2.3 `WSL_OP` serialization with a `Busy` error and the GUI message. Verify with a unit test (the second `try_lock` fails) and a manual double click. IMPLEMENTED (`WSL_OP` try_lock -> `WslError::Busy`, GUI message; unit test for the second claim failing). Left unchecked: the manual double click on Windows.

## 3. Docs and verification

- [x] 3.1 Update `docs/index.html` and `docs/wsl-windows.md` (stopped distros, Detect action, MCP config refusal behavior). Verify by review.
- [ ] 3.2 Run `cargo clippy --all-targets && cargo test`, and a manual Windows run of the WSL panel. Record the results. `cargo test` and `cargo clippy --all-targets` run on Linux (no new warnings in touched code). Left unchecked: the manual Windows run of the WSL panel.
