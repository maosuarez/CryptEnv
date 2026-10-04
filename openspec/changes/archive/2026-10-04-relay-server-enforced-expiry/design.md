## Context

The relay is a user-configured Supabase project (URL + anon key in settings). The client uses PostgREST directly. Payloads are encrypted client-side with a key derived from `code + passphrase`, so the relay only ever sees ciphertext. The weak points are availability, single-use and enumeration, not confidentiality of the payload.

## Goals / Non-Goals

**Goals:** make single-use and TTL server-side invariants; remove anon table access; give a clean upgrade path.

**Non-Goals:** backward compatibility with v1 tables.

## Decisions

### D1. SQL v2 (shown in Settings, versioned)
```sql
create table if not exists relay_packages_v2 (
  code_hash  text primary key,
  payload    text not null check (octet_length(payload) <= 1048576),
  expires_at timestamptz not null default now() + interval '24 hours'
);
alter table relay_packages_v2 enable row level security;
revoke all on relay_packages_v2 from anon, authenticated;

create or replace function relay_put(p_code_hash text, p_payload text) returns void
language plpgsql security definer set search_path = public as $$
begin
  delete from relay_packages_v2 where expires_at < now();
  insert into relay_packages_v2(code_hash, payload) values (p_code_hash, p_payload);
end $$;

create or replace function relay_claim(p_code_hash text) returns text
language plpgsql security definer set search_path = public as $$
declare v text;
begin
  delete from relay_packages_v2 where expires_at < now();
  delete from relay_packages_v2 where code_hash = p_code_hash returning payload into v;
  return v;
end $$;

create or replace function relay_schema_version() returns int language sql as $$ select 2 $$;
grant execute on function relay_put(text,text), relay_claim(text), relay_schema_version() to anon;
```
- A new table name (`_v2`) avoids clashing with the permissive v1 policies. The docs tell users to `drop table relay_packages` afterwards.
- `code_hash` = hex(SHA-256(`"cryptenv-relay-lookup-v1:" || code`)). The domain separation keeps it distinct from the Argon2 salt (`SHA-256(code)`).
- Rejected: `pg_cron` purging (not available on every plan); an Edge Function (extra deploy step).

### D2. Client
- `relay_upload` → `POST /rest/v1/rpc/relay_put`. `relay_download`/`relay_delete` are merged into `relay_claim` → `POST /rest/v1/rpc/relay_claim`. A `null` result means not found or used.
- A 404 or `PGRST202` (function not found) maps to `ShareError::RelaySchemaOutdated`, which the GUI renders with the SQL.
- The reqwest blocking client is built with `connect_timeout(10s)` and `timeout(30s)`.
- Errors carry the status and a PostgREST error code only, never the payload.

### D3. Argon2 bounding
A `static RELAY_KDF: Semaphore(2)` (tokio). All call sites wrap `derive_relay_key` in `spawn_blocking` after acquiring a permit. The same helper is reused by `api/mod.rs:2964,3294`.

## Security & Threat Model

- **Adversary:** anyone holding the anon key (it's in every client config).
- **v1:** read, enumerate, tamper, replay.
- **v2:** can only upload (≤1 MiB) or claim by exact code. Guessing requires the code's entropy, and a guessed claim burns the package without revealing it (it is still encrypted with the passphrase).
- **Residual:**
  - DoS by filling the table: bounded per row and purged at 24h; a quota is a Supabase-side concern.
  - A claim by a code-guesser destroys the share: the receiver sees "already used" and the sender re-shares. This is documented.

## Risks / Trade-offs

- [BREAKING: existing users must apply the SQL] → Explicit detection and message, a Settings check button, and docs.
- [SECURITY DEFINER functions are misconfigurable] → `set search_path`, and the SQL is provided verbatim.

## Migration Plan

1. Ship the client plus the SQL.
2. The user applies the SQL.
3. The old table can be dropped.

Rollback: an old client with the v2 schema fails cleanly (no table); re-running the v1 SQL restores the old behavior.
