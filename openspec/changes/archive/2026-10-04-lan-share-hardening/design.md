## Context

The current flow works like this:
1. The listener binds `0.0.0.0:0` and advertises mDNS with `pc = hex(SHA-256(code)[..4])`.
2. The receiver browses for a matching `pc` and connects.
3. Both sides exchange x25519 ephemeral keys and derive the key with HKDF.
4. Both sides show a short fingerprint; after confirmation, items are sent AES-GCM-encrypted.

`ShareState` holds a single `Mutex<Option<ShareSession>>`, which is overwritten on each start. The vault key (`[u8;32]`) is cloned into the session for building the encrypted items.

## Goals / Non-Goals

**Goals:** authentication bound to the pairing code; bounded resource lifetime; exclusivity; lock-safety.

**Non-Goals:** backwards protocol compatibility.

## Decisions

### D1. SPAKE2 (symmetric, `spake2` crate, Ed25519 group)
The flow:
1. Both sides run `Spake2::<Ed25519Group>::start_symmetric(Password(code), Identity("cryptenv-lan-v2"))`.
2. They exchange messages.
3. `K = finish()`.
4. `session_key = HKDF-SHA256(K, info="cryptenv-lan-v2 session")`.
5. Confirmation: `MAC_A = HMAC(K_c, "A"||transcript)` and `MAC_B` likewise, where `K_c` comes from the HKDF with a different info string.
6. Both MACs are exchanged *after* both SPAKE messages. With a PAKE, the MAC reveals nothing that allows offline guessing (the property the HKDF-mix alternative lacks).

The fingerprint is `hex(SHA-256(transcript)[..8])`, shown as `XXXX-XXXX-XXXX-XXXX`.

*Rejected:*
- **Mixing the code into HKDF after ECDH:** a MITM receives the first MAC, brute-forces 10^6 codes offline in milliseconds, then completes the handshake.
- **CPace:** no maintained crate.
- **Keeping x25519 plus a long fingerprint only:** relies entirely on humans comparing it.

The x25519 ephemeral exchange is dropped (SPAKE2 provides forward secrecy per session).

### D2. Discovery
The TXT record becomes `{sid: random 8 bytes hex, v: "2"}`. The receiver cannot pick a service by code, so it tries each advertised service in turn (at most 5, each bounded by the handshake timeout). A wrong service fails PAKE fast and is skipped. With the attempt limit of 3 per listener, a legitimate receiver facing many attacker services could be delayed. That is accepted, and the UI lists the discovered hosts.

### D3. Timeouts and attempts
- `set_read_timeout`/`set_write_timeout(30s)` on every stream.
- A handshake deadline checked with `Instant`.
- The listener loop accepts connections and handles one handshake at a time; after 3 failures the session goes to `Failed("too many attempts")`.
- The listener socket is non-blocking with a 500 ms poll, so cancellation is observed promptly.

### D4. Session exclusivity
- `ShareSession` gets `id: u64`, taken from an incrementing counter.
- Background tasks capture their `id` and, at each state poll, exit if `session.id != id` or the state is `Cancelled`.
- `start_*` returns `ShareError::SessionActive` when the state is not terminal.
- `cancel_session` sets `Cancelled`, drops the `ServiceDaemon` (which unregisters it), and closes the listener by dropping it. The task observes this within 500 ms.

### D5. Lock integration
`vault::lock_vault` calls `share::cancel_all(&share_state)`. The vault orchestrates both, which respects the module boundaries. The session's key copy becomes `Zeroizing<[u8;32]>` and is dropped on cancel. The frontend modals call `share_cancel` in their `useEffect` cleanup.

## Security & Threat Model

| Attacker | Before | After |
|---|---|---|
| LAN host racing the real receiver | Gets items if the user confirms a fingerprint they didn't compare carefully | Cannot derive the key without the code; one online guess per connection, at most 3 per session |
| Passive mDNS observer | Recovers the code | Learns nothing |
| MITM | Grinds the 32-bit fingerprint | PAKE blocks it |
| Stuck peer | Hangs forever | Bounded to 30 s / 60 s |

## Risks / Trade-offs

- [New dependency `spake2`] → pure Rust, audited RustCrypto family, and the curve is already in the tree.
- [BREAKING protocol] → a version check with a clear message.
- [Multiple-advertiser delay] → capped at 5 services; the UI shows progress.

## Migration Plan

Both devices must update. No stored data changes.
