// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::fmt::{self, Write as _};

use rand::{RngCore, rngs::OsRng};

/// The keychain service every desktop secret is filed under.
const KEYCHAIN_SERVICE: &str = "assimilate-desktop";

/// Random bytes behind every generated secret; 32 matches the agent tokens.
const SECRET_BYTES: usize = 32;

/// The secrets the desktop app generates once and keeps for good.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::Display)]
#[cfg_attr(test, derive(strum_macros::EnumIter))]
#[strum(serialize_all = "kebab-case")]
pub enum SecretName {
    /// `ASSIMILATE_SECRET_KEY`: derives the key repository passphrases are
    /// encrypted with. Losing it makes every stored passphrase unreadable.
    ServerSecretKey,
    /// The generated password of the server's built-in `admin` user.
    AdminPassword,
    /// The embedded `PostgreSQL` superuser's password.
    PostgresPassword,
}

/// A secret value. `Debug` and `Display` never print it; read it with
/// [`Secret::expose`] at the one place it has to leave the process.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Generates a fresh secret: 32 bytes from the OS RNG, hex-encoded.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0u8; SECRET_BYTES];
        OsRng.fill_bytes(&mut bytes);
        let hex = bytes
            .iter()
            .fold(String::with_capacity(SECRET_BYTES * 2), |mut hex, byte| {
                // Writing to a String can't fail.
                let _ = write!(hex, "{byte:02x}");
                hex
            });
        Self(hex)
    }

    /// Wraps a value read back from a store.
    #[must_use]
    pub fn from_stored(value: String) -> Self {
        Self(value)
    }

    /// The plaintext, for handing to a child process or an HTTP request.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

/// Why a secret couldn't be read or written.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// The OS keychain refused or failed the request.
    #[error("keychain error for {name}: {source}")]
    Keychain {
        /// Which secret was being accessed.
        name: SecretName,
        /// What the keychain reported.
        source: keyring::Error,
    },
}

/// Somewhere secrets persist between runs.
///
/// Implementations may block (the OS keychain does), so async callers should
/// call them from `tokio::task::spawn_blocking`.
pub trait SecretStore {
    /// Reads a secret, or `None` if it was never stored.
    ///
    /// # Errors
    ///
    /// Fails if the store can't be read.
    fn get(&self, name: SecretName) -> Result<Option<Secret>, SecretError>;

    /// Stores a secret, replacing any earlier value.
    ///
    /// # Errors
    ///
    /// Fails if the store can't be written.
    fn set(&self, name: SecretName, secret: &Secret) -> Result<(), SecretError>;
}

/// Returns the stored secret, generating and storing one on first use.
///
/// # Errors
///
/// Fails if the store can't be read or written.
pub fn load_or_create(store: &dyn SecretStore, name: SecretName) -> Result<Secret, SecretError> {
    if let Some(secret) = store.get(name)? {
        return Ok(secret);
    }
    let secret = Secret::generate();
    store.set(name, &secret)?;
    tracing::info!(%name, "generated and stored a new desktop secret");
    Ok(secret)
}

/// The OS keychain: Keychain Services on macOS, the Secret Service on Linux.
#[derive(Debug, Clone)]
pub struct KeychainStore {
    service: String,
}

impl Default for KeychainStore {
    fn default() -> Self {
        Self::with_service(KEYCHAIN_SERVICE)
    }
}

impl KeychainStore {
    /// Files secrets under `service` instead of the app's own, so tests
    /// never touch the real entries.
    #[must_use]
    pub fn with_service(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, name: SecretName) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(&self.service, &name.to_string())
            .map_err(|source| SecretError::Keychain { name, source })
    }

    /// Removes a secret; removing one that was never stored is not an error.
    ///
    /// # Errors
    ///
    /// Fails if the keychain refuses the request.
    pub fn delete(&self, name: SecretName) -> Result<(), SecretError> {
        let deleted = match self.entry(name)?.delete_credential() {
            Err(keyring::Error::NoEntry) => Ok(()),
            other => other,
        };
        deleted.map_err(|source| SecretError::Keychain { name, source })
    }
}

