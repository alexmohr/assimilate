// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{collections::HashMap, net::IpAddr};

use serde::Deserialize;

use super::NotificationError;

#[derive(Debug, Deserialize)]
pub struct WebhookConfig {
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
    }
}

async fn validate_webhook_url(url: &str) -> Result<(), NotificationError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| NotificationError::Config(format!("invalid webhook URL: {e}")))?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(NotificationError::Config(format!(
                "webhook URL scheme '{scheme}' is not allowed; use http or https"
            )));
        }
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| NotificationError::Config("webhook URL has no host".to_string()))?;

    let addrs = tokio::net::lookup_host(format!("{host}:443"))
        .await
        .map_err(|e| {
            NotificationError::Config(format!("failed to resolve webhook host '{host}': {e}"))
        })?;

    for addr in addrs {
        if is_private_ip(addr.ip()) {
            return Err(NotificationError::Config(format!(
                "webhook URL resolves to a private/reserved address ({}) which is not allowed",
                addr.ip()
            )));
        }
    }

    Ok(())
}

pub async fn send(
    client: &reqwest::Client,
    config: &WebhookConfig,
    payload: &serde_json::Value,
) -> Result<(), NotificationError> {
    validate_webhook_url(&config.url).await?;

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
