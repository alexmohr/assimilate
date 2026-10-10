// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Upper bounds on the length of user-supplied strings.
//!
//! Every column these strings land in is `TEXT`, so without a cap here the
//! request body limit is the only ceiling: an oversized name would be stored
//! verbatim and then break every table and card that renders it. Limits count
//! `char`s rather than bytes so a non-ASCII name gets the same allowance as an
//! ASCII one.

use shared::hooks::HookCommand;

use crate::error::ApiError;

/// The kind of string being bounded, each with its own character limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaxLen {
    /// Names and labels: repository, schedule, tag, group, role, token and
    /// channel names, display names, usernames, SSH users, service names and
    /// hostname patterns.
    Name,
    /// DNS names: agent hostnames, domains and SSH hosts. 253 is the longest
    /// name DNS can represent (RFC 1035).
    Hostname,
    /// Filesystem paths. 4096 matches Linux's `PATH_MAX`.
    Path,
    /// URLs, such as the server's public URL.
    Url,
    /// Free-form, single-paragraph descriptions.
    Description,
    /// Multi-line text: pattern lists and systemd unit files.
    Text,
}

impl MaxLen {
    /// The maximum number of characters allowed.
    #[must_use]
    pub const fn chars(self) -> usize {
        match self {
            Self::Name => 255,
            Self::Hostname => 253,
            Self::Path => 4096,
            Self::Url => 2048,
            Self::Description => 1024,
            Self::Text => 65_536,
        }
    }
}

/// Rejects `value` if it is longer than `max` allows, naming `field_name` in
/// the error.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if `value` has more than `max.chars()`
/// characters.
pub fn validate_max_len(value: &str, field_name: &str, max: MaxLen) -> Result<(), ApiError> {
    let limit = max.chars();
    // `nth(limit)` stops after `limit + 1` chars instead of walking a
    // multi-megabyte string just to learn it is too long.
    if value.chars().nth(limit).is_some() {
        return Err(ApiError::BadRequest(format!(
            "{field_name} must be at most {limit} characters"
        )));
    }
    Ok(())
}

/// Like [`validate_max_len`], for a field that may be absent.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if `value` is present and too long.
pub fn validate_opt_max_len(
    value: Option<&str>,
    field_name: &str,
    max: MaxLen,
) -> Result<(), ApiError> {
    value.map_or(Ok(()), |value| validate_max_len(value, field_name, max))
}

/// Like [`validate_max_len`], for every entry of a list, naming the offending
/// entry's index in the error.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for the first entry that is too long.
pub fn validate_each_max_len(
    values: &[String],
    field_name: &str,
    max: MaxLen,
) -> Result<(), ApiError> {
    values
        .iter()
        .enumerate()
        .try_for_each(|(i, value)| validate_max_len(value, &format!("{field_name}[{i}]"), max))
}

/// Caps every hook command's script at [`MaxLen::Text`], naming the offending
/// command's index in the error (e.g. `pre_backup_commands[1]`).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] for the first command that is too long.
pub fn validate_each_command_max_len(
    commands: &[HookCommand],
    field_name: &str,
) -> Result<(), ApiError> {
    commands.iter().enumerate().try_for_each(|(i, command)| {
        validate_max_len(
            &command.command,
            &format!("{field_name}[{i}]"),
            MaxLen::Text,
        )
    })
}

/// The message of the [`ApiError::BadRequest`] that `result` must be, for
/// tests asserting which field a length check names.
///
/// # Panics
///
/// If `result` is anything other than a `BadRequest`.
#[cfg(test)]
pub(crate) fn rejection_message(result: Result<(), ApiError>) -> String {
    let error = result.expect_err("an over-limit value must be refused");
    assert!(
        matches!(error, ApiError::BadRequest(_)),
        "expected BadRequest, got {error:?}"
    );
    // A `BadRequest` displays as `bad request: <message>`.
    error.to_string().replacen("bad request: ", "", 1)
}

