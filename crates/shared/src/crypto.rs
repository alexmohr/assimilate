// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use aes_gcm::{
    AeadCore, Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, OsRng},
};
use hkdf::Hkdf;
use sha2::Sha256;

const NONCE_SIZE: usize = 12;
const HKDF_INFO: &[u8] = b"borg-backup-server-passphrase-key";

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("invalid ciphertext: too short or malformed")]
    InvalidCiphertext,
    #[error("decryption failed")]
    DecryptionFailed,
    #[error("decrypted bytes are not valid UTF-8")]
    InvalidUtf8,
}

pub fn derive_key(secret: &[u8]) -> [u8; 32] {
    let hkdf = Hkdf::<Sha256>::new(None, secret);
    let mut key = [0u8; 32];
    hkdf.expand(HKDF_INFO, &mut key)
        .expect("HKDF-SHA256 expand to 32 bytes should never fail");
    key
}

pub fn encrypt_passphrase(plaintext: &str, key: &[u8; 32]) -> Result<Vec<u8>, CryptoError> {
    let cipher = Aes256Gcm::new(key.into());
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|_| CryptoError::DecryptionFailed)?;

    let mut output = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
    output.extend_from_slice(&nonce);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

pub fn decrypt_passphrase(data: &[u8], key: &[u8; 32]) -> Result<String, CryptoError> {
    if data.len() <= NONCE_SIZE {
        return Err(CryptoError::InvalidCiphertext);
    }

    let (nonce_bytes, ciphertext) = data.split_at(NONCE_SIZE);
    let nonce = Nonce::from_slice(nonce_bytes);
    let cipher = Aes256Gcm::new(key.into());

    let plaintext_bytes = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| CryptoError::DecryptionFailed)?;

    String::from_utf8(plaintext_bytes).map_err(|_| CryptoError::InvalidUtf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_then_decrypt_roundtrip() {
        let key = derive_key(b"test-secret-key");
        let plaintext = "test-passphrase";

        let encrypted = encrypt_passphrase(plaintext, &key).unwrap();
        let decrypted = decrypt_passphrase(&encrypted, &key).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn different_nonces_each_call() {
        let key = derive_key(b"test-secret-key");
        let plaintext = "same-passphrase";

        let encrypted1 = encrypt_passphrase(plaintext, &key).unwrap();
        let encrypted2 = encrypt_passphrase(plaintext, &key).unwrap();

        assert_ne!(encrypted1, encrypted2);
    }

    #[test]
    fn wrong_key_fails() {
        let key1 = derive_key(b"correct-key");
        let key2 = derive_key(b"wrong-key");
        let plaintext = "secret-passphrase";

        let encrypted = encrypt_passphrase(plaintext, &key1).unwrap();
        let result = decrypt_passphrase(&encrypted, &key2);

        assert!(matches!(result, Err(CryptoError::DecryptionFailed)));
    }

    #[test]
    fn invalid_ciphertext_too_short() {
        let key = derive_key(b"any-key");
        let result = decrypt_passphrase(&[0u8; 5], &key);

        assert!(matches!(result, Err(CryptoError::InvalidCiphertext)));
    }
}
