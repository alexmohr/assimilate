// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The one encrypted-at-rest shape every stored channel secret shares.

use std::fmt;

use shared::crypto::CryptoError;

use super::NotificationError;

/// A channel secret as stored: AES-256-GCM `nonce || ciphertext` under the server's encryption
/// key, the same scheme as a repository passphrase. Only the notification senders decrypt it,
/// right before they use it.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredSecret(Vec<u8>);

impl fmt::Debug for StoredSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StoredSecret([REDACTED])")
    }
}

impl From<Vec<u8>> for StoredSecret {
    fn from(stored: Vec<u8>) -> Self {
        Self(stored)
    }
}

impl StoredSecret {
    /// Encrypts a secret an admin just entered, ready to be stored.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::EncryptionFailed`] if AES-256-GCM encryption fails.
    pub fn encrypt(plaintext: &str, key: &[u8; 32]) -> Result<Self, CryptoError> {
        shared::crypto::encrypt_passphrase(plaintext, key).map(Self)
    }

    /// The stored `nonce || ciphertext` bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub(super) fn decrypt(&self, key: &[u8; 32]) -> Result<String, NotificationError> {
        Ok(shared::crypto::decrypt_passphrase(&self.0, key)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> [u8; 32] {
        shared::crypto::derive_key(b"stored-secret-test-key").unwrap()
    }

    #[test]
    fn round_trips_without_storing_the_plaintext() {
        let secret = StoredSecret::encrypt("hunter2", &key()).unwrap();
        assert!(!secret.as_bytes().windows(7).any(|w| w == b"hunter2"));
        assert_eq!(secret.decrypt(&key()).unwrap(), "hunter2");
    }

    #[test]
    fn debug_output_is_redacted() {
        let secret = StoredSecret::encrypt("hunter2", &key()).unwrap();
        assert_eq!(format!("{secret:?}"), "StoredSecret([REDACTED])");
    }
}
