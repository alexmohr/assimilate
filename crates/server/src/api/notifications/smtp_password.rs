// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! How the `smtp_password` in a channel request becomes the encrypted
//! `notification_channels.smtp_password_encrypted` column, and when the stored one is kept.
//!
//! The password never enters the `config` JSONB and is never sent back. A blank or missing
//! field on an update keeps the stored password, so the edit dialog does not need to know it.
//! One exception: pointing the channel at a different SMTP host requires typing the password
//! again. Otherwise anyone who can edit a channel could redirect the stored password to a
//! server they control and read it there, which would undo the point of not returning it.

use serde_json::Value;

use crate::{
    error::ApiError,
    notifications::{
        ChannelType,
        email::{EncryptedSmtpPassword, EnteredSmtpPassword},
    },
};

const FIELD: &str = "smtp_password";

/// What an update does to a channel's stored SMTP password.
#[derive(Debug)]
pub(super) enum SmtpPasswordChange {
    /// Leave the stored password (or its absence) as it is.
    Keep,
    /// Remove the stored password; the channel is no longer an email channel.
    Clear,
    /// Replace it with a newly entered one.
    Set(EncryptedSmtpPassword),
}

impl SmtpPasswordChange {
    /// `(replace, value)` for the UPDATE's
    /// `CASE WHEN replace THEN value ELSE smtp_password_encrypted END`.
    pub(super) fn update_params(&self) -> (bool, Option<&[u8]>) {
        match self {
            Self::Keep => (false, None),
            Self::Clear => (true, None),
            Self::Set(encrypted) => (true, Some(encrypted.as_bytes())),
        }
    }
}

/// The parts of a saved channel an update needs to decide what happens to its password.
#[derive(Debug)]
pub(super) struct StoredChannel {
    pub(super) channel_type: ChannelType,
    pub(super) config: Value,
    pub(super) has_password: bool,
}

/// Removes `smtp_password` from a request's `config` so it can never be written to the JSONB
/// column. Returns it unless it was missing, `null` or blank - all of which mean "none given".
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if `smtp_password` is present but not a string.
pub(super) fn take_entered_password(
    config: &mut Value,
) -> Result<Option<EnteredSmtpPassword>, ApiError> {
    let Some(fields) = config.as_object_mut() else {
        return Ok(None);
    };
    match fields.remove(FIELD) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(entered)) => {
            Ok(Some(EnteredSmtpPassword::new(entered)).filter(|entered| !entered.is_empty()))
        }
        Some(Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => {
            Err(ApiError::BadRequest(format!("{FIELD} must be a string")))
        }
    }
}

/// Encrypts the password a new channel was created with, if any.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if a password was given for a non-email channel, or
/// [`ApiError::Crypto`] if encryption fails.
pub(super) fn for_new_channel(
    channel_type: ChannelType,
    entered: Option<&EnteredSmtpPassword>,
    key: &[u8; 32],
) -> Result<Option<EncryptedSmtpPassword>, ApiError> {
    let Some(entered) = entered else {
        return Ok(None);
    };
    match channel_type {
        ChannelType::Email => Ok(Some(entered.encrypt(key)?)),
        ChannelType::Webhook | ChannelType::WebPush => Err(only_for_email()),
    }
}

/// Decides what an update does to `existing`'s stored password.
///
/// `new_config` is the update's `config` (with the password already taken out), or `None`
/// when the update leaves the config alone.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if a password was given for a non-email channel, or if
/// the update moves an email channel with a stored password to another SMTP host without
/// entering the password again. Returns [`ApiError::Crypto`] if encryption fails.
pub(super) fn for_update(
    existing: &StoredChannel,
    channel_type: ChannelType,
    new_config: Option<&Value>,
    entered: Option<&EnteredSmtpPassword>,
    key: &[u8; 32],
) -> Result<SmtpPasswordChange, ApiError> {
    match (channel_type, entered) {
        (ChannelType::Webhook | ChannelType::WebPush, Some(_)) => Err(only_for_email()),
        (ChannelType::Webhook | ChannelType::WebPush, None) => Ok(SmtpPasswordChange::Clear),
        (ChannelType::Email, Some(entered)) => Ok(SmtpPasswordChange::Set(entered.encrypt(key)?)),
        (ChannelType::Email, None) => {
            let keeps_email = existing.channel_type == ChannelType::Email;
            if let Some(new_config) = new_config
                && keeps_email
                && existing.has_password
            {
                ensure_same_smtp_host(
                    &existing.config,
                    new_config.get("smtp_host").and_then(Value::as_str),
                )?;
            }
            Ok(SmtpPasswordChange::Keep)
        }
    }
}

/// Fails unless `requested_host` is the SMTP host in the channel's `stored` config, so a
/// stored password is only ever sent to the server it was entered for.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the hosts differ.
pub(super) fn ensure_same_smtp_host(
    stored: &Value,
    requested_host: Option<&str>,
) -> Result<(), ApiError> {
    let normalize = |host: &str| host.trim().to_ascii_lowercase();
    let stored_host = stored.get("smtp_host").and_then(Value::as_str);
    if stored_host.map(normalize) == requested_host.map(normalize) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            "enter the SMTP password again when changing the SMTP host".to_owned(),
        ))
    }
}

