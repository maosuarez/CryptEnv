## 1. Shared setup contract (depends on cli-remote-endpoint-config)

- [x] 1.1 Factor the `setup wsl` file logic (env.sh generation, marker-block insert/detect, one-time backup, surgical `--remove`) into a reusable library function (`cryptenv_setup::apply` / `remove`) shared by the `crypt-env` binary. Verify existing `crypt-env setup wsl` unit tests still pass against the extracted function.
- [x] 1.2 Add a `crypt-env-setup` helper bin target that calls `cryptenv_setup` and prints a JSON action report (`{env_file, env_file_changed, rc_files, backups, marker_added, marker_removed}`). Verify `cargo build -p crypt-env-setup` and a unit test of the JSON shape.
- [x] 1.3 Add a `x86_64-unknown-linux-musl` build of `crypt-env-setup` to the release/CI steps. Verify the static binary runs on a bare WSL distro with no toolchain (`ldd` shows not-a-dynamic-executable or only linux-vdso).

## 2. Windows installer ships the CLI

- [ ] 2.1 Add a pre-`tauri build` step that copies `crypt-env` and `crypt-env-mcp` (Windows target) into `src-tauri/binaries/` with the `-x86_64-pc-windows-msvc.exe` suffix, only for the Windows target. Verify the files land and macOS/Linux builds leave `binaries/` empty.
- [ ] 2.2 Add `bundle.externalBin` entries for both to `tauri.conf.json`. Verify `pnpm tauri build` on Windows produces an installer containing both `.exe`s.
- [ ] 2.3 Extend `nsis/installer_hooks.nsi`: on install add the install dir to `HKCU\Environment` `PATH` and broadcast `WM_SETTINGCHANGE`; on uninstall remove exactly that entry. Verify: fresh install → new terminal → `crypt-env --version` works; uninstall → entry gone.
- [x] 2.4 Add a packaging test/checklist asserting the bundled `crypt-env --version` equals the GUI version for a release build.

## 3. WSL backend module

- [x] 3.1 Extend the existing `src-tauri/src/wsl/` module (new `client_setup.rs`) with typed `WslError` (`Unsupported`, `NotAvailable`, `UnknownDistro`, `Tooling(String)`) and `WslStatus` / `WslActionReport` structs. Verify `cargo check`.
- [x] 3.2 Implement `wsl_detect`: `wsl.exe --list --quiet` UTF-16LE decode; per-distro default user (`-e whoami`) and `configured` probe (`-e sh -c 'test -f ... && grep -q ...'`, exit code only); parse `%USERPROFILE%\.wslconfig` for `[wsl2] networkingMode=mirrored`. All via argument vectors. Verify unit tests with fixture byte strings for list output, a `.wslconfig` with/without the key, and a garbled-bytes case returning `Tooling`.
- [x] 3.3 Implement `wsl_configure_client(distro)`: re-validate `distro` against a fresh `wsl --list`; locate the bundled `crypt-env-setup` via the app resource dir as a `/mnt/c/...` path (fallback `\\wsl$\<distro>`); copy into `/tmp`, `chmod +x`, run with the derived `%APPDATA%` path arg, capture the JSON report, delete the helper. Verify with a mocked runner unit test (no real WSL) covering: unknown distro rejected, arg vector shape, report passthrough.
- [x] 3.4 Implement `wsl_remove_client(distro)`: same validation, runs the helper with `--remove`. Verify mocked-runner unit test.
- [x] 3.5 Add `#[cfg(not(windows))]` bodies (or a runtime guard) returning `WslError::Unsupported` with no side effects for all three. Verify a unit test on Linux asserts each returns Unsupported and writes nothing.
- [x] 3.6 Register the three commands in `src-tauri/src/lib.rs` `generate_handler!`. Verify `cargo check` and that the frontend can `invoke` them.

## 4. Frontend Settings subview

- [x] 4.1 Add a `wsl` Zustand slice (selected distro) and TanStack Query hooks for `wsl_detect` (query) + `wsl_configure_client` / `wsl_remove_client` (mutations invalidating the detect query). Verify types compile and a mock-provider test renders.
- [x] 4.2 Add the "WSL Integration" section to Settings, rendered only when `platform === 'windows'` and `status.available`. Verify it is absent on a simulated non-Windows platform and when `available` is false.
- [x] 4.3 Per-distro row UI (name, user, Configured/Not, Configure/Remove) + post-action report panel echoing `WslActionReport`. Tailwind utilities only. Verify against mocked command responses.
- [x] 4.4 Mirrored-networking banner: show the `[wsl2]`/`networkingMode=mirrored` snippet + `wsl --shutdown` caveat with copy-to-clipboard when `mirrored === false`; hidden when true; no apply control. Verify both states.

## 5. Documentation

- [x] 5.1 Update the WSL topology guide (from `cli-remote-endpoint-config`) with the GUI panel path: install on Windows → Settings → WSL Integration → Configure. Verify the guide covers both the manual (`setup wsl`) and GUI routes and states `.wslconfig` is not auto-edited.
- [x] 5.2 Document the three `wsl_*` commands and the `WslActionReport` shape in the internal API/command reference. Verify all three are listed.

## 6. Verification

- [x] 6.1 `cd src-tauri && cargo test` green including the new `wsl` module unit tests (mocked runner) and the shared `cryptenv_setup` tests.
- [x] 6.2 `cd src-tauri && cargo clippy --all-targets` clean; no `unwrap`/`expect` in new production paths; `grep` confirms no `sh -c "…{distro}…"` string interpolation.
- [x] 6.3 Frontend: `pnpm build` + component tests for the gated section pass.
- [ ] 6.4 Manual end-to-end on Windows 11 + WSL: install build → `crypt-env` on PATH → Settings shows distros → Configure → open WSL shell → `crypt-env search <name>` works against the running GUI → Remove → `~/.bashrc` restored (only marker block gone), `.cryptenv.bak` present.
- [ ] 6.5 Manual check with mirrored networking OFF: banner appears; after applying `.wslconfig` + `wsl --shutdown`, detection flips to enabled.
- [x] 6.6 `openspec validate windows-installer-cli-and-wsl-panel --strict` passes.
