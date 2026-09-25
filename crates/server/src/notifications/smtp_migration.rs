// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Moves SMTP passwords that older versions stored in plaintext inside
//! `notification_channels.config` into the encrypted `smtp_password_encrypted` column.
//!
//! This cannot be a SQL migration: the key is derived from `ASSIMILATE_SECRET_KEY` and never
//! reaches the database. So the server runs it at startup, after `sqlx::migrate!` has added the
//! column. It is idempotent - once a row's plaintext is gone there is nothing left to match.

use sqlx::PgPool;

use super::{ChannelType, NotificationError, email::EncryptedSmtpPassword};

/// Encrypts every plaintext `config.smtp_password` into `smtp_password_encrypted` and removes
/// it from `config`, in one transaction. A channel that already has an encrypted password
/// keeps it: that one was saved through the API after this version shipped, so it is newer
/// than any plaintext left behind. Returns how many rows it rewrote.
///
/// # Errors
///
/// Returns [`NotificationError::Database`] if a query fails, or
/// [`NotificationError::Crypto`] if encryption fails.
pub async fn encrypt_plaintext_smtp_passwords(
    pool: &PgPool,
    key: &[u8; 32],
) -> Result<u64, NotificationError> {
    let mut tx = pool.begin().await?;

    let rows = sqlx::query!(
        r#"
        SELECT id, channel_type as "channel_type: ChannelType",
               config->>'smtp_password' AS plaintext
        FROM notification_channels
        WHERE config ? 'smtp_password'
        FOR UPDATE
        "#,
    )
    .fetch_all(&mut *tx)
    .await?;

    let mut rewritten: u64 = 0;
    for row in rows {
        let encrypted = match row.channel_type {
            ChannelType::Email => row
                .plaintext
                .as_deref()
                .filter(|plaintext| !plaintext.is_empty())
                .map(|plaintext| EncryptedSmtpPassword::encrypt(plaintext, key))
                .transpose()?,
            ChannelType::Webhook | ChannelType::WebPush => None,
        };
        sqlx::query!(
            r#"
            UPDATE notification_channels
            SET config = config - 'smtp_password',
                smtp_password_encrypted = COALESCE(smtp_password_encrypted, $2)
            WHERE id = $1
            "#,
            row.id,
            encrypted.as_ref().map(EncryptedSmtpPassword::as_bytes),
        )
        .execute(&mut *tx)
        .await?;
        rewritten = rewritten.saturating_add(1);
    }

    tx.commit().await?;
    Ok(rewritten)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        shared::crypto::derive_key(b"smtp-migration-test-key").unwrap()
    }

    async fn insert_channel(
        pool: &PgPool,
        channel_type: ChannelType,
        config: serde_json::Value,
    ) -> i64 {
        sqlx::query_scalar!(
            "INSERT INTO notification_channels (name, channel_type, config) VALUES ($1, $2, $3) \
             RETURNING id",
            format!("legacy-{channel_type}"),
            channel_type.to_string(),
            config,
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn stored(pool: &PgPool, id: i64) -> (serde_json::Value, Option<Vec<u8>>) {
        let row = sqlx::query!(
            "SELECT config, smtp_password_encrypted FROM notification_channels WHERE id = $1",
            id,
        )
        .fetch_one(pool)
        .await
        .unwrap();
        (row.config, row.smtp_password_encrypted)
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn moves_a_legacy_plaintext_password_into_the_encrypted_column(pool: PgPool) {
        let key = test_key();
        let id = insert_channel(
            &pool,
            ChannelType::Email,
            serde_json::json!({ "smtp_host": "smtp.example.com", "smtp_password": "hunter2" }),
        )
        .await;

        assert_eq!(
            encrypt_plaintext_smtp_passwords(&pool, &key).await.unwrap(),
            1
        );

        let (config, encrypted) = stored(&pool, id).await;
        assert_eq!(
            config,
            serde_json::json!({ "smtp_host": "smtp.example.com" })
        );
        let encrypted = encrypted.expect("password moved into the encrypted column");
        assert!(!encrypted.windows(7).any(|w| w == b"hunter2"));
        assert_eq!(
            shared::crypto::decrypt_passphrase(&encrypted, &key).unwrap(),
            "hunter2"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_blank_legacy_password_is_dropped_without_storing_anything(pool: PgPool) {
        let id = insert_channel(
            &pool,
            ChannelType::Email,
            serde_json::json!({ "smtp_host": "smtp.example.com", "smtp_password": "" }),
        )
        .await;

        encrypt_plaintext_smtp_passwords(&pool, &test_key())
            .await
            .unwrap();

        let (config, encrypted) = stored(&pool, id).await;
        assert!(config.get("smtp_password").is_none());
        assert!(encrypted.is_none());
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_already_encrypted_password_wins_over_leftover_plaintext(pool: PgPool) {
        let key = test_key();
        let id = insert_channel(
            &pool,
            ChannelType::Email,
            serde_json::json!({ "smtp_host": "smtp.example.com", "smtp_password": "old" }),
        )
        .await;
        let current = EncryptedSmtpPassword::encrypt("new", &key).unwrap();
        sqlx::query!(
            "UPDATE notification_channels SET smtp_password_encrypted = $1 WHERE id = $2",
            current.as_bytes(),
            id,
        )
        .execute(&pool)
        .await
        .unwrap();

        encrypt_plaintext_smtp_passwords(&pool, &key).await.unwrap();

        let (config, encrypted) = stored(&pool, id).await;
        assert!(config.get("smtp_password").is_none());
        assert_eq!(
            shared::crypto::decrypt_passphrase(&encrypted.unwrap(), &key).unwrap(),
            "new"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_second_run_finds_nothing_left_to_do(pool: PgPool) {
        let key = test_key();
        insert_channel(
            &pool,
            ChannelType::Email,
            serde_json::json!({ "smtp_host": "smtp.example.com", "smtp_password": "hunter2" }),
        )
        .await;
        insert_channel(
            &pool,
            ChannelType::Webhook,
            serde_json::json!({ "url": "https://hooks.example.com/notify" }),
        )
        .await;

        assert_eq!(
            encrypt_plaintext_smtp_passwords(&pool, &key).await.unwrap(),
            1
        );
        assert_eq!(
            encrypt_plaintext_smtp_passwords(&pool, &key).await.unwrap(),
            0
        );
    }
}