impl SecretStore for KeychainStore {
    fn get(&self, name: SecretName) -> Result<Option<Secret>, SecretError> {
        let value = match self.entry(name)?.get_password() {
            Err(keyring::Error::NoEntry) => return Ok(None),
            other => other,
        };
        value
            .map(|value| Some(Secret::from_stored(value)))
            .map_err(|source| SecretError::Keychain { name, source })
    }

    fn set(&self, name: SecretName, secret: &Secret) -> Result<(), SecretError> {
        self.entry(name)?
            .set_password(secret.expose())
            .map_err(|source| SecretError::Keychain { name, source })
    }
}

/// Secrets kept only in memory, for tests and throwaway runs. Nothing
/// survives the process.
#[derive(Debug, Default)]
pub struct InMemoryStore {
    secrets: std::sync::Mutex<std::collections::HashMap<String, Secret>>,
}

impl InMemoryStore {
    fn secrets(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, Secret>> {
        // A poisoned lock only means another test thread panicked mid-insert;
        // the map itself is still usable.
        self.secrets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl SecretStore for InMemoryStore {
    fn get(&self, name: SecretName) -> Result<Option<Secret>, SecretError> {
        Ok(self.secrets().get(&name.to_string()).cloned())
    }

    fn set(&self, name: SecretName, secret: &Secret) -> Result<(), SecretError> {
        self.secrets().insert(name.to_string(), secret.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::HashMap};

    use strum::IntoEnumIterator;

    use super::*;

    #[derive(Default)]
    struct MemoryStore {
        secrets: RefCell<HashMap<String, Secret>>,
        writes: RefCell<Vec<SecretName>>,
    }

    impl SecretStore for MemoryStore {
        fn get(&self, name: SecretName) -> Result<Option<Secret>, SecretError> {
            Ok(self.secrets.borrow().get(&name.to_string()).cloned())
        }

        fn set(&self, name: SecretName, secret: &Secret) -> Result<(), SecretError> {
            self.writes.borrow_mut().push(name);
            self.secrets
                .borrow_mut()
                .insert(name.to_string(), secret.clone());
            Ok(())
        }
    }

    #[test]
    fn generated_secrets_are_64_hex_chars_and_unique() {
        let first = Secret::generate();
        let second = Secret::generate();
        assert_eq!(first.expose().len(), SECRET_BYTES * 2);
        assert!(first.expose().chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn debug_never_prints_the_value() {
        let secret = Secret::from_stored("hunter2".to_string());
        let printed = format!("{secret:?}");
        assert!(!printed.contains("hunter2"));
        assert!(printed.contains("[REDACTED]"));
    }

    #[test]
    fn load_or_create_generates_once_then_returns_the_stored_value() {
        let store = MemoryStore::default();

        let created = load_or_create(&store, SecretName::ServerSecretKey).unwrap();
        let loaded = load_or_create(&store, SecretName::ServerSecretKey).unwrap();

        assert_eq!(created, loaded);
        assert_eq!(*store.writes.borrow(), [SecretName::ServerSecretKey]);
    }

    #[test]
    fn each_secret_is_independent() {
        let store = MemoryStore::default();

        let key = load_or_create(&store, SecretName::ServerSecretKey).unwrap();
        let admin = load_or_create(&store, SecretName::AdminPassword).unwrap();

        assert_ne!(key, admin);
    }

    #[test]
    fn every_secret_has_a_distinct_keychain_account() {
        let accounts: Vec<String> = SecretName::iter().map(|name| name.to_string()).collect();
        assert_eq!(
            accounts,
            ["server-secret-key", "admin-password", "postgres-password"]
        );
    }
}
