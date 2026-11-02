// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The server's second SSH key, used only for maintenance of append-only
//! repositories.
//!
//! It sits next to the server's own key in `SSH_KEY_DIR`, but unlike that key
//! it is never loaded into the SSH agent: the relay hands every key in the
//! agent to every agent, and this one must stay out of their reach. The server
//! passes it to borg and SFTP directly.

use std::path::{Path, PathBuf};

use ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey, PublicKey, rand_core::OsRng};
use tokio::io::AsyncWriteExt;

use super::{SshError, ssh_key_dir};

const MAINTENANCE_KEY_FILE: &str = "id_ed25519_maintenance";

/// Where the maintenance private key is stored.
pub(crate) fn maintenance_key_path() -> PathBuf {
    ssh_key_dir().join(MAINTENANCE_KEY_FILE)
}

fn key_error(what: &str, e: impl std::fmt::Display) -> SshError {
    SshError::Auth(format!("{what}: {e}"))
}

/// Reads the maintenance key, if it has been generated.
///
/// # Errors
///
/// Returns [`SshError::Auth`] if the file exists but is not a usable key.
pub async fn read_maintenance_key() -> Result<Option<PrivateKey>, SshError> {
    read_key_at(&maintenance_key_path()).await
}

async fn read_key_at(path: &Path) -> Result<Option<PrivateKey>, SshError> {
    let text = match tokio::fs::read_to_string(path).await {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(SshError::Io(e)),
    };
    PrivateKey::from_openssh(&text)
        .map(Some)
        .map_err(|e| key_error("failed to decode the maintenance key", e))
}

/// Returns the maintenance key, generating it first if the server has none.
///
/// The key is written with mode 0600 from the start, so it is never readable
/// by anyone else, not even briefly.
///
/// # Errors
///
/// Returns [`SshError::Io`] if the key cannot be stored, and
/// [`SshError::Auth`] if an existing file is not a usable key.
pub async fn ensure_maintenance_key() -> Result<PrivateKey, SshError> {
    ensure_key_at(&maintenance_key_path()).await
}

async fn ensure_key_at(path: &Path) -> Result<PrivateKey, SshError> {
    if let Some(key) = read_key_at(path).await? {
        return Ok(key);
    }

    let key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519)
        .map_err(|e| key_error("failed to generate the maintenance key", e))?;
    let pem = key
        .to_openssh(LineEnding::LF)
        .map_err(|e| key_error("failed to encode the maintenance key", e))?;

    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    match options.open(path).await {
        Ok(mut file) => {
            file.write_all(pem.as_bytes()).await?;
            file.flush().await?;
            Ok(key)
        }
        // Another request generated it first; use that one so both agree.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => read_key_at(path)
            .await?
            .ok_or_else(|| key_error("the maintenance key disappeared", e)),
        Err(e) => Err(SshError::Io(e)),
    }
}

/// `SHA256:...` fingerprint of `key`.
#[must_use]
pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

/// The server's backup key (the one the agents use), as an `ssh-key` key.
///
/// # Errors
///
/// Returns [`SshError::PublicKeyNotFound`] if the server has no key yet, and
/// [`SshError::Auth`] if the file is not a usable key.
pub async fn read_backup_public_key() -> Result<PublicKey, SshError> {
    let path = ssh_key_dir().join("id_ed25519.pub");
    let text = tokio::fs::read_to_string(&path)
        .await
        .map_err(|_| SshError::PublicKeyNotFound(path))?;
    PublicKey::from_openssh(text.trim())
        .map_err(|e| key_error("failed to decode the server's public key", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_maintenance_key_is_generated_once_and_kept_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(MAINTENANCE_KEY_FILE);

        assert!(read_key_at(&path).await.unwrap().is_none());
        let first = ensure_key_at(&path).await.unwrap();
        let second = ensure_key_at(&path).await.unwrap();
        assert_eq!(
            first.public_key().key_data(),
            second.public_key().key_data()
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = tokio::fs::metadata(&path)
                .await
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[tokio::test]
    async fn the_key_directory_is_created_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(MAINTENANCE_KEY_FILE);
        ensure_key_at(&path).await.unwrap();
        assert!(tokio::fs::try_exists(&path).await.unwrap());
    }

    #[tokio::test]
    async fn a_corrupt_maintenance_key_is_reported_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(MAINTENANCE_KEY_FILE);
        tokio::fs::write(&path, b"not a key").await.unwrap();

        let result = ensure_key_at(&path).await;
        assert!(matches!(result, Err(SshError::Auth(_))), "{result:?}");
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"not a key");
    }

    #[test]
    fn a_fingerprint_is_a_sha256_one() {
        let key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap();
        assert!(fingerprint(key.public_key()).starts_with("SHA256:"));
    }
}
