## Context

The token handling lives in `client.rs` (`write_token_file_inner`, `save_token`, `session_alive`), and terminal identity in `terminal.rs`. The TUI loop is `tui::run` → `loop { draw; poll(100ms); handle }`.

## Decisions

### D1. Atomic private token write
`tempfile::Builder::new().prefix(".tok-").tempfile_in(dir)`, which creates the file with 0600 on unix. Then `write_all`, `sync_all`, `persist(path)`. The directory is created with `DirBuilder::new().mode(0o700).recursive(true)`. If `persist` fails on DrvFs, fall back to the existing write + harden path with the warning. `tempfile` is a regular dependency after `harden-cli-manifest-and-sessions`.

### D2. Windows terminal id
```
hwnd = GetConsoleWindow()
if hwnd == 0 → "win:proc:{pid}:{creation_time(self)}"
else pid = GetWindowThreadProcessId(hwnd) (the conhost/OpenConsole or terminal process)
     "win:{pid}:{creation_time(pid)}"
```
`GetProcessTimes` goes through `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`. If that fails, fall back to the per-process id, which is safe (only extra prompts).

The WSL launcher appends `$(cut -d' ' -f22 /proc/$sid/stat)` (the leader start time), and mirrors the Unix native id.

### D3. TUI loop hygiene
- `App.workspace: Option<Workspace>` is cached; it is invalidated on `r` and after `init`/`config`.
- `App.filter_cache: Option<(String, Regex)>`.
- `client::http_client_with_timeout(5s)` is used when `NON_INTERACTIVE` is set (the flag from harden D3).
- A `status = "working…"` frame is drawn before a blocking call.

### D4. Restore guard
`struct TerminalGuard;` with `impl Drop` (leave the alternate screen, disable raw mode) is created right after `enable_raw_mode()` succeeds. The existing panic hook stays.

### D5. Error semantics
- `session_alive`: `401 → clear_token; Ok(false)`, `403 → VaultLocked`, `5xx/429 → Err(Api(status))`, network → `ConnectionRefused`.
- `add` collects `Vec<(key, Result)>`, prints the summary, and returns `Err(CliError::Partial)` → exit code 2.

## Security & Threat Model

- Closes the token-read race for other local users.
- Prevents session inheritance by a new terminal on Windows or WSL.
- There are no new secret outputs; `add` reports key names only.

## Risks / Trade-offs

- [One-time re-prompt after the upgrade on Windows] → Documented.
- [`OpenProcess` denied for an elevated terminal host] → The fallback is the per-process id (a safe failure mode).
