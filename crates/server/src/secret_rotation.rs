// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Rotating `ASSIMILATE_SECRET_KEY`: re-encrypts every secret stored under the key derived from
//! the previous secret (`ASSIMILATE_PREVIOUS_SECRET_KEY`) under the key derived from the current
//! one.
//!
//! It runs at startup, in one transaction, before anything reads a stored secret. A value that
//! already decrypts under the current key is left alone, so the rotation is idempotent: a
//! restart with the previous secret still set finds nothing left to do, and one interrupted
//! midway rolls back and simply runs again. A value that decrypts under neither key stops the
//! rotation with nothing changed, since that almost always means the previous secret is wrong.

use shared::crypto::{CryptoError, decrypt_passphrase, encrypt_passphrase};
use sqlx::PgPool;

/// The encrypted columns a rotation re-encrypts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::Display)]
pub enum StoredSecretKind {
    /// `repos.passphrase_encrypted`.
    #[strum(to_string = "repository passphrase")]
    RepositoryPassphrase,
    /// `users.totp_secret_encrypted`.
    #[strum(to_string = "two-factor secret")]
    TotpSecret,
    /// `notification_channels.smtp_password_encrypted`.
    #[strum(to_string = "SMTP password")]
    SmtpPassword,
    /// `notification_channel_headers.value_encrypted`.
    #[strum(to_string = "webhook header value")]
    WebhookHeaderValue,
}

/// Why a rotation failed. Nothing is changed in any of these cases.
#[derive(Debug, thiserror::Error)]
pub enum SecretRotationError {
    /// A query failed.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    /// Re-encrypting a value failed.
    #[error("crypto error: {0}")]
    Crypto(#[from] CryptoError),
    /// A stored value decrypts under neither the current nor the previous key.
    #[error(
        "the {kind} of {owner} decrypts with neither ASSIMILATE_SECRET_KEY nor \
         ASSIMILATE_PREVIOUS_SECRET_KEY; check that ASSIMILATE_PREVIOUS_SECRET_KEY is the secret \
         the server ran with before"
    )]
    Undecryptable {
        /// Which column holds it.
        kind: StoredSecretKind,
        /// Which row, for the operator to find it.
        owner: String,
    },
}

/// The two keys a rotation works with, both derived with [`shared::crypto::derive_key`].
pub struct RotationKeys {
    current: [u8; 32],
    previous: [u8; 32],
}

impl std::fmt::Debug for RotationKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RotationKeys([REDACTED])")
    }
}

impl RotationKeys {
    /// Keys moving values from `previous` to `current`.
    #[must_use]
    pub fn new(current: [u8; 32], previous: [u8; 32]) -> Self {
        Self { current, previous }
    }

    /// The value re-encrypted under the current key, or `None` when it already is.
    fn rekey(
        &self,
        stored: &[u8],
        kind: StoredSecretKind,
        owner: impl FnOnce() -> String,
    ) -> Result<Option<Vec<u8>>, SecretRotationError> {
        if decrypt_passphrase(stored, &self.current).is_ok() {
            return Ok(None);
        }
        let Ok(plaintext) = decrypt_passphrase(stored, &self.previous) else {
            return Err(SecretRotationError::Undecryptable {
                kind,
                owner: owner(),
            });
        };
        Ok(Some(encrypt_passphrase(&plaintext, &self.current)?))
    }
}

/// How many values of each kind a rotation re-encrypted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RotatedSecrets {
    /// Repository passphrases.
    pub repository_passphrases: u64,
    /// Two-factor secrets.
    pub totp_secrets: u64,
    /// SMTP passwords.
    pub smtp_passwords: u64,
    /// Webhook header values.
    pub webhook_header_values: u64,
}

impl RotatedSecrets {
    /// Every value re-encrypted, of any kind.
    #[must_use]
    pub fn total(&self) -> u64 {
        [
            self.repository_passphrases,
            self.totp_secrets,
            self.smtp_passwords,
            self.webhook_header_values,
        ]
        .iter()
        .sum()
    }
}

