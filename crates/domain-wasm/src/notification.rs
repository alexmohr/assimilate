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

/// See [`template::DEFAULT_TITLE_TEMPLATE`].
#[must_use]
#[wasm_bindgen(js_name = defaultTitleTemplate)]
pub fn default_title_template() -> String {
    template::DEFAULT_TITLE_TEMPLATE.to_owned()
}

/// See [`template::DEFAULT_BODY_TEMPLATE`].
#[must_use]
#[wasm_bindgen(js_name = defaultBodyTemplate)]
pub fn default_body_template() -> String {
    template::DEFAULT_BODY_TEMPLATE.to_owned()
}

/// See [`template::DEFAULT_PUSH_BODY_TEMPLATE`].
#[must_use]
#[wasm_bindgen(js_name = defaultPushBodyTemplate)]
pub fn default_push_body_template() -> String {
    template::DEFAULT_PUSH_BODY_TEMPLATE.to_owned()
}

#[cfg(test)]
mod tests {
    use super::{
        default_body_template, default_push_body_template, default_title_template,
        notification_template_placeholder_keys, render,
    };

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
        assert_eq!(default_title_template(), "{{event}}: {{host}}");
        assert!(default_body_template().contains("{{dedup_size}}"));
        assert_eq!(
            default_push_body_template(),
            "{{host}} {{repository}} {{error}}"
        );
        let keys = notification_template_placeholder_keys();
        assert_eq!(keys.first().map(String::as_str), Some("event"));
        assert_eq!(keys.len(), 16);
    }
}
