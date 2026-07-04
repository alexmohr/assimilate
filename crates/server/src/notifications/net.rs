// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::net::{IpAddr, SocketAddr};

use super::NotificationError;

enum Scheme {
    Http,
    Https,
    Other(String),
}

impl From<&str> for Scheme {
    fn from(s: &str) -> Self {
        match s {
            "http" => Self::Http,
            "https" => Self::Https,
            other => Self::Other(other.to_owned()),
        }
    }
}

pub(crate) fn is_private_ip(ip: IpAddr) -> bool {
    // Unwrap IPv4-mapped IPv6 addresses (e.g. ::ffff:169.254.169.254)
    // so they are checked against the IPv4 rules rather than treated as
    // ordinary IPv6.
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
        other => other,
    };

    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unspecified() || v6.is_unicast_link_local() || {
                // Unique-local addresses (fc00::/7) -- the IPv6 equivalent
                // of RFC 1918 private space.  Ipv6Addr::is_unique_local()
                // is nightly-only, so check the first two hex digits.
                let segments = v6.segments();
                (segments[0] & 0xFE00) == 0xFC00
            }
        }
    }
}

pub(crate) async fn validate_outbound_url(
    url: &str,
) -> Result<(reqwest::Url, Vec<SocketAddr>), NotificationError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| NotificationError::Config(format!("invalid URL: {e}")))?;

    match Scheme::from(parsed.scheme()) {
        Scheme::Http | Scheme::Https => {}
        Scheme::Other(scheme) => {
            return Err(NotificationError::Config(format!(
                "URL scheme '{scheme}' is not allowed; use http or https"
            )));
        }
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| NotificationError::Config("URL has no host".to_string()))?;

    let addrs: Vec<SocketAddr> = tokio::net::lookup_host(format!("{host}:443"))
        .await
        .map_err(|e| NotificationError::Config(format!("failed to resolve host '{host}': {e}")))?
        .collect();

    for addr in &addrs {
        if is_private_ip(addr.ip()) {
            return Err(NotificationError::Config(format!(
                "URL resolves to a private/reserved address ({}) which is not allowed",
                addr.ip()
            )));
        }
    }

    Ok((parsed, addrs))
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
    fn is_private_ip_rejects_unique_local_v6() {
        assert!(is_private_ip("fc00::1".parse().unwrap()));
        assert!(is_private_ip("fd12:3456::1".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_rejects_link_local_v6() {
        assert!(is_private_ip("fe80::1".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_rejects_ipv4_mapped() {
        // ::ffff:169.254.169.254 -- should be caught by the IPv4 rules
        let mapped: IpAddr = "::ffff:169.254.169.254".parse().unwrap();
        assert!(is_private_ip(mapped));
    }

    #[test]
    fn is_private_ip_allows_public() {
        assert!(!is_private_ip("1.1.1.1".parse().unwrap()));
        assert!(!is_private_ip("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_allows_public_v6() {
        assert!(!is_private_ip("2001:db8::1".parse().unwrap()));
        assert!(!is_private_ip("2606:4700::1".parse().unwrap()));
    }

    #[tokio::test]
    async fn validate_rejects_non_http_scheme() {
        let result = validate_outbound_url("file:///etc/passwd").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not allowed"));
    }

    #[tokio::test]
    async fn validate_rejects_invalid_url() {
        let result = validate_outbound_url("not-a-url").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn validate_rejects_loopback_ip() {
        let result = validate_outbound_url("http://127.0.0.1/hook").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("private/reserved"));
    }

    #[tokio::test]
    async fn validate_rejects_metadata_endpoint() {
        let result = validate_outbound_url("http://169.254.169.254/latest/meta-data/").await;
        assert!(result.is_err());
    }
}