/// Re-encrypts every stored secret still under the previous key under the current one, in one
/// transaction.
///
/// # Errors
///
/// Returns [`SecretRotationError::Undecryptable`] if a value decrypts under neither key,
/// [`SecretRotationError::Database`] if a query fails, or [`SecretRotationError::Crypto`] if
/// encryption fails. The transaction is rolled back in every case.
pub async fn rotate_stored_secrets(
    pool: &PgPool,
    keys: &RotationKeys,
) -> Result<RotatedSecrets, SecretRotationError> {
    let mut tx = pool.begin().await?;
    let rotated = RotatedSecrets {
        repository_passphrases: rotate_repository_passphrases(&mut tx, keys).await?,
        totp_secrets: rotate_totp_secrets(&mut tx, keys).await?,
        smtp_passwords: rotate_smtp_passwords(&mut tx, keys).await?,
        webhook_header_values: rotate_webhook_header_values(&mut tx, keys).await?,
    };
    tx.commit().await?;
    Ok(rotated)
}

type Tx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

async fn rotate_repository_passphrases(
    tx: &mut Tx<'_>,
    keys: &RotationKeys,
) -> Result<u64, SecretRotationError> {
    let rows = sqlx::query!("SELECT id, name, passphrase_encrypted FROM repos FOR UPDATE")
        .fetch_all(&mut **tx)
        .await?;
    let mut rotated = 0u64;
    for row in rows {
        let Some(encrypted) = keys.rekey(
            &row.passphrase_encrypted,
            StoredSecretKind::RepositoryPassphrase,
            || format!("repository {:?}", row.name),
        )?
        else {
            continue;
        };
        sqlx::query!(
            "UPDATE repos SET passphrase_encrypted = $2 WHERE id = $1",
            row.id,
            encrypted,
        )
        .execute(&mut **tx)
        .await?;
        rotated = rotated.saturating_add(1);
    }
    Ok(rotated)
}

async fn rotate_totp_secrets(
    tx: &mut Tx<'_>,
    keys: &RotationKeys,
) -> Result<u64, SecretRotationError> {
    let rows = sqlx::query!(
        r#"
        SELECT id, username, totp_secret_encrypted AS "secret!"
        FROM users
        WHERE totp_secret_encrypted IS NOT NULL
        FOR UPDATE
        "#,
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut rotated = 0u64;
    for row in rows {
        let Some(encrypted) = keys.rekey(&row.secret, StoredSecretKind::TotpSecret, || {
            format!("user {:?}", row.username)
        })?
        else {
            continue;
        };
        sqlx::query!(
            "UPDATE users SET totp_secret_encrypted = $2 WHERE id = $1",
            row.id,
            encrypted,
        )
        .execute(&mut **tx)
        .await?;
        rotated = rotated.saturating_add(1);
    }
    Ok(rotated)
}

async fn rotate_smtp_passwords(
    tx: &mut Tx<'_>,
    keys: &RotationKeys,
) -> Result<u64, SecretRotationError> {
    let rows = sqlx::query!(
        r#"
        SELECT id, name, smtp_password_encrypted AS "secret!"
        FROM notification_channels
        WHERE smtp_password_encrypted IS NOT NULL
        FOR UPDATE
        "#,
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut rotated = 0u64;
    for row in rows {
        let Some(encrypted) = keys.rekey(&row.secret, StoredSecretKind::SmtpPassword, || {
            format!("notification channel {:?}", row.name)
        })?
        else {
            continue;
        };
        sqlx::query!(
            "UPDATE notification_channels SET smtp_password_encrypted = $2 WHERE id = $1",
            row.id,
            encrypted,
        )
        .execute(&mut **tx)
        .await?;
        rotated = rotated.saturating_add(1);
    }
    Ok(rotated)
}

async fn rotate_webhook_header_values(
    tx: &mut Tx<'_>,
    keys: &RotationKeys,
) -> Result<u64, SecretRotationError> {
    let rows = sqlx::query!(
        r#"
        SELECT h.channel_id, h.name, c.name AS channel, h.value_encrypted AS "secret!"
        FROM notification_channel_headers h
        JOIN notification_channels c ON c.id = h.channel_id
        WHERE h.value_encrypted IS NOT NULL
        FOR UPDATE OF h
        "#,
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut rotated = 0u64;
    for row in rows {
        let Some(encrypted) =
            keys.rekey(&row.secret, StoredSecretKind::WebhookHeaderValue, || {
                format!(
                    "header {:?} of notification channel {:?}",
                    row.name, row.channel
                )
            })?
        else {
            continue;
        };
        sqlx::query!(
            "UPDATE notification_channel_headers SET value_encrypted = $3 WHERE channel_id = $1 \
             AND name = $2",
            row.channel_id,
            row.name,
            encrypted,
        )
        .execute(&mut **tx)
        .await?;
        rotated = rotated.saturating_add(1);
    }
    Ok(rotated)
}

#[cfg(test)]
mod tests;
