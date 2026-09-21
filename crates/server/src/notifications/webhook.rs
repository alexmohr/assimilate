// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::collections::HashMap;

use serde::Deserialize;

use super::{NotificationError, template::render_template};

/// Configuration for an HTTP webhook notification channel.
#[derive(Debug, Deserialize)]
pub struct WebhookConfig {
    /// Target URL to POST the notification payload to.
    pub url: String,
    /// Optional custom HTTP headers to include in the request.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// This channel's own title template, rendered into a `title` field alongside the raw
    /// event fields already in the JSON payload. Independent of every other channel's
    /// template -- see [`super::template::render_template`] for the placeholder syntax.
    #[serde(default)]
    pub title_template: Option<String>,
    /// This channel's own body template, rendered into a `message` field alongside the raw
    /// event fields already in the JSON payload. Independent of every other channel's
    /// template -- see [`super::template::render_template`] for the placeholder syntax.
    #[serde(default)]
    pub body_template: Option<String>,
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

/// # Errors
///
/// Returns [`NotificationError::Config`] if the notification channel is misconfigured.
pub async fn send(
    config: &WebhookConfig,
    payload: &serde_json::Value,
) -> Result<(), NotificationError> {
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
    let mut request = client.post(&config.url).json(&body);

    for (key, value) in &config.headers {
        request = request.header(key.as_str(), value.as_str());
    }

    let response = request.send().await?;

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
            headers: HashMap::new(),
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
        // Mirrors what `deliver_to_channel` does before deserializing into `WebhookConfig`: a
        // pre-existing channel with neither template in its raw config would otherwise hit
        // `build_payload`'s own "leave the payload unchanged" fallback above.
        let mut raw_config = serde_json::json!({ "url": "https://hooks.example.com/notify" });
        super::super::template::apply_default_template(
            &mut raw_config,
            super::super::ChannelType::Webhook,
        );
        let cfg: WebhookConfig = serde_json::from_value(raw_config).unwrap();

        let payload = serde_json::json!({ "event_type": "backup_success", "hostname": "myhost" });
        let merged = build_payload(&cfg, &payload);
        assert_eq!(
            merged.get("title").and_then(serde_json::Value::as_str),
            Some("Backup succeeded: myhost")
        );
        assert_ne!(merged, payload);
    }
}