fn only_for_email() -> ApiError {
    ApiError::BadRequest(format!("{FIELD} is only valid for email channels"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn key() -> [u8; 32] {
        shared::crypto::derive_key(b"smtp-password-api-test-key").unwrap()
    }

    fn entered(plaintext: &str) -> EnteredSmtpPassword {
        EnteredSmtpPassword::new(plaintext.to_owned())
    }

    fn email_channel(host: &str, has_password: bool) -> StoredChannel {
        StoredChannel {
            channel_type: ChannelType::Email,
            config: json!({ "smtp_host": host, "smtp_port": 587 }),
            has_password,
        }
    }

    #[test]
    fn take_entered_password_removes_the_field_from_the_config() {
        let mut config = json!({ "smtp_host": "smtp.example.com", "smtp_password": "hunter2" });
        let taken = take_entered_password(&mut config).unwrap();
        assert!(taken.is_some());
        assert_eq!(config, json!({ "smtp_host": "smtp.example.com" }));
    }

    #[test]
    fn take_entered_password_treats_blank_null_and_missing_as_none_given() {
        for mut config in [
            json!({ "smtp_password": "" }),
            json!({ "smtp_password": null }),
            json!({}),
        ] {
            assert!(take_entered_password(&mut config).unwrap().is_none());
            assert!(config.get(FIELD).is_none());
        }
    }

    #[test]
    fn take_entered_password_rejects_a_non_string() {
        let mut config = json!({ "smtp_password": 1234 });
        assert!(matches!(
            take_entered_password(&mut config),
            Err(ApiError::BadRequest(_))
        ));
    }

    #[test]
    fn entered_password_debug_output_is_redacted() {
        let debug = format!("{:?}", entered("hunter2"));
        assert!(!debug.contains("hunter2"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn a_new_email_channel_stores_its_password_encrypted() {
        let encrypted = for_new_channel(ChannelType::Email, Some(&entered("hunter2")), &key())
            .unwrap()
            .unwrap();
        assert_eq!(
            shared::crypto::decrypt_passphrase(encrypted.as_bytes(), &key()).unwrap(),
            "hunter2"
        );
    }

    #[test]
    fn a_new_webhook_channel_rejects_a_password() {
        assert!(matches!(
            for_new_channel(ChannelType::Webhook, Some(&entered("hunter2")), &key()),
            Err(ApiError::BadRequest(_))
        ));
        assert!(
            for_new_channel(ChannelType::Webhook, None, &key())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_blank_password_on_update_keeps_the_stored_one() {
        let existing = email_channel("smtp.example.com", true);
        let new_config = json!({ "smtp_host": "smtp.example.com", "smtp_port": 465 });
        let change = for_update(
            &existing,
            ChannelType::Email,
            Some(&new_config),
            None,
            &key(),
        )
        .unwrap();
        assert!(matches!(change, SmtpPasswordChange::Keep));
        assert_eq!(change.update_params(), (false, None));
    }

    #[test]
    fn host_comparison_ignores_case_and_surrounding_whitespace() {
        let existing = email_channel("smtp.example.com", true);
        let new_config = json!({ "smtp_host": " SMTP.Example.com " });
        assert!(matches!(
            for_update(
                &existing,
                ChannelType::Email,
                Some(&new_config),
                None,
                &key()
            ),
            Ok(SmtpPasswordChange::Keep)
        ));
    }

    #[test]
    fn a_new_host_without_the_password_is_rejected() {
        let existing = email_channel("smtp.example.com", true);
        let new_config = json!({ "smtp_host": "smtp.attacker.example" });
        assert!(matches!(
            for_update(
                &existing,
                ChannelType::Email,
                Some(&new_config),
                None,
                &key()
            ),
            Err(ApiError::BadRequest(_))
        ));
    }

    #[test]
    fn a_new_host_is_fine_when_nothing_is_stored_or_the_password_is_entered_again() {
        let new_config = json!({ "smtp_host": "smtp.other.example" });
        assert!(matches!(
            for_update(
                &email_channel("smtp.example.com", false),
                ChannelType::Email,
                Some(&new_config),
                None,
                &key(),
            ),
            Ok(SmtpPasswordChange::Keep)
        ));
        let change = for_update(
            &email_channel("smtp.example.com", true),
            ChannelType::Email,
            Some(&new_config),
            Some(&entered("hunter2")),
            &key(),
        )
        .unwrap();
        let (replace, value) = change.update_params();
        assert!(replace);
        assert_eq!(
            shared::crypto::decrypt_passphrase(value.unwrap(), &key()).unwrap(),
            "hunter2"
        );
    }

    #[test]
    fn switching_away_from_email_clears_the_stored_password() {
        let existing = email_channel("smtp.example.com", true);
        let new_config = json!({ "url": "https://hooks.example.com/notify" });
        let change = for_update(
            &existing,
            ChannelType::Webhook,
            Some(&new_config),
            None,
            &key(),
        )
        .unwrap();
        assert_eq!(change.update_params(), (true, None));
    }
}
