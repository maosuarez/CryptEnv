## Why

The file-writing core used by `/fill`, `/environments/:id/example`, GUI inject and CLI `fill` has content and robustness defects. These are separate from the path and symlink issues handled in `harden-cli-manifest-and-sessions`:
- **API-1:** `envfile::inspect` reads the whole target with `read_to_string` (`envfile/mod.rs:157`). If the target is `/dev/zero` the process reads until it runs out of memory; if it is a FIFO, a tokio worker blocks forever. Any caller that can choose `output_path` can do this. The MCP principal is being confined in `mcp-token-capabilities`, but the core must also be safe for session callers and configured paths.
- **API-5:** writes are not atomic. `write_with_mode` truncates the file and then calls `write_all` (`envfile/mod.rs:233-252`). An I/O error or a full disk mid-write leaves a partial plaintext file. If the target was a managed file, its previous contents are lost with no `.bak`. The `TempEnvFile` guard is armed only after `commit` and is disarmed straight away (`api/mod.rs:1614`).
- **CORE-8:** values and keys are written as `KEY=value` with no quoting or escaping (`project/mod.rs:797,819`). A multi-line secret (a PEM key, or note content) corrupts the file, and a value like `x\nADMIN=1` **injects an extra variable**. Inject also decrypts *every* item in the vault into a plain `HashMap<String,String>` (`project/mod.rs:759`), not just the items the environment references. So every secret is held in plaintext memory for each inject, and one undecryptable item makes every inject fail.

## What Changes

- **Only regular files are inspected or replaced.** Anything else (device, FIFO, socket, directory) is refused with `NOT_REGULAR_FILE`. Inspection reads at most 1 MiB; a larger existing file is treated as foreign and never read fully.
- **Atomic replace.** Content is written to an exclusively created temp file in the target directory with the final mode (0600), `fsync`ed, then renamed over the target. A failure leaves the previous file intact and no temp file behind. The symlink refusal from `harden-cli-manifest-and-sessions` is preserved: it is checked before the rename.
- **Correct dotenv serialization.**
  - Values made only of safe characters are written bare.
  - Other values without a newline or `'` are single-quoted (literal in common dotenv parsers).
  - Anything else is double-quoted, escaping `\`, `"`, `\n`, `\r` and `$`.
  - Keys must match `^[A-Za-z_][A-Za-z0-9_.]*$`, otherwise the key is refused and reported.
  - Existing managed lines are updated with the same serializer.
- **Decrypt only what is needed.** Inject decrypts only the item ids referenced by the environment's variables. It holds values in zeroizing containers, and reports an undecryptable referenced item by key name without failing the other keys.

## Capabilities

### New Capabilities
- `env-file-content`: rules for the content crypt-env writes into `.env` files (serialization, key validity, what gets decrypted), and the robustness of the write itself (file-type checks, bounded reads, atomic replace).

### Modified Capabilities
<!-- none -->

## Impact

- `envfile/mod.rs` (inspect, commit, write, new `serialize_line`), `project/mod.rs` (inject merge, targeted decrypt), `api/mod.rs` (`TempEnvFile` guard, error mapping for `NOT_REGULAR_FILE`), and the CLI `sync --global` `.env` writer (reuses the serializer).
- `db/mod.rs`: a `get_items_by_ids` helper.
- **Behavior visible to users:** values with spaces, quotes or newlines are now quoted in written `.env` files. This is more correct, but existing parsers that expected raw values could differ; see design Risks. Documented in `docs/index.html`.
- **Depends on** `harden-cli-manifest-and-sessions` (the no-follow open and `Target::Symlink`).

## Non-Goals

- Writing to formats other than dotenv (e.g. `.json`, `.toml`).
- Validating what consumers do with the values.
- Path confinement for the MCP principal (in `mcp-token-capabilities`).
