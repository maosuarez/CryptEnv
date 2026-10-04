use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;
use serde::Deserialize;
use zeroize::Zeroizing;

/// A 32-byte AES-256 key (vault key, wrap key, relay key) that is wiped on
/// drop. It is not `Copy`, has no `Deref` to the array and no `Serialize`;
/// `Debug` is redacted. Cloning is allowed (each clone also wipes on drop) so
/// a key can move into `spawn_blocking`. Raw bytes are only reachable via
/// `expose`, which is for key-wrapping code and tests, never for storing
/// copies in plain arrays.
#[derive(Clone)]
pub struct VaultKey(Zeroizing<[u8; 32]>);

impl VaultKey {
    /// Takes ownership of key bytes already held in a zeroizing buffer.
    pub fn new(bytes: Zeroizing<[u8; 32]>) -> Self {
        Self(bytes)
    }

    /// Copies `bytes` into a new key. The caller still owns (and must wipe)
    /// the source slice; used by key-unwrapping code and tests.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, String> {
        let mut buf = Zeroizing::new([0u8; 32]);
        if bytes.len() != buf.len() {
            return Err("invalid key length".into());
        }
        buf.copy_from_slice(bytes);
        Ok(Self(buf))
    }

    /// Constant-time equality.
    pub fn ct_eq(&self, other: &Self) -> bool {
        use subtle::ConstantTimeEq;
        bool::from(self.0.as_slice().ct_eq(other.0.as_slice()))
    }

    /// Borrow the raw key bytes. Do not copy them into a plain array.
    pub(crate) fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for VaultKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VaultKey(<redacted>)")
    }
}

/// A password or passphrase that is wiped on drop. Deserializes transparently
/// from a JSON/Tauri string; no `Serialize`, no `Clone`, redacted `Debug`.
#[derive(Deserialize)]
#[serde(transparent)]
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    pub fn new(s: String) -> Self {
        Self(Zeroizing::new(s))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl From<Zeroizing<String>> for SecretString {
    fn from(s: Zeroizing<String>) -> Self {
        Self(s)
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretString(<redacted>)")
    }
}

const VERIFY_MAGIC: &[u8] = b"vault_ok_v1";

/// Error text of `unlock_vault_crypto` for a wrong password, so callers can
/// tell it apart from a corrupt vault without matching on a literal.
pub const INCORRECT_PASSWORD: &str = "incorrect password";

fn argon2_inst() -> Argon2<'static> {
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(65536, 3, 4, Some(32)).expect("valid params"),
    )
}

/// Derives a 32-byte AES key from password + raw salt (Argon2id).
pub fn derive_key(password: &[u8], salt: &[u8; 32]) -> Result<VaultKey, String> {
    let mut key = Zeroizing::new([0u8; 32]);
    argon2_inst()
        .hash_password_into(password, salt, &mut *key)
        .map_err(|e| format!("key derivation failed: {e}"))?;
    Ok(VaultKey::new(key))
}

/// AES-256-GCM encrypt. Returns hex(nonce || ciphertext_with_tag).
pub fn encrypt(key: &VaultKey, plaintext: &[u8]) -> Result<String, String> {
    let cipher = Aes256Gcm::new_from_slice(key.expose()).map_err(|e| e.to_string())?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher.encrypt(nonce, plaintext).map_err(|e| e.to_string())?;
    let mut out = nonce_bytes.to_vec();
    out.extend_from_slice(&ct);
    Ok(hex_encode(&out))
}

/// AES-256-GCM decrypt. Expects hex(nonce || ciphertext_with_tag).
pub fn decrypt(key: &VaultKey, ciphertext: &str) -> Result<Zeroizing<Vec<u8>>, String> {
    let bytes = hex_decode(ciphertext).map_err(|_| "invalid ciphertext encoding".to_string())?;
    if bytes.len() < 12 {
        return Err("ciphertext too short".into());
    }
    let nonce = Nonce::from_slice(&bytes[..12]);
    let cipher = Aes256Gcm::new_from_slice(key.expose()).map_err(|e| e.to_string())?;
    cipher
        .decrypt(nonce, &bytes[12..])
        .map(Zeroizing::new)
        .map_err(|_| "decryption failed".into())
}

/// First-time vault setup. Returns (salt_hex, verify_token_hex, key).
pub fn init_vault_crypto(password: &[u8]) -> Result<(String, String, VaultKey), String> {
    let mut salt = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut salt);
    let key = derive_key(password, &salt)?;
    let verify_token = encrypt(&key, VERIFY_MAGIC)?;
    Ok((hex_encode(&salt), verify_token, key))
}

/// Unlock: verify password, derive key. Returns key if password correct.
pub fn unlock_vault_crypto(
    password: &[u8],
    salt_hex: &str,
    verify_token: &str,
) -> Result<VaultKey, String> {
    let salt_bytes = hex_decode(salt_hex).map_err(|_| "corrupt salt".to_string())?;
    let salt: [u8; 32] = salt_bytes
        .try_into()
        .map_err(|_| "invalid salt length".to_string())?;
    let key = derive_key(password, &salt)?;
    decrypt(&key, verify_token).map_err(|_| INCORRECT_PASSWORD.to_string())?;
    Ok(key)
}

pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn hex_decode(s: &str) -> Result<Vec<u8>, ()> {
    if s.len() % 2 != 0 {
        return Err(());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| ()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> VaultKey { VaultKey::from_slice(&[0x42u8; 32]).expect("32 bytes") }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = test_key();
        let plaintext = b"super secret value";
        let ct = encrypt(&key, plaintext).expect("encrypt should succeed");
        let recovered = decrypt(&key, &ct).expect("decrypt should succeed");
        assert_eq!(&*recovered, plaintext);
    }

    #[test]
    fn decrypt_with_wrong_key_returns_error() {
        let key = test_key();
        let wrong_key = VaultKey::from_slice(&[0xFFu8; 32]).expect("32 bytes");
        let ct = encrypt(&key, b"some data").expect("encrypt should succeed");
        assert!(decrypt(&wrong_key, &ct).is_err(), "wrong key must return Err");
    }

    #[test]
    fn encrypt_decrypt_empty_plaintext() {
        let key = test_key();
        let ct = encrypt(&key, b"").expect("encrypt of empty slice should succeed");
        let recovered = decrypt(&key, &ct).expect("decrypt of empty slice should succeed");
        assert_eq!(&*recovered, b"");
    }

    #[test]
    fn two_encryptions_produce_different_ciphertexts() {
        let key = test_key();
        let ct1 = encrypt(&key, b"same input").expect("first encrypt should succeed");
        let ct2 = encrypt(&key, b"same input").expect("second encrypt should succeed");
        assert_ne!(ct1, ct2, "nonces are random — ciphertexts must differ");
    }
    // ── VaultKey / SecretString hygiene ──────────────────────────────────────

    /// Compile-time "type does NOT implement trait": the call is ambiguous
    /// (and fails to compile) exactly when `$ty` implements `$tr`.
    macro_rules! assert_not_impl {
        ($ty:ty, $tr:path) => {{
            trait AmbiguousIfImpl<A> {
                fn check() {}
            }
            impl<T: ?Sized> AmbiguousIfImpl<()> for T {}
            #[allow(dead_code)]
            struct Invalid;
            impl<T: ?Sized + $tr> AmbiguousIfImpl<Invalid> for T {}
            let _ = <$ty as AmbiguousIfImpl<_>>::check;
        }};
    }

    #[test]
    fn vault_key_and_secret_string_lack_copy_display_serialize_deref() {
        assert_not_impl!(VaultKey, Copy);
        assert_not_impl!(VaultKey, std::fmt::Display);
        assert_not_impl!(VaultKey, serde::Serialize);
        assert_not_impl!(VaultKey, std::ops::Deref);
        assert_not_impl!(SecretString, Copy);
        assert_not_impl!(SecretString, Clone);
        assert_not_impl!(SecretString, std::fmt::Display);
        assert_not_impl!(SecretString, serde::Serialize);
        assert_not_impl!(SecretString, std::ops::Deref);
    }

    #[test]
    fn debug_output_never_contains_key_or_password_material() {
        let key = VaultKey::from_slice(&[0xABu8; 32]).unwrap();
        let dbg = format!("{key:?} {:#?}", Some(&key));
        assert!(dbg.contains("redacted"));
        assert!(!dbg.to_lowercase().contains("ab, ab") && !dbg.contains("171"));

        let pw = SecretString::new("hunter2-very-secret".to_string());
        let dbg = format!("{pw:?}");
        assert!(dbg.contains("redacted"));
        assert!(!dbg.contains("hunter2"));
    }

    #[test]
    fn vault_key_clone_is_independent() {
        let a = VaultKey::from_slice(&[1u8; 32]).unwrap();
        let b = a.clone();
        assert!(a.ct_eq(&b));
        assert_ne!(a.expose().as_ptr(), b.expose().as_ptr(), "clone owns its own buffer");
        drop(a);
        assert_eq!(b.expose(), &[1u8; 32], "clone survives dropping the original");
        let other = VaultKey::from_slice(&[2u8; 32]).unwrap();
        assert!(!b.ct_eq(&other));
    }

    #[test]
    fn vault_key_rejects_wrong_length() {
        assert!(VaultKey::from_slice(&[0u8; 31]).is_err());
        assert!(VaultKey::from_slice(&[0u8; 33]).is_err());
    }

    #[test]
    fn vault_key_bytes_are_wiped_on_drop() {
        let mut slot = std::mem::ManuallyDrop::new(VaultKey::from_slice(&[0x5Au8; 32]).unwrap());
        let ptr = slot.expose().as_ptr();
        // SAFETY: `slot` is still allocated (ManuallyDrop); dropping in place
        // runs the zeroizing destructor, after which the same storage is read
        // back as plain bytes.
        unsafe {
            std::mem::ManuallyDrop::drop(&mut slot);
            for i in 0..32 {
                assert_eq!(std::ptr::read_volatile(ptr.add(i)), 0, "byte {i} not wiped");
            }
        }
    }

    #[test]
    fn secret_string_deserializes_from_json_string_and_nested() {
        let pw: SecretString = serde_json::from_str("\"pa55\"").unwrap();
        assert_eq!(pw.expose(), "pa55");

        #[derive(serde::Deserialize)]
        struct Body {
            master_password: SecretString,
            current: Option<SecretString>,
        }
        let b: Body = serde_json::from_str(r#"{"master_password":"x1","current":"y2"}"#).unwrap();
        assert_eq!(b.master_password.expose(), "x1");
        assert_eq!(b.current.as_ref().map(|p| p.expose()), Some("y2"));
    }

    #[test]
    fn derived_key_unlocks_and_decrypt_returns_wiping_buffer() {
        let (_, token, key) = init_vault_crypto(b"pw").unwrap();
        let plain = decrypt(&key, &token).unwrap();
        let _: &Zeroizing<Vec<u8>> = &plain;
        assert_eq!(&**plain, VERIFY_MAGIC);
    }
}
