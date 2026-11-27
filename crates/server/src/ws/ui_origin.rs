// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Which web origins may open the cookie-authenticated UI WebSocket.
//!
//! Browsers attach the session cookie to a WebSocket handshake from any page
//! on the same *site* (a sibling subdomain, or another port on the same host)
//! and a WebSocket has no CORS preflight, so the cookie alone would let such a
//! page read the live event stream. The browser's `Origin` header names the
//! page that opened the socket; only the server's own origin, or one listed in
//! [`ALLOWED_ORIGINS_ENV`], may open it.

use std::{net::IpAddr, str::FromStr};

use axum::http::{
    HeaderMap, HeaderName, HeaderValue, Uri, header,
    uri::{Authority, Scheme},
};

use crate::client_ip::ClientIpResolver;

/// The environment variable listing extra origins allowed to open the UI socket.
pub const ALLOWED_ORIGINS_ENV: &str = "ASSIMILATE_ALLOWED_ORIGINS";

const X_FORWARDED_HOST: HeaderName = HeaderName::from_static("x-forwarded-host");
const X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");

/// Why a string is not a web origin.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OriginParseError {
    /// Not `scheme://host[:port]`.
    #[error("not an origin of the form scheme://host[:port]")]
    Malformed,
    /// A scheme other than `http` or `https`.
    #[error("unsupported scheme (expected http or https)")]
    UnsupportedScheme,
}

/// The scheme of a web origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginScheme {
    /// `http`.
    Http,
    /// `https`.
    Https,
}

impl OriginScheme {
    const fn default_port(self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
        }
    }
}

impl TryFrom<&Scheme> for OriginScheme {
    type Error = OriginParseError;

    fn try_from(scheme: &Scheme) -> Result<Self, Self::Error> {
        if *scheme == Scheme::HTTP {
            Ok(Self::Http)
        } else if *scheme == Scheme::HTTPS {
            Ok(Self::Https)
        } else {
            Err(OriginParseError::UnsupportedScheme)
        }
    }
}

impl FromStr for OriginScheme {
    type Err = OriginParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("http") {
            Ok(Self::Http)
        } else if s.eq_ignore_ascii_case("https") {
            Ok(Self::Https)
        } else {
            Err(OriginParseError::UnsupportedScheme)
        }
    }
}

/// A host name or address as it appears in an origin or a `Host` header,
/// lowercased (an IPv6 address keeps its brackets).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Host(String);

impl From<&Authority> for Host {
    fn from(authority: &Authority) -> Self {
        Self(authority.host().to_ascii_lowercase())
    }
}

/// A web origin: scheme, host and port, with the scheme's default port filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebOrigin {
    scheme: OriginScheme,
    host: Host,
    port: u16,
}

impl std::fmt::Display for WebOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let scheme = match self.scheme {
            OriginScheme::Http => "http",
            OriginScheme::Https => "https",
        };
        write!(f, "{scheme}://{}:{}", self.host.0, self.port)
    }
}

impl FromStr for WebOrigin {
    type Err = OriginParseError;

    /// Parses `scheme://host[:port]`. A single trailing `/` is tolerated, so
    /// an origin copied from the address bar parses; any other path, a query
    /// or user info is refused.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let s = s.strip_suffix('/').unwrap_or(s);
        let uri: Uri = s.parse().map_err(|_| OriginParseError::Malformed)?;
        let (Some(raw_scheme), Some(authority)) = (uri.scheme(), uri.authority()) else {
            return Err(OriginParseError::Malformed);
        };
        let scheme = OriginScheme::try_from(raw_scheme)?;
        // `Uri` reports an absolute URI without a path as path `/`, so a path
        // or query shows only as text beyond `scheme://authority`.
        let bare = format!("{raw_scheme}://{authority}");
        if !s.eq_ignore_ascii_case(&bare) || authority.as_str().contains('@') {
            return Err(OriginParseError::Malformed);
        }
        Ok(Self {
            scheme,
            host: Host::from(authority),
            port: authority
                .port_u16()
                .unwrap_or_else(|| scheme.default_port()),
        })
    }
}

