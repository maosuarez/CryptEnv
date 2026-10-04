## Why

Several vault write paths are multi-statement but not transactional, and some rely on per-connection pragmas that silently go away:
- **CORE-3:** `save_categories` runs `DELETE FROM categories` and then re-inserts, with no transaction (`db/mod.rs:960`). `project_categories.category_id` is `ON DELETE CASCADE`, so **every category save in the GUI, and every restore, erases all project–category links**. A failure halfway also leaves categories missing.
- **CORE-7:** `set_environment_vars` / `set_environment_paths` delete everything and then re-insert without a transaction, and don't reject duplicate keys. One duplicate wipes the environment's variables, and their items become prunable orphans. `save_environment` also has partial effects when it returns an error (rename, paths and global ownership are already applied).
- **CORE-6:** `do_unlock` stores the key *before* `migrate_literal_vars_to_items` and `list_items` run (`vault/mod.rs:202-205`). If either fails, the GUI stays on the lock screen while the backend is unlocked for REST and MCP.
- **API-9:** `PUT /items/:id` reads, merges and writes with the vault lock released in between (`api/mod.rs:935→976`), so concurrent GUI and CLI updates overwrite each other.
- **CORE-14 (part):** `set_item_global` forking and `migrate_literal_vars_to_items` are not atomic; a partial failure leaves duplicate or ownerless copies.
- **CORE-11:** `PRAGMA secure_delete=ON` (and `foreign_keys`) are set with a one-off query on whichever pooled connection runs it (`db/mod.rs:246-256`). Connections that are recycled or newly opened don't have them, so later deletes leave plaintext-free but still recoverable ciphertext pages. Freed pages are no longer zeroed.

## What Changes

- **Category save preserves links.** `save_categories` becomes a diff-based upsert and delete inside one transaction. Links are removed only for categories that were actually deleted.
- **Environment save is atomic.** Rename, paths, variables and ownership changes are all applied in one transaction. Duplicate variable keys or paths are rejected up front with a validation error, before anything is written.
- **Unlock commits last.** The key becomes visible in `VaultState` only after migration and item loading have succeeded.
- **Item update holds the lock end to end.** Read-merge-write for `PUT /items/:id` and the GUI update run while holding the vault lock, inside one transaction.
- **Atomic forking and migration.** `set_item_global` forking and literal-var migration each run in one transaction.
- **Pragmas on every connection.** `foreign_keys`, `secure_delete` and `journal_mode` are set through the connection options, so every pooled connection has them.

## Capabilities

### New Capabilities
- `vault-data-integrity`: atomicity and consistency guarantees for vault write operations, plus the per-connection storage settings.

### Modified Capabilities
<!-- none -->

## Impact

- `db/mod.rs` (`save_categories`, `set_environment_vars`, `set_environment_paths`, fork and migrate helpers, pool options), `project/mod.rs` (`save_environment`), `vault/mod.rs` (`do_unlock`, `migrate_literal_vars_to_items`, `set_item_global`), `api/mod.rs` (`PUT /items/:id`).
- No schema change and no migration. The existing data model stays.
- Security: `secure_delete` becomes reliable; the backend can no longer be left unlocked after a failed unlock.

## Non-Goals

- Backup and restore atomicity: covered by `backup-restore-completeness`.
- Optimistic concurrency and version columns: holding the lock is enough for a single-process backend.
- Repairing links already lost by past category saves. They can't be reconstructed; a note goes in the release notes.
