## Why

LAN sharing sends decrypted vault items to a peer, but its pairing, lifetime and resource handling are weak:
- **MCP-6:** no socket timeouts. `sender_handshake`, `receiver_handshake` and `receiver_receive_items` block in `read_exact` forever (`share/lan.rs:215,272`, `share/protocol.rs:57`). A port scanner that connects and stays silent hangs the session. It cannot be cancelled, and the 5-minute timeout doesn't cover it.
- **MCP-7:** starting a new session overwrites the single session slot without stopping the old one (`share/mod.rs:218-221,461-481`). Old tasks keep their listener and mDNS registration, and they watch the *new* session's state. Confirming session B can make task A send A's items to A's peer.
- **MCP-8:** pairing is not authenticated:
  - The sender accepts the first TCP connection.
  - mDNS broadcasts 4 bytes of `SHA-256(pairing_code)`, which reveals the 6-digit code after at most 10^6 hashes.
  - The final check is a short fingerprint that the MCP tool could confirm itself (MCP confirmation is removed in `mcp-token-capabilities`).
- **MCP-11, CLI-5:** the session copies the raw vault key (`share/mod.rs:192-200,255`, `vault/share_commands.rs:61`). Locking the vault neither cancels the session nor wipes that copy. The share modals only stop polling on unmount; they never cancel the session.

## What Changes

- **PAKE pairing.** Pairing uses SPAKE2 keyed by the pairing code, so only a peer that knows the code can derive the session key. Offline guessing from a transcript is impossible, and an online guess costs one attempt per session. The pairing-code hash is removed from mDNS. The TXT record carries a random session id and a protocol version.
- **Key confirmation plus a longer SAS.** Both sides exchange key-confirmation MACs before any item is sent. The human-compared fingerprint becomes 64 bits, shown as 4 groups of 4 hex digits.
- **Timeouts.** Every accepted and connected socket gets 30-second read and write timeouts. Handshakes must finish within 60 seconds. After 3 failed PAKE attempts the session aborts.
- **One session at a time.** Starting a session while one is active is rejected, unless the caller cancels the current one first. Every background task checks the session id it was started for and exits when it no longer matches. Cancelling closes the listener, unregisters mDNS, and ends the tasks.
- **Lock cancels sharing.** Locking the vault cancels any share session and drops its key copy, which is held in `Zeroizing`. The share modals call `share_cancel` on unmount.
- **BREAKING:** LAN sharing is incompatible with v1.0.6 peers. The protocol version is checked, and a mismatch shows "update the other device".

## Capabilities

### New Capabilities
- `lan-sharing`: authenticated pairing, session exclusivity and lifetime, timeouts, and lock interaction for peer-to-peer LAN sharing.

### Modified Capabilities
<!-- none -->

## Impact

- `share/lan.rs`, `share/protocol.rs`, `share/mod.rs`, `share/crypto.rs`, `vault/share_commands.rs`, `vault/mod.rs` (`lock_vault` → cancel share).
- `src/components/ShareModal.tsx` and `ReceiveModal.tsx` (cancel on unmount, new fingerprint format).
- **New dependency:** `spake2` (RustCrypto, pure Rust, already built on `curve25519-dalek`, which is in the tree through `x25519-dalek`). It is cross-platform with no native code.
- Docs: the LAN sharing section in `docs/index.html`.

## Non-Goals

- NAT traversal or internet P2P.
- Changing the item payload encryption after the key is established.
- Relay sharing, which is covered by `relay-server-enforced-expiry`.
