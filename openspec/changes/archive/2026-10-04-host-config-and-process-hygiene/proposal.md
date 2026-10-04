## Why

Two host-integration features can damage the user's machine state:
- **CORE-12:** generating the MCP client config (`vault/mod.rs:1385-1430`) parses the existing `.mcp.json` / host config with `serde_json::from_str(...).unwrap_or_else(|_| json!({}))`. A file with comments or a trailing comma, which many MCP hosts accept, **is silently replaced, deleting every other MCP server the user had configured**. The write is also non-atomic, and the file holding the MCP token is created with default permissions.
- **CORE-13:** WSL detection runs about 3 blocking `wsl.exe -d <name>` calls per distribution through `std::process` (`wsl/client_setup.rs:377-395`), which **boots every installed distribution**, including `docker-desktop` and stopped ones. When the 120-second budget expires, `tokio::time::timeout` abandons the `spawn_blocking` thread, but the `wsl.exe` children keep running. Configure or remove may still finish after the user was shown an error, and repeated clicks pile up processes.

## What Changes

- **MCP config writes never destroy content.**
  - An existing file that doesn't parse as strict JSON is **not** overwritten. The operation fails with a message showing the exact JSON snippet the user can paste in by hand.
  - A file that parses is updated with only the `cryptenv` entry changed.
  - The write is atomic (temp file + rename), and a `.bak` of the previous file is kept once.
  - The file is created with owner-only permissions (0600 on Unix) when crypt-env creates it.

  The same rules apply to the MCP-host config writer that `mcp-token-capabilities` moves into the backend.
- **WSL detection never starts a stopped distribution.** Detection reads distribution states with `wsl.exe -l -v` and probes only running ones. Stopped distributions are listed as `stopped`, with a per-distribution "Detect (starts the distribution)" action. Docker Desktop's internal distributions are skipped.
- **WSL child processes are always reaped.** All `wsl.exe` invocations go through `tokio::process` with `kill_on_drop(true)` and a per-call timeout (30 s for probes, 120 s for configure/remove). On timeout the child is killed and awaited, and a configure/remove that timed out reports that the result is unknown. Concurrent WSL operations are serialized: a second click while one is running is rejected with "operation in progress".

## Capabilities

### New Capabilities
- `host-integration-safety`: rules for crypt-env writing into third-party configuration files (preserve existing content, atomic writes, permissions).

### Modified Capabilities
- `wsl-integration`: *WSL environment detection* must not start stopped distributions, and WSL tooling processes must be bounded and reaped.

## Impact

- `vault/mod.rs` (MCP config generation), a new `hostcfg.rs` helper (safe JSON config merge + atomic write), `wsl/client_setup.rs` and `wsl/mod.rs` (runner → `tokio::process`, state listing, serialization), and the GUI WSL panel (a stopped state with a Detect action; i18n).
- Docs: the WSL panel behavior and the MCP config generation note in `docs/index.html` and `docs/wsl-windows.md`.

## Non-Goals

- Parsing JSONC/JSON5 and preserving comments. We refuse instead of rewriting.
- Changing what the WSL configure step installs.
