// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Length caps on the strings a submitted channel configuration carries,
//! checked on create and update before anything is encrypted or written.

use shared::notifications::{
    ChannelConfigInput, EmailConfigInput, WebPushSettings, WebhookConfigInput,
};

use crate::{
    api::helpers::{self, MaxLen},
    error::ApiError,
};

/// Rejects the first string in `input` that exceeds its [`MaxLen`] cap,
/// naming it by its path in the request body (e.g. `config.smtp_host`).
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] naming the offending field.
pub(super) fn validate(input: &ChannelConfigInput) -> Result<(), ApiError> {
    match input {
        ChannelConfigInput::Email(email) => validate_email(email),
        ChannelConfigInput::Webhook(webhook) => validate_webhook(webhook),
        ChannelConfigInput::WebPush(settings) => validate_web_push(settings),
    }
}

fn validate_email(input: &EmailConfigInput) -> Result<(), ApiError> {
    let config = &input.config;
    helpers::validate_max_len(&config.smtp_host, "config.smtp_host", MaxLen::Hostname)?;
    helpers::validate_max_len(&config.smtp_user, "config.smtp_user", MaxLen::Name)?;
    helpers::validate_max_len(&config.from_address, "config.from_address", MaxLen::Name)?;
    helpers::validate_each_max_len(&config.to_addresses, "config.to_addresses", MaxLen::Name)?;
    validate_templates(
        config.title_template.as_deref(),
        config.body_template.as_deref(),
    )
}

fn validate_webhook(input: &WebhookConfigInput) -> Result<(), ApiError> {
    let config = &input.config;
    helpers::validate_max_len(&config.url, "config.url", MaxLen::Url)?;
    validate_templates(
        config.title_template.as_deref(),
        config.body_template.as_deref(),
    )
}

fn validate_web_push(settings: &WebPushSettings) -> Result<(), ApiError> {
    validate_templates(
        settings.title_template.as_deref(),
        settings.body_template.as_deref(),
    )
}

fn validate_templates(title: Option<&str>, body: Option<&str>) -> Result<(), ApiError> {
    helpers::validate_opt_max_len(title, "config.title_template", MaxLen::Text)?;
    helpers::validate_opt_max_len(body, "config.body_template", MaxLen::Text)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use shared::notifications::ChannelConfigInput;

    use super::validate;
    use crate::api::helpers::{MaxLen, rejection_message};

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Transport {
        Email,
        Webhook,
        WebPush,
    }

    impl Transport {
        /// A request body for this transport with every string well inside
        /// its limit.
        fn body(self) -> Value {
            match self {
                Self::Email => json!({
                    "channel_type": "email",
                    "config": {
                        "smtp_host": "smtp.example.com",
                        "smtp_port": 587,
                        "smtp_user": "alerts",
                        "from_address": "alerts@example.com",
                        "to_addresses": ["ops@example.com", "oncall@example.com"],
                        "security": "starttls",
                    },
                }),
                Self::Webhook => json!({
                    "channel_type": "webhook",
                    "config": { "url": "https://hooks.example.com/borg" },
                }),
                Self::WebPush => json!({ "channel_type": "web_push", "config": {} }),
            }
        }
    }

    fn input(body: Value) -> ChannelConfigInput {
        serde_json::from_value(body).unwrap()
    }

    /// Sets `body.config[path...] = replacement`, where a numeric step indexes
    /// an array.
    fn with(mut body: Value, path: &[&str], replacement: Value) -> Value {
        let slot = std::iter::once(&"config")
            .chain(path)
            .fold(&mut body, |node, step| match step.parse::<usize>() {
                Ok(index) => &mut node[index],
                Err(_) => &mut node[*step],
            });
        *slot = replacement;
        body
    }

    fn at(max: MaxLen) -> Value {
        Value::String("a".repeat(max.chars()))
    }

    fn over(max: MaxLen) -> Value {
        Value::String("a".repeat(max.chars().saturating_add(1)))
    }

    /// Every capped string of each transport, with its limit and the field the
    /// rejection names.
    const FIELDS: [(Transport, &[&str], MaxLen, &str); 11] = [
        (
            Transport::Email,
            &["smtp_host"],
            MaxLen::Hostname,
            "config.smtp_host ",
        ),
        (
            Transport::Email,
            &["smtp_user"],
            MaxLen::Name,
            "config.smtp_user ",
        ),
        (
            Transport::Email,
            &["from_address"],
            MaxLen::Name,
            "config.from_address ",
        ),
        (
            Transport::Email,
            &["to_addresses", "1"],
            MaxLen::Name,
            "config.to_addresses[1] ",
        ),
        (
            Transport::Email,
            &["title_template"],
            MaxLen::Text,
            "config.title_template ",
        ),
        (
            Transport::Email,
            &["body_template"],
            MaxLen::Text,
            "config.body_template ",
        ),
        (Transport::Webhook, &["url"], MaxLen::Url, "config.url "),
        (
            Transport::Webhook,
            &["title_template"],
            MaxLen::Text,
            "config.title_template ",
        ),
        (
            Transport::Webhook,
            &["body_template"],
            MaxLen::Text,
            "config.body_template ",
        ),
        (
            Transport::WebPush,
            &["title_template"],
            MaxLen::Text,
            "config.title_template ",
        ),
        (
            Transport::WebPush,
            &["body_template"],
            MaxLen::Text,
            "config.body_template ",
        ),
    ];

    #[test]
    fn configs_with_every_string_at_its_limit_are_accepted() {
        for transport in [Transport::Email, Transport::Webhook, Transport::WebPush] {
            let body = FIELDS
                .iter()
                .filter(|(of, ..)| *of == transport)
                .fold(transport.body(), |body, (_, path, max, _)| {
                    with(body, path, at(*max))
                });
            assert!(validate(&input(body)).is_ok());
        }
    }

    #[test]
    fn each_over_limit_field_is_rejected_by_name() {
        for (transport, path, max, field) in FIELDS {
            let body = with(transport.body(), path, over(max));
            let message = rejection_message(validate(&input(body)));
            assert!(message.starts_with(field), "{field}: {message}");
        }
    }
}
