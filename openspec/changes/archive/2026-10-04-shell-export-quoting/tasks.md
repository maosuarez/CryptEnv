## 1. Formatter

- [x] 1.1 Create `shellfmt.rs` in the library crate (PowerShell five-quote doubling; bash/zsh single-quote or ANSI-C; sh refuses newlines; key regex); `bin/crypt-env/shell.rs` re-exports it. Verify with unit tests over the corpus (string-level).
- [x] 1.2 Add execution tests with real shells when present (bash, zsh, sh, pwsh): byte-exact round trip and sentinel-file non-execution. Verify with `cargo test shellfmt` (it skips shells that are missing and logs which were skipped).

## 2. CLI and GUI

- [x] 2.1 Make `inject` validate the key and propagate `ShellFmtError` to exit ≠ 0 with nothing on stdout. Verify with CLI unit tests.
- [ ] 2.2 Add the Tauri command `shell_format_assignment` registered in `lib.rs`; `SecretRow.tsx` copy-as uses it (and the `.env` serializer for the env format). Verify with `pnpm build` and a manual copy of a value containing `$(...)` and `’`. (code done; manual GUI check pending)

## 3. Docs and verification

- [x] 3.1 Switch every `eval $(crypt-env inject …)` in `docs/index.html`, `docs/cli.md`, `docs/reference.md` and the CLI help to the quoted form, and add the PowerShell form. Verify with `grep -rn 'eval \$(crypt-env' docs src-tauri/src` returning nothing.
- [x] 3.2 Run `cargo clippy --all-targets && cargo test`. All pass.