#[cfg(test)]
mod tests {
    use shared::hooks::HookCommand;

    use super::{
        MaxLen, rejection_message, validate_each_command_max_len, validate_each_max_len,
        validate_max_len, validate_opt_max_len,
    };

    const ALL: [MaxLen; 6] = [
        MaxLen::Name,
        MaxLen::Hostname,
        MaxLen::Path,
        MaxLen::Url,
        MaxLen::Description,
        MaxLen::Text,
    ];

    #[test]
    fn limits_match_the_documented_caps() {
        assert_eq!(MaxLen::Name.chars(), 255);
        assert_eq!(MaxLen::Hostname.chars(), 253);
        assert_eq!(MaxLen::Path.chars(), 4096);
        assert_eq!(MaxLen::Url.chars(), 2048);
        assert_eq!(MaxLen::Description.chars(), 1024);
        assert_eq!(MaxLen::Text.chars(), 65_536);
    }

    #[test]
    fn accepts_empty_and_exactly_the_limit() {
        for max in ALL {
            assert!(validate_max_len("", "field", max).is_ok());
            assert!(validate_max_len(&"a".repeat(max.chars()), "field", max).is_ok());
        }
    }

    #[test]
    fn rejects_one_over_the_limit_naming_the_field() {
        for max in ALL {
            let message = rejection_message(validate_max_len(
                &"a".repeat(max.chars().saturating_add(1)),
                "name",
                max,
            ));
            assert_eq!(
                message,
                format!("name must be at most {} characters", max.chars())
            );
        }
    }

    #[test]
    fn counts_characters_not_bytes() {
        // U+00FC is two bytes in UTF-8: 255 of them are 510 bytes but only 255
        // characters, and must fit a 255-character name.
        let two_byte = "\u{fc}";
        assert_eq!(two_byte.len(), 2);
        assert!(validate_max_len(&two_byte.repeat(255), "name", MaxLen::Name).is_ok());
        assert!(validate_max_len(&two_byte.repeat(256), "name", MaxLen::Name).is_err());
    }

    #[test]
    fn optional_field_is_only_checked_when_present() {
        assert!(validate_opt_max_len(None, "display_name", MaxLen::Name).is_ok());
        assert!(validate_opt_max_len(Some("web"), "display_name", MaxLen::Name).is_ok());
        let message = rejection_message(validate_opt_max_len(
            Some(&"a".repeat(256)),
            "display_name",
            MaxLen::Name,
        ));
        assert!(message.starts_with("display_name "), "{message}");
    }

    #[test]
    fn list_entries_are_checked_individually_and_named_by_index() {
        let at_limit = vec!["a".repeat(4096), "/etc".to_owned()];
        assert!(validate_each_max_len(&at_limit, "paths", MaxLen::Path).is_ok());
        assert!(validate_each_max_len(&[], "paths", MaxLen::Path).is_ok());

        let over = vec!["/etc".to_owned(), "a".repeat(4097)];
        let message = rejection_message(validate_each_max_len(&over, "paths", MaxLen::Path));
        assert_eq!(message, "paths[1] must be at most 4096 characters");
    }

    #[test]
    fn hook_command_scripts_are_checked_individually_and_named_by_index() {
        let limit = MaxLen::Text.chars();
        let at_limit = [
            HookCommand::new("a".repeat(limit)),
            HookCommand::new("true"),
        ];
        assert!(validate_each_command_max_len(&at_limit, "pre_backup_commands").is_ok());
        assert!(validate_each_command_max_len(&[], "pre_backup_commands").is_ok());

        let over = [
            HookCommand::new("true"),
            HookCommand::new("a".repeat(limit.saturating_add(1))),
        ];
        let message =
            rejection_message(validate_each_command_max_len(&over, "pre_backup_commands"));
        assert_eq!(
            message,
            format!("pre_backup_commands[1] must be at most {limit} characters")
        );
    }
}
