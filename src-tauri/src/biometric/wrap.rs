//! Wrapping of the vault key under a key derived from a Hello signature.
//!
//! Stored blob (`settings.biometric_blob`):
//! `{"v":2,"challenge":hex(32 random bytes),"wrapped":hex(nonce||ct)}`.
//! `wrap_key = HKDF-SHA256(ikm = signature, salt = challenge,
//! info = "cryptenv-bio-v2")` and `wrapped = AES-256-GCM(wrap_key, vault_key)`.
//! Nothing stored allows recovering the key, or the master password, without a
//! fresh Hello signature over `challenge`.

use hkdf::Hkdf;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use super::{HelloError, HelloSigner};
use crate::crypto::{self, VaultKey};

const VERSION: u32 = 2;
const CHALLENGE_LEN: usize = 32;
const HKDF_INFO: &[u8] = b"cryptenv-bio-v2";

#[derive(Serialize, Deserialize)]
pub struct Enrollment {
    v: u32,
    challenge: String,
    wrapped: String,
}

impl Enrollment {
    pub fn to_json(&self) -> Result<String, HelloError> {
        serde_json::to_string(self).map_err(|_| HelloError::Unusable)
    }

    /// `None` for anything that is not a well-formed v2 blob (including the
    /// legacy hex DPAPI blob).
    pub fn parse(blob: &str) -> Option<Self> {
        let e: Enrollment = serde_json::from_str(blob).ok()?;
        let challenge_ok = crypto::hex_decode(&e.challenge)
            .map(|c| c.len() == CHALLENGE_LEN)
            .unwrap_or(false);
        (e.v == VERSION && challenge_ok && crypto::hex_decode(&e.wrapped).is_ok()).then_some(e)
    }
}

fn derive_wrap_key(signature: &[u8], challenge: &[u8]) -> Result<VaultKey, HelloError> {
    let mut key = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(Some(challenge), signature)
        .expand(HKDF_INFO, key.as_mut_slice())
        .map_err(|_| HelloError::Unusable)?;
    Ok(VaultKey::new(key))
}

/// Wraps `vault_key` under a key derived from `signature` over `challenge`.
fn wrap(
    signature: &[u8],
    challenge: &[u8],
    vault_key: &VaultKey,
) -> Result<Enrollment, HelloError> {
    let wrap_key = derive_wrap_key(signature, challenge)?;
    let wrapped = crypto::encrypt(&wrap_key, vault_key.expose()).map_err(|_| HelloError::Unusable)?;
    Ok(Enrollment { v: VERSION, challenge: crypto::hex_encode(challenge), wrapped })
}

/// Inverse of [`wrap`]; fails for any other signature.
fn unwrap(signature: &[u8], enrollment: &Enrollment) -> Result<VaultKey, HelloError> {
    let challenge = crypto::hex_decode(&enrollment.challenge).map_err(|_| HelloError::Unusable)?;
    let wrap_key = derive_wrap_key(signature, &challenge)?;
    let plain = crypto::decrypt(&wrap_key, &enrollment.wrapped).map_err(|_| HelloError::Unusable)?;
    VaultKey::from_slice(&plain).map_err(|_| HelloError::Unusable)
}

/// Creates the Hello credential, checks that its signature is deterministic
/// and returns the blob JSON to store. Blocks (Hello prompts). On any failure
/// the credential is deleted again so nothing is left half-enrolled.
pub fn enroll(signer: &dyn HelloSigner, vault_key: &VaultKey) -> Result<String, HelloError> {
    signer.create()?;
    let result = enroll_with_credential(signer, vault_key);
    if result.is_err() {
        let _ = signer.delete();
    }
    result
}

fn enroll_with_credential(
    signer: &dyn HelloSigner,
    vault_key: &VaultKey,
) -> Result<String, HelloError> {
    let mut challenge = [0u8; CHALLENGE_LEN];
    rand::thread_rng().fill_bytes(&mut challenge);

    let sig1 = signer.sign(&challenge)?;
    let sig2 = signer.sign(&challenge)?;
    // A scheme with randomized signatures could never be unwrapped again.
    if !bool::from(sig1.as_slice().ct_eq(sig2.as_slice())) {
        return Err(HelloError::NotDeterministic);
    }
    wrap(&sig1, &challenge, vault_key)?.to_json()
}

