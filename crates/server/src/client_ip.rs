// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::net::IpAddr;

use axum::http::HeaderMap;
use ipnetwork::IpNetwork;

/// Resolves the real client IP from the request, optionally walking
/// `X-Forwarded-For` right-to-left when a set of trusted proxies is configured.
///
/// When no trusted proxies are configured (the default) the header is
/// **never** honoured — the socket peer address is always returned.
#[derive(Clone, Debug)]
pub struct ClientIpResolver {
    trusted: Vec<IpNetwork>,
}

impl Default for ClientIpResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientIpResolver {
    /// Create a resolver that trusts no upstream proxy.
    /// `X-Forwarded-For` is always ignored.
    pub fn new() -> Self {
        Self {
            trusted: Vec::new(),
        }
    }

    /// Create a resolver from a comma- or space-separated list of CIDR
    /// notations or single IP addresses.  Invalid entries are silently
    /// skipped.
    pub fn from_env(env_value: Option<String>) -> Self {
        let trusted = env_value
            .into_iter()
            .flat_map(|val| {
                val.split([',', ' '])
                    .filter(|s| !s.is_empty())
                    .filter_map(|s| s.parse::<IpNetwork>().ok())
                    .collect::<Vec<_>>()
            })
            .collect();
        Self { trusted }
    }

    /// Resolve the real client IP.
    ///
    /// * `peer_ip` — the socket peer address (from `ConnectInfo<SocketAddr>`).
    /// * `headers` — the request headers (inspected for `X-Forwarded-For`).
    pub fn resolve(&self, peer_ip: IpAddr, headers: &HeaderMap) -> IpAddr {
        // No trusted proxies configured — never trust the header.
        if self.trusted.is_empty() {
            return peer_ip;
        }

        let Some(xff_value) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) else {
            return peer_ip;
        };

        // Walk right-to-left: the rightmost IP is the most recent hop.
        let hops: Vec<&str> = xff_value.split(',').map(|s| s.trim()).collect();

        // Find the first hop that is *not* in the trusted set.
        // If all hops are trusted, return the rightmost (the load balancer / proxy itself).
        for hop in hops.iter().rev() {
            let ip: IpAddr = match hop.parse() {
                Ok(ip) => ip,
                Err(_) => continue,
            };
            if !self.is_trusted(&ip) {
                return ip;
            }
        }

        // All hops are trusted (or all unparseable) — return the rightmost.
        hops.last().and_then(|h| h.parse().ok()).unwrap_or(peer_ip)
    }

    fn is_trusted(&self, ip: &IpAddr) -> bool {
        self.trusted.iter().any(|net| net.contains(*ip))
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use axum::http::HeaderMap;

    use super::*;

    fn headers_with_xff(xff: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", xff.parse().unwrap());
        h
    }

    fn peer_v4(octets: [u8; 4]) -> IpAddr {
        IpAddr::V4(Ipv4Addr::from(octets))
    }

    #[test]
    fn no_proxies_configured_returns_peer_ip() {
        let resolver = ClientIpResolver::new();
        let headers = headers_with_xff("1.2.3.4");
        assert_eq!(
            resolver.resolve(peer_v4([10, 0, 0, 1]), &headers),
            peer_v4([10, 0, 0, 1])
        );
    }

    #[test]
    fn no_proxies_spoofed_header_ignored() {
        let resolver = ClientIpResolver::new();
        let headers = headers_with_xff("203.0.113.99");
        assert_eq!(
            resolver.resolve(peer_v4([192, 168, 1, 1]), &headers),
            peer_v4([192, 168, 1, 1])
        );
    }

    #[test]
    fn single_trusted_proxy_uses_xff() {
        let resolver = ClientIpResolver::from_env(Some("10.0.0.0/8".to_string()));
        let headers = headers_with_xff("203.0.113.99, 10.0.0.1");
        assert_eq!(
            resolver.resolve(peer_v4([10, 0, 0, 1]), &headers),
            peer_v4([203, 0, 113, 99])
        );
    }

    #[test]
    fn multi_hop_skips_trusted_returns_first_untrusted() {
        let resolver = ClientIpResolver::from_env(Some("10.0.0.0/8, 172.16.0.0/12".to_string()));
        let headers = headers_with_xff("203.0.113.99, 10.0.0.1, 172.16.0.1");
        assert_eq!(
            resolver.resolve(peer_v4([10, 0, 0, 1]), &headers),
            peer_v4([203, 0, 113, 99])
        );
    }

    #[test]
    fn all_hops_trusted_returns_rightmost() {
        let resolver = ClientIpResolver::from_env(Some("10.0.0.0/8".to_string()));
        let headers = headers_with_xff("10.0.0.1, 10.0.0.2");
        assert_eq!(
            resolver.resolve(peer_v4([10, 0, 0, 1]), &headers),
            peer_v4([10, 0, 0, 2])
        );
    }

    #[test]
    fn no_xff_header_returns_peer() {
        let resolver = ClientIpResolver::from_env(Some("10.0.0.0/8".to_string()));
        let headers = HeaderMap::new();
        assert_eq!(
            resolver.resolve(peer_v4([10, 0, 0, 1]), &headers),
            peer_v4([10, 0, 0, 1])
        );
    }

    #[test]
    fn invalid_cidr_in_env_is_skipped() {
        let resolver = ClientIpResolver::from_env(Some("not-a-cidr, 10.0.0.0/8".to_string()));
        let headers = headers_with_xff("203.0.113.99, 10.0.0.1");
        assert_eq!(
            resolver.resolve(peer_v4([10, 0, 0, 1]), &headers),
            peer_v4([203, 0, 113, 99])
        );
    }

    #[test]
    fn empty_env_is_no_proxies() {
        let resolver = ClientIpResolver::from_env(Some("".to_string()));
        let headers = headers_with_xff("1.2.3.4");
        assert_eq!(
            resolver.resolve(peer_v4([10, 0, 0, 1]), &headers),
            peer_v4([10, 0, 0, 1])
        );
    }

    #[test]
    fn from_env_none_is_no_proxies() {
        let resolver = ClientIpResolver::from_env(None);
        let headers = headers_with_xff("1.2.3.4");
        assert_eq!(
            resolver.resolve(peer_v4([10, 0, 0, 1]), &headers),
            peer_v4([10, 0, 0, 1])
        );
    }

    #[test]
    fn multiple_trusted_cidrs() {
        let resolver = ClientIpResolver::from_env(Some("192.168.0.0/16 10.0.0.0/8".to_string()));
        let headers = headers_with_xff("203.0.113.99, 10.0.0.1, 192.168.1.1");
        assert_eq!(
            resolver.resolve(peer_v4([192, 168, 1, 1]), &headers),
            peer_v4([203, 0, 113, 99])
        );
    }

    #[test]
    fn ipv6_trusted_proxy() {
        let resolver = ClientIpResolver::from_env(Some("fd00::/8".to_string()));
        let headers = headers_with_xff("2001:db8::1, fd00::1");
        let peer = IpAddr::V6(Ipv6Addr::new(0xFD00, 0, 0, 0, 0, 0, 0, 1));
        let expected = IpAddr::V6(Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0, 1));
        assert_eq!(resolver.resolve(peer, &headers), expected);
    }
}
