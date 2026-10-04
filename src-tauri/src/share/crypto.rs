use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};
use zeroize::Zeroizing;

use super::ShareError;

type HmacSha256 = Hmac<Sha256>;

// ─── LAN pairing: SPAKE2 + key confirmation ───────────────────────────────────

/// Protocol identity, also used as the SPAKE2 identity and transcript prefix.
const LAN_PROTOCOL_ID: &[u8] = b"cryptenv-lan-v2";
const SESSION_KEY_INFO: &[u8] = b"cryptenv-lan-v2 session";
const CONFIRM_KEY_INFO: &[u8] = b"cryptenv-lan-v2 confirm";

/// Which end of the LAN pairing we are. Labels the key-confirmation MACs so a
/// reflected MAC never verifies, and orders the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairRole {
    Sender,
    Receiver,
}

impl PairRole {
    fn label(self) -> &'static [u8] {
        match self {
            PairRole::Sender => b"A",
            PairRole::Receiver => b"B",
        }
    }

    fn peer(self) -> PairRole {
        match self {
            PairRole::Sender => PairRole::Receiver,
            PairRole::Receiver => PairRole::Sender,
        }
    }
}

/// First half of the SPAKE2 exchange: holds our state and the message to send.
pub struct PakeStart {
    state: Spake2<Ed25519Group>,
    our_msg: Vec<u8>,
    role: PairRole,
}

/// Result of a completed SPAKE2 exchange. The keys are only trustworthy after
/// `verify_peer_confirmation` succeeds.
pub struct PakeKeys {
    pub session_key: Zeroizing<[u8; 32]>,
    confirm_key: Zeroizing<[u8; 32]>,
    transcript: Vec<u8>,
    role: PairRole,
}

impl PakeStart {
    /// Begin the exchange keyed by the pairing code (symmetric SPAKE2, Ed25519 group).
    pub fn new(pairing_code: &str, role: PairRole) -> Self {
        let (state, our_msg) = Spake2::<Ed25519Group>::start_symmetric(
            &Password::new(pairing_code.as_bytes()),
            &Identity::new(LAN_PROTOCOL_ID),
        );
        PakeStart { state, our_msg, role }
    }

    pub fn message(&self) -> &[u8] {
        &self.our_msg
    }

    /// Combine with the peer's SPAKE2 message. A wrong pairing code does not
    /// fail here; it yields different keys and is caught by key confirmation.
    pub fn finish(self, peer_msg: &[u8]) -> Result<PakeKeys, ShareError> {
        // Symmetric mode would accept our own message reflected back.
        if peer_msg == self.our_msg.as_slice() {
            return Err(ShareError::Protocol("reflected pairing message".into()));
        }
        let (sender_msg, receiver_msg) = match self.role {
            PairRole::Sender => (self.our_msg.as_slice(), peer_msg),
            PairRole::Receiver => (peer_msg, self.our_msg.as_slice()),
        };
        let mut transcript =
            Vec::with_capacity(LAN_PROTOCOL_ID.len() + sender_msg.len() + receiver_msg.len());
        transcript.extend_from_slice(LAN_PROTOCOL_ID);
        transcript.extend_from_slice(sender_msg);
        transcript.extend_from_slice(receiver_msg);

        let role = self.role;
        let shared = Zeroizing::new(
            self.state
                .finish(peer_msg)
                .map_err(|_| ShareError::Protocol("invalid pairing message".into()))?,
        );
        let hk = Hkdf::<Sha256>::new(None, &shared);
        let mut session = [0u8; 32];
        let mut confirm = [0u8; 32];
        hk.expand(SESSION_KEY_INFO, &mut session)
            .map_err(|_| ShareError::Crypto("HKDF expand failed".into()))?;
        hk.expand(CONFIRM_KEY_INFO, &mut confirm)
            .map_err(|_| ShareError::Crypto("HKDF expand failed".into()))?;
        Ok(PakeKeys {
            session_key: Zeroizing::new(session),
            confirm_key: Zeroizing::new(confirm),
            transcript,
            role,
        })
    }
}

impl PakeKeys {
    fn mac_for(&self, role: PairRole) -> Result<HmacSha256, ShareError> {
        let mut mac = <HmacSha256 as Mac>::new_from_slice(&*self.confirm_key)
            .map_err(|_| ShareError::Crypto("invalid confirmation key".into()))?;
        mac.update(role.label());
        mac.update(&self.transcript);
        Ok(mac)
    }

    /// Our key-confirmation MAC, to send to the peer.
    pub fn our_confirmation(&self) -> Result<Vec<u8>, ShareError> {
        Ok(self.mac_for(self.role)?.finalize().into_bytes().to_vec())
    }

    /// Verify the peer's key-confirmation MAC (constant time). Failure means
    /// the peer does not know the pairing code (or the transcript was altered).
    pub fn verify_peer_confirmation(&self, mac: &[u8]) -> Result<(), ShareError> {
        self.mac_for(self.role.peer())?
            .verify_slice(mac)
            .map_err(|_| ShareError::PairingFailed)
    }