/// Asks Hello to sign the stored challenge and unwraps the vault key, then
/// verifies it against the vault's verify token. Blocks (Hello prompt).
pub fn unlock_key(
    signer: &dyn HelloSigner,
    enrollment: &Enrollment,
    verify_token: &str,
) -> Result<VaultKey, HelloError> {
    let challenge = crypto::hex_decode(&enrollment.challenge).map_err(|_| HelloError::Unusable)?;
    let signature = signer.sign(&challenge)?;
    let key = unwrap(&signature, enrollment)?;
    crypto::decrypt(&key, verify_token).map_err(|_| HelloError::Unusable)?;
    Ok(key)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Fake signer: deterministic signature derived from a secret + challenge;
    /// optionally randomized or cancelling. Counts deletes.
    pub struct FakeHello {
        pub secret: Mutex<u8>,
        pub randomized: bool,
        pub cancel: bool,
        pub deletes: Mutex<u32>,
        counter: Mutex<u8>,
    }

    impl FakeHello {
        pub fn new() -> Self {
            FakeHello {
                secret: Mutex::new(7),
                randomized: false,
                cancel: false,
                deletes: Mutex::new(0),
                counter: Mutex::new(0),
            }
        }
    }

    impl HelloSigner for FakeHello {
        fn create(&self) -> Result<(), HelloError> {
            if self.cancel {
                return Err(HelloError::Cancelled);
            }
            Ok(())
        }
        fn sign(&self, challenge: &[u8]) -> Result<Zeroizing<Vec<u8>>, HelloError> {
            if self.cancel {
                return Err(HelloError::Cancelled);
            }
            let secret = *self.secret.lock().unwrap();
            let mut c = self.counter.lock().unwrap();
            *c = c.wrapping_add(1);
            let salt = if self.randomized { *c } else { 0 };
            let mut sig: Vec<u8> = challenge.iter().map(|b| b ^ secret).collect();
            sig.push(salt);
            Ok(Zeroizing::new(sig))
        }
        fn delete(&self) -> Result<(), HelloError> {
            *self.deletes.lock().unwrap() += 1;
            Ok(())
        }
    }

    fn test_key() -> VaultKey {
        VaultKey::from_slice(&[0x11; 32]).unwrap()
    }

    #[test]
    fn wrap_round_trips() {
        let e = wrap(b"signature-a", &[1u8; 32], &test_key()).unwrap();
        let back = unwrap(b"signature-a", &e).unwrap();
        assert!(back.ct_eq(&test_key()));
    }

    #[test]
    fn different_signature_does_not_unwrap() {
        let e = wrap(b"signature-a", &[1u8; 32], &test_key()).unwrap();
        assert_eq!(unwrap(b"signature-b", &e).unwrap_err(), HelloError::Unusable);
    }

    #[test]
    fn blob_does_not_contain_the_key() {
        let e = wrap(b"signature-a", &[1u8; 32], &test_key()).unwrap();
        let json = e.to_json().unwrap();
        assert!(!json.contains(&crypto::hex_encode(test_key().expose())));
        assert!(Enrollment::parse(&json).is_some());
    }

    #[test]
    fn parse_rejects_legacy_and_malformed_blobs() {
        assert!(Enrollment::parse("0a1b2c3d4e5f").is_none());
        assert!(Enrollment::parse("").is_none());
        assert!(Enrollment::parse(r#"{"v":1,"challenge":"00","wrapped":"00"}"#).is_none());
        assert!(Enrollment::parse(r#"{"v":2,"challenge":"00","wrapped":"00"}"#).is_none());
    }

    #[test]
    fn enroll_then_unlock_key_round_trips() {
        let (_, token, key) = crypto::init_vault_crypto(b"pw-for-wrap-test").unwrap();
        let hello = FakeHello::new();
        let blob = enroll(&hello, &key).unwrap();
        let e = Enrollment::parse(&blob).unwrap();
        assert!(unlock_key(&hello, &e, &token).unwrap().ct_eq(&key));
    }

    #[test]
    fn enroll_refuses_non_deterministic_signatures_and_cleans_up() {
        let mut hello = FakeHello::new();
        hello.randomized = true;
        assert_eq!(enroll(&hello, &test_key()).unwrap_err(), HelloError::NotDeterministic);
        assert_eq!(*hello.deletes.lock().unwrap(), 1);
    }

    #[test]
    fn enroll_cancelled_stores_nothing() {
        let mut hello = FakeHello::new();
        hello.cancel = true;
        assert_eq!(enroll(&hello, &test_key()).unwrap_err(), HelloError::Cancelled);
    }

    #[test]
    fn unlock_key_fails_when_credential_changed_or_token_mismatch() {
        let (_, token, key) = crypto::init_vault_crypto(b"pw-for-wrap-test").unwrap();
        let hello = FakeHello::new();
        let e = Enrollment::parse(&enroll(&hello, &key).unwrap()).unwrap();

        *hello.secret.lock().unwrap() = 9; // credential was re-created
        assert_eq!(unlock_key(&hello, &e, &token).unwrap_err(), HelloError::Unusable);

        *hello.secret.lock().unwrap() = 7;
        let (_, other_token, _) = crypto::init_vault_crypto(b"another-vault-pw").unwrap();
        assert_eq!(unlock_key(&hello, &e, &other_token).unwrap_err(), HelloError::Unusable);
    }

    #[test]
    fn unlock_key_cancel_is_reported() {
        let (_, token, key) = crypto::init_vault_crypto(b"pw-for-wrap-test").unwrap();
        let mut hello = FakeHello::new();
        let e = Enrollment::parse(&enroll(&hello, &key).unwrap()).unwrap();
        hello.cancel = true;
        assert_eq!(unlock_key(&hello, &e, &token).unwrap_err(), HelloError::Cancelled);
    }
}
