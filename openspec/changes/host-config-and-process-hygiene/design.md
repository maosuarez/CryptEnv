## Context

- **MCP config generation:** a Tauri command resolves the target path, reads the file, merges, and writes with `std::fs::write`.
- **WSL:** the `WslRunner` trait's `SystemRunner` uses `std::process::Command::output()` inside `spawn_blocking` under a 120 s `tokio::time::timeout`. Some paths in `wsl/mod.rs` already use `tokio::process` with timeouts (lines 149, 180, 200).

## Decisions

### D1. `hostcfg::merge_json_entry(path, pointer, value)`
- Read. If the file is missing, create a new `{}`. If parsing fails, return `HostCfgError::Unparseable { snippet }`, where `snippet` contains the entry with the token replaced by `<your MCP token — shown in Settings>`.
- Write: `tempfile` in the same dir → `persist`. A `.bak` is written once (only if none exists) before the first modification. Unix: `create_new` + 0600 when the target didn't exist; the mode of an existing file is preserved.
- The GUI shows the snippet with a copy button. The token itself is copied through `copySecret` (see `gui-lock-hygiene`).

*Rejected:* a JSONC-preserving editor (a new dependency, and complexity for a rare case).

### D2. Async WSL runner
`WslRunner::wsl` becomes `async fn run(&self, args, timeout)` using `tokio::process::Command` with `kill_on_drop(true)`, `CREATE_NO_WINDOW`, and `WSL_UTF8=1`. On timeout: `child.kill().await` + `child.wait().await`. Configure/remove use 120 s, probes 30 s.

The state listing parses `wsl.exe -l -v` (UTF-16LE, via the existing decoder) into `{name, state, version}`. Only `Running` distributions are probed. Names starting with `docker-desktop` are excluded.

### D3. Serialization
A `static WSL_OP: tokio::sync::Mutex<()>`, taken with `try_lock()`; failure → `WslError::Busy`, shown as "operation in progress".

## Security & Threat Model

- The token file is not world-readable when created.
- No silent destruction of user configuration.
- No orphaned `wsl.exe` processes. Stopped distributions (which may contain sensitive services) are not booted as a side effect.

## Risks / Trade-offs

- [Users expect auto-detection of stopped distros] → An explicit Detect button with a clear label.
- [`wsl -l -v` localisation of state words] → Parse by column position and map the known localized "Running" strings; unknown → treat as stopped (safe: the user can still click Detect).
