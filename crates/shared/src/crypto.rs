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

/// Errors that can occur while deriving keys or encrypting/decrypting
/// passphrases with AES-256-GCM.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// The ciphertext passed to [`decrypt_passphrase`] was shorter than the
    /// nonce prefix or otherwise malformed, so it cannot be decrypted.
    #[error("invalid ciphertext: too short or malformed")]
    InvalidCiphertext,
    /// AES-256-GCM encryption of the plaintext failed.
    #[error("encryption failed")]
    EncryptionFailed,
    /// AES-256-GCM decryption failed, typically because the wrong key was
    /// used or the ciphertext was tampered with.
    #[error("decryption failed")]
    DecryptionFailed,
    /// Decryption succeeded but the resulting bytes are not valid UTF-8, so
    /// they cannot be returned as a `String`.
    #[error("decrypted bytes are not valid UTF-8")]
    InvalidUtf8,
    /// HKDF key expansion failed while deriving the encryption key.
    #[error("key derivation failed")]
    KeyDerivationFailed,
}

/// Derives a 256-bit AES key from `secret` using HKDF-SHA256, so callers can
/// turn a passphrase or other secret material into a fixed-size encryption
/// key without ever storing the raw secret as the key itself.
///
/// # Errors
///
/// Returns [`CryptoError::KeyDerivationFailed`] if HKDF expansion fails.
pub fn derive_key(secret: &[u8]) -> Result<[u8; 32], CryptoError> {
    let hkdf = Hkdf::<Sha256>::new(None, secret);
    let mut key = [0u8; 32];
    hkdf.expand(HKDF_INFO, &mut key)
        .map_err(|_| CryptoError::KeyDerivationFailed)?;
    Ok(key)
}

/// Encrypts `plaintext` with AES-256-GCM under `key`, using a freshly
/// generated random nonce, and returns the nonce concatenated with the
/// ciphertext so it can be stored as a single opaque blob.
///
/// # Errors
///
/// Returns [`CryptoError::EncryptionFailed`] if AES-256-GCM encryption fails.
pub fn encrypt_passphrase(plaintext: &str, key: &[u8; 32]) -> Result<Vec<u8>, CryptoError> {
    let cipher = Aes256Gcm::new(key.into());
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|_| CryptoError::EncryptionFailed)?;

    let mut output = Vec::with_capacity(NONCE_SIZE.saturating_add(ciphertext.len()));
    output.extend_from_slice(&nonce);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

/// Decrypts a blob previously produced by [`encrypt_passphrase`], splitting
/// off the leading nonce before running AES-256-GCM decryption under `key`.
///
/// # Errors
///
/// Returns [`CryptoError::InvalidCiphertext`] if `data` is shorter than the nonce,
/// [`CryptoError::DecryptionFailed`] if AES-256-GCM decryption fails (e.g. wrong
/// key or tampered ciphertext), or [`CryptoError::InvalidUtf8`] if the decrypted
/// bytes are not valid UTF-8.
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
        let key = derive_key(b"test-secret-key").unwrap();
        let plaintext = "test-passphrase";

        let encrypted = encrypt_passphrase(plaintext, &key).unwrap();
        let decrypted = decrypt_passphrase(&encrypted, &key).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn different_nonces_each_call() {
        let key = derive_key(b"test-secret-key").unwrap();
        let plaintext = "same-passphrase";

        let encrypted1 = encrypt_passphrase(plaintext, &key).unwrap();
        let encrypted2 = encrypt_passphrase(plaintext, &key).unwrap();

        assert_ne!(encrypted1, encrypted2);
    }

    #[test]
    fn wrong_key_fails() {
        let key1 = derive_key(b"correct-key").unwrap();
        let key2 = derive_key(b"wrong-key").unwrap();
        let plaintext = "secret-passphrase";

        let encrypted = encrypt_passphrase(plaintext, &key1).unwrap();
        let result = decrypt_passphrase(&encrypted, &key2);

        assert!(matches!(result, Err(CryptoError::DecryptionFailed)));
    }

    #[test]
    fn invalid_ciphertext_too_short() {
        let key = derive_key(b"any-key").unwrap();
        let result = decrypt_passphrase(&[0u8; 5], &key);

        assert!(matches!(result, Err(CryptoError::InvalidCiphertext)));
    }
}
