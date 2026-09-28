## 1. Protocol

- [ ] 1.1 Add the `spake2` dependency, and implement the PAKE, HKDF, key confirmation and 64-bit fingerprint in `share/crypto.rs`. Verify with unit tests: same code → same key and fingerprint; wrong code → confirmation failure; the transcript fingerprint differs across sessions.
- [ ] 1.2 Add the protocol version message and a mismatch error. Verify with a unit test using an in-process socket pair.
- [ ] 1.3 Replace the x25519 handshake in `share/lan.rs` with the PAKE handshake; set 30 s socket timeouts and a 60 s handshake deadline. Verify with tests: a silent peer is dropped (short test timeouts through config); a wrong-code peer fails without sending items.

## 2. Discovery and sessions

- [ ] 2.1 Change the mDNS TXT to `{sid, v}`; the receiver tries at most 5 services in turn. Verify with a unit test that the TXT contains no code-derived value.
- [ ] 2.2 Add a session `id`, reject starts while a session is active (`SessionActive`), make tasks exit on id mismatch or cancel, poll the listener non-blocking, and allow at most 3 failed attempts. Verify with tests: second start rejected; a stale task sends nothing after cancel and a new session; the attempt limit aborts the session.

## 3. Lock integration

- [ ] 3.1 Hold the session key in `Zeroizing`; `lock_vault` calls `share::cancel_all`. Verify with a test: lock → the session is Cancelled, the listener port is closed (connect refused), and the key field is dropped.
- [ ] 3.2 Make `ShareModal`/`ReceiveModal` call `share_cancel` on unmount, and display the new fingerprint format. Verify with `pnpm build` and a manual check (closing the modal frees the port).

## 4. Docs and verification

- [ ] 4.1 Update the LAN sharing section in `docs/index.html` (pairing, fingerprint format, compatibility note). Verify by review.
- [ ] 4.2 Manual two-machine test (Windows and Linux): successful transfer; wrong code; cancel mid-handshake; auto-lock during listen. Record the results.
- [ ] 4.3 Run `cargo clippy --all-targets && cargo test`. All pass.
