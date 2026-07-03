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

enum WebhookScheme {
    Http,
    Https,
    Other,
}

impl From<&str> for WebhookScheme {
    fn from(scheme: &str) -> Self {
        match scheme {
            "http" => Self::Http,
            "https" => Self::Https,
            _ => Self::Other,
        }
    }
}

async fn validate_webhook_url(url: &str) -> Result<(), NotificationError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| NotificationError::Config(format!("invalid webhook URL: {e}")))?;

    let scheme = parsed.scheme();
    match WebhookScheme::from(scheme) {
        WebhookScheme::Http | WebhookScheme::Https => {}
        WebhookScheme::Other => {
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

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    fn is_private_ip_rejects_loopback() {
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(is_private_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn is_private_ip_rejects_private_ranges() {
        assert!(is_private_ip("10.0.0.1".parse().unwrap()));
        assert!(is_private_ip("172.16.0.1".parse().unwrap()));
        assert!(is_private_ip("192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_rejects_link_local() {
        assert!(is_private_ip("169.254.169.254".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_allows_public() {
        assert!(!is_private_ip("1.1.1.1".parse().unwrap()));
        assert!(!is_private_ip("8.8.8.8".parse().unwrap()));
    }

    #[tokio::test]
    async fn validate_rejects_non_http_scheme() {
        let result = validate_webhook_url("file:///etc/passwd").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not allowed"));
    }

    #[tokio::test]
    async fn validate_rejects_invalid_url() {
        let result = validate_webhook_url("not-a-url").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn validate_rejects_loopback_ip() {
        let result = validate_webhook_url("http://127.0.0.1/hook").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("private/reserved"));
    }

    #[tokio::test]
    async fn validate_rejects_metadata_endpoint() {
        let result = validate_webhook_url("http://169.254.169.254/latest/meta-data/").await;
        assert!(result.is_err());
    }
}
