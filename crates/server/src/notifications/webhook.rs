// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::fmt;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use shared::crypto::CryptoError;
pub use shared::notifications::{EnteredHeaderValue, WebhookConfig};

use super::{NotificationError, template::render_template};

/// A webhook header value as stored in `notification_channel_headers.value_encrypted`:
/// AES-256-GCM `nonce || ciphertext` under the server's encryption key, the same scheme as a
/// repository passphrase. Every header value is treated as a secret (it is usually an
/// `Authorization` token), and only this module decrypts it, right before the request is sent.
#[derive(Clone, PartialEq, Eq)]
pub struct EncryptedHeaderValue(Vec<u8>);

impl fmt::Debug for EncryptedHeaderValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EncryptedHeaderValue([REDACTED])")
    }
}

impl From<Vec<u8>> for EncryptedHeaderValue {
    fn from(stored: Vec<u8>) -> Self {
        Self(stored)
    }
}

impl EncryptedHeaderValue {
    /// Encrypts a header value, ready to be stored.
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

    fn decrypt(&self, key: &[u8; 32]) -> Result<String, NotificationError> {
        Ok(shared::crypto::decrypt_passphrase(&self.0, key)?)
    }
}

/// Whether `value` can be sent as an HTTP header value (visible ASCII, no line breaks).
#[must_use]
pub fn is_valid_header_value(value: &EnteredHeaderValue) -> bool {
    HeaderValue::from_str(value.expose()).is_ok()
}

/// One custom header of a webhook channel, as stored in `notification_channel_headers`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredWebhookHeader {
    /// Header name, as the admin typed it. Not secret.
    pub name: String,
    /// The encrypted value, or `None` for a header sent with an empty value.
    pub value: Option<EncryptedHeaderValue>,
}

/// Loads `channel_id`'s custom headers, still encrypted, ordered by name.
///
/// # Errors
///
/// Returns [`NotificationError::Database`] if the query fails.
pub async fn load_headers(
    executor: impl sqlx::PgExecutor<'_>,
    channel_id: i64,
) -> Result<Vec<StoredWebhookHeader>, NotificationError> {
    let rows = sqlx::query!(
        r#"
        SELECT name, value_encrypted
        FROM notification_channel_headers
        WHERE channel_id = $1
        ORDER BY lower(name)
        "#,
        channel_id,
    )
    .fetch_all(executor)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| StoredWebhookHeader {
            name: row.name,
            value: row.value_encrypted.map(EncryptedHeaderValue::from),
        })
        .collect())
}

/// Decrypts `headers` under `key` into the map sent with the request. Every value is marked
/// sensitive so the HTTP stack never prints it.
///
/// # Errors
///
/// Returns [`NotificationError::Crypto`] if a value cannot be decrypted, or
/// [`NotificationError::Config`] if a stored name or value is not valid HTTP.
fn build_headers(
    headers: &[StoredWebhookHeader],
    key: &[u8; 32],
) -> Result<HeaderMap, NotificationError> {
    headers
        .iter()
        .map(|header| {
            let name = HeaderName::from_bytes(header.name.as_bytes()).map_err(|_| {
                NotificationError::Config(format!("invalid webhook header name '{}'", header.name))
            })?;
            let plaintext = header
                .value
                .as_ref()
                .map(|value| value.decrypt(key))
                .transpose()?
                .unwrap_or_default();
            let mut value = HeaderValue::from_str(&plaintext).map_err(|_| {
                NotificationError::Config(format!(
                    "invalid value for webhook header '{}'",
                    header.name
                ))
            })?;
            value.set_sensitive(true);
            Ok((name, value))
        })
        .collect()
}

/// Adds `title`/`message` fields rendered from this channel's own templates to a copy of the
/// raw event payload, leaving the payload unchanged if `config` truly has neither template set.
/// In practice `deliver_to_channel` backfills both from [`super::template::apply_default_template`]
/// before this ever runs, so this fallback only matters for a caller that bypasses that
/// backfill (as some of the tests below deliberately do, to pin down `build_payload`'s own
/// contract in isolation).
fn build_payload(config: &WebhookConfig, payload: &serde_json::Value) -> serde_json::Value {
    if config.title_template.is_none() && config.body_template.is_none() {
        return payload.clone();
    }

    let mut merged = payload.clone();
    if let Some(obj) = merged.as_object_mut() {
        if let Some(tpl) = config.title_template.as_deref() {
            obj.insert(
                "title".to_owned(),
                serde_json::Value::String(render_template(tpl, payload)),
            );
        }
        if let Some(tpl) = config.body_template.as_deref() {
            obj.insert(
                "message".to_owned(),
                serde_json::Value::String(render_template(tpl, payload)),
            );
        }
    }
    merged
}

