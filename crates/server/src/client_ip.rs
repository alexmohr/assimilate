// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::net::IpAddr;

use axum::http::HeaderMap;
use ipnetwork::IpNetwork;

/// Resolves the real client IP from the request, optionally walking
/// `X-Forwarded-For` right-to-left when a set of trusted proxies is configured.
///
/// When no trusted proxies are configured (the default) the header is
/// **never** honoured -- the socket peer address is always returned.
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
    /// notations or single IP addresses.  Invalid entries are logged
    /// and silently skipped.
    pub fn from_env(env_value: Option<String>) -> Self {
        let trusted = env_value
            .into_iter()
            .flat_map(|val| {
                val.split([',', ' '])
                    .filter(|s| !s.is_empty())
                    .filter_map(|s| match s.parse::<IpNetwork>() {
                        Ok(net) => Some(net),
                        Err(e) => {
                            tracing::warn!(entry = %s, error = %e, "ignoring invalid proxy CIDR");
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        Self { trusted }
    }

    /// Resolve the real client IP.
    ///
    /// * `peer_ip` -- the socket peer address (from `ConnectInfo<SocketAddr>`).
    /// * `headers` -- the request headers (inspected for `X-Forwarded-For`).
    pub fn resolve(&self, peer_ip: IpAddr, headers: &HeaderMap) -> IpAddr {
        // No trusted proxies configured -- never trust the header.
        if self.trusted.is_empty() {
            tracing::debug!(%peer_ip, "no trusted proxies configured, returning peer IP");
            return peer_ip;
        }

        // If the connecting peer is not itself a trusted proxy, do not honour
        // the X-Forwarded-For header -- this prevents external parties from
        // spoofing their IP.
        if !self.is_trusted(&peer_ip) {
            tracing::info!(%peer_ip, "peer IP is not a trusted proxy, returning peer IP");
            return peer_ip;
        }

        // Collect all X-Forwarded-For values from potentially multiple header
        // lines (RFC 7230-style duplicate headers).
        let xff_value: String = headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect::<Vec<_>>()
            .join(",");
        if xff_value.is_empty() {
            tracing::debug!(%peer_ip, "no X-Forwarded-For from trusted proxy, using peer IP");
            return peer_ip;
        }

        // Walk right-to-left: the rightmost IP is the most recent hop.
        let hops: Vec<&str> = xff_value.split(',').map(|s| s.trim()).collect();

        // Find the first hop that is *not* in the trusted set.
        // If all hops are trusted, return the rightmost (the load balancer / proxy itself).
        for hop in hops.iter().rev() {
            let ip: IpAddr = match hop.parse() {
                Ok(ip) => ip,
                Err(_) => {
                    tracing::warn!(hop = %hop, "skipping unparseable IP in X-Forwarded-For header");
                    continue;
                }
            };
            if !self.is_trusted(&ip) {
                tracing::info!(%ip, %peer_ip, "resolved client IP from X-Forwarded-For header");
                return ip;
            }
        }

        // All hops are trusted (or all unparseable) -- return the rightmost.
        let result = hops.last().and_then(|h| h.parse().ok()).unwrap_or(peer_ip);
        if result == peer_ip {
            tracing::debug!(%peer_ip, "all X-Forwarded-For hops trusted, returning peer IP");
        } else {
            tracing::info!(%result, %peer_ip, "resolved last X-Forwarded-For hop as client IP");
        }
        result
    }

    fn is_trusted(&self, ip: &IpAddr) -> bool {
        // Normalize IPv6-mapped IPv4 addresses (e.g. ::ffff:10.0.0.1 -> 10.0.0.1)
        // so CIDR rules like 10.0.0.0/8 match regardless of the wire format.
        let normalized = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(*ip, IpAddr::V4),
            _ => *ip,
        };
        self.trusted.iter().any(|net| net.contains(normalized))
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

    fn headers_with_xff_multi(values: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for v in values {
            h.append("x-forwarded-for", v.parse().unwrap());
        }
        h
    }

    fn peer_v4(octets: [u8; 4]) -> IpAddr {
        IpAddr::V4(Ipv4Addr::from(octets))
    }

    fn ipv6_mapped_v4(octets: [u8; 4]) -> IpAddr {
        IpAddr::V6(Ipv6Addr::new(
            0,
            0,
            0,
            0,
            0,
            0xFFFF,
            u16::from_be_bytes([octets[0], octets[1]]),
            u16::from_be_bytes([octets[2], octets[3]]),
        ))
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

    #[test]
    fn untrusted_peer_ignores_xff_header() {
        // trusted proxies configured, peer is NOT in trusted set,
        // X-Forwarded-For contains attacker-controlled value
        let resolver = ClientIpResolver::from_env(Some("10.0.0.0/8".to_string()));
        let headers = headers_with_xff("203.0.113.99");
        assert_eq!(
            resolver.resolve(peer_v4([192, 168, 1, 1]), &headers),
            peer_v4([192, 168, 1, 1])
        );
    }

    #[test]
    fn multiple_xff_header_lines_are_combined() {
        let resolver = ClientIpResolver::from_env(Some("10.0.0.0/8".to_string()));
        let headers = headers_with_xff_multi(&["203.0.113.99", "10.0.0.1"]);
        assert_eq!(
            resolver.resolve(peer_v4([10, 0, 0, 1]), &headers),
            peer_v4([203, 0, 113, 99])
        );
    }

    #[test]
    fn ipv6_mapped_peer_matches_ipv4_cidr() {
        // ::ffff:10.0.0.1 is the IPv6-mapped form of 10.0.0.1
        let resolver = ClientIpResolver::from_env(Some("10.0.0.0/8".to_string()));
        let headers = headers_with_xff("203.0.113.99, 10.0.0.1");
        let peer = ipv6_mapped_v4([10, 0, 0, 1]);
        assert_eq!(resolver.resolve(peer, &headers), peer_v4([203, 0, 113, 99]));
    }

    #[test]
    fn ipv6_mapped_hop_matches_ipv4_cidr() {
        let resolver = ClientIpResolver::from_env(Some("10.0.0.0/8".to_string()));
        // X-Forwarded-For contains the IPv6-mapped form of the trusted proxy
        let headers = headers_with_xff("203.0.113.99, ::ffff:10.0.0.1");
        assert_eq!(
            resolver.resolve(peer_v4([10, 0, 0, 1]), &headers),
            peer_v4([203, 0, 113, 99])
        );
    }
}
