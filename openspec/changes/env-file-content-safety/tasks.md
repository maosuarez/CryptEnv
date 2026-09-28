## 1. Serializer

- [ ] 1.1 Implement `envfile::serialize_value` and key validation. Verify with unit tests for bare, single-quoted and double-quoted forms, the injection attempt `x\nADMIN=1`, PEM, `$` and `'`.
- [ ] 1.2 Update crypt-env's own dotenv reader (used by `add .env` / `sync`) to the same grammar. Verify with a property test: for random strings, serialize → parse returns the original.
- [ ] 1.3 Add a compatibility fixture test: generate a sample `.env` and check it with python-dotenv, node dotenv and `bash -c 'set -a; . ./file'` when available (the test skips if a tool is missing). Record the results in the design's matrix.

## 2. Robust inspect and write

- [ ] 2.1 Add a file-type gate plus a 1 MiB bounded read in `inspect`, and `NotRegularFile` errors mapped to `NOT_REGULAR_FILE` (409) in the API. Verify with unix tests: `/dev/zero` → error quickly (under 1 s); a FIFO → error without blocking; a 2 MiB file → Foreign.
- [ ] 2.2 Atomic `commit` via tempfile + `sync_all` + symlink re-check + `persist`; arm the `TempEnvFile` guard before commit. Verify with tests: a simulated write failure (a writer seam) leaves the previous content and no `.cenv-*` temp files; mode 0600 on creation.

## 3. Inject

- [ ] 3.1 Use `db.get_items_by_ids` with zeroizing values in inject; add `failed_keys` to `InjectResult`; merge managed lines with the serializer. Verify with tests: an unrelated corrupt item doesn't fail the inject; a referenced corrupt item → listed in `failed_keys`, other keys written.
- [ ] 3.2 Render `failed_keys` in CLI `fill` and the GUI inject result. Verify with `cargo test` (CLI formatting) and `pnpm build`.

## 4. Docs and verification

- [ ] 4.1 Document the quoting rules, `NOT_REGULAR_FILE` and `failed_keys` in `docs/index.html` / `docs/reference.md`. Verify by review.
- [ ] 4.2 Run `cargo clippy --all-targets && cargo test`. All pass.
