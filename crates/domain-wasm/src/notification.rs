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
    let payload: serde_json::Value =
        serde_json::from_str(payload_json).map_err(|e| JsError::new(&e.to_string()))?;
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
