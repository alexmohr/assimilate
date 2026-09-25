// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use domain::notification::template;
use wasm_bindgen::prelude::*;

/// Renders a notification template against a JSON payload exactly as a channel
/// delivers it; see [`template::render_template`].
///
/// # Errors
///
/// Fails if `payload_json` is not valid JSON.
#[wasm_bindgen(js_name = renderNotificationTemplate)]
pub fn render_notification_template(template: &str, payload_json: &str) -> Result<String, JsError> {
    render(template, payload_json).map_err(|e| JsError::new(&e))
}

fn render(template: &str, payload_json: &str) -> Result<String, String> {
    let payload: serde_json::Value =
        serde_json::from_str(payload_json).map_err(|e| e.to_string())?;
    Ok(template::render_template(template, &payload))
}

/// Every `{{placeholder}}` key the renderer understands; see
/// [`template::placeholder_keys`].
#[must_use]
#[wasm_bindgen(js_name = notificationTemplatePlaceholderKeys)]
pub fn notification_template_placeholder_keys() -> Vec<String> {
    template::placeholder_keys().map(str::to_owned).collect()
}

/// The default title, body and web-push body templates, in that order; see
/// [`template::DEFAULT_TITLE_TEMPLATE`], [`template::DEFAULT_BODY_TEMPLATE`]
/// and [`template::DEFAULT_PUSH_BODY_TEMPLATE`].
#[must_use]
#[wasm_bindgen(
    js_name = notificationTemplateDefaults,
    unchecked_return_type = "[title: string, body: string, pushBody: string]"
)]
pub fn notification_template_defaults() -> Vec<String> {
    [
        template::DEFAULT_TITLE_TEMPLATE,
        template::DEFAULT_BODY_TEMPLATE,
        template::DEFAULT_PUSH_BODY_TEMPLATE,
    ]
    .map(str::to_owned)
    .to_vec()
}

#[cfg(test)]
mod tests {
    use super::{notification_template_defaults, notification_template_placeholder_keys, render};

    #[test]
    fn renders_a_json_payload_like_a_channel_does() {
        let payload = r#"{"event_type":"backup_failed","hostname":"web-01","duration_secs":272}"#;
        assert_eq!(
            render("{{event}} on {{host}} after {{duration}}", payload).unwrap(),
            "Backup failed on web-01 after 4m 32s"
        );
    }

    #[test]
    fn rejects_a_payload_that_is_not_json() {
        assert!(render("{{host}}", "not json").is_err());
    }

    #[test]
    fn defaults_and_keys_are_the_domain_ones() {
        let [title, body, push_body] =
            <[String; 3]>::try_from(notification_template_defaults()).unwrap();
        assert_eq!(title, "{{event}}: {{host}}");
        assert!(body.contains("{{dedup_size}}"));
        assert_eq!(push_body, "{{host}} {{repository}} {{error}}");
        let keys = notification_template_placeholder_keys();
        assert_eq!(keys.first().map(String::as_str), Some("event"));
        assert_eq!(keys.len(), 16);
    }
}
