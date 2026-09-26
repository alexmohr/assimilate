// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Moves webhook headers that older versions stored in plaintext inside
//! `notification_channels.config->'headers'` into `notification_channel_headers`, with each
//! value encrypted.
//!
//! This cannot be a SQL migration: the key is derived from `ASSIMILATE_SECRET_KEY` and never
//! reaches the database. So the server runs it at startup, after `sqlx::migrate!` has created
//! the table. It is idempotent - once a row's `headers` key is gone there is nothing left to
//! match.

use serde_json::Value;
use sqlx::PgPool;

use super::{NotificationError, webhook::EncryptedHeaderValue};

/// Encrypts every plaintext `config.headers` entry into `notification_channel_headers` and
/// removes `headers` from `config`, in one transaction. A header the channel already has in
/// the table keeps its stored value: that one was saved through the API after this version
/// shipped, so it is newer than any plaintext left behind. Non-string values cannot have been
/// sent by any version and are dropped. Returns how many channels it rewrote.
///
/// # Errors
///
/// Returns [`NotificationError::Database`] if a query fails, or
/// [`NotificationError::Crypto`] if encryption fails.
pub async fn encrypt_plaintext_webhook_headers(
    pool: &PgPool,
    key: &[u8; 32],
) -> Result<u64, NotificationError> {
    let mut tx = pool.begin().await?;

    let rows = sqlx::query!(
        r#"
        SELECT id, config->'headers' AS "headers!"
        FROM notification_channels
        WHERE channel_type = 'webhook' AND jsonb_typeof(config->'headers') = 'object'
        FOR UPDATE
        "#,
    )
    .fetch_all(&mut *tx)
    .await?;

    for row in rows {
        let Value::Object(headers) = row.headers else {
            continue;
        };
        let plaintext_headers = headers.into_iter().filter_map(|(name, value)| match value {
            Value::String(value) => Some((name, value)),
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::Array(_)
            | Value::Object(_) => None,
        });
        for (name, value) in plaintext_headers {
            let encrypted = Some(value)
                .filter(|value| !value.is_empty())
                .map(|value| EncryptedHeaderValue::encrypt(&value, key))
                .transpose()?;
            // No conflict target: a case-only duplicate (`Authorization` next to
            // `authorization`) hits the `lower(name)` index, and the first one wins.
            sqlx::query!(
                r#"
                INSERT INTO notification_channel_headers (channel_id, name, value_encrypted)
                VALUES ($1, $2, $3)
                ON CONFLICT DO NOTHING
                "#,
                row.id,
                name,
                encrypted.as_ref().map(EncryptedHeaderValue::as_bytes),
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    let rewritten = sqlx::query!(
        "UPDATE notification_channels SET config = config - 'headers' WHERE config ? 'headers'",
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();

    tx.commit().await?;
    Ok(rewritten)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::notifications::{
        ChannelType,
        webhook::{StoredWebhookHeader, load_headers},
    };

    fn test_key() -> [u8; 32] {
        shared::crypto::derive_key(b"webhook-header-migration-test-key").unwrap()
    }

    async fn insert_channel(pool: &PgPool, channel_type: ChannelType, config: Value) -> i64 {
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

    async fn config(pool: &PgPool, id: i64) -> Value {
        sqlx::query_scalar!("SELECT config FROM notification_channels WHERE id = $1", id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    fn decrypt(header: &StoredWebhookHeader, key: &[u8; 32]) -> Option<String> {
        header
            .value
            .as_ref()
            .map(|value| shared::crypto::decrypt_passphrase(value.as_bytes(), key).unwrap())
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn moves_legacy_plaintext_headers_into_the_encrypted_table(pool: PgPool) {
        let key = test_key();
        let id = insert_channel(
            &pool,
            ChannelType::Webhook,
            json!({
                "url": "https://hooks.example.com/notify",
                "headers": { "Authorization": "Bearer hunter2", "X-Empty": "" },
            }),
        )
        .await;

        assert_eq!(
            encrypt_plaintext_webhook_headers(&pool, &key)
                .await
                .unwrap(),
            1
        );

        assert_eq!(
            config(&pool, id).await,
            json!({ "url": "https://hooks.example.com/notify" })
        );
        let headers = load_headers(&pool, id).await.unwrap();
        let names: Vec<&str> = headers.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["Authorization", "X-Empty"]);
        let stored = headers.first().unwrap().value.as_ref().unwrap().as_bytes();
        assert!(!stored.windows(7).any(|w| w == b"hunter2"));
        assert_eq!(
            decrypt(headers.first().unwrap(), &key).as_deref(),
            Some("Bearer hunter2")
        );
        assert_eq!(decrypt(headers.get(1).unwrap(), &key), None);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_already_stored_header_wins_over_leftover_plaintext(pool: PgPool) {
        let key = test_key();
        let id = insert_channel(
            &pool,
            ChannelType::Webhook,
            json!({
                "url": "https://hooks.example.com/notify",
                "headers": { "authorization": "Bearer old" },
            }),
        )
        .await;
        let current = EncryptedHeaderValue::encrypt("Bearer new", &key).unwrap();
        sqlx::query!(
            "INSERT INTO notification_channel_headers (channel_id, name, value_encrypted) VALUES \
             ($1, 'Authorization', $2)",
            id,
            current.as_bytes(),
        )
        .execute(&pool)
        .await
        .unwrap();

        encrypt_plaintext_webhook_headers(&pool, &key)
            .await
            .unwrap();

        assert!(config(&pool, id).await.get("headers").is_none());
        let headers = load_headers(&pool, id).await.unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(
            decrypt(headers.first().unwrap(), &key).as_deref(),
            Some("Bearer new")
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_second_run_finds_nothing_left_to_do(pool: PgPool) {
        let key = test_key();
        insert_channel(
            &pool,
            ChannelType::Webhook,
            json!({
                "url": "https://hooks.example.com/notify",
                "headers": { "Authorization": "Bearer hunter2" },
            }),
        )
        .await;
        let email = insert_channel(
            &pool,
            ChannelType::Email,
            json!({ "smtp_host": "smtp.example.com", "headers": { "X": "stray" } }),
        )
        .await;

        assert_eq!(
            encrypt_plaintext_webhook_headers(&pool, &key)
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            encrypt_plaintext_webhook_headers(&pool, &key)
                .await
                .unwrap(),
            0
        );
        assert!(config(&pool, email).await.get("headers").is_none());
        assert_eq!(load_headers(&pool, email).await.unwrap(), []);
    }
}
