## Context

`envfile` owns `inspect` (classifies Absent/Managed/Foreign) and `commit` (backup, marker, write). `project::inject_environment` builds the content by merging with existing managed lines. After `harden-cli-manifest-and-sessions`, `open_nofollow` and `Target::Symlink` exist.

## Goals / Non-Goals

**Goals:** a parse-faithful serializer; atomic writes; no unbounded or blocking reads; decrypt minimisation.

**Non-Goals:** dotenv dialect detection per consumer.

## Decisions

### D1. File-type gate
`inspect` calls `symlink_metadata` first:
- `NotFound` → Absent;
- symlink → `Target::Symlink` (from harden);
- `!file_type().is_file()` → `EnvFileError::NotRegularFile`.

Otherwise it opens with no-follow plus `O_NONBLOCK` on unix (belt-and-braces against a race to a FIFO), re-checks `fstat` is regular, and reads with `take(1 MiB + 1)`. Over 1 MiB → Foreign.

### D2. Atomic write
`tempfile::Builder::new().prefix(".cenv-").tempfile_in(dir)`: exclusive, random name, 0600 on unix by default. Then `write_all`, `sync_all`, and a symlink re-check on the target (refuse if it became a symlink), then `persist(target)`. On Windows, `persist` uses `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`. The `.bak` for Foreign targets is taken before `persist`, as today. The `TempEnvFile` guard in `api/mod.rs` is armed before commit and disarmed only on success of the whole multi-path operation.

Directory `fsync` after the rename happens on unix only (best effort).

### D3. Serializer
```
fn serialize_value(v) -> String:
  if v matches ^[A-Za-z0-9_./:@+,=-]*$ → v
  else if !v.contains('\n') && !v.contains('\r') && !v.contains('\'') → "'" + v + "'"
  else → "\"" + escape(v, ['\\'→"\\\\", '"'→"\\\"", '$'→"\\$", '\n'→"\\n", '\r'→"\\r"]) + "\""
```
The merge of existing managed lines replaces the whole line with `KEY=serialize(v)`. The crypt-env `.env` reader (`add .env`, `sync`) is updated to the same grammar, so its round trip holds (verified by a property test).

*Compatibility matrix* (task 1.3 checks it with fixtures): python-dotenv, Node `dotenv` ≥16 (multiline double quotes), docker compose v2, and bash `source` for the single-quoted and bare forms.
- *Results (task 1.3, `envfile::content_safety_tests::compat_fixture_*`, 2026-10-04, Linux/WSL):* python-dotenv (current pip release) and Node `dotenv` (current npm release) read bare, single-quoted, `'`-containing and multi-line PEM values back exactly. bash `set -a; . file` reads the bare and single-quoted forms exactly (values with spaces, `$` and `#` included); the double-quoted form is not shell-compatible by design (`\n` stays literal). docker compose v2 was not run (not installed).
- Node `dotenv` does not unescape `\$`. Its behavior for `$` inside double quotes is documented as a limitation: values that contain both `$` and a newline or `'`.

*Rejected:* always double-quote (breaks the bare-value expectations of some tools and changes every line); base64 (unreadable).

### D4. Targeted decrypt
`db.get_items_by_ids(&ids)` returns the rows for the referenced ids only. Values go into `HashMap<String, Zeroizing<String>>`. A decrypt error adds the key to `InjectResult.failed_keys: Vec<String>`, a new field (additive to the REST response).

## Security & Threat Model

- Value-driven variable injection into `.env` files (e.g. a relay-received or imported value adding `NODE_OPTIONS=--require /tmp/x`) is closed.
- DoS through a device or FIFO output target is closed for all callers.
- Plaintext exposure per inject shrinks from the whole vault to the environment's referenced items.

## Risks / Trade-offs

- [Quoted values change consumer behavior for tools that read raw lines] → Only values that previously produced *wrong* files change form, apart from values with spaces, now single-quoted. Documented as a behavior change.
- [`failed_keys` field is new] → Additive JSON; CLI and GUI render it.

## Migration Plan

None. The next inject rewrites managed files with the new serializer.
