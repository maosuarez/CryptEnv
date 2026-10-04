## Why

Backups today are neither complete nor safe to restore (CORE-5, `vault/mod.rs:1010-1230`):
- **Incomplete.** A `.cenvbak` contains only items and categories. Projects, environments, environment variables, item ownership (`item_projects`) and project categories are lost. Since the Projects & Environments model is now the main way the app is organised, a restore loses most of a user's structure.
- **Destructive on failure.** A replace-restore wipes the database first and then inserts items one by one with no transaction. Any failure (disk full, a bad row) leaves the old vault destroyed and the new one incomplete.
- **No unlock needed.** Replace-restore works while the vault is locked, without the current master password; only the backup's password is required. Anyone at the unlocked desktop session with a backup file of a *different* vault can replace the user's vault.
- **Pruning bait.** Restored non-global items, and items created by `vault_import_items`, have no owner. They show up in `list_orphan_item_ids` straight away and are offered for pruning.
- Restore also goes through `save_categories`, which currently cascades and wipes links. That is fixed in `vault-db-transactional-integrity`, which this change depends on.

## What Changes

- **Backup format v2.** It adds projects (including `root_path` and categories), environments (including paths and default flag), environment variables, and item ownership. Item data stays as the vault's AES-GCM ciphertext. Plaintext metadata (names, paths) is protected by encrypting the whole v2 metadata section with the vault key.
- **Restore is atomic.** Restore builds a complete new database file next to the current one, verifies it, and then swaps it in atomically. On any failure the current vault is untouched. The previous database is kept as `vault.db.pre-restore` until the next successful unlock.
- **Replace-restore needs an unlocked vault.** It requires the vault to be unlocked with the *current* master password, as well as the backup's password.
- **Legacy and import items get an owner.** v1 backups remain restorable. Their non-global items, and items from `vault_import_items` without a target, are assigned to an auto-created project named `Restored YYYY-MM-DD` / `Imported YYYY-MM-DD`, so they are never orphans.
- **Merge restore stays available** and runs in a single transaction.

## Capabilities

### New Capabilities
- `backup-restore`: backup completeness, restore atomicity, restore authorization, and ownership of restored or imported items.

### Modified Capabilities
<!-- none -->

## Impact

- `vault/mod.rs` (export, restore, import), `vault/import.rs`, `db/mod.rs` (a builder for a fresh DB at a path, a pool swap, bulk inserts in a transaction), and `lib.rs` (managed state swap).
- GUI Settings → Backup/Restore: a v2 file extension note, a "requires unlocked vault" state, and the restore result summary.
- Docs: the backup section in `docs/index.html`.
- **Depends on** `vault-db-transactional-integrity` (category save and transaction helpers).

## Non-Goals

- Cloud or scheduled backups.
- Backing up settings that are secrets or device-bound: the MCP token and the biometric enrollment are intentionally excluded.
- Cross-vault merge conflict UI beyond the existing skip/replace choice.
