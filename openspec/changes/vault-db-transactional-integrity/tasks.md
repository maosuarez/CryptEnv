## 1. Connection settings

- [ ] 1.1 Move the pragmas into `SqliteConnectOptions` (`foreign_keys`, WAL, `secure_delete`, `busy_timeout` 5 s) and remove them from `init_schema`. Verify with a test that acquires 3 concurrent connections and asserts `PRAGMA secure_delete` = 1 and `PRAGMA foreign_keys` = 1 on each.

## 2. Categories

- [ ] 2.1 Rewrite `save_categories` as diff + upsert in one transaction. Verify with a test: tag a project, save the category list with one category renamed → the project link survives; delete a category → only its links go; an injected failure (duplicate name) → nothing changes.

## 3. Environments

- [ ] 3.1 Add `_tx` variants of the env vars/paths/ownership helpers, plus duplicate key and path validation. Verify with unit tests that duplicate input is rejected before any write.
- [ ] 3.2 Make `project::save_environment` a single transaction. Verify with a test: a forced failure on the vars insert leaves the name, paths and ownership unchanged.

## 4. Unlock and items

- [ ] 4.1 Reorder `do_unlock` so the key is set last; run `migrate_literal_vars_to_items` in a transaction. Verify with a test: a forced migration failure → `key` stays `None` and the REST MCP token gets 403.
- [ ] 4.2 Hold the vault lock and a transaction across read-merge-write in `PUT /items/:id` and `vault_update_item`. Verify with a concurrency test: two tasks updating different fields → both are present.
- [ ] 4.3 Make the `set_item_global` fork atomic. Verify with a test: an injected failure after the copy insert → no copy exists and the links are unchanged.

## 5. Verification

- [ ] 5.1 Run `cargo clippy --all-targets && cargo test`. All pass. Mention the lost-links caveat in the release notes/CHANGELOG.
