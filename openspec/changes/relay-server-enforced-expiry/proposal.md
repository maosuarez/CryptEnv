## Why

The internet relay promises burn-after-read and a 24-hour TTL. Neither is enforced (audit MCP-5):
- `relay_download` filters on `retrieved=eq.false`, but nothing ever sets `retrieved`, and `expires_at` is never checked (`share/relay.rs:199-231`).
- The SQL we tell users to run (`src/components/Settings.tsx:78-89`) grants anon `SELECT using(true)` and `UPDATE using(true)`, and grants no DELETE. `relay_delete` therefore removes 0 rows, and its result is ignored (`let _ =`).
- Anyone with the (bundled or shared) anon key can list every code (`select=*`), download any payload at any time, overwrite payloads, and destroy other users' shares.
- Argon2id key derivation (32 MiB) also runs on async runtime threads in several handlers, with no concurrency limit (MCP-12).

## What Changes

- **New relay schema v2 (BREAKING, needs the user to re-run SQL):**
  - The table stores a domain-separated hash of the code, never the code itself.
  - Row-level security is enabled with **no** table grants for `anon`.
  - Every access goes through two `SECURITY DEFINER` RPCs: `relay_put` (size-capped insert with a 24h expiry) and `relay_claim`, which atomically deletes the row and returns the payload only if it has not expired.
  - Expired rows are purged on every RPC call.
- **Client uses the RPCs.** It checks HTTP status on every call, and detects the old schema (a missing RPC) with a clear "apply relay SQL v2" message that shows the SQL.
- **Settings shows the v2 SQL**, with a one-click copy and a "check schema" button.
- **Argon2 is bounded.** Relay key derivation runs in `spawn_blocking` behind a process-wide semaphore of 2.
- **Explicit HTTP timeouts on relay calls:** 10 s to connect, 30 s total.

## Capabilities

### New Capabilities
- `relay-sharing`: server-enforced single-use, expiry and access rules for the internet relay, schema-version detection, and bounded client resource use.

### Modified Capabilities
<!-- none -->

## Impact

- `share/relay.rs` (upload, claim, schema probe, timeouts); `vault/share_commands.rs`, `project/relay_commands.rs` and the `api/mod.rs` relay handlers (spawn_blocking + semaphore, error mapping).
- `src/components/Settings.tsx` (SQL v2, schema check UI) and the i18n strings.
- Docs: the relay setup in `docs/index.html` and the relevant `docs/*.md` (migration steps).
- Security: the relay really becomes single-use and expiring; codes can no longer be enumerated; anon clients can no longer tamper with other users' shares.

## Non-Goals

- Hosting a crypt-env-operated relay.
- Changing the payload crypto (AES-256-GCM + Argon2id with the passphrase) or the code/passphrase formats.
- Migrating in-flight v1 shares. They expire, and senders re-share after upgrading.
