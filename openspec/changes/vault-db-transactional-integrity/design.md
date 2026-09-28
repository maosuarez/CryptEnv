## Context

`VaultDb` wraps a `SqlitePool` (sqlx 0.8). A few operations already use `pool.begin()` (e.g. `db/mod.rs:1072,1341`). The pragmas are executed once in `init_schema` over the pool, so they apply to just one connection. The vault's `SharedState` is a `tokio::Mutex<VaultState>`.

## Goals / Non-Goals

**Goals:** transactional multi-statement writes; correct pragmas on every connection; unlock atomicity.

**Non-Goals:** schema redesign.

## Decisions

### D1. Pragmas via `SqliteConnectOptions`
`SqliteConnectOptions::new().filename(path).foreign_keys(true).journal_mode(Wal).pragma("secure_delete","ON")`. The one-off `PRAGMA` statements are removed from `init_schema`. A test opens the pool with `max_connections(3)`, forces three connections (three concurrent acquires), and asserts that `PRAGMA secure_delete` returns 1 on each.

### D2. Diff-based `save_categories`
Everything runs in one transaction:
1. load the existing `cid`s;
2. `DELETE WHERE cid NOT IN (new set)`: cascades apply only here, which is the correct behavior;
3. `INSERT ... ON CONFLICT(cid) DO UPDATE SET name, color, description`.

Restore callers go through the same function (see `backup-restore-completeness`).

### D3. Transaction-accepting DB helpers
The `set_environment_vars`, `set_environment_paths`, ownership and fork helpers get `_tx(&mut Transaction)` variants. `project::save_environment` opens one transaction, validates input first (duplicates, via a `HashSet`), calls the `_tx` helpers, and commits. The pool-level wrappers remain for single-call users and delegate to the `_tx` variants in their own transaction.

### D4. Unlock ordering
In `do_unlock`: derive the key → run the migration with an explicit `&key` (already the signature) → `list_items` + decrypt → **then** `s.key = Some(...)` and `touch()`. Everything is still under the same `MutexGuard`, so no interface observes the intermediate state. The migration runs in its own transaction (D5).

### D5. Item update under lock
`PUT /items/:id` and `vault_update_item` take the vault lock for the whole read-decrypt-merge-encrypt-write. Encryption is fast; Argon2 is not involved. A transaction wraps the item row and its link-table changes. This trades a little concurrency for correctness, which is acceptable for a single-user local app.

## Security & Threat Model

- `secure_delete` everywhere: deleted ciphertext pages are overwritten. This reduces forensic recovery of deleted items (which are encrypted anyway; it matters mostly for metadata).
- Unlock atomicity: removes an "unlocked backend, locked UI" state in which REST and MCP had access without the user knowing.
- No plaintext is newly logged. Validation errors name the keys and paths, not values.

## Risks / Trade-offs

- [Longer lock hold in item update] → Microseconds to milliseconds; no Argon2 inside.
- [Transactions under WAL with the GUI and REST concurrently] → SQLite serializes writers; `busy_timeout` is set to 5 s in the options to avoid SQLITE_BUSY surfacing.

## Migration Plan

No schema change. Links lost by earlier versions cannot be restored; this is mentioned in the release notes.