/// The origin the request was addressed to: the host and port the browser
/// connected to, and the scheme when it is known.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ServedOrigin {
    /// Known only from a trusted proxy's `X-Forwarded-Proto`: the server
    /// itself speaks plain HTTP, whether or not a TLS proxy sits in front.
    scheme: Option<OriginScheme>,
    host: Host,
    /// The port the `Host` header names, if it names one; without one, the
    /// browser connected on its scheme's default port.
    port: Option<u16>,
}

impl ServedOrigin {
    /// Reads the origin from `X-Forwarded-Host`/`X-Forwarded-Proto` when they
    /// come from a trusted proxy, otherwise from `Host` (or, for an HTTP/2
    /// request, the request target's authority).
    fn of_request(
        headers: &HeaderMap,
        uri_authority: Option<&Authority>,
        forwarded_by_trusted_proxy: bool,
    ) -> Option<Self> {
        let forwarded = |name: &HeaderName| {
            forwarded_by_trusted_proxy
                .then(|| first_value(headers, name))
                .flatten()
        };
        let scheme = forwarded(&X_FORWARDED_PROTO).and_then(|v| v.parse().ok());
        let authority = forwarded(&X_FORWARDED_HOST)
            .or_else(|| first_value(headers, &header::HOST))
            .map_or_else(
                || uri_authority.cloned(),
                |host| host.parse::<Authority>().ok(),
            )?;
        if authority.as_str().contains('@') {
            return None;
        }
        Some(Self {
            scheme,
            host: Host::from(&authority),
            port: authority.port_u16(),
        })
    }

    fn matches(&self, origin: &WebOrigin) -> bool {
        let scheme = self.scheme.unwrap_or(origin.scheme);
        let port = self.port.unwrap_or_else(|| scheme.default_port());
        scheme == origin.scheme && self.host == origin.host && port == origin.port
    }
}

/// The first comma-separated value of `name`: the one the client-facing
/// proxy set, for a header each hop appends to.
fn first_value<'h>(headers: &'h HeaderMap, name: &HeaderName) -> Option<&'h str> {
    headers
        .get(name)?
        .to_str()
        .ok()?
        .split(',')
        .map(str::trim)
        .find(|v| !v.is_empty())
}

/// The outcome of checking a UI socket handshake's `Origin`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginCheck {
    /// The server's own origin, or one in [`ALLOWED_ORIGINS_ENV`].
    Allowed,
    /// No `Origin` header. Browsers always send one on a WebSocket
    /// handshake, so this is not a browser.
    Missing,
    /// Another origin, or a header that is not an origin (`null` from a
    /// sandboxed frame).
    Refused(HeaderValue),
}

/// The origins the UI socket accepts besides the server's own.
#[derive(Debug, Clone, Default)]
pub struct UiOriginPolicy {
    extra: Vec<WebOrigin>,
}

impl UiOriginPolicy {
    /// Builds the policy from the value of [`ALLOWED_ORIGINS_ENV`]: a comma-
    /// or space-separated list of `scheme://host[:port]` origins. Invalid
    /// entries are logged and skipped, the same as `ASSIMILATE_TRUSTED_PROXIES`.
    #[must_use]
    pub fn from_env_value(env_value: Option<&str>) -> Self {
        let extra = env_value
            .into_iter()
            .flat_map(|val| val.split([',', ' ']))
            .filter(|s| !s.is_empty())
            .filter_map(|s| match s.parse::<WebOrigin>() {
                Ok(origin) => Some(origin),
                Err(e) => {
                    tracing::warn!(entry = %s, error = %e, "ignoring invalid allowed origin");
                    None
                }
            })
            .collect();
        Self { extra }
    }

