# lan-sharing Specification

## Purpose
Defines the security and lifetime guarantees of peer-to-peer LAN secret sharing. Only a peer that knows the pairing code, and whose fingerprint the user has confirmed, can receive items. Sessions are exclusive, bounded in time, and end when the vault locks.

## Requirements

### Requirement: Pairing-code authenticated key exchange

The session key MUST be derived through a password-authenticated key exchange keyed by the pairing code. A peer that does not know the pairing code MUST NOT be able to derive the session key. It MUST NOT be able to test more than one pairing-code guess per connection attempt, and it MUST NOT be able to test guesses offline from observed traffic. No value derived from the pairing code alone (such as a hash of it) SHALL be broadcast or sent.

#### Scenario: Wrong code
- **WHEN** a peer connects using the wrong pairing code
- **THEN** key confirmation fails, no item is sent, and the attempt counts toward the session's failure limit

#### Scenario: Discovery reveals nothing about the code
- **WHEN** an observer captures the mDNS advertisement
- **THEN** the advertisement contains only a random session id and a protocol version

### Requirement: Key confirmation and human-verified fingerprint

Both peers MUST verify key-confirmation messages before any item is transmitted. After that, both peers SHALL display a 64-bit fingerprint of the session transcript. Items SHALL be sent only after the user confirms the fingerprint in the desktop app. That confirmation MUST NOT be possible through the MCP interface.

#### Scenario: Confirmed transfer
- **WHEN** both users see the same fingerprint and confirm it
- **THEN** the items are transferred

### Requirement: Bounded network waits

Every share socket SHALL have read and write timeouts of 30 seconds. The handshake SHALL complete within 60 seconds of the TCP connection. After 3 failed pairing attempts, the session SHALL abort. A silent or slow peer MUST NOT keep a session, a thread, or a listener alive past these bounds.

#### Scenario: Silent connection
- **WHEN** a host connects to the listener and sends nothing
- **THEN** the connection is dropped after at most 30 seconds, and the session keeps waiting for a legitimate peer

### Requirement: Exclusive, cancellable sessions

At most one share session SHALL exist at a time. Starting a session while another one is active SHALL be rejected with an error. Cancelling a session SHALL close its listener, unregister its discovery advertisement, and stop all of its background work. Background work belonging to an ended session MUST NOT read or change the state of any later session.

#### Scenario: Second listen while active
- **WHEN** the user starts a listen session while another one is awaiting fingerprint confirmation
- **THEN** the new request is rejected, and the existing session is unaffected

#### Scenario: Stale task after cancel
- **WHEN** session A is cancelled and session B is started and confirmed
- **THEN** nothing from session A is sent to any peer

### Requirement: Locking ends sharing

Locking the vault MUST cancel any active share session and discard its copy of the vault key. Closing a share or receive dialog SHALL cancel its session.

#### Scenario: Auto-lock during pairing
- **WHEN** the vault auto-locks while a session is listening
- **THEN** the listener is closed, the discovery advertisement is removed, and no vault key copy remains in the session

### Requirement: Protocol version check

Peers SHALL exchange a protocol version first. A mismatch SHALL end the session with a message asking the user to update the other device.

#### Scenario: Old peer
- **WHEN** a v1.0.6 peer connects to a newer listener
- **THEN** the session ends with an "update the other device" message, and no items are sent
