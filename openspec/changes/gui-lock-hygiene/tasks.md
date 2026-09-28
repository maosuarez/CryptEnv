## 1. UI state

- [ ] 1.1 Add `resetSecretUi()` to the store, move the wizard open state into the store, and gate the overlays on `screen !== 'lock'` in `App.tsx`. Verify with `pnpm build` and a manual check: auto-lock with the placeholder modal open and with the wizard open → only the lock screen shows.

## 2. Backend clipboard

- [ ] 2.1 Add `clipboard.rs`: the `clipboard_write_secret` command (Windows exclusion formats + sequence number; hash elsewhere), a 30 s clear task, and `clear_if_ours`; register it in `lib.rs`; call it from `lock_vault`. Verify with a unit test of the ownership logic using a fake clipboard (ours → cleared; replaced → untouched), and `cargo check` for both targets.

## 3. Frontend migration

- [ ] 3.1 Add `src/lib/clipboard.ts` (`copySecret`/`copyPlain`) and migrate all 13 `writeText` call sites (secret vs plain classified in this task's notes); toast "clears in 30 s". Verify with `pnpm build`.
- [ ] 3.2 Add the guard test that fails on a `writeText` import outside `lib/clipboard.ts`. Verify that it fails with a deliberately added import and passes on the clean tree.

## 4. Docs and verification

- [ ] 4.1 Add a security note to `docs/index.html` (30 s clear, excluded from history). Verify by review.
- [ ] 4.2 Manual Windows check: Win+V does not show the copied secret; clipboard cleared at 30 s and at lock; a later user copy survives. Record the results.
