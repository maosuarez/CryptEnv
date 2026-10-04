use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;

pub type CryptoKey = [u8; 32];

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
pub fn derive_key(password: &[u8], salt: &[u8; 32]) -> Result<CryptoKey, String> {
    let mut key = [0u8; 32];
    argon2_inst()
        .hash_password_into(password, salt, &mut key)
        .map_err(|e| format!("key derivation failed: {e}"))?;
    Ok(key)
}

/// AES-256-GCM encrypt. Returns hex(nonce || ciphertext_with_tag).
pub fn encrypt(key: &CryptoKey, plaintext: &[u8]) -> Result<String, String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher.encrypt(nonce, plaintext).map_err(|e| e.to_string())?;
    let mut out = nonce_bytes.to_vec();
    out.extend_from_slice(&ct);
    Ok(hex_encode(&out))
}

/// AES-256-GCM decrypt. Expects hex(nonce || ciphertext_with_tag).
pub fn decrypt(key: &CryptoKey, ciphertext: &str) -> Result<Vec<u8>, String> {
    let bytes = hex_decode(ciphertext).map_err(|_| "invalid ciphertext encoding".to_string())?;
    if bytes.len() < 12 {
        return Err("ciphertext too short".into());
    }
    let nonce = Nonce::from_slice(&bytes[..12]);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    cipher
        .decrypt(nonce, &bytes[12..])
        .map_err(|_| "decryption failed".into())
}

/// First-time vault setup. Returns (salt_hex, verify_token_hex, key).
pub fn init_vault_crypto(password: &[u8]) -> Result<(String, String, CryptoKey), String> {
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
) -> Result<CryptoKey, String> {
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

    fn test_key() -> CryptoKey { [0x42u8; 32] }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = test_key();
        let plaintext = b"super secret value";
        let ct = encrypt(&key, plaintext).expect("encrypt should succeed");
        let recovered = decrypt(&key, &ct).expect("decrypt should succeed");
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn decrypt_with_wrong_key_returns_error() {
        let key = test_key();
        let wrong_key = [0xFFu8; 32];
        let ct = encrypt(&key, b"some data").expect("encrypt should succeed");
        assert!(decrypt(&wrong_key, &ct).is_err(), "wrong key must return Err");
    }

    #[test]
    fn encrypt_decrypt_empty_plaintext() {
        let key = test_key();
        let ct = encrypt(&key, b"").expect("encrypt of empty slice should succeed");
        let recovered = decrypt(&key, &ct).expect("decrypt of empty slice should succeed");
        assert_eq!(recovered, b"");
    }

    #[test]
    fn two_encryptions_produce_different_ciphertexts() {
        let key = test_key();
        let ct1 = encrypt(&key, b"same input").expect("first encrypt should succeed");
        let ct2 = encrypt(&key, b"same input").expect("second encrypt should succeed");
        assert_ne!(ct1, ct2, "nonces are random — ciphertexts must differ");
    }
}
