# crypt-env Reference

## REST API

Endpoint: `https://127.0.0.1:47821`

Authentication: Header `X-Vault-Token` containing either a session token (from POST /unlock, has TTL) or a static MCP token (stored in database, no expiry). Token verification uses constant-time comparison. Rate limiting enforced on /unlock: 5 attempts per 60-second window.

| Method | Endpoint | Auth | Description |
|--------|----------|------|-------------|
| POST | /unlock | none | Derives AES-GCM key from master password + Argon2 salt, generates 16-byte session token with configurable TTL |
| GET | /health | none | Returns version, status, vault_locked bool, mcp_token_configured. No longer returns item_count (removed — see Notes) |
| GET | /items | token | List items (redacted — no secret values), **scoped**: requires `environment_id`, or `project`+`environment` (case-insensitive names) query params — 422 `VALIDATION_ERROR` if unresolvable. **Discovery endpoint** — see "Global items and scope" below: accepts `include_global=true\|false\|only` (default `true`), unioning in reusable global items not linked into this environment by default. Every item carries `isGlobal` and `linked`. `type`/`category`/`search` filters apply on top of the union |
| POST | /items | token | Create item, **scoped** (same query params as GET /items). Validates: name (req, max 255), type (one of: secret/credential/link/note/command), value (req non-empty). Body accepts optional `key` (environment-var key, defaults to `name`). Accepts `?on_conflict=update\|replace\|error` (default `update`; invalid value → 422 `VALIDATION_ERROR`). On no collision: creates, owns it in the resolved project, links it under `key`, returns `201`. On a collision (an `environment_vars` row already linked to `key`): `update` re-encrypts onto the existing item **in place** if it is exclusive (non-global, linked only here, owned only by this project) and returns `200`; if the existing item is shared instead, returns `409 SHARED_ITEM_CONFLICT`. `replace` always creates a new item and repoints the link (`201`), deleting the superseded item only if it is now unreachable (unlinked and non-global) — a shared item survives, still referenced elsewhere. `error` returns `409 KEY_EXISTS` on any collision. Create-or-update-and-link is one SQLite transaction — no interleaving leaves an item owned but unlinked. Caller-supplied `isGlobal` is ignored on both `Created` and `Updated` outcomes — the response reports the value actually persisted (always `false` on this path). Encrypts with AES-GCM before storing |
| GET | /items/:id | token | Get single item metadata (redacted). **Unscoped** — reachable by id regardless of project/environment (scope is a display filter, not an access boundary; see Notes) |
| PUT | /items/:id | token | Update item. Merges — omitted fields keep existing values including secret fields. Unscoped, same as GET /items/:id |
| DELETE | /items/:id | token | Delete item. Returns 204. Unscoped |
| POST | /items/:id/reveal | token | Returns plaintext secret value. Requires `{"confirm": true}` in body. Logs access to stderr. Unscoped |
| GET | /categories | token | List categories (id, name, color, description). Unscoped — categories are global |
| POST | /categories | token | Create category. Validates name (req, max 100) and color (req). Generates random hex cid |
| PUT | /categories/:id | token | Update category fields. Passing `description: ""` clears it |
| DELETE | /categories/:id | token | Delete category. Returns 204 |
| GET | /commands | token | List items of type "command" with extracted `{{VAR}}` placeholders, **scoped** (same query params as GET /items). **Discovery endpoint** — same `include_global` contract as GET /items (default `true`); each command carries `isGlobal` and `linked` |
| GET | /commands/:id | token | Get single command with placeholders. Unscoped |
| GET | /settings | token | Get auto_lock_timeout (minutes) and hotkey. Unscoped — settings are global |
| PUT | /settings | token | Update auto_lock_timeout and/or hotkey |
| POST | /fill | token | Fill a .env template with real values, **scoped** (same query params as GET /items). Matches template keys against the resolved environment's `environment_vars.key` (not a vault-wide name search). A template key not found in scope has its **original line preserved unchanged** (not blanked) and is reported as a warning. `output_path` given: writes there via the RAII `TempEnvFile` guard, returns stats only — no secret in response. No `output_path` but `output_dir` given: the environment-name-derived filename is resolved via `fsguard::resolve_within` (issue #7) before any decryption happens — `422 PATH_NOT_CONTAINED` if it can't stay inside `output_dir`, `path` in the response is the resolved (post-canonicalization) path. Neither: returns filled content inline. Body accepts `overwrite` (default `false`) — see **Write-target gating** below |
| POST | /environments/:id/example | token | Generate a placeholder-only env file/content for the environment (`environment_id` in URL path, same convention as `/environments/:id/inject`) — `KEY=` for every linked var key, values always empty, explicitly safe to commit. Never decrypts or reads item values. Body `{output_path?, output_dir?, overwrite?}`: `output_path` writes there; `output_dir` writes to the environment-name-derived filename, contained within `output_dir` via `fsguard::resolve_within` (issue #7, same as `/fill`) — `422 PATH_NOT_CONTAINED` on escape; neither returns `{content, keys}` inline. 404 if the environment doesn't resolve. Same `overwrite` gating as `/fill` — see below |
| POST | /share/listen | token | Start LAN share session as sender, **scoped** (same query params as GET /items). Every id in `items` must already be linked into the resolved environment or the call 422s. Registers mDNS, returns `pairing_code` |
| POST | /share/connect | token | Connect as receiver using pairing_code, **scoped** (same query params as GET /items). On successful transfer, received items are owned by the resolved project and linked into the resolved environment under the sender's item names — **except** where that name collides with a key already linked in the target environment, in which case the item is still imported/owned but the existing link is left untouched and the collision is reported (see `/share/status`'s `skipped_keys`, and Notes). Returns ECDH fingerprint |
| POST | /share/confirm | token | Confirm (or reject) fingerprint. Both sides must call this |
| GET | /share/status | token | Returns session state, fingerprint, direction, received_names, skipped_keys (env-var keys NOT linked due to a collision with an existing link — see /share/connect) |
| DELETE | /share/session | token | Cancel active share session |
| POST | /share/export | token | Export items as AES-256-GCM encrypted `.vault` file. Returns passphrase in response. Unscoped (operates on item IDs directly) |
| POST | /share/import | token | Import from `.vault` file using passphrase, **scoped** (same query params as GET /items). Imported items are owned by the resolved project and linked into the resolved environment, with the same collision-skip behavior as `/share/connect`. (The Tauri GUI command for this import path still passes no scope — items land ownerless/unlinked from the GUI, same pre-existing gap as before, not addressed this pass) |
| GET | /projects | token | List all projects with their typed environments (name, template, paths, variable count) |
| POST | /projects | token | Create or update project. Returns project ID (upsert by id=0 for creation). `name` is unique case-insensitively at the DB level — creating with a name that already exists (any case) returns 409 CONFLICT instead of creating a duplicate. `name` must also pass the filesystem-hostile deny-list (issue #7: no separators, control characters, NTFS-hostile characters, reserved device names, leading/trailing dot or whitespace; 128 chars max) — 422 `VALIDATION_ERROR` otherwise |
| DELETE | /projects/:id | token | Delete project and all its environments. Returns impact summary |
| GET | /projects/:id/preview-delete | token | Show what will be deleted (impact preview) without performing the deletion |
| POST | /environments | token | Create or update environment within a project. `projectId` is now required and validated to reference an existing project — 422 if missing/invalid. `name` must match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$` (issue #7) — 422 `VALIDATION_ERROR` otherwise. Returns environment ID (upsert by id=0 for creation) |
| DELETE | /environments/:id | token | Delete a single environment. Returns 204 |
| POST | /environments/:id/inject | token | Inject environment's variables into its configured .env path(s). Takes a JSON body `{output_path?, output_dir?, overwrite?}` (previously bodyless — an empty `{}` body preserves the old behavior). `output_path`, if given, is added to (not a replacement for) the environment's configured `paths[]` — all get written. If `paths[]` is empty and no `output_path`, falls back to `{output_dir}/.env.<environment-name>`. Returns paths written, keys injected, `unmanagedPaths` (configured paths that were unmanaged and got written through anyway — see below) and `backups` (`.bak` paths created). Same `overwrite` gating as `/fill`, but only for `output_path`/`output_dir` — see below |
| POST | /relay/send | token | Encrypt selected items with Argon2id-derived key and upload to Supabase relay. Returns code + passphrase. Requires relay_supabase_url and relay_supabase_anon_key in settings |
| POST | /relay/receive | token | Download from Supabase relay, decrypt with key+passphrase, import items, **scoped** (same query params as GET /items). Imported items are owned by the resolved project and linked into the resolved environment, with the same collision-skip behavior as `/share/connect`. Burns after read (best-effort delete) |
| POST | /projects/:id/relay/send | token | Share a whole project via relay: structure (environment names, `isDefault`) plus decrypted values for the selected `environment_ids`, deduped by item across environments. Returns `{code, passphrase, project, environment_count, item_count}`. Requires relay_supabase_url and relay_supabase_anon_key in settings |
| POST | /projects/relay/receive | token | Receive a shared project from relay. Always creates a **new** project (never merges) — a case-insensitive name collision returns `409 CONFLICT`; retry with `project_name_override` in the body. Received items are owned by the new project only (`isGlobal: false`), never linked into any pre-existing project. Returns `{project, environments, item_count}`. Burns after read (best-effort delete) |
| GET | /maintenance/orphans | token | Read-only, redacted list of items with zero `environment_vars` references and `isGlobal:false` (unreachable from any list/GUI surface). No REST prune endpoint by design — a static MCP token must not be able to bulk-delete vault rows; pruning is Tauri-command-only (`vault_prune_orphan_items`), behind the GUI's confirmation, same as every other destructive vault operation |

### Examples

`curl` examples against the local server. `-k` is required — the certificate
is self-signed (see Notes below). Replace `$TOKEN` with a session token from
`/unlock` or the static MCP token from Settings.

```bash
# Unlock — returns a session token with a configurable TTL
curl -sk -X POST https://127.0.0.1:47821/unlock \
  -H 'Content-Type: application/json' \
  -d '{"master_password": "your-master-password"}'

# List items — scoped by environment_id
curl -sk https://127.0.0.1:47821/items?environment_id=1 \
  -H "X-Vault-Token: $TOKEN"

# List items — scoped by project + environment names (case-insensitive)
curl -sk 'https://127.0.0.1:47821/items?project=demo&environment=production' \
  -H "X-Vault-Token: $TOKEN"

# Create an item, linked into an environment as DB_HOST
curl -sk -X POST 'https://127.0.0.1:47821/items?environment_id=1' \
  -H "X-Vault-Token: $TOKEN" -H 'Content-Type: application/json' \
  -d '{"type": "secret", "name": "DB_HOST", "value": "localhost", "key": "DB_HOST"}'

# Reveal a plaintext value — requires explicit confirm
curl -sk -X POST https://127.0.0.1:47821/items/1/reveal \
  -H "X-Vault-Token: $TOKEN" -H 'Content-Type: application/json' \
  -d '{"confirm": true}'

# Fill a .env template inline, scoped by project + environment
curl -sk -X POST 'https://127.0.0.1:47821/fill?project=demo&environment=production' \
  -H "X-Vault-Token: $TOKEN" -H 'Content-Type: application/json' \
  -d '{"template": "DB_HOST=\nPORT=3000\n"}'

# List all projects with their nested environments
curl -sk https://127.0.0.1:47821/projects -H "X-Vault-Token: $TOKEN"

# Inject an environment's variables into its configured .env path(s)
curl -sk -X POST https://127.0.0.1:47821/environments/1/inject \
  -H "X-Vault-Token: $TOKEN" -H 'Content-Type: application/json' -d '{}'
```

The server is HTTPS-only on `127.0.0.1:47821` with a self-signed certificate
generated on first launch (`tls::ensure_tls_config`) — clients must either
pass `-k`/`--insecure` (as above) or trust that certificate explicitly.

**On the retired Postman collection**: `src-tauri/tests/crypt-env-api.postman_collection.json`
was deleted (tech-debt issue #11) — it asserted a `GET /health` field that no
longer exists and none of its 15 requests carried the (now mandatory)
project/environment scope, so every one of them 422'd. Nothing in CI ever
executed it, so it silently drifted out of date across a whole schema
migration. This reference section plus the `api::tests::*` suite (executed
on every push/PR — see `.github/workflows/test.yml`) are the replacement:
one documents the contract, the other proves it. A Postman collection may
return only alongside a test that replays it through the same router and
fails the build on drift — see the plan doc for the full reasoning.

### Global items and scope

**Discovery surfaces union globals; materialization surfaces never do; `linked` is the discriminator.** `GET /items` and `GET /commands` accept `include_global=true|false|only` (default `true`): `true` returns (items linked in the resolved environment) ∪ (all `isGlobal:true` items), deduplicated by id; `false` restricts to exactly what's linked — byte-for-byte what `/fill`/`/inject`/`/environments/:id/example` will materialize; `only` returns just the global set regardless of linkage (the REST equivalent of the GUI's Global Secrets screen), still requiring a valid scope. An invalid value 422s with `include_global` named in the message. Every returned item/command carries `isGlobal` (is it marked reusable) and `linked` (is it actually linked into the queried environment) so a caller can always tell "exists and reusable" apart from "will be written by fill/inject". `POST /fill`, `POST /environments/:id/inject`, `POST /environments/:id/example`, and `POST /share/listen` are unaffected by `include_global` — they resolve strictly through `environment_vars`, so linking a global into an environment remains a deliberate, explicit act.

### Examples

`curl` examples against the local server. `-k` is required — the certificate
is self-signed (see Notes below). Replace `$TOKEN` with a session token from
`/unlock` or the static MCP token from Settings.

```bash
# Unlock — returns a session token with a configurable TTL
curl -sk -X POST https://127.0.0.1:47821/unlock \
  -H 'Content-Type: application/json' \
  -d '{"master_password": "your-master-password"}'

# List items — scoped by environment_id
curl -sk https://127.0.0.1:47821/items?environment_id=1 \
  -H "X-Vault-Token: $TOKEN"

# List items — scoped by project + environment names (case-insensitive)
curl -sk 'https://127.0.0.1:47821/items?project=demo&environment=production' \
  -H "X-Vault-Token: $TOKEN"

# Create an item, linked into an environment as DB_HOST
curl -sk -X POST 'https://127.0.0.1:47821/items?environment_id=1' \
  -H "X-Vault-Token: $TOKEN" -H 'Content-Type: application/json' \
  -d '{"type": "secret", "name": "DB_HOST", "value": "localhost", "key": "DB_HOST"}'

# Reveal a plaintext value — requires explicit confirm
curl -sk -X POST https://127.0.0.1:47821/items/1/reveal \
  -H "X-Vault-Token: $TOKEN" -H 'Content-Type: application/json' \
  -d '{"confirm": true}'

# Fill a .env template inline, scoped by project + environment
curl -sk -X POST 'https://127.0.0.1:47821/fill?project=demo&environment=production' \
  -H "X-Vault-Token: $TOKEN" -H 'Content-Type: application/json' \
  -d '{"template": "DB_HOST=\nPORT=3000\n"}'

# List all projects with their nested environments
curl -sk https://127.0.0.1:47821/projects -H "X-Vault-Token: $TOKEN"

# Inject an environment's variables into its configured .env path(s)
curl -sk -X POST https://127.0.0.1:47821/environments/1/inject \
  -H "X-Vault-Token: $TOKEN" -H 'Content-Type: application/json' -d '{}'
```

The server is HTTPS-only on `127.0.0.1:47821` with a self-signed certificate
generated on first launch (`tls::ensure_tls_config`) — clients must either
pass `-k`/`--insecure` (as above) or trust that certificate explicitly.

**On the retired Postman collection**: `src-tauri/tests/crypt-env-api.postman_collection.json`
was deleted (tech-debt issue #11) — it asserted a `GET /health` field that no
longer exists and none of its 15 requests carried the (now mandatory)
project/environment scope, so every one of them 422'd. Nothing in CI ever
executed it, so it silently drifted out of date across a whole schema
migration. This reference section plus the `api::tests::*` suite (executed
on every push/PR — see `.github/workflows/test.yml`) are the replacement:
one documents the contract, the other proves it. A Postman collection may
return only alongside a test that replays it through the same router and
fails the build on drift — see the plan doc for the full reasoning.

### Notes

`decrypt_all_items` decrypts the entire vault on every authenticated request (no caching, no index), making every GET /items a full decryption pass — O(n) per request regardless of filters. Scoped endpoints add a second cost on top: `resolve_scope` loads the full project→environment→vars graph (`GET /projects`-equivalent) before the item decryption pass, so every scoped request is now O(vault) + O(project graph).

`handle_delete_item` and `handle_update_item` each acquire the vault lock twice: once to read/verify existence and once to commit changes, with full AES-GCM re-encryption of the item held between acquisitions.

Session tokens: see *Password gate / sessions* above (up to 64 concurrent sessions, invalidated by the lock epoch).

`TempEnvFile` zeros output via `std::fs::write` which passes through OS page cache; on SSDs with wear leveling, overwritten data may persist in flash cells indefinitely — acceptable for most threat models but not forensic-grade. Note this is a plain (non-atomic) write to the final destination, not a temp-file-plus-rename — a process crash mid-write can leave the target file truncated. `TempEnvFile::create_guarded` delegates the existence check, `.bak` backup and permission mode to `envfile::commit` (see **Write-target gating** below); on the error path where the guard drops without `persist()`, a path that already existed before the write is zeroed and truncated to length 0 rather than removed, so the underlying inode/permissions a caller may depend on are preserved.

**Write-target gating** (`src-tauri/src/envfile/`): every write to a caller- or owner-supplied path across `/fill`, `/environments/:id/example` and `/environments/:id/inject` goes through a shared gate before any secret is decrypted. A target that already exists and does not start with the marker line `# crypt-env: managed file (project: <name>, environment: <name>)` is classified `Foreign`. For `output_path`/`output_dir`-derived paths, a `Foreign` target is refused with `409 Conflict` (`code: "TARGET_EXISTS"`) unless the request body sets `overwrite: true`, in which case the prior contents are copied to `<path>.bak` (refusing with `409` / `code: "BACKUP_EXISTS"` if a `.bak` from an earlier overwrite is still there) before the new content is written with the marker prepended. For `/environments/:id/inject`'s owner-configured `environment.paths[]`, a `Foreign` target is never refused — written through with the same `.bak` backup, and reported once in the response's `unmanagedPaths` (it self-heals to `Managed` from then on, since the marker is now in place). Marker detection is prefix-only (first non-empty line, trimmed) — renaming the project or environment never invalidates an existing marker. Add `*.bak` to `.gitignore` in any directory these endpoints write into. **Symlinks and containment:** a target whose final component is a symlink (or Windows reparse point) is refused with `409 Conflict` (`code: "TARGET_SYMLINK"`) and nothing is written or backed up (`O_NOFOLLOW` on Unix; a pre-open check on Windows). A relative `environment.paths[]` entry must stay inside the canonical project root after symlinks are resolved (`fsguard::contain_relative`); `.env.example` writes use the same no-follow opener. `.crypt-env.yaml` is written through an exclusively created temporary file in the same directory and renamed over the target; `project_write_yaml` releases the vault lock before touching the filesystem.

`relay_delete` after receive is best-effort (error ignored), so relay payloads remain accessible to anyone with code+passphrase until the 24-hour TTL expires if deletion fails.

CORS guard accepts `Origin: null`, correctly matching local file:// and Tauri webviews, but also matches any sandboxed iframe — minimal practical impact but violates defense-in-depth.

`PRAGMA secure_delete=ON` (issue #9): SQLite now zeroes freed page content on every `DELETE`/overwriting `UPDATE`, not just unlinking it — covers `delete_item`, `delete_project`'s cascades, the `POST /items?on_conflict=replace` superseded-item delete, and the orphan prune path uniformly. This changes on-disk behavior for *all* deletes going forward (connection-level setting, no on-disk format change, safe to remove at any time); rows deleted before this pragma was added are not retroactively scrubbed.

Projects/environments model replaces the old workspaces. A Project contains multiple typed Environments, each with its own paths and variables. Environment variables are real FK-based links (`environment_vars.item_id` → a vault item), not name-search — an item is only "in" an environment if explicitly linked via `POST /items` (with `key`), `POST /environments` (saving the var list), or one of the import paths above. Deleting a project cascades to delete all its environments via foreign key constraints.

`/projects/:id/preview-delete` returns the impact without executing the deletion — allows clients to show the user what will be removed before confirming.

**Scoping contract**: `GET /items`, `POST /items`, `GET /commands`, `POST /fill`, `POST /share/listen`, `POST /share/connect`, `POST /share/import`, `POST /relay/receive` all require project+environment scope: query params `environment_id` (i64, takes precedence if both are given) or `project`+`environment` (name strings, case-insensitive, resolved via the same lookup `POST /environments/:id/inject` has always used). Missing/unresolvable scope → 422 `VALIDATION_ERROR`.

**Scope is a filter, not an access-control boundary.** `GET/PUT/DELETE /items/:id` and `POST /items/:id/reveal` remain unscoped by design — any holder of a valid token can read/modify/reveal any item by id regardless of project. This is an acceptable model for a single-user local vault but is easy to assume otherwise given how consistently every list/create endpoint enforces scope.

`projects.name` has a case-insensitive UNIQUE index (`idx_projects_name_nocase`) — duplicate-name creation now returns 409 instead of silently succeeding. `environments.name` is only unique per-project under SQLite's default (case-sensitive) collation — two environments in the same project differing only by case (e.g. `Production`/`production`) can still coexist, and name-pair resolution (case-insensitive, picks the lowest-id match) will silently prefer one over the other with no ambiguity error. Known limitation, not fixed.

**Environment/project name validation (issue #7, fixed):** `environment.name` must match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$` (1-64 chars, starts with a letter or digit, no trailing `.` or `-`) — enforced in `project::save_environment` before the row is ever persisted, so it applies identically to `POST /environments`, the Tauri `environment_save` command, an imported `.cryptenv-proj` template, and the CLI. `project.name` gets a laxer deny-list instead (rejects separators, control characters, NTFS-hostile characters, reserved device names, leading/trailing dot or whitespace; allows spaces and non-ASCII letters; 128 chars max), enforced the same way in `project::save_project`. Rejection is `422 VALIDATION_ERROR`. Existing rows created before this validation shipped are **not** migrated or rejected at read time — they keep working (see the belt-and-braces containment below) and only get corrected the next time the row is edited. A second, independent layer (`fsguard::resolve_within`) additionally guarantees that no write derived from an environment name — validated or legacy — can land outside the caller-supplied `output_dir` on `/fill`, `/environments/:id/example`, or `project::inject_environment`'s default-filename branch; a name that somehow escapes both is a bug in this layer, not an accepted risk.

**Known, deferred issues** (found in review, not fixed in this pass): (1) `PUT /items/:id`'s "merge" behavior only applies to the `Option<T>` fields on `VaultItem` — `type` has no `#[serde(default)]`, so a client omitting it entirely gets a 422 from axum's `Json` extractor before the merge logic (or `validate_update`) ever runs; every partial update must still resend `type`. Found and pinned by `api::tests::items::update_item_partial_update_preserves_other_fields` (issue #11), not changed there since that's a behavior fix out of scope for a test-only PR. (2) `GET`/`PUT`/`DELETE /items/:id` and `POST /items/:id/reveal` perform no project/environment scope check — any valid token can reach any item by id. Scope is a display filter on the *list* endpoints, not an access boundary; issue #13 changes list visibility only and does not close this. Tracked separately.

`project::save_environment` → `set_environment_vars` still deletes and re-inserts an environment's whole var set on save, so removing a variable in the GUI unlinks its item without deleting it (same orphan shape `POST /items` used to produce, from a different route). Deliberately not fixed here — whether unlinking a variable should also destroy its secret is a product decision; the orphan report below is the intended surface for it. `POST /share/import`, `POST /relay/receive`, and the GUI backup-import path can also create unlinked items and are equally not orphan-cleanup targets (see the prune predicate note below).

**Orphan items** (unlinked, non-global vault rows — issue #9): every `environment_vars` key collision on `POST /items` used to leave the previously-linked item behind: still in `items`, still owned, still fully readable via `/items/:id` and `/items/:id/reveal`, just invisible to scoped listings. Fixed — see the `POST /items` row above for the new `on_conflict` semantics. `GET /maintenance/orphans` (redacted, read-only) and the Tauri commands `vault_list_orphan_items` (redacted list) / `vault_prune_orphan_items(ids)` (delete, GUI-only, typed confirmation) surface and clean up rows left behind by data written *before* this fix, or by the GUI/import routes noted above. This reference doesn't otherwise catalogue Tauri `invoke()` commands (GUI-internal, not a network-facing surface); these two are called out here because they are the only user-facing remediation for orphans and have no REST equivalent for the delete half. The underlying query (`items` with no `environment_vars` row and `isGlobal:false`) intentionally excludes global items — they retain a reachable surface via Global Secrets even when unlinked, so pruning them would delete something still visible elsewhere. There is deliberately no startup migration that prunes automatically: the same predicate matches freshly-received, not-yet-linked share/relay/backup imports, so an automatic sweep on unlock could delete something the user hasn't seen yet.

---

## CLI

Command: `crypt-env`. Rewritten by openspec change `cli-tui-parity`; the previous command set was removed.

**Workspace resolution** (`commands/scope.rs`): the nearest `.crypt-env.yaml` from the cwd upward (a legacy `crypt-env.json` at the same or a higher level is read only when no YAML is found, with a stderr migration notice). Its directory is the project root. The project is looked up in `GET /projects` by name (case-insensitive); the environment is `--env`, else the manifest's `isDefault` environment if it exists in the vault, else the vault's default. No command auto-creates a project except `init` and `config` (push when missing).

**`.crypt-env.yaml`** (serializer shared with the GUI in `src-tauri/src/project/manifest.rs`): `project.{name, description?, categories? (alias tags), environments[{name, isDefault, paths[]}]}`, unknown fields rejected, at most one default, environment names unique case-insensitively. Metadata only — never a value.

**Password gate / sessions** (`client::ensure_session`, design D4): `fill`, `sync`, `inject`, `search` and the `add` collision reveal pass when this terminal's token is a live session (probed with `GET /settings`, which renews it); otherwise they prompt (`rpassword`, `Zeroizing`) and call `POST /unlock`. Server: `api::SessionStore` keeps up to 64 sessions (LRU eviction), each sliding by `auto_lock_timeout` minutes (0/"never" → 5) on every authenticated request; tokens are compared in constant time against every entry. Client: token file `<CRYPTENV_TOKEN_PATH or default>.<first 16 hex of SHA-256(terminal id)>` (`terminal.rs`); terminal id = `CRYPTENV_TERMINAL_ID`, else Unix `getsid` + tty + Linux session-leader start time, else Windows `GetConsoleWindow`. Only regular files named exactly `<base>.<16 lowercase hex>` are pruned (24 h unused). Binding is client-side only. Server: `VaultState.epoch` is bumped by `set_key` on every unlock/lock/reset/restore/re-key; each session records the epoch it was issued in, `verify_token` returns `403` for a locked vault before touching sessions and drops stale-epoch sessions, so a pre-lock token gets `401` after the next unlock. The client never prompts while the TUI owns the terminal (`client::NON_INTERACTIVE`, `CliError::SessionRequired` opens the TUI password modal).

**Paths**: vault paths are host paths (as the GUI process sees them). A native Linux CLI inside WSL (`WSL_DISTRO_NAME`) rewrites absolute paths `/x` ↔ `\\wsl.localhost\<distro>\x` and `/mnt/c/x` ↔ `C:\x` (`paths.rs`, pure string mapping assuming the default `/mnt` automount); `CRYPTENV_PATH_TRANSLATION=off` disables it. Relative environment paths are never translated.

| Command | Flags | Description |
|---------|-------|-------------|
| `init` | `[NAME] [--path PATH]` | Creates the project (`POST /projects` with `rootPath`) or links an existing one of that name (sets its root), makes the default environment target `.env` or `--path` (dir → `<dir>/.env`), writes `.crypt-env.yaml`. Refuses to touch an existing YAML (warning, exit 0) |
| `config` | — | Local mtime vs `max(project.updated, environments[].updated)`. Local strictly newer → push: missing categories created (`POST /categories`), `POST /projects` (rootPath = YAML dir), `POST /environments` per YAML env (matched by name, existing `vars` preserved). Vault envs absent from the YAML are kept and reported. Otherwise → pull: YAML regenerated (no write when identical), mtime set to the vault timestamp; a vault project without a root adopts the YAML dir. No YAML → error suggesting `init` |
| `add` | `KEY=value \| $VAR \| FILE [--env] [--global]` | Collision check against the environment's keys (+ global item names with `--global`). Any collision aborts the whole batch; per colliding key, `[y/N]` then password then the value is printed (warning on stderr). New items created with `POST /items?on_conflict=error`; `--global` then `PUT /items/:id {isGlobal:true}` |
| `fill` | `[--env]` | 🔒 `POST /environments/:id/inject` for each environment (or one) with paths; writes/extends `.env.example` (keys only, existing lines kept) in each target's directory |
| `sync` | `[--global] [--env] [--example PATH]` | 🔒 Keys in `.env.example` (cwd, then root) missing from the environment → items with value `change-me`. `--global`: matching global item names are linked instead (`POST /environments`), then the environment is injected (configured paths, or `.env` beside the template) |
| `inject` | `KEY... [--env] [--shell pwsh\|bash\|zsh\|sh]` | 🔒 Resolves every key (environment vars, then global items) before printing; stdout carries only the single-quoted assignments |
| `search` | `[PATTERN] [--global]` | 🔒 KEY / ENVIRONMENT / TYPE / SCOPE (`project`, `global+linked`, `global`) — never values. `%x`/`x%` substring, otherwise case-insensitive regex (invalid regex → substring). `--global` works outside a workspace |
| `doctor` | — | Health, vault lock, MCP token flag, TLS cert (`tls::parse_not_after_from_pem`, warn < 30 days), token file (warn if group/other-readable on Unix), MCP token file, project config, WSL status |
| `setup wsl` | `[DISTRO] [--remove] [--cert-path] [--api-url]` | Inside WSL: local shell setup (a different `DISTRO` is an error). On Windows: `wsl.exe -l -q`; none → notice, no arg → list + instruction, known arg → bundled helper (`<exe dir>/wsl/crypt-env-setup`, same as the GUI panel) |
| `tui` | — | See TUI |
| `memory` `list` `exec` `cmd` `project` `share` `relay` `category` `set` | — | Removed: clap reports an unrecognized subcommand |

---

## TUI

Command: `crypt-env tui` (`commands/tui.rs`, ratatui 0.29 + crossterm 0.28)

Panes: projects · environments (+ a trailing "★ global items" entry backed by `GET /items?include_global=only`) · variables (values always masked). Starts on the cwd workspace's project. Without a cached token, a login password modal is shown first.

| Key | Action |
|-----|--------|
| ←/→, h/l, Tab/Shift+Tab, Enter | Switch pane |
| ↑/↓, j/k | Move selection (wraps) |
| / | Filter variables (same matcher as `search`); Enter keeps, Esc clears |
| v | Password modal only without a live terminal session → `POST /items/:id/reveal` → popup; the value is a `Zeroizing<String>` dropped on dismissal |
| i | Name prompt → `init` on the cwd |
| c | `config` on the cwd workspace |
| f / s / S | Session check (password modal if none) → `fill` / `sync` / `sync --global` |
| d | `doctor` report |
| r | Reload projects |
| ? / q | Help / quit |

### Notes

Every vault-reaching action (`r`, `c`, `i`, `f`, `s`, `v`) goes through `gate()`; `c` opens a confirm modal for a secret-routing diff and applies it through `gate()` on `y`. Ctrl+C quits in every state and other Ctrl combinations are ignored. The workspace actions reuse the CLI command cores (`init::execute`, `config::prepare`/`apply`, `fill::execute`, `sync::execute`, `doctor::execute`), which never print — the TUI renders their reports in a modal. A panic hook restores the terminal. There is no auto-lock timer in the TUI; the server-side session expiry still applies.

---

## MCP

Tool namespace: `crypt-env` (invoked as `crypt_env_*`)

Protocol: JSON-RPC 2.0 over stdio (protocol version 2024-11-05)

Authentication: Automatic — MCP server reads the REST API session token from disk on startup and reuses it for all tool invocations.

| Tool | Required | Optional | Description |
|------|----------|----------|-------------|
| `crypt_env_list_items` | `environment_id` or (`project`+`environment`) | `type`, `category`, `include_global` | List item metadata (no values), scoped to a project+environment (required — the underlying `GET /items` now enforces it). `include_global` (`true`\|`false`\|`only`, default `true`) also lists reusable global secrets not yet linked into this environment — these appear with `linked: false` and will NOT be written by generate/inject/fill until linked. Filter by type or category on top |
| `crypt_env_get_item` | `id` | — | Get single item metadata (no value). Unscoped — `GET /items/:id` was not changed by this migration, reachable by id regardless of project |
| `crypt_env_search_items` | `query`, `environment_id` or (`project`+`environment`) | `include_global` | Search items by name within scope. Same `include_global` contract as `crypt_env_list_items`. Returns metadata only |
| `crypt_env_add_item` | `type`, `name`, `environment_id` or (`project`+`environment`) | `value`, `category`, `notes`, `url`, `username`, `key`, `on_conflict` | Add item to vault, owned by the resolved project and linked into the resolved environment under `key` (defaults to `name`). If `key` already exists in the target environment, the existing item is updated in place (`on_conflict` default `update`) — its previous value is destroyed. If that item is shared with other environments/projects (or is global), the call fails with `SHARED_ITEM_CONFLICT` instead of silently changing it elsewhere; retry with `on_conflict: "replace"` to create a new item and repoint just this link, or use `crypt_env_update_item` to change the shared value everywhere. `on_conflict: "error"` fails on any existing key. Value passes through MCP → REST in plaintext |
| `crypt_env_update_item` | `id` | `name`, `value`, `url`, `username`, `password`, `title`, `description`, `notes`, `content`, `command`, `shell`, `categories` | Update item. Omitted fields keep existing values server-side. Unscoped — `PUT /items/:id` was not changed by this migration |
| `crypt_env_delete_item` | `id` | — | Permanently delete item. Unscoped — `DELETE /items/:id` was not changed by this migration |
| `crypt_env_generate_env` | `keys`, `environment_id` or (`project`+`environment`) | — | Write .env file to temp dir with real values for given key names, looked up by item name within scope (see Notes for the name-vs-key mismatch vs `crypt_env_fill_env`). Returns path + count. Values never in response. Cleans up previous temp file on next call |
| `crypt_env_inject_env` | `key`, `environment_id` or (`project`+`environment`) | — | Inject one secret as env var into the MCP process via `std::env::set_var`. Does not return value |
| `crypt_env_fill_env` | `template` | `output_path`, `output_dir`, `overwrite`, `environment_id` or (`project`+`environment`) | Fill a template with vault secrets from the resolved environment, matched by `environment_vars.key`. `output_path` given: writes there. No `output_path` but `output_dir`: writes `{output_dir}/.env.<environment-name>`. Neither: **filled content returned inline in the tool response** — a value-exposure path that didn't exist when `output_path` was required. Refuses to overwrite a file it didn't create unless `overwrite: true` — see Notes |
| `crypt_env_import_env_file` | `path`, `environment_id` or (`project`+`environment`) | `category`, `overwrite` | Read .env file from disk, parse KEY=value pairs, import each as a vault item owned by the resolved project and linked into the resolved environment. Existing item found by name: `overwrite:true` updates it in place (`PUT`, destroying its previous value), `overwrite:false` (default) skips it. Not found by name but the environment key still collides (renamed item, or a race): the create call maps `overwrite:true` → `on_conflict=update` and `overwrite:false` → `on_conflict=error`, and a resulting `409` is folded into the same `skipped_existing` report bucket rather than `errors` — this is what stops bulk import from mass-producing orphans (issue #9). Values never in MCP response |
| `crypt_env_update_settings` | — | `auto_lock_timeout`, `hotkey` | Update vault settings |
| `crypt_env_doctor` | — | — | Health check: app status, lock state, token config. No longer reports item count — `GET /health` stopped returning `item_count` |
| `crypt_env_list_commands` | `environment_id` or (`project`+`environment`) | — | List saved commands with placeholders, scoped to a project+environment |
| `crypt_env_run_command` | `name`, `environment_id` or (`project`+`environment`) | `params` | Resolve {{VAR}} placeholders and execute command via shell. Command lookup is scoped; the detail fetch itself is unscoped. Returns stdout/stderr (truncated 2000 chars) and exit code. Resolved command never returned |
| `crypt_env_share_listen` | `items`, `environment_id` or (`project`+`environment`) | — | Start LAN share session. `items` must already be linked into the resolved environment or the call fails. Returns pairing_code |
| `crypt_env_share_connect` | `pairing_code`, `environment_id` or (`project`+`environment`) | — | Connect as receiver. Received items get owned by the resolved project and linked into the resolved environment (skipping any that collide with an existing key — see Notes). Returns fingerprint |
| `crypt_env_share_confirm` | `confirmed` | — | Confirm/reject fingerprint (boolean) |
| `crypt_env_share_cancel` | — | — | Cancel active share session |
| `crypt_env_share_status` | — | — | Get session state, fingerprint, direction, skipped_keys |
| `crypt_env_share_export` | `items`, `output_path` | — | Export as .vault package. Passphrase shown in response once. Unscoped (operates on item IDs directly) |
| `crypt_env_share_import` | `path`, `passphrase`, `environment_id` or (`project`+`environment`) | — | Import from .vault package. Imported items are owned by the resolved project and linked into the resolved environment, same collision-skip behavior as `crypt_env_share_connect` |
| `crypt_env_list_categories` | — | — | List categories |
| `crypt_env_create_category` | `name`, `color` | `description` | Create category |
| `crypt_env_update_category` | `id` | `name`, `color`, `description` | Update category. Empty string description clears it |
| `crypt_env_delete_category` | `id` | — | Delete category |
| `crypt_env_list_projects` | — | — | List all projects with their typed environments (name, template, paths, var count) |
| `crypt_env_inject_environment` | — | `environment_id` or `project`+`environment`, `output_path`, `output_dir`, `overwrite` | Inject an environment's variables into its configured .env path(s), plus `output_path`/`output_dir` if given (appended to, not replacing, configured paths). Identify by `environment_id` or project+environment names. Refuses to overwrite an `output_path`/`output_dir` target it didn't create unless `overwrite: true` — see Notes |
| `crypt_env_generate_example_env` | — | `environment_id` or `project`+`environment`, `output_path`, `output_dir`, `overwrite` | Generate a placeholder-only (`KEY=`, no values) env file/content for an environment — safe to commit. Never reads or decrypts item values. Same `overwrite` gating as the two tools above |
| `crypt_env_list_environments_by_name` | — | — | List all environments across all projects, grouped by their real environment name (production, local, test, etc.) |
| `crypt_env_inject_env_by_name` | `project_path`, `environment` | `output_path` (unused, kept for compatibility) | Inject environment variables for a project directory and environment name. Matches by real environment name; if no matching environment is found, returns an error with next steps — the previous item-naming-convention fallback was removed (see Notes) |
| `crypt_env_relay_send` | `item_ids` | — | Send via internet relay. Returns code + passphrase (show immediately, only once) |
| `crypt_env_relay_receive` | `code`, `passphrase`, `environment_id` or (`project`+`environment`) | — | Receive via internet relay. Imported items are owned by the resolved project and linked into the resolved environment, same collision-skip behavior as `crypt_env_share_connect` |
| `crypt_env_list_mcp_servers` | — | `scope` | List registered MCP servers from Claude config. Scope: global/project/all. Env values never returned |
| `crypt_env_add_mcp_server` | `name`, `command` | `args`, `env`, `scope` | Add MCP server to Claude config. `env` stores KEY: "" placeholders — never real values |
| `crypt_env_update_mcp_server` | `name` | `command`, `args`, `env`, `scope` | Merge-update existing MCP server entry |
| `crypt_env_delete_mcp_server` | `name` | `scope` | Remove MCP server entry from Claude config |

### Notes

`crypt_env_add_item` with `value` parameter passes the secret plaintext through the LLM's tool call context — this is the only tool that breaks the "values never visible to LLM" design principle, and any agent/LLM host logging tool calls will capture the raw secret value permanently.

`crypt_env_generate_env` writes to `std::env::temp_dir()` with random filename but does NOT enforce restrictive permissions (unlike the API's TempEnvFile with 0o600) — on multi-user systems the temp .env is world-readable until cleanup or next call.

`crypt_env_generate_env`/`crypt_env_inject_env` look up keys by vault **item name** (`GET /items?search=`); `crypt_env_fill_env` matches by `environment_vars.key`. When an item's linked key differs from its name (supported since `POST /items` gained the `key` field), these two families disagree on which variables they can find for the same environment.

`crypt_env_inject_environment`'s new `output_path`/`output_dir` parameters let a single call write an environment's full decrypted variable set to any filesystem path, appended to (not replacing) the environment's GUI-configured paths — previously this tool could only write to paths a human had configured through the app. A prompt-injected or confused agent can use this for a one-shot full-environment secret dump; there is no per-key selection or additional confirmation gate on this path beyond normal tool-call approval.

`crypt_env_inject_env` uses `std::env::set_var` which is marked `unsafe` in Rust with `#[allow(unused_unsafe)]` — calling set_var from a multi-threaded context is undefined behavior if any thread reads env vars concurrently; the MCP server's single-threaded main loop mitigates this in practice.

`crypt_env_run_command` substitutes {{VAR}} placeholders into shell commands without sanitization — if params contain shell metacharacters or a vault item's command template is user-controlled, this is a command injection vector.

`crypt_env_inject_environment` resolves environment by `environment_id` or by project+environment names (both case-insensitive). The environment's configured paths (plus `output_path`/`output_dir` if given) are used to write .env files; variables are matched via `environment_vars.item_id` (a real FK), not by name search. Returns paths written and keys injected.

**Scope-parameter naming is now consistent across every environment-scoped tool** (issue #10): `crypt_env_inject_environment` and `crypt_env_generate_example_env` used to be the only tools naming the scope parameter `id` instead of `environment_id`, which let an LLM that inferred the name from every other tool's schema pass a key that was silently ignored — in the worst case resolving a *different* environment by name with no error. Both schemas now advertise `environment_id` with the same wording used everywhere else. The bare `id` key is still accepted as an unadvertised, deprecated alias for the 1.0.x line only (removed in 1.1.0): calls using it still succeed, but the response text appends `note: parameter 'id' is deprecated on this tool; use 'environment_id' instead.` The invariant that no environment-scoped tool declares a bare `id` is enforced by tests in `src-tauri/src/bin/crypt-env-mcp.rs` (`#[cfg(test)] mod tests`, run via `cargo test --bin crypt-env-mcp`).

`crypt_env_fill_env`, `crypt_env_inject_environment` and `crypt_env_generate_example_env` all gate their `output_path`/`output_dir` write target the same way as the REST API (see REST API's **Write-target gating**): an existing file not carrying the crypt-env marker gets a `409` from the underlying endpoint, surfaced by the tool as `target_exists: <server message>. Ask the user before retrying with overwrite=true.` — advisory only, since an agent can ignore the instruction and retry anyway; the actual control is that the prior contents are preserved in `<path>.bak` the moment `overwrite: true` is passed. `crypt_env_inject_environment`'s owner-configured `environment.paths[]` are never gated this way and don't need `overwrite` to write through them.

`crypt_env_list_environments_by_name` lists all environments grouped by their real environment name field, providing a different view than `crypt_env_list_projects` (which groups by project).

`crypt_env_inject_env_by_name` resolves by project directory + environment name. If a real environment is found, its paths are used. If not found, the tool now returns an error with next steps (pass an explicit `environment_id`, or `project`+`environment`) — the previous fallback to item-naming-convention matching (name prefix, category) was removed, since it relied on the now-scope-required `/items`/`/fill` endpoints in a way that could no longer work safely. `output_path` is accepted for backward compatibility but is currently unused.

Fixed (issue #13): `crypt_env_list_items` and `crypt_env_search_items` now default to `include_global=true`, unioning in reusable global items (`isGlobal: true`) that are not yet linked into the queried environment — each result carries `isGlobal` and `linked` so the agent can tell "exists and reusable" apart from "will actually be written". Pass `include_global=false` to see exactly the linked set, or `only` to see just the globals. The remaining gap is unchanged: `crypt_env_generate_env` and `crypt_env_inject_env` still resolve strictly by linkage (matching `/fill`'s/`/inject`'s materialization-only contract) — a global secret discovered via `crypt_env_list_items` still requires an explicit link into the environment (e.g. via `crypt_env_add_item`) before either of those tools can write it.

Whole-project relay sharing (`POST /projects/:id/relay/send` / `/projects/relay/receive`) intentionally has **no MCP tool** — sending an entire project's decrypted secrets to a third-party relay from a single agent-callable tool, with no way for the agent to meaningfully obtain the sender's confirmation, is a materially different trust decision than the existing per-item `crypt_env_relay_send`/`_receive`. If ever wanted, it needs its own consent design as a separate change.

MCP server is single-threaded with a blocking I/O loop (`stdin.lock().lines()`) — a slow or stalled request blocks all subsequent MCP tool calls and the LLM host may time out other concurrent requests.

`TEMP_FILES` cleanup in `crypt_env_generate_env` is best-effort (called at start of next call) — if the MCP process exits before another call, temp .env files are abandoned on disk.

MCP config management tools (`crypt_env_add/update/delete_mcp_server`) do not validate the `command` field for path traversal or injection — a crafted command string could write an adversarial entry to the Claude config files.

---

## Tauri commands — WSL Integration (Windows only)

Invoked by the Settings → WSL Integration panel via `invoke()`. On every non-Windows build they return `{ "kind": "unsupported" }` immediately, spawning nothing and touching no files. `wsl.exe` is always driven with argument vectors — the distro name is a discrete argument, never interpolated into a shell string. See [`wsl-windows.md`](wsl-windows.md#gui-route-settings--wsl-integration).

| Command | Args | Returns | Side effects |
|---------|------|---------|--------------|
| `wsl_detect` | — | `WslStatus` | None (read-only). Runs `wsl --list --quiet`, then per distro `whoami` and a fixed `test`/`grep` probe; reads `%UserProfile%\.wslconfig`. |
| `wsl_configure_client` | `distro: string` | `WslActionReport` | Re-validates `distro` against a fresh `wsl --list`, then runs the bundled `crypt-env-setup` helper inside it (copied to `/tmp`, deleted afterwards). Writes only `~/.config/cryptenv/env.sh`, the rc marker block, and the one-time `.cryptenv.bak`. |
| `wsl_remove_client` | `distro: string` | `WslActionReport` | Same validation; runs the helper with `--remove`. No-op when nothing is installed. |

```ts
interface WslStatus {
  available: boolean;                 // at least one distro detected
  distros: { name: string; defaultUser: string | null; configured: boolean }[];
  mirrored: boolean;                  // [wsl2] networkingMode=mirrored in .wslconfig
}

// cryptenv_setup::ActionReport — also the helper's stdout JSON (snake_case).
interface WslActionReport {
  env_file: string;                   // ~/.config/cryptenv/env.sh
  env_file_changed: boolean;          // configure: (re)written · remove: deleted
  rc_files: string[];                 // rc files modified by this run
  backups: string[];                  // one-time backups created by this run
  marker_added: boolean;              // configure: block inserted somewhere
  marker_removed: boolean;            // remove: block deleted somewhere
}

// Rejection payload
type WslError =
  | { kind: 'unsupported' }           // not Windows
  | { kind: 'notAvailable' }          // wsl.exe missing
  | { kind: 'unknownDistro'; message: string }
  | { kind: 'tooling'; message: string };  // wsl.exe / helper failure, garbled output
```

Neither command reads, logs, or passes any secret value; reports contain only paths and booleans.
