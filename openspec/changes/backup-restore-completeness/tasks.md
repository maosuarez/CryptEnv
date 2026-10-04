## 1. Format

- [x] 1.1 Define the v2 structs and the `Meta` encryption/decryption with the vault key; export writes v2. Verify with a unit test: export → the `meta` hex does not contain project names or paths; a round-trip decode matches.
- [x] 1.2 Parse v1 and v2 on import. Verify with fixture tests for both versions.

## 2. Staging and swap

- [x] 2.1 Add `VaultDb::create_at(path)`, a bulk transactional insert with id remapping, and `VaultDb::close`. Audit for pool clones outside `VaultState` (grep for `pool.clone()` / `db.clone()`) and record the findings in this task. Verify with `cargo check` and the audit note.
  - Audit: `grep` for `pool.clone()` / `db.clone()` / `.pool` outside `db/mod.rs` finds no pool clones; `VaultDb` is not `Clone` and lives only in `VaultState.db` (REST handlers, Tauri commands and the share layer reach it through the `VaultState` mutex guard). The only other `.pool` use is one test in `project/mod.rs`. `VaultDb::open` is called from `lib.rs` (startup) and tests only. Swapping `s.db` under the guard is therefore safe.
  - Deviation: `projects.is_holding` column (additive migration) added so the orphan predicate (`list_orphan_item_ids`, `delete_items_cascade`) can skip items owned by `Restored`/`Imported` projects; required by the 3.2 scenario because the old predicate ignores `item_projects`. The free-space pre-check from design Risks is not implemented (best effort, failure is safe).
- [x] 2.2 Implement the staging restore (replace + merge via `VACUUM INTO`), integrity and token checks, the checkpoint/close/rename swap with rollback, and cleanup of `pre-restore` on the next unlock. Verify with tests: a successful replace round trip; an injected failure before the swap → the original is intact; an injected failure at the second rename → rollback, and the original opens.

## 3. Authorization and ownership

- [x] 3.1 Require unlocked + current-password re-verification for replace, and unlocked for merge. Verify with tests: locked → refused; wrong current password → refused.
- [x] 3.2 Add `ensure_holding_project` for v1 restores and project-less imports. Verify with a test: v1 restore with non-global items → owned by `Restored <date>`; `list_orphan_item_ids` is empty.

## 4. GUI and docs

- [ ] 4.1 Settings Backup/Restore: current-password field for replace, disabled while locked, restore summary (including the v1 "projects not included" note), i18n. Verify with `pnpm build` and a manual round trip in `pnpm tauri dev`.
- [x] 4.2 Update the backup section in `docs/index.html`. Verify by review.
- [x] 4.3 Run `cargo clippy --all-targets && cargo test`. All pass.
