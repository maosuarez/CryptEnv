## Why

Some smaller CLI/TUI defects weaken session secrecy or leave the terminal broken:
- **CLI-8:** the session token is written with `std::fs::write` (mode 0644 under the default umask), then `chmod`ed to 0600 (`bin/crypt-env/client.rs:271-297`), inside a directory created with default 0755 (`:309`). For a moment the bearer token is readable by other local users, and on DrvFs the chmod failure is ignored.
- **CLI-10:** the Windows terminal id is only the console window handle (`terminal.rs:63-66`). There is no start-time guard like the Unix one has. A recycled `HWND` within the 24-hour prune window inherits a live session with no password, and every console-less process shares `win:0`. The WSL launcher's id (`crates/crypt-env-setup/src/lib.rs:370`: distro + sid + tty) has the same reuse problem.
- **CLI-11:** the TUI does blocking work in its 100 ms event loop:
  - `find_config` stats every ancestor directory each frame (slow over 9P on `/mnt/c`);
  - `rows()` recompiles the filter regex several times per frame;
  - `gate()`/`reload()` make synchronous HTTP calls with reqwest's default 30 s timeout.

  So a stalled backend freezes the UI.
- **CLI-12:**
  - `tui::run` enables raw mode before `EnterAlternateScreen` and `Terminal::new`. If either fails, `?` returns with the terminal still in raw mode (`tui.rs:207-210`).
  - `add` logs a failed item POST and still exits 0 (`commands/add.rs:108-113`).
  - `session_alive` deletes the token on any status other than 2xx or 403, including 5xx and 429 (`client.rs:502-504`), which forces a needless password prompt.

## What Changes

- **Token files are created private.** They are created with `create_new` and mode 0600 as a uniquely named temp file, then renamed into place. The token directory is created with 0700. The existing best-effort rule for filesystems that can't honor mode bits is kept.
- **Terminal ids include a start time:**
  - Windows: the console host process id plus its creation time (`GetConsoleWindow` → `GetWindowThreadProcessId` → `GetProcessTimes`). With no console, a per-process id is used, so detached processes each have their own session and never share one.
  - WSL launcher: the id also includes the session leader's start time from `/proc/<sid>/stat`.
- **TUI responsiveness.** The workspace root is cached and re-resolved only on `r` or a directory change. The compiled filter is cached per filter string. HTTP calls from the TUI use a 5-second timeout, and a "working…" status is shown.
- **Terminal restore guard.** A guard object restores the terminal (leave the alternate screen, disable raw mode) on every exit path, including setup failures after raw mode was enabled.
- **`add` exit status.** `add` exits non-zero when any item fails to be added and prints a summary of added and failed key names.
- **Token kept on transient errors.** The session token is deleted only on 401. On 5xx, 429 or a network error it is kept, and the error is reported.

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `cli`: *Session-token storage and permissions*, plus new requirements for terminal identity uniqueness, token retention on transient errors, and the `add` exit status.
- `tui`: new requirements for a responsive event loop and terminal restoration.

## Impact

- `bin/crypt-env/client.rs`, `bin/crypt-env/terminal.rs`, `bin/crypt-env/commands/{tui.rs,add.rs}`, `crates/crypt-env-setup/src/lib.rs` (launcher script), and the `windows` crate features (`Win32_System_Threading`, `Win32_UI_WindowsAndMessaging`).
- Existing Windows sessions get new ids, so users re-enter the password once after upgrading.
- Docs: `docs/cli.md` (the `add` exit code, session behavior note).

## Non-Goals

- The TUI password-prompt hang and the Ctrl+C handling: handled in `harden-cli-manifest-and-sessions`.
- Converting the TUI to async I/O.