    /// Reads [`ALLOWED_ORIGINS_ENV`] from the process environment.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_env_value(std::env::var(ALLOWED_ORIGINS_ENV).ok().as_deref())
    }

    /// Checks the handshake's `Origin` against the origin it was addressed to
    /// and the extra allowed origins. `peer` is the socket peer; forwarded
    /// headers count only when `proxies` trusts it.
    #[must_use]
    pub fn check(
        &self,
        headers: &HeaderMap,
        uri_authority: Option<&Authority>,
        peer: Option<IpAddr>,
        proxies: &ClientIpResolver,
    ) -> OriginCheck {
        let Some(value) = headers.get(header::ORIGIN) else {
            return OriginCheck::Missing;
        };
        let Some(origin) = value
            .to_str()
            .ok()
            .and_then(|v| v.parse::<WebOrigin>().ok())
        else {
            return OriginCheck::Refused(value.clone());
        };
        let trusted = peer.is_some_and(|peer| proxies.trusts(peer));
        let same_origin = ServedOrigin::of_request(headers, uri_authority, trusted)
            .is_some_and(|served| served.matches(&origin));
        if same_origin || self.extra.contains(&origin) {
            OriginCheck::Allowed
        } else {
            OriginCheck::Refused(value.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    const PROXY: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5));
    const STRANGER: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7));

    fn headers(pairs: &[(HeaderName, &str)]) -> HeaderMap {
        pairs
            .iter()
            .map(|(name, value)| (name.clone(), value.parse().unwrap()))
            .collect()
    }

    fn check(policy: &UiOriginPolicy, pairs: &[(HeaderName, &str)], peer: IpAddr) -> OriginCheck {
        let proxies = ClientIpResolver::from_env(Some("10.0.0.0/8".to_string()));
        policy.check(&headers(pairs), None, Some(peer), &proxies)
    }

    fn allowed(pairs: &[(HeaderName, &str)]) -> bool {
        check(&UiOriginPolicy::default(), pairs, STRANGER) == OriginCheck::Allowed
    }

    #[test]
    fn parses_origins_with_and_without_a_port() {
        let origin: WebOrigin = "HTTPS://Backup.Example.com".parse().unwrap();
        assert_eq!(origin.to_string(), "https://backup.example.com:443");
        let origin: WebOrigin = "http://[::1]:8080/".parse().unwrap();
        assert_eq!(origin.to_string(), "http://[::1]:8080");
    }

    #[test]
    fn refuses_what_is_not_an_origin() {
        for value in [
            "null",
            "backup.example.com",
            "https://backup.example.com/path",
            "https://backup.example.com?q=1",
            "https://user@backup.example.com",
            "",
        ] {
            assert_eq!(
                value.parse::<WebOrigin>(),
                Err(OriginParseError::Malformed),
                "{value}"
            );
        }
        assert_eq!(
            "ftp://backup.example.com".parse::<WebOrigin>(),
            Err(OriginParseError::UnsupportedScheme)
        );
    }

    #[test]
    fn same_host_and_port_is_allowed() {
        assert!(allowed(&[
            (header::HOST, "backup.example.com"),
            (header::ORIGIN, "https://backup.example.com"),
        ]));
        assert!(allowed(&[
            (header::HOST, "localhost:8080"),
            (header::ORIGIN, "http://localhost:8080"),
        ]));
        assert!(allowed(&[
            (header::HOST, "Backup.Example.com:443"),
            (header::ORIGIN, "https://backup.example.com"),
        ]));
    }

    #[test]
    fn another_site_subdomain_or_port_is_refused() {
        for origin in [
            "https://evil.example.com",
            "https://backup.example.com.evil.test",
            "https://backup.example.com:8443",
            "http://backup.example.com:3000",
            "null",
        ] {
            assert!(
                !allowed(&[
                    (header::HOST, "backup.example.com"),
                    (header::ORIGIN, origin)
                ]),
                "{origin}"
            );
        }
    }

    #[test]
    fn a_missing_origin_is_reported_as_missing() {
        assert_eq!(
            check(
                &UiOriginPolicy::default(),
                &[(header::HOST, "backup.example.com")],
                STRANGER
            ),
            OriginCheck::Missing
        );
    }

    #[test]
    fn a_missing_host_falls_back_to_the_request_authority() {
        let authority: Authority = "backup.example.com".parse().unwrap();
        let check = UiOriginPolicy::default().check(
            &headers(&[(header::ORIGIN, "https://backup.example.com")]),
            Some(&authority),
            None,
            &ClientIpResolver::new(),
        );
        assert_eq!(check, OriginCheck::Allowed);
    }

    #[test]
    fn forwarded_headers_count_only_from_a_trusted_proxy() {
        let pairs = [
            (header::HOST, "assimilate:8080"),
            (X_FORWARDED_HOST, "backup.example.com"),
            (X_FORWARDED_PROTO, "https"),
            (header::ORIGIN, "https://backup.example.com"),
        ];
        let policy = UiOriginPolicy::default();
        assert_eq!(check(&policy, &pairs, PROXY), OriginCheck::Allowed);
        assert_ne!(check(&policy, &pairs, STRANGER), OriginCheck::Allowed);
    }

    #[test]
    fn a_trusted_proxy_scheme_must_match() {
        let pairs = [
            (header::HOST, "assimilate:8080"),
            (X_FORWARDED_HOST, "backup.example.com, inner.proxy"),
            (X_FORWARDED_PROTO, "https"),
            (header::ORIGIN, "http://backup.example.com"),
        ];
        assert_ne!(
            check(&UiOriginPolicy::default(), &pairs, PROXY),
            OriginCheck::Allowed
        );
    }

    #[test]
    fn a_trusted_proxy_can_report_plain_http() {
        let pairs = [
            (header::HOST, "assimilate:8080"),
            (X_FORWARDED_HOST, "backup.lan"),
            (X_FORWARDED_PROTO, "HTTP"),
            (header::ORIGIN, "http://backup.lan"),
        ];
        assert_eq!(
            check(&UiOriginPolicy::default(), &pairs, PROXY),
            OriginCheck::Allowed
        );
    }

    /// A forwarded scheme that is neither `http` nor `https` says nothing
    /// about the scheme, so only host and port are compared.
    #[test]
    fn an_unknown_forwarded_scheme_leaves_the_scheme_unknown() {
        assert_eq!(
            "gopher".parse::<OriginScheme>(),
            Err(OriginParseError::UnsupportedScheme)
        );
        let pairs = [
            (header::HOST, "assimilate:8080"),
            (X_FORWARDED_HOST, "backup.example.com"),
            (X_FORWARDED_PROTO, "gopher"),
            (header::ORIGIN, "https://backup.example.com"),
        ];
        assert_eq!(
            check(&UiOriginPolicy::default(), &pairs, PROXY),
            OriginCheck::Allowed
        );
    }

    /// Without a usable address to compare with, no origin is the server's
    /// own: a `Host` that is not an authority, one with user info, or none.
    #[test]
    fn an_unusable_host_refuses_every_origin() {
        for host in ["not a host", "user@backup.example.com"] {
            assert_ne!(
                check(
                    &UiOriginPolicy::default(),
                    &[
                        (header::HOST, host),
                        (header::ORIGIN, "https://backup.example.com")
                    ],
                    STRANGER
                ),
                OriginCheck::Allowed,
                "{host}"
            );
        }
        assert_ne!(
            check(
                &UiOriginPolicy::default(),
                &[(header::ORIGIN, "https://backup.example.com")],
                STRANGER
            ),
            OriginCheck::Allowed
        );
    }

    #[test]
    fn an_untrusted_peer_cannot_forge_the_host() {
        let pairs = [
            (header::HOST, "backup.example.com"),
            (X_FORWARDED_HOST, "evil.example.com"),
            (header::ORIGIN, "https://evil.example.com"),
        ];
        assert_ne!(
            check(&UiOriginPolicy::default(), &pairs, STRANGER),
            OriginCheck::Allowed
        );
    }

    #[test]
    fn configured_extra_origins_are_allowed() {
        let policy = UiOriginPolicy::from_env_value(Some(
            "https://ui.example.com/, not-an-origin http://localhost:5173",
        ));
        assert_eq!(policy.extra.len(), 2);
        let pairs = [
            (header::HOST, "backup.example.com"),
            (header::ORIGIN, "http://localhost:5173"),
        ];
        assert_eq!(check(&policy, &pairs, STRANGER), OriginCheck::Allowed);
        let pairs = [
            (header::HOST, "backup.example.com"),
            (header::ORIGIN, "https://ui.example.com"),
        ];
        assert_eq!(check(&policy, &pairs, STRANGER), OriginCheck::Allowed);
    }

    #[test]
    fn no_configured_origins_means_none_extra() {
        assert_eq!(UiOriginPolicy::from_env_value(None).extra, []);
        assert_eq!(UiOriginPolicy::from_env_value(Some("")).extra, []);
    }
}
