// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{fmt, str::FromStr};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hkdf::Hkdf;
use sha2::{Digest as _, Sha256};
use snow::{
    params::DHChoice,
    resolvers::{CryptoResolver as _, DefaultResolver},
};

use super::ChannelError;

/// Length of an X25519 key.
pub const KEY_LEN: usize = 32;

const HKDF_INFO: &[u8] = b"assimilate-agent-channel-static-key-v1";

/// The server's static X25519 private key. Derived from the server's secret
/// key, so it is stable across restarts without being stored anywhere.
pub struct ChannelPrivateKey([u8; KEY_LEN]);

impl ChannelPrivateKey {
    /// Derives the key from the server's secret (`ASSIMILATE_SECRET_KEY`)
    /// with HKDF-SHA256 under a label of its own, so it shares no key
    /// material with the passphrase encryption key derived from the same
    /// secret.
    ///
    /// # Errors
    ///
    /// Returns [`ChannelError::KeyDerivation`] if HKDF expansion fails.
    pub fn derive(secret: &[u8]) -> Result<Self, ChannelError> {
        let mut key = [0u8; KEY_LEN];
        Hkdf::<Sha256>::new(None, secret)
            .expand(HKDF_INFO, &mut key)
            .map_err(|_| ChannelError::KeyDerivation)?;
        Ok(Self(key))
    }

    /// The matching public key, which agents pin.
    ///
    /// # Errors
    ///
    /// Returns [`ChannelError::KeyDerivation`] if the X25519 implementation
    /// is unavailable or returns a key of the wrong length.
    pub fn public_key(&self) -> Result<ChannelPublicKey, ChannelError> {
        let mut dh = DefaultResolver
            .resolve_dh(&DHChoice::Curve25519)
            .ok_or(ChannelError::KeyDerivation)?;
        dh.set(&self.0);
        let public: [u8; KEY_LEN] = dh
            .pubkey()
            .try_into()
            .map_err(|_| ChannelError::KeyDerivation)?;
        Ok(ChannelPublicKey(public))
    }

    pub(super) fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl fmt::Debug for ChannelPrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChannelPrivateKey([REDACTED])")
    }
}

/// The server's static X25519 public key, written as 44 characters of
/// standard base64. Agents are configured with it and refuse a server that
/// cannot prove it holds the matching private key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelPublicKey([u8; KEY_LEN]);

impl ChannelPublicKey {
    /// A short fingerprint for display: the first 16 bytes of the key's
    /// SHA-256, as unpadded base64 behind `SHA256:`, the way OpenSSH shows
    /// its key fingerprints.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        let digest = Sha256::digest(self.0);
        let short = digest.get(..16).unwrap_or_default();
        format!(
            "SHA256:{}",
            base64::engine::general_purpose::STANDARD_NO_PAD.encode(short)
        )
    }

    pub(super) fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl fmt::Display for ChannelPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&STANDARD.encode(self.0))
    }
}

impl FromStr for ChannelPublicKey {
    type Err = ChannelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = STANDARD
            .decode(s.trim())
            .map_err(|_| ChannelError::InvalidPublicKey)?;
        let key: [u8; KEY_LEN] = bytes
            .try_into()
            .map_err(|_| ChannelError::InvalidPublicKey)?;
        Ok(Self(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_secret_derives_the_same_key() {
        let a = ChannelPrivateKey::derive(b"secret").unwrap();
        let b = ChannelPrivateKey::derive(b"secret").unwrap();
        assert_eq!(a.public_key().unwrap(), b.public_key().unwrap());
    }

    #[test]
    fn another_secret_derives_another_key() {
        let a = ChannelPrivateKey::derive(b"secret").unwrap();
        let b = ChannelPrivateKey::derive(b"other secret").unwrap();
        assert_ne!(a.public_key().unwrap(), b.public_key().unwrap());
    }

    #[test]
    fn the_channel_key_is_not_the_passphrase_key() {
        let channel = ChannelPrivateKey::derive(b"secret").unwrap();
        let passphrase = crate::crypto::derive_key(b"secret").unwrap();
        assert_ne!(channel.as_bytes(), &passphrase);
    }

    #[test]
    fn a_public_key_round_trips_through_its_text_form() {
        let key = ChannelPrivateKey::derive(b"secret")
            .unwrap()
            .public_key()
            .unwrap();
        let text = key.to_string();
        assert_eq!(text.len(), 44);
        assert_eq!(text.parse::<ChannelPublicKey>().unwrap(), key);
        assert_eq!(
            format!(" {text}\n").parse::<ChannelPublicKey>().unwrap(),
            key
        );
    }

    #[test]
    fn malformed_public_keys_are_rejected() {
        assert!("not base64!".parse::<ChannelPublicKey>().is_err());
        assert!(
            STANDARD
                .encode([1u8; 31])
                .parse::<ChannelPublicKey>()
                .is_err()
        );
        assert!("".parse::<ChannelPublicKey>().is_err());
    }

    #[test]
    fn the_fingerprint_is_short_and_stable() {
        let key = ChannelPrivateKey::derive(b"secret")
            .unwrap()
            .public_key()
            .unwrap();
        let fingerprint = key.fingerprint();
        assert!(fingerprint.starts_with("SHA256:"));
        assert_eq!(fingerprint.len(), "SHA256:".len() + 22);
        assert_eq!(fingerprint, key.fingerprint());
    }

    #[test]
    fn the_private_key_is_redacted_in_debug_output() {
        let key = ChannelPrivateKey::derive(b"secret").unwrap();
        assert_eq!(format!("{key:?}"), "ChannelPrivateKey([REDACTED])");
    }
}
