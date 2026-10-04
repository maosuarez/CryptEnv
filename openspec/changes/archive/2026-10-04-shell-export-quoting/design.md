## Context

The CLI has `bin/crypt-env/shell.rs`. The GUI builds strings in TypeScript. The library crate (`crypt_env_lib`) is shared by the GUI and the bins.

## Decisions

### D1. Formatter in the library crate
`src-tauri/src/shellfmt.rs`: `enum Shell`, `fn format_assignment(shell, key, value) -> Result<String, ShellFmtError>`. The CLI's `shell.rs` keeps `detect_shell` and re-exports the formatter. A Tauri command `shell_format_assignment` exposes it to the GUI (the value is passed from the already-revealed item in the frontend; no new secret exposure).

### D2. Escaping rules
- **PowerShell:** `'` + value with each of `['\'', '\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}']` doubled + `'`. PowerShell single-quoted strings are fully literal otherwise, and newlines are allowed.
- **bash/zsh:** if the value has no `\n`/`\r`: `'` + replace(`'`, `'\''`) + `'`. Otherwise use ANSI-C `$'...'`, escaping `\\`, `\'`, `\n`, `\r`, `\t`, and every other control character as `\xHH`.
- **sh:** a newline value → `ShellFmtError::UnsupportedValue` (POSIX sh has no portable single-token newline quoting that `eval` preserves reliably).
- **Keys:** the regex `^[A-Za-z_][A-Za-z0-9_]*$`.

*Rejected:* base64 plus a decode pipeline (needs `base64` on the PATH and differs per OS).

### D3. Verification by execution
The tests run the real shells when available (`bash`, `zsh`, `sh`, `pwsh`) over a corpus: all five quote characters, `$()`, backticks, `;`, `&`, `|`, newlines, a CRLF, unicode, a 64 KiB value, and the empty string. Each test evaluates the output and compares the variable's value byte for byte, using a sentinel file to detect execution.

## Security & Threat Model

- **Adversary:** a crafted secret value or key (a relay share, an import, a teammate's `.env`).
- **After:** no code execution through inject or copy-as for the supported shells.
- **Residual:** the user pastes into a different shell than the one selected. The GUI labels the shell clearly.

## Risks / Trade-offs

- [`$'...'` is not POSIX] → Used only for bash and zsh; `sh` refuses newline values.
- [The GUI now round-trips through a Tauri command for a copy] → Negligible latency.
