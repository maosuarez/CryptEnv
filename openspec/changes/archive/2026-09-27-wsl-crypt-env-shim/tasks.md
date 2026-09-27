## 1. Shared setup contract (`src-tauri/crates/crypt-env-setup`)

- [x] 1.1 Add `LauncherStatus` enum and `launcher` / `launcher_status` / `launcher_note` fields to `ActionReport` with `#[serde(default)]`. Verify with a unit test that a pre-change report JSON still deserializes.
- [x] 1.2 Add the idempotent `PATH`-append line for `$HOME/.local/share/cryptenv/bin` to `write_env_sh`. Verify with a test that asserts the content, and a `sh -c '. env.sh; . env.sh; echo $PATH'` test showing a single occurrence at the end.
- [x] 1.3 Implement `write_launcher(home, linux_exe_path)` per design D3 (fixed template, `sh_single_quote`, marker line, atomic rename, mode 0755, refuse a foreign file). Verify with tests for fresh write, rewrite, foreign file left byte-identical, and hostile path characters (`'`, `$()`, spaces) round-tripping through `sh`.
- [x] 1.4 Implement `remove_launcher(home)`: delete only the marked file, then remove the empty owned directory; no-op when absent. Verify with tests for marked, foreign and absent cases.
- [x] 1.5 Add `apply_launcher(home, Option<&Path>, &mut report)` (Windows CLI, already in the Linux view), called after `apply` so `apply`'s signature and the Linux CLI path stay unchanged, and remove the launcher in `remove`. Update `crypt-env setup wsl` (`src-tauri/src/bin/crypt-env/commands/setup.rs`) to pass `None`. Verify with `cargo test -p crypt-env-setup` and `cargo check`.
- [x] 1.6 Add the `--windows-cli WINPATH` argument to the helper `main.rs`, translated with `wslpath -u` and falling back to `windows_path_to_wsl`. A missing argument gives `skipped: windows CLI not found`. Verify with parse-args tests and a run test with a fake `$HOME`.
- [x] 1.7 Add an integration test that runs the generated launcher against a stub "exe" script. Assert that arguments are forwarded verbatim, the exit code propagates, `CRYPTENV_*` are unset in the child, and a missing target gives exit 127 with the message on stderr.

## 2. GUI backend (`src-tauri/src/wsl/client_setup.rs`)

- [x] 2.1 Resolve `current_exe().parent()/crypt-env.exe` in `host::configure` (existing non-empty file, else `None`) and pass `--windows-cli <path>` as a discrete argv element. Verify with a `MockRunner` test asserting the exact argv, including a path with spaces.
- [x] 2.2 Add a fixed `LAUNCHER_PROBE` constant and a `launcher: bool` field on `WslDistro`. Verify that detection tests still show read-only behavior (no helper/wslpath calls) and that launcher true/false is reported per distro.
- [x] 2.3 Run `cd src-tauri && cargo check && cargo test` and `cargo clippy` with no new warnings.

## 3. Frontend (`src/components/settings/WslIntegrationSection.tsx`, `src/hooks/useWsl.ts`)

- [x] 3.1 Extend the TS types with `launcher`, `launcher_status` and `launcher_note`. Verify with `pnpm tsc --noEmit`.
- [x] 3.2 Row status: "Configured", "Configured · no `crypt-env` command" (offers RECONFIGURE), or "Not configured". Verify with a new case in `WslIntegrationSection.test.tsx`.
- [x] 3.3 Report lines for launcher written, unchanged, deleted and skipped (with reason). Verify with a test.
- [x] 3.4 Reword the mirrored banner: the launcher works without it, and the snippet applies only to a native Linux `crypt-env`. Verify with an updated snapshot/text assertion and the `pnpm test` run.

## 4. Documentation

- [x] 4.1 Update `docs/wsl-windows.md` GUI route step 5 and the mirrored note: `crypt-env` works right after Configure; native Linux build optional; precedence rule; interop and UNC caveats. Verify by reviewing the rendered markdown.

## 5. Manual verification on Windows (installer build)

- [ ] 5.1 With NAT networking, Configure Ubuntu, open a new shell, and check that `command -v crypt-env` resolves to the owned directory. `crypt-env --version` should match the GUI and `crypt-env search <name>` should hit the vault.
- [ ] 5.2 Install a native `crypt-env` into `~/.local/bin` and confirm it takes precedence. Remove it, then run Remove from the GUI and confirm the launcher and its directory are gone and the rc files are unchanged.
- [ ] 5.3 Exercise `crypt-env tui` and an `.env`-writing command through the launcher from a `/home/...` working directory. Record the behavior in `docs/wsl-windows.md`.
- [ ] 5.4 Uninstall CryptEnv and confirm `crypt-env` prints the actionable error and exits non-zero.