/// POSTs `payload` to the channel's URL with its custom `headers`, decrypted under `key`.
///
/// # Errors
///
/// Returns [`NotificationError::Config`] if the notification channel is misconfigured, or
/// [`NotificationError::Crypto`] if a stored header value cannot be decrypted.
pub async fn send(
    config: &WebhookConfig,
    headers: &[StoredWebhookHeader],
    key: &[u8; 32],
    payload: &serde_json::Value,
) -> Result<(), NotificationError> {
    let headers = build_headers(headers, key)?;
    let (url, addrs) = super::net::validate_outbound_url(&config.url).await?;

    let host = url
        .host_str()
        .ok_or_else(|| NotificationError::Config("webhook URL has no host".to_string()))?;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addrs)
        .build()
        .map_err(|e| NotificationError::Config(format!("failed to build HTTP client: {e}")))?;

    let body = build_payload(config, payload);
    let response = client
        .post(&config.url)
        .headers(headers)
        .json(&body)
        .send()
        .await?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(NotificationError::Config(format!(
            "webhook returned status {}",
            response.status()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(title_template: Option<&str>, body_template: Option<&str>) -> WebhookConfig {
        WebhookConfig {
            url: "https://hooks.example.com/notify".to_owned(),
            title_template: title_template.map(str::to_owned),
            body_template: body_template.map(str::to_owned),
        }
    }

    #[test]
    fn build_payload_leaves_raw_payload_unchanged_without_a_template() {
        let payload = serde_json::json!({ "event_type": "backup_success", "hostname": "myhost" });
        let merged = build_payload(&config(None, None), &payload);
        assert_eq!(merged, payload);
    }

    #[test]
    fn build_payload_adds_title_and_message_from_the_channels_own_templates() {
        let payload = serde_json::json!({
            "event_type": "backup_success",
            "hostname": "myhost",
            "deduplicated_size": 524_288_000i64,
        });
        let merged = build_payload(
            &config(Some("{{event}} on {{host}}"), Some("{{dedup_size}} new")),
            &payload,
        );
        assert_eq!(
            merged.get("title").and_then(serde_json::Value::as_str),
            Some("Backup succeeded on myhost")
        );
        assert_eq!(
            merged.get("message").and_then(serde_json::Value::as_str),
            Some("500.0 MiB new")
        );
        // the raw fields are still present alongside the rendered ones
        assert_eq!(
            merged.get("hostname").and_then(serde_json::Value::as_str),
            Some("myhost")
        );
    }

    #[test]
    fn deliver_to_channel_backfill_makes_build_payload_add_title_and_message() {
        // Mirrors what `deliver_to_channel` does before delivering: a pre-existing channel with
        // neither template in its stored config would otherwise hit `build_payload`'s own
        // "leave the payload unchanged" fallback above.
        let mut config = super::super::stored_channel_config(
            "webhook",
            serde_json::json!({ "url": "https://hooks.example.com/notify" }),
        )
        .unwrap();
        super::super::template::apply_default_template(&mut config);
        let super::super::ChannelConfig::Webhook(cfg) = config else {
            panic!("a stored webhook config parses as one");
        };

        let payload = serde_json::json!({ "event_type": "backup_success", "hostname": "myhost" });
        let merged = build_payload(&cfg, &payload);
        assert_eq!(
            merged.get("title").and_then(serde_json::Value::as_str),
            Some("Backup succeeded: myhost")
        );
        assert_ne!(merged, payload);
    }

    mod headers {
        //! The stored header values are decrypted here, at send time, and nowhere else.

        use super::*;

        fn key() -> [u8; 32] {
            shared::crypto::derive_key(b"webhook-header-test-key").unwrap()
        }

        fn stored(name: &str, plaintext: Option<&str>) -> StoredWebhookHeader {
            StoredWebhookHeader {
                name: name.to_owned(),
                value: plaintext.map(|p| EncryptedHeaderValue::encrypt(p, &key()).unwrap()),
            }
        }

        #[test]
        fn build_headers_decrypts_every_stored_value() {
            let map = build_headers(
                &[
                    stored("Authorization", Some("Bearer hunter2")),
                    stored("X-Team", Some("ops")),
                ],
                &key(),
            )
            .unwrap();
            assert_eq!(map.get("authorization").unwrap(), "Bearer hunter2");
            assert_eq!(map.get("x-team").unwrap(), "ops");
        }

        #[test]
        fn build_headers_marks_every_value_sensitive() {
            let map =
                build_headers(&[stored("Authorization", Some("Bearer hunter2"))], &key()).unwrap();
            assert!(map.get("authorization").unwrap().is_sensitive());
            assert!(!format!("{map:?}").contains("hunter2"));
        }

        #[test]
        fn a_header_without_a_stored_value_is_sent_empty() {
            let map = build_headers(&[stored("X-Empty", None)], &key()).unwrap();
            assert_eq!(map.get("x-empty").unwrap(), "");
        }

        #[test]
        fn a_value_encrypted_under_another_key_fails() {
            let other = shared::crypto::derive_key(b"some-other-server").unwrap();
            let header = StoredWebhookHeader {
                name: "Authorization".to_owned(),
                value: Some(EncryptedHeaderValue::encrypt("Bearer hunter2", &other).unwrap()),
            };
            assert!(matches!(
                build_headers(&[header], &key()),
                Err(NotificationError::Crypto(_))
            ));
        }

        #[test]
        fn an_invalid_stored_name_is_reported_without_the_value() {
            let err = build_headers(&[stored("Bad Name", Some("Bearer hunter2"))], &key())
                .unwrap_err()
                .to_string();
            assert!(err.contains("Bad Name"));
            assert!(!err.contains("hunter2"));
        }

        #[test]
        fn an_invalid_stored_value_is_reported_without_the_value() {
            let err = build_headers(&[stored("X-Token", Some("hunter2\nX-Evil: 1"))], &key())
                .unwrap_err()
                .to_string();
            assert!(err.contains("X-Token"));
            assert!(!err.contains("hunter2"));
        }

        #[test]
        fn header_value_debug_output_is_redacted() {
            let encrypted = EncryptedHeaderValue::encrypt("Bearer hunter2", &key()).unwrap();
            assert_eq!(format!("{encrypted:?}"), "EncryptedHeaderValue([REDACTED])");
        }

        #[test]
        fn entered_values_with_line_breaks_are_not_valid_http() {
            let valid = |v: &str| is_valid_header_value(&EnteredHeaderValue::new(v.to_owned()));
            assert!(valid("Bearer x"));
            assert!(!valid("a\r\nb"));
        }
    }
}
