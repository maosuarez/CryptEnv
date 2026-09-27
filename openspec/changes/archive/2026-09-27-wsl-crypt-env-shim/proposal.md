## Why

After **Settings → WSL Integration → Configure**, a WSL distro has `CRYPTENV_API_URL` and `CRYPTENV_CERT_PATH` set, but there is still no `crypt-env` command inside it. The only CLI that works immediately is the Windows `crypt-env.exe`, which WSL runs through interop, but users have to type the `.exe` suffix. A native Linux `crypt-env` also needs mirrored networking (`wsl --shutdown`), a Linux build and a separate session token. Most users only want `crypt-env` to work in their WSL shell after one click.

## What Changes

- **Configure** also installs a small managed POSIX launcher named `crypt-env` inside the distro. It runs the Windows-installed `crypt-env.exe` through WSL interop, so users type `crypt-env …` in WSL with no `.exe`.
- The location of the Windows CLI is resolved when Configure runs, from wherever that CryptEnv install actually lives. It is then translated into the distro's view of that path. No user name, drive or install directory is hard-coded, so this works for any Windows user, any install location and any WSL automount root.
- The launcher lives in a directory CryptEnv owns. That directory is added to the **end** of `PATH` by the managed `env.sh`, so a native Linux `crypt-env` that is already on `PATH` still wins.
- The launcher never overwrites a file it did not create. If the Windows CLI cannot be found, Configure still applies the environment configuration and reports that the launcher was skipped and why.
- **Remove** deletes the launcher (and its now-empty directory) along with what it already removes.
- Detection reports, for each distro, whether the launcher is installed. The panel shows "Configured" vs "Configured, no `crypt-env` command (Reconfigure)" and lists the launcher in the action report.
- Mirrored-networking guidance is reworded: it is required only for a native Linux `crypt-env`, not for the launcher.

## Non-Goals

- Shipping, downloading or building a native Linux `crypt-env` binary.
- A second command name (for example `cryptenv`). The only command is `crypt-env`.
- Translating Linux path arguments (`/home/...`) into Windows paths for the Windows CLI. Relative paths and the current directory are handled by WSL interop as-is.
- Installing the launcher from the Linux `crypt-env setup wsl` subcommand. That path already implies a native binary.
- Changing `.wslconfig`, the REST API, the token model or TLS.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `wsl-integration`: Configure/Remove/detection additionally install, remove and report a managed `crypt-env` launcher that delegates to the Windows CLI. Mirrored-networking guidance is scoped to native Linux clients. This capability is introduced by the not-yet-archived change `windows-installer-cli-and-wsl-panel`, so this delta adds requirements and must be archived **after** that change.

## Impact

- `src-tauri/crates/crypt-env-setup` (shared `cryptenv_setup` lib and the static helper): launcher write/remove, the `PATH` line in `env.sh`, a new `--windows-cli` helper argument, and new optional `ActionReport` fields (backward-compatible serde defaults).
- `src-tauri/src/wsl/client_setup.rs`: resolve the Windows CLI path next to the running GUI and pass it to the helper; extend the detection probe.
- `src/components/settings/WslIntegrationSection.tsx`, `src/hooks/useWsl.ts`: launcher status, report lines and banner copy.
- `docs/wsl-windows.md`: document the GUI route without a Linux binary.
- **Security:** no secrets are involved. The launcher is a fixed script whose only variable is a single-quoted path. The Windows CLI keeps using its own Windows-side token and certificate, and WSL's `CRYPTENV_*` variables are removed before the call so a `WSLENV` forward cannot point the Windows CLI at a `/mnt/c` path. The new write stays inside a CryptEnv-owned directory in the user's `$HOME` and refuses to replace foreign files.