    /// 64-bit transcript fingerprint, formatted `XXXX-XXXX-XXXX-XXXX`.
    pub fn fingerprint(&self) -> String {
        use sha2::Digest;
        let digest = Sha256::digest(&self.transcript);
        let hex: String = digest[..8].iter().map(|b| format!("{:02X}", b)).collect();
        hex.as_bytes()
            .chunks(4)
            .map(|c| String::from_utf8_lossy(c).into_owned())
            .collect::<Vec<_>>()
            .join("-")
    }
}

/// AES-256-GCM encrypt. Prepends 12-byte random nonce to ciphertext.
pub fn encrypt_message(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new_from_slice(key).expect("32-byte key is always valid");
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let mut ct = cipher
        .encrypt(nonce, plaintext)
        .expect("AES-GCM encrypt should not fail with valid inputs");
    let mut out = nonce_bytes.to_vec();
    out.append(&mut ct);
    out
}

/// AES-256-GCM decrypt. Expects nonce (12 bytes) prepended to ciphertext.
pub fn decrypt_message(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, ShareError> {
    if data.len() < 12 {
        return Err(ShareError::Protocol("message too short to contain nonce".into()));
    }
    let nonce = Nonce::from_slice(&data[..12]);
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| ShareError::Crypto(e.to_string()))?;
    cipher
        .decrypt(nonce, &data[12..])
        .map_err(|_| ShareError::Crypto("decryption failed — message may be tampered".into()))
}

/// Derive a package encryption key from a passphrase using Argon2id.
/// Uses lighter parameters than the vault KDF because the passphrase is a
/// random 12-char string (high entropy), so lower work factors are acceptable.
pub fn derive_package_key(passphrase: &str, salt: &[u8; 32]) -> Zeroizing<[u8; 32]> {
    let params = Params::new(32768, 2, 2, Some(32)).expect("valid Argon2 params");
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .expect("Argon2id key derivation failed");
    Zeroizing::new(key)
}

/// Generate a 12-character random passphrase from [a-zA-Z0-9].
pub fn generate_passphrase() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::thread_rng();
    let mut out = String::with_capacity(12);
    let mut buf = [0u8; 12];
    rng.fill_bytes(&mut buf);
    for byte in buf {
        out.push(ALPHABET[(byte as usize) % ALPHABET.len()] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs both sides in-process; returns (sender keys, receiver keys).
    fn pair(sender_code: &str, receiver_code: &str) -> (PakeKeys, PakeKeys) {
        let s = PakeStart::new(sender_code, PairRole::Sender);
        let r = PakeStart::new(receiver_code, PairRole::Receiver);
        let (s_msg, r_msg) = (s.message().to_vec(), r.message().to_vec());
        (s.finish(&r_msg).unwrap(), r.finish(&s_msg).unwrap())
    }

    #[test]
    fn same_code_gives_same_key_fingerprint_and_valid_confirmations() {
        let (s, r) = pair("123456", "123456");
        assert_eq!(*s.session_key, *r.session_key);
        assert_eq!(s.fingerprint(), r.fingerprint());
        r.verify_peer_confirmation(&s.our_confirmation().unwrap()).unwrap();
        s.verify_peer_confirmation(&r.our_confirmation().unwrap()).unwrap();
    }

    #[test]
    fn wrong_code_fails_confirmation_both_ways() {
        let (s, r) = pair("123456", "654321");
        assert_ne!(*s.session_key, *r.session_key);
        assert!(matches!(
            r.verify_peer_confirmation(&s.our_confirmation().unwrap()),
            Err(ShareError::PairingFailed)
        ));
        assert!(matches!(
            s.verify_peer_confirmation(&r.our_confirmation().unwrap()),
            Err(ShareError::PairingFailed)
        ));
    }

    #[test]
    fn fingerprint_differs_across_sessions_and_has_64_bit_format() {
        let (a, _) = pair("123456", "123456");
        let (b, _) = pair("123456", "123456");
        assert_ne!(a.fingerprint(), b.fingerprint());
        let fp = a.fingerprint();
        let groups: Vec<&str> = fp.split('-').collect();
        assert_eq!(groups.len(), 4);
        assert!(groups
            .iter()
            .all(|g| g.len() == 4 && g.chars().all(|c| c.is_ascii_hexdigit())));
    }

    #[test]
    fn own_confirmation_is_not_accepted_as_peer_confirmation() {
        let (s, _) = pair("123456", "123456");
        assert!(s.verify_peer_confirmation(&s.our_confirmation().unwrap()).is_err());
    }

    #[test]
    fn reflected_pake_message_is_rejected() {
        let s = PakeStart::new("123456", PairRole::Sender);
        let own = s.message().to_vec();
        assert!(s.finish(&own).is_err());
    }
}
