// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::collections::HashMap;

use serde::Deserialize;

use super::NotificationError;

#[derive(Debug, Deserialize)]
pub struct WebhookConfig {
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

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

    let mut request = client.post(&config.url).json(payload);

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
