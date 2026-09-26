// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! How the `headers` a webhook channel is submitted with become rows of
//! `notification_channel_headers`, and when a stored value is kept.
//!
//! Every header value is treated as a secret: it never enters the `config` JSONB, is stored
//! encrypted, and is never sent back - the response lists header names only. On an update the
//! submitted `headers` object is the channel's complete new header set; a blank or `null` value
//! keeps the value stored under that name, so the edit dialog does not need to know it.
//! One exception: pointing the webhook at a different origin (scheme, host or port) requires
//! typing every stored value again. Otherwise anyone who can edit a channel could redirect a
//! stored bearer token to a server they control and read it there, which would undo the point
//! of not returning it.

use std::collections::{BTreeMap, HashSet};

use reqwest::{Url, header::HeaderName};
use shared::notifications::ChannelConfig;
use sqlx::PgConnection;

use crate::{
    error::ApiError,
    notifications::webhook::{
        EncryptedHeaderValue, EnteredHeaderValue, StoredWebhookHeader, is_valid_header_value,
    },
};

/// One submitted header. `value` is `None` when it was left blank or `null`.
#[derive(Debug)]
pub(super) struct EnteredHeader {
    name: String,
    value: Option<EnteredHeaderValue>,
}

/// What an update does to a channel's stored headers.
#[derive(Debug)]
pub(super) enum HeaderChange {
    /// Leave the stored headers as they are.
    Keep,
    /// Replace them all with this set.
    Replace(Vec<StoredWebhookHeader>),
}

/// Checks the submitted headers and sorts out which values were left blank.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if a name is not a valid HTTP header name, two names differ
/// only in case, or a value is not a valid HTTP header value. No error repeats a value.
pub(super) fn parse(
    submitted: BTreeMap<String, Option<EnteredHeaderValue>>,
) -> Result<Vec<EnteredHeader>, ApiError> {
    let mut seen = HashSet::new();
    submitted
        .into_iter()
        .map(|(name, value)| {
            if HeaderName::from_bytes(name.as_bytes()).is_err() {
                return Err(ApiError::BadRequest(format!(
                    "'{name}' is not a valid HTTP header name"
                )));
            }
            if !seen.insert(name.to_ascii_lowercase()) {
                return Err(ApiError::BadRequest(format!(
                    "header '{name}' is given more than once"
                )));
            }
            let value = value.filter(|value| !value.is_empty());
            if value
                .as_ref()
                .is_some_and(|value| !is_valid_header_value(value))
            {
                return Err(ApiError::BadRequest(format!(
                    "the value of header '{name}' is not a valid HTTP header value"
                )));
            }
            Ok(EnteredHeader { name, value })
        })
        .collect()
}

/// Encrypts the headers a new channel was created with.
///
/// # Errors
///
/// Returns [`ApiError::Crypto`] if encryption fails.
pub(super) fn for_new_channel(
    entered: Vec<EnteredHeader>,
    key: &[u8; 32],
) -> Result<Vec<StoredWebhookHeader>, ApiError> {
    entered
        .into_iter()
        .map(|header| {
            Ok(StoredWebhookHeader {
                value: header
                    .value
                    .map(|value| EncryptedHeaderValue::encrypt(value.expose(), key))
                    .transpose()?,
                name: header.name,
            })
        })
        .collect()
}

