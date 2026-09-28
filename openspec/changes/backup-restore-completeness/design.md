## Context

`BackupFile { version: 1, salt, token, items: [ciphertext], categories }`. Restore-replace calls `db.wipe_and_reset()` and then `init_vault`, followed by per-item inserts. `VaultState.db` owns the `SqlitePool`; the DB file lives in the app data dir.

## Goals / Non-Goals

**Goals:** a complete, confidential backup; crash-safe restore; restores only by an authorized user.

**Non-Goals:** incremental backups.

## Decisions

### D1. Format v2
```json
{ "version": 2, "created_at": ..., "salt": "...", "token": "...",
  "items": [ {id, item_type, data, created, is_global} ],
  "meta": "<hex(nonce||AES-GCM(vault_key, json(Meta)))>" }
Meta = { categories, projects:[{id,name,description,template,root_path,categories}],
         environments:[{id,project_id,name,is_default,paths}],
         environment_vars:[{environment_id,key,item_id}],
         item_projects:[{item_id,project_id}] }
```
The ids are backup-local; restore remaps them. Categories move into `meta` because their names can be sensitive. Rejected: a separate password for backups (more UX and state; restoring already needs the vault password).

### D2. Restore into a staging DB, then swap
1. Create `vault.db.restore-tmp` with `VaultDb::create_at(path)`, running the schema and pragmas.
2. Insert everything in one transaction. For merge-restore, first copy the current DB with `VACUUM INTO 'vault.db.restore-tmp'`, then apply the merge in a transaction.
3. Run `PRAGMA integrity_check` and a decrypt check of the verify token.
4. Close the current pool (`pool.close().await`).
5. `rename(vault.db → vault.db.pre-restore)` and delete stale `-wal`/`-shm` files after a checkpoint (`PRAGMA wal_checkpoint(TRUNCATE)` before closing).
6. `rename(restore-tmp → vault.db)` and reopen the pool into `VaultState.db`.
7. On failure in steps 5–6, rename back and reopen.

`vault.db.pre-restore` is deleted on the next successful unlock. Rename is atomic within the same directory on NTFS and ext4. On Windows, the pool must be closed before renaming, which is why step 4 comes first.

### D3. Authorization
`vault_restore_backup(mode, path, backup_password)`:
- Both modes require `s.key.is_some()`.
- Replace additionally re-verifies that the *current* password is active; the unlocked state proves it, but the GUI also asks for it again (a new `current_password` parameter, re-derived and compared in constant time against the verify token) to protect against a walk-up attacker.
- Merge requires that the backup's key decrypts its token.

### D4. Owned legacy and import items
A helper `ensure_holding_project(tx, "Restored", date) -> project_id` creates the project (template `generic`, no root) and inserts `item_projects` rows. `vault_import_items` uses the same helper with "Imported" when no project is given.

## Security & Threat Model

- **Backup file theft:** items are already ciphertext; the metadata is now ciphertext too. Brute force costs Argon2id with the vault's parameters.
- **Walk-up replace:** now needs the current password.
- **Crash during restore:** the staging file protects the original.

## Risks / Trade-offs

- [Pool swap while REST handlers hold clones of the pool] → The swap happens under the vault `MutexGuard`, and handlers take the pool through the guard; the audit found no long-lived pool clones outside `VaultState` (verified in task 2.1).
- [Disk space: the staging file doubles usage temporarily] → A pre-check of free space is best effort; failure is safe anyway.

## Migration Plan

Export writes v2; import reads v1 and v2. No DB migration.
