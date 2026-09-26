// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! When an update keeps an email channel's stored SMTP password, and when it replaces it.
//!
//! The password never enters the `config` JSONB and is never sent back. A blank or missing
//! password on an update keeps the stored one, so the edit dialog does not need to know it.
//! One exception: pointing the channel at a different SMTP host requires entering the password
//! again. Otherwise anyone who can edit a channel could redirect the stored password to a
//! server they control and read it there, which would undo the point of not returning it.

use shared::notifications::ChannelConfig;

use crate::{
    error::ApiError,
    notifications::email::{EncryptedSmtpPassword, EnteredSmtpPassword},
};

/// The password to store for `updated`, replacing `existing`'s: `Some` to replace the stored
/// password, `None` to keep it (or its absence).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the update moves an email channel with a stored password
/// to another SMTP host without entering the password again, or [`ApiError::Crypto`] if
/// encryption fails.
pub(super) fn for_update(
    existing: &ChannelConfig,
    has_password: bool,
    updated: &ChannelConfig,
    entered: Option<&EnteredSmtpPassword>,
    key: &[u8; 32],
) -> Result<Option<EncryptedSmtpPassword>, ApiError> {
    if let Some(entered) = entered {
        return Ok(Some(EncryptedSmtpPassword::encrypt(entered, key)?));
    }
    if let (ChannelConfig::Email(existing), ChannelConfig::Email(updated)) = (existing, updated)
        && has_password
    {
        ensure_same_smtp_host(&existing.smtp_host, &updated.smtp_host)?;
    }
    Ok(None)
}

/// Fails unless `requested_host` is the host a stored password was entered for, so a stored
/// password is only ever sent to that server.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the hosts differ.
pub(super) fn ensure_same_smtp_host(
    stored_host: &str,
    requested_host: &str,
) -> Result<(), ApiError> {
    let normalize = |host: &str| host.trim().to_ascii_lowercase();
    if normalize(stored_host) == normalize(requested_host) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            "enter the SMTP password again when changing the SMTP host".to_owned(),
        ))
    }
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

    fn email(host: &str) -> ChannelConfig {
        ChannelConfig::Email(
            serde_json::from_value(json!({
                "smtp_host": host,
                "smtp_port": 587,
                "smtp_user": "alerts",
                "from_address": "alerts@example.com",
                "to_addresses": ["ops@example.com"],
            }))
            .unwrap(),
        )
    }

    #[test]
    fn no_password_entered_keeps_the_stored_one() {
        let kept = for_update(
            &email("smtp.example.com"),
            true,
            &email("smtp.example.com"),
            None,
            &key(),
        )
        .unwrap();
        assert!(kept.is_none());
    }

    #[test]
    fn host_comparison_ignores_case_and_surrounding_whitespace() {
        assert!(
            for_update(
                &email("smtp.example.com"),
                true,
                &email(" SMTP.Example.com "),
                None,
                &key(),
            )
            .is_ok()
        );
    }

    #[test]
    fn a_new_host_without_the_password_is_rejected() {
        assert!(matches!(
            for_update(
                &email("smtp.example.com"),
                true,
                &email("smtp.attacker.example"),
                None,
                &key(),
            ),
            Err(ApiError::BadRequest(_))
        ));
    }

    #[test]
    fn a_new_host_is_fine_when_nothing_is_stored() {
        assert!(
            for_update(
                &email("smtp.example.com"),
                false,
                &email("smtp.other.example"),
                None,
                &key(),
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn an_entered_password_replaces_the_stored_one_even_on_a_new_host() {
        let replaced = for_update(
            &email("smtp.example.com"),
            true,
            &email("smtp.other.example"),
            Some(&entered("hunter2")),
            &key(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            shared::crypto::decrypt_passphrase(replaced.as_bytes(), &key()).unwrap(),
            "hunter2"
        );
    }
}