/// Decides what an update from `existing` to `updated` does to the channel's `stored` headers.
/// `entered` is the update's `headers`, or `None` when it left them out.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the update moves the webhook to another origin while
/// keeping a stored value without entering it again, or [`ApiError::Crypto`] if encryption
/// fails.
pub(super) fn for_update(
    existing: &ChannelConfig,
    updated: &ChannelConfig,
    stored: &[StoredWebhookHeader],
    entered: Option<Vec<EnteredHeader>>,
    key: &[u8; 32],
) -> Result<HeaderChange, ApiError> {
    let moved = if let (ChannelConfig::Webhook(existing), ChannelConfig::Webhook(updated)) =
        (existing, updated)
    {
        !same_origin(&existing.url, &updated.url)
    } else {
        false
    };
    let Some(entered) = entered else {
        return match stored.iter().find(|header| header.value.is_some()) {
            Some(kept) if moved => Err(enter_again(&kept.name)),
            Some(_) | None => Ok(HeaderChange::Keep),
        };
    };
    entered
        .into_iter()
        .map(|header| {
            if let Some(value) = header.value {
                return Ok(StoredWebhookHeader {
                    name: header.name,
                    value: Some(EncryptedHeaderValue::encrypt(value.expose(), key)?),
                });
            }
            let kept = stored
                .iter()
                .find(|stored| stored.name.eq_ignore_ascii_case(&header.name))
                .and_then(|stored| stored.value.clone());
            if kept.is_some() && moved {
                return Err(enter_again(&header.name));
            }
            Ok(StoredWebhookHeader {
                name: header.name,
                value: kept,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(HeaderChange::Replace)
}

/// Whether both URLs share scheme, host and port, i.e. whether a stored header is still going
/// to the server it was entered for. A URL that does not parse never matches.
fn same_origin(stored: &str, requested: &str) -> bool {
    let origin = |url: &str| Url::parse(url.trim()).ok().map(|url| url.origin());
    match (origin(stored), origin(requested)) {
        (Some(stored), Some(requested)) => stored.is_tuple() && stored == requested,
        (Some(_) | None, None) | (None, Some(_)) => false,
    }
}

/// Writes `headers` as `channel_id`'s complete header set, replacing whatever was stored.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a query fails.
pub(super) async fn replace(
    conn: &mut PgConnection,
    channel_id: i64,
    headers: &[StoredWebhookHeader],
) -> Result<(), ApiError> {
    sqlx::query!(
        "DELETE FROM notification_channel_headers WHERE channel_id = $1",
        channel_id,
    )
    .execute(&mut *conn)
    .await?;
    for header in headers {
        sqlx::query!(
            r#"
            INSERT INTO notification_channel_headers (channel_id, name, value_encrypted)
            VALUES ($1, $2, $3)
            "#,
            channel_id,
            header.name,
            header.value.as_ref().map(EncryptedHeaderValue::as_bytes),
        )
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

fn enter_again(name: &str) -> ApiError {
    ApiError::BadRequest(format!(
        "enter the value of header '{name}' again when changing the webhook URL's scheme, host or \
         port"
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn key() -> [u8; 32] {
        shared::crypto::derive_key(b"webhook-headers-api-test-key").unwrap()
    }

    fn submitted(headers: &serde_json::Value) -> BTreeMap<String, Option<EnteredHeaderValue>> {
        serde_json::from_value(headers.clone()).unwrap()
    }

    fn entered(headers: &serde_json::Value) -> Vec<EnteredHeader> {
        parse(submitted(headers)).unwrap()
    }

    fn stored(name: &str, plaintext: Option<&str>) -> StoredWebhookHeader {
        StoredWebhookHeader {
            name: name.to_owned(),
            value: plaintext.map(|p| EncryptedHeaderValue::encrypt(p, &key()).unwrap()),
        }
    }

    fn decrypt(header: &StoredWebhookHeader) -> Option<String> {
        header
            .value
            .as_ref()
            .map(|value| shared::crypto::decrypt_passphrase(value.as_bytes(), &key()).unwrap())
    }

    fn webhook(url: &str) -> ChannelConfig {
        ChannelConfig::Webhook(serde_json::from_value(json!({ "url": url })).unwrap())
    }

    fn replaced(change: HeaderChange) -> Vec<StoredWebhookHeader> {
        match change {
            HeaderChange::Replace(headers) => headers,
            HeaderChange::Keep => panic!("expected the headers to be replaced"),
        }
    }

    const URL: &str = "https://hooks.example.com/a";

    #[test]
    fn blank_and_null_values_are_left_blank() {
        let headers = entered(&json!({ "A": "", "B": null, "C": "x" }));
        let blanks: Vec<bool> = headers.iter().map(|h| h.value.is_none()).collect();
        assert_eq!(blanks, [true, true, false]);
    }

    #[test]
    fn parse_rejects_invalid_headers() {
        for headers in [
            json!({ "Bad Name": "x" }),
            json!({ "X-Token": "a\r\nX-Evil: 1" }),
            json!({ "Authorization": "a", "authorization": "b" }),
        ] {
            assert!(
                matches!(parse(submitted(&headers)), Err(ApiError::BadRequest(_))),
                "{headers} should be rejected"
            );
        }
    }

    #[test]
    fn rejected_values_are_not_echoed_in_the_error() {
        let err = parse(submitted(&json!({ "X-Token": "hunter2\nX-Evil: 1" })))
            .unwrap_err()
            .to_string();
        assert!(!err.contains("hunter2"));
    }

    #[test]
    fn a_new_channel_stores_its_values_encrypted() {
        let headers = for_new_channel(
            entered(&json!({ "Authorization": "Bearer hunter2", "X-Empty": "" })),
            &key(),
        )
        .unwrap();
        let values: Vec<Option<String>> = headers.iter().map(decrypt).collect();
        assert_eq!(values, [Some("Bearer hunter2".to_owned()), None]);
    }

    #[test]
    fn a_blank_value_on_update_keeps_the_stored_one() {
        let headers = [stored("Authorization", Some("Bearer hunter2"))];
        let change = for_update(
            &webhook(URL),
            &webhook("https://hooks.example.com/b"),
            &headers,
            Some(entered(&json!({ "authorization": "", "x-new": "v" }))),
            &key(),
        )
        .unwrap();
        let headers = replaced(change);
        let names: Vec<&str> = headers.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["authorization", "x-new"]);
        let values: Vec<Option<String>> = headers.iter().map(decrypt).collect();
        assert_eq!(
            values,
            [Some("Bearer hunter2".to_owned()), Some("v".to_owned())]
        );
    }

    #[test]
    fn leaving_a_header_out_removes_it() {
        let headers = [stored("Authorization", Some("Bearer hunter2"))];
        let change = for_update(
            &webhook(URL),
            &webhook(URL),
            &headers,
            Some(Vec::new()),
            &key(),
        )
        .unwrap();
        assert_eq!(replaced(change), []);
    }

    #[test]
    fn omitting_headers_keeps_them_all() {
        let headers = [stored("Authorization", Some("Bearer hunter2"))];
        assert!(matches!(
            for_update(&webhook(URL), &webhook(URL), &headers, None, &key()),
            Ok(HeaderChange::Keep)
        ));
    }

    #[test]
    fn a_new_origin_without_the_values_is_rejected() {
        let headers = [stored("Authorization", Some("Bearer hunter2"))];
        for url in [
            "https://hooks.attacker.example/a",
            "http://hooks.example.com/a",
            "https://hooks.example.com:8443/a",
            "not a url",
        ] {
            for entered_headers in [None, Some(entered(&json!({ "Authorization": "" })))] {
                assert!(
                    matches!(
                        for_update(
                            &webhook(URL),
                            &webhook(url),
                            &headers,
                            entered_headers,
                            &key()
                        ),
                        Err(ApiError::BadRequest(_))
                    ),
                    "moving to {url} must require the value again"
                );
            }
        }
    }

    #[test]
    fn a_new_origin_is_fine_when_the_values_are_entered_again_or_nothing_is_stored() {
        let new_url = "https://other.example.com/a";
        let headers = [stored("Authorization", Some("Bearer hunter2"))];
        let change = for_update(
            &webhook(URL),
            &webhook(new_url),
            &headers,
            Some(entered(&json!({ "Authorization": "Bearer new" }))),
            &key(),
        )
        .unwrap();
        let values: Vec<Option<String>> = replaced(change).iter().map(decrypt).collect();
        assert_eq!(values, [Some("Bearer new".to_owned())]);

        let empty_only = [stored("X-Empty", None)];
        assert!(matches!(
            for_update(&webhook(URL), &webhook(new_url), &empty_only, None, &key()),
            Ok(HeaderChange::Keep)
        ));
    }

    #[test]
    fn host_comparison_ignores_case_and_the_path() {
        let headers = [stored("Authorization", Some("Bearer hunter2"))];
        assert!(matches!(
            for_update(
                &webhook(URL),
                &webhook("https://HOOKS.example.com:443/other?x=1"),
                &headers,
                None,
                &key()
            ),
            Ok(HeaderChange::Keep)
        ));
    }
}
