# 🔗 Secret Sharing Guide

CryptEnv offers three secure methods for sharing secrets with team members without third-party plain-text services:

1. **LAN Bridge** — Real-time local network sharing via mDNS and a pairing-code-authenticated key exchange (SPAKE2).
2. **Encrypted Packages** — Offline portable `.vault` files with Argon2id-derived keys.
3. **Internet Relay** — Remote team sharing via an ephemeral, burn-after-read Supabase table.

---

## 1. LAN Bridge (Local Network, Real-Time)

Ideal for colleagues on the same Wi-Fi / local network.

### How It Works:
- **Discovery**: mDNS advertises only a random session id and the protocol version. Nothing derived from the pairing code is broadcast; the receiver tries each advertised service (at most 5) until one accepts its code.
- **Key Exchange**: SPAKE2 keyed by the 6-digit pairing code. Only a peer that knows the code can derive the session key, an online guess costs one connection attempt, and nothing useful for offline guessing is exposed.
- **Pairing & Verification**: Sender displays a 6-digit code (valid 5 min). Both sides first exchange key-confirmation MACs (a wrong code fails here and no item is sent), then show a 64-bit transcript fingerprint (`XXXX-XXXX-XXXX-XXXX`) that both users compare and confirm in the desktop app.
- **Limits**: 30 s socket read/write timeouts, 60 s handshake deadline, 3 failed pairing attempts abort the session, one session at a time (a second start is rejected until the first is cancelled or finished).
- **Lifetime**: Locking the vault or closing the share/receive dialog cancels the session, closes the listener, removes the mDNS advertisement and drops the session's copy of the vault key.
- **Compatibility**: Protocol v2 is incompatible with v1.0.6. A version mismatch ends the session with an "update the other device" message; update both devices.
- **Data Transfer**: Direct AES-256-GCM encrypted TCP stream.

---

## 2. Encrypted Package (Offline / Portable)

Ideal for asynchronous sharing via email, Slack, or USB drives.

### How It Works:
- Sender exports items into a self-contained `.vault` package from the desktop app (the CLI `share` commands were removed).
- Generates a random 12-character high-entropy passphrase.
- Encrypted using **AES-256-GCM** with keys derived via **Argon2id** (32MB memory cost).
- Receiver imports the file in the desktop app, entering the passphrase received out-of-band.

---

## 3. Internet Relay (Remote Teams)

For teams working remotely across different networks, CryptEnv provides ephemeral cloud relay sharing through Supabase.

### Security Highlights:
- **Zero-Knowledge Relay**: Supabase stores only the AES-256-GCM ciphertext (and a hash of the code, never the code). The decryption passphrase **never touches the server**.
- **Burn-After-Read (server-enforced)**: retrieval atomically deletes the package; a second claim returns "code not found or already used".
- **24-Hour TTL (server-enforced)**: expired packages are never returned and are purged on every relay call.
- **No direct table access**: the `anon` key cannot list, read, update or delete rows. It can only call `relay_put` (payload at most 1 MiB) and `relay_claim`.
- **Timeouts and bounds**: 10 s connect / 30 s total per relay call; at most 2 Argon2id derivations run concurrently.

### Supabase Relay Setup (One-Time) — SQL v2

1. Create a free project at [supabase.com](https://supabase.com).
2. Run this SQL in the **SQL Editor** (also shown in **Settings** → **Internet Sharing** → **Setup SQL**):

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

3. In CryptEnv desktop app (**Settings** → **Internet Sharing**), input:
   - Supabase Project URL
   - Supabase `anon` public key
4. Click **Check schema**. It confirms the v2 functions are present.

### Migrating from the old relay SQL (BREAKING)

Earlier versions used a permissive `relay_packages` table whose burn-after-read and TTL were not enforced. The app no longer falls back to it: send and receive fail with `RELAY_SCHEMA_OUTDATED` (and show the SQL) until you apply the v2 SQL above. Steps:

1. Run the v2 SQL in the Supabase SQL Editor.
2. Click **Check schema** in Settings.
3. Optionally remove the old table: `drop table relay_packages;`

Shares created on the old table are not migrated; re-share them.

Note: anyone who guesses a live code can claim it first, which burns the package (it remains encrypted with the passphrase). The receiver sees "already used" and the sender re-shares. A wrong passphrase also consumes the package.

---

## 📋 Audit Trail

All sharing operations (sending, receiving, exporting, importing) are recorded in the local SQLite `share_log` table with timestamps and fingerprint hashes for local auditing.
