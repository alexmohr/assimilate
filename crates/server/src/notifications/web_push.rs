// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::net::SocketAddr;

use isahc::{
    AsyncReadResponseExt, HttpClient,
    config::{Configurable, ResolveMap},
};
use reqwest::Url;
use web_push::{
    ContentEncoding, SubscriptionInfo, VapidSignatureBuilder, WebPushMessageBuilder,
    request_builder,
};

use super::NotificationError;

/// # Errors
///
/// Returns [`NotificationError::Config`] if the notification channel is misconfigured.
pub async fn send(
    vapid_private_key: &str,
    endpoint: String,
    p256dh: String,
    auth: String,
    payload: &serde_json::Value,
    url: &Url,
    addrs: &[SocketAddr],
) -> Result<(), NotificationError> {
    let subscription = SubscriptionInfo::new(endpoint, p256dh, auth);

    let mut sig_builder = VapidSignatureBuilder::from_base64(vapid_private_key, &subscription)
        .map_err(|e| NotificationError::Config(format!("VAPID key parse error: {e}")))?;
    sig_builder.add_claim("sub", "mailto:noreply@assimilate.local");

    let content = serde_json::to_vec(payload)?;

    let mut builder = WebPushMessageBuilder::new(&subscription);
    builder.set_payload(ContentEncoding::Aes128Gcm, &content);
    builder.set_vapid_signature(
        sig_builder
            .build()
            .map_err(|e| NotificationError::Config(format!("VAPID build error: {e}")))?,
    );

    let message = builder
        .build()
        .map_err(|e| NotificationError::Config(format!("web push message build error: {e}")))?;

    let host = url
        .host_str()
        .ok_or_else(|| NotificationError::Config("push endpoint URL has no host".to_string()))?;
    let port = url.port_or_known_default().unwrap_or(443);

    let pinned_addrs = reachable_addrs(addrs).await;
    let resolve_map = pinned_addrs.iter().fold(ResolveMap::new(), |map, addr| {
        map.add(host, port, addr.ip())
    });

    let client = HttpClient::builder()
        .redirect_policy(isahc::config::RedirectPolicy::None)
        .dns_resolve(resolve_map)
        .build()
        .map_err(|e| NotificationError::Config(format!("web push client error: {e}")))?;

    // Send the request with our own client instead of going through
    // `IsahcWebPushClient::send`: that wrapper collapses any transport-level failure
    // (DNS, TLS, connect, timeout, proxy, ...) into `WebPushError::Unspecified` with no
    // detail at all (`impl From<isahc::Error> for WebPushError` discards the error),
    // which made every non-HTTP-response failure indistinguishable and undiagnosable.
    let request = request_builder::build_request::<isahc::AsyncBody>(message);
    let mut response = match client.send_async(request).await {
        Ok(response) => response,
        Err(e) => {
            let detail = describe_transport_error(&e);
            let probe = probe_tcp_connect(addrs).await;
            return Err(NotificationError::Config(format!(
                "web push transport error: {detail}{probe}"
            )));
        }
    };

    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|e| NotificationError::Config(format!("web push response read error: {e}")))?;

    request_builder::parse_response(status, body)?;

    Ok(())
}

/// Renders an [`isahc::Error`] together with its full error-source chain.
///
/// `isahc::Error`'s own `Display` only prints its generalized [`isahc::error::ErrorKind`]
/// description (e.g. "failed to connect to the server") and omits the wrapped
/// `curl::Error`, which at least adds the curl error code (e.g. `[7] Could not connect to
/// server`). That's still the ceiling of what libcurl gives us here: the *reason* (refused,
/// timed out, no route, ...) normally lives in curl's `CURLOPT_ERRORBUFFER`-backed "extra"
/// description, which isahc never wires up -- see `probe_tcp_connect` for how that gap gets
/// filled for connection failures specifically.
fn describe_transport_error(error: &isahc::Error) -> String {
    let mut description = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(err) = source {
        description.push_str(" -> ");
        description.push_str(&err.to_string());
        source = err.source();
    }
    description
}

/// Filters `addrs` down to the ones a real TCP connect can actually reach, probed
/// concurrently. Exists because pinning every DNS-resolved address (including ones from an
/// unreachable address family) via `CURLOPT_RESOLVE` does not reliably fall back to a
/// working address the way curl's own default resolver does: an IPv6 candidate that fails
/// with a *synchronous* "network unreachable" (no outbound IPv6 route, as opposed to an
/// async connect timeout) can make the whole pinned request fail outright instead of
/// falling through to a working IPv4 address -- confirmed against a real deployment where
/// a plain `curl` with no `--resolve` override succeeded by trying every DNS-returned
/// candidate itself, while our DNS-pinned client failed completely with the exact same
/// candidate set. Probing first and pinning only what's verified reachable sidesteps
/// whatever curl-internal difference causes that. Falls back to the full, unfiltered list
/// if nothing responds (e.g. every candidate is genuinely down or this probe itself is
/// blocked), so the real request still gets attempted and produces a normal, diagnosable
/// failure instead of silently being skipped. Single-address lists skip the probe
/// entirely -- there's no fallback candidate to select between.
async fn reachable_addrs(addrs: &[SocketAddr]) -> Vec<SocketAddr> {
    if addrs.len() <= 1 {
        return addrs.to_vec();
    }

    let checked: Vec<(SocketAddr, bool)> =
        futures_util::future::join_all(addrs.iter().map(|&addr| async move {
            let reachable = matches!(
                tokio::time::timeout(PROBE_TIMEOUT, tokio::net::TcpStream::connect(addr)).await,
                Ok(Ok(_))
            );
            (addr, reachable)
        }))
        .await;

    let reachable: Vec<SocketAddr> = checked
        .into_iter()
        .filter_map(|(addr, ok)| ok.then_some(addr))
        .collect();

    if reachable.is_empty() {
        addrs.to_vec()
    } else {
        reachable
    }
}

/// Runs a bare TCP connect against *every* pinned address used for the failed isahc/curl
/// request, not just one -- a dual-stack endpoint can have some addresses reachable and
/// others not (e.g. no outbound IPv6 route while IPv4 works fine), and curl's own error
/// only ever describes its last attempt. Probing every candidate, the same way `curl -v`
/// itself reports each attempt, makes that split visible instead of hiding it behind
/// whichever address curl happened to try last. Only called after the real request has
/// already failed, so this never adds latency to the success path. All probes run
/// concurrently so the total added latency is bounded by one timeout window, not
/// `addrs.len()` of them.
async fn probe_tcp_connect(addrs: &[SocketAddr]) -> String {
    if addrs.is_empty() {
        return String::new();
    }

    let results: Vec<String> = futures_util::future::join_all(addrs.iter().map(|&addr| {
        describe_probe_result(addr, tokio::net::TcpStream::connect(addr), PROBE_TIMEOUT)
    }))
    .await;

    format!(" (raw TCP probe: {})", results.join("; "))
}

const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Races `connect` against `timeout`, rendering the outcome. Split out from
/// `probe_tcp_connect` so the timeout branch -- not reliably reproducible with a real
/// socket in a fast, deterministic test -- can be exercised with an injected connect
/// future that simply never resolves.
async fn describe_probe_result<F>(
    addr: SocketAddr,
    connect: F,
    timeout: std::time::Duration,
) -> String
where
    F: std::future::Future<Output = std::io::Result<tokio::net::TcpStream>>,
{
    match tokio::time::timeout(timeout, connect).await {
        Ok(Ok(_)) => format!("{addr} connected"),
        Ok(Err(e)) => format!("{addr} failed: {e}"),
        Err(_) => format!("{addr} timed out after {timeout:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Public example VAPID key and subscription fixture from the `web-push` crate's own
    // test suite (`vapid/builder.rs`) -- shape-valid but not tied to any real subscriber,
    // safe to use as a throwaway key/subscription for exercising the send path.
    const VAPID_PRIVATE_KEY: &str = "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";
    const P256DH: &str =
        "BH1HTeKM7-NwaLGHEqxeu2IamQaVVLkcsFHPIHmsCnqxcBHPQBprF41bEMOr3O1hUQ2jU1opNEm1F_lZV_sxMP8";
    const AUTH: &str = "sBXU5_tIYz-5w7G2B25BEw";

    /// Exercises the response-handling path (status/body read/`parse_response`) that
    /// only runs once a connection actually succeeds -- the transport-failure test above
    /// never reaches it. Spins up a bare-bones local HTTP server rather than mocking, so
    /// the real request/response round trip through our own client is what's covered.
    #[tokio::test]
    async fn send_completes_successfully_against_a_reachable_endpoint() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let mut received = Vec::new();
            while !received.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = socket.read(&mut buf).await.unwrap();
                received.extend_from_slice(buf.get(..n).expect("read() length is within buf"));
            }
            socket
                .write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });

        let endpoint = format!("http://127.0.0.1:{}/push", addr.port());
        let url: Url = endpoint.parse().unwrap();
        let addrs = vec![addr];

        let result = send(
            VAPID_PRIVATE_KEY,
            endpoint,
            P256DH.to_owned(),
            AUTH.to_owned(),
            &serde_json::json!({"title": "t"}),
            &url,
            &addrs,
        )
        .await;

        server.await.unwrap();
        result.expect("a 201 response from the push service must be treated as success");
    }

    /// Regression test for a bug where any transport-level failure (DNS, TLS,
    /// connection refused, timeout, ...) surfaced as `web push error: unspecified
    /// error`, because `IsahcWebPushClient::send` discards the underlying `isahc::Error`
    /// entirely. Connecting to a closed local port exercises exactly that failure mode,
    /// and the resulting message must retain the real cause instead of "unspecified".
    #[tokio::test]
    async fn send_reports_transport_errors_instead_of_swallowing_them() {
        let endpoint = "https://127.0.0.1:1/push";
        let url: Url = endpoint.parse().unwrap();
        let addrs = vec![SocketAddr::from(([127, 0, 0, 1], 1))];

        let result = send(
            VAPID_PRIVATE_KEY,
            endpoint.to_owned(),
            P256DH.to_owned(),
            AUTH.to_owned(),
            &serde_json::json!({"title": "t"}),
            &url,
            &addrs,
        )
        .await;

        let err = result.expect_err("connecting to a closed local port must fail");
        match err {
            NotificationError::Config(msg) => {
                assert!(
                    msg.contains("web push transport error"),
                    "expected a transport error with real detail, got: {msg}"
                );
                assert!(
                    !msg.contains("unspecified"),
                    "must not collapse to the crate's opaque Unspecified error: {msg}"
                );
                // curl::Error's Display always starts with "[<code>] <description>" -- its
                // presence proves we walked past isahc::Error's generic ErrorKind text down
                // into the actual curl-level cause (via `describe_transport_error`'s source
                // chain), not just isahc's opaque summary.
                assert!(
                    msg.contains('['),
                    "expected the underlying curl error detail to be included, got: {msg}"
                );
                // isahc/curl never surface *why* the connect failed (no CURLOPT_ERRORBUFFER
                // wiring -- see describe_transport_error's doc comment), so the raw TCP probe
                // is what actually carries the OS-level reason (e.g. "Connection refused").
                assert!(
                    msg.contains("raw TCP probe"),
                    "expected the raw TCP probe detail to be included, got: {msg}"
                );
            }
            other => panic!("expected NotificationError::Config, got: {other}"),
        }
    }

    /// The TCP probe must report a clean "connected" outcome, not a misleading failure,
    /// when a retry connect actually succeeds.
    #[tokio::test]
    async fn tcp_probe_reports_success_when_the_retry_connects() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accept = tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        let probe = probe_tcp_connect(&[addr]).await;
        accept.await.unwrap();

        assert_eq!(probe, format!(" (raw TCP probe: {addr} connected)"));
    }

    /// Exercises the actual bug this probe exists to catch: a dual-stack endpoint where
    /// one address family is unreachable and another works. curl's own error only ever
    /// describes its last attempt, so probing every candidate is what makes a partial
    /// failure like this visible instead of hidden.
    #[tokio::test]
    async fn tcp_probe_reports_every_address_not_just_the_first() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let good_addr = listener.local_addr().unwrap();
        let accept = tokio::spawn(async move {
            let _ = listener.accept().await;
        });
        let bad_addr = SocketAddr::from(([127, 0, 0, 1], 1));

        let probe = probe_tcp_connect(&[bad_addr, good_addr]).await;
        accept.await.unwrap();

        assert!(
            probe.contains(&format!("{bad_addr} failed")),
            "expected the failing address to be reported, got: {probe}"
        );
        assert!(
            probe.contains(&format!("{good_addr} connected")),
            "expected the succeeding address to also be reported, got: {probe}"
        );
    }

    #[tokio::test]
    async fn tcp_probe_reports_nothing_for_an_empty_address_list() {
        assert_eq!(probe_tcp_connect(&[]).await, "");
    }

    /// A real connect attempt that genuinely hangs (a firewall silently dropping SYN
    /// packets, an unreachable-but-not-yet-rejected route) isn't reproducible fast or
    /// deterministically with a real socket, so this drives `describe_probe_result`
    /// directly with a connect future that never resolves.
    #[tokio::test]
    async fn probe_reports_a_timeout_when_connect_never_completes() {
        let addr: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let never_completes = std::future::pending::<std::io::Result<tokio::net::TcpStream>>();

        let probe =
            describe_probe_result(addr, never_completes, std::time::Duration::from_millis(1)).await;

        assert!(
            probe.contains("timed out"),
            "expected a timeout description, got: {probe}"
        );
    }

    /// The exact bug this exists to fix: given a mix of reachable and unreachable
    /// candidates (e.g. working IPv4 alongside IPv6 with no outbound route), only the
    /// reachable one(s) should end up pinned.
    #[tokio::test]
    async fn reachable_addrs_filters_out_unreachable_candidates() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let good_addr = listener.local_addr().unwrap();
        let accept = tokio::spawn(async move {
            let _ = listener.accept().await;
        });
        let bad_addr = SocketAddr::from(([127, 0, 0, 1], 1));

        let result = reachable_addrs(&[bad_addr, good_addr]).await;
        accept.await.unwrap();

        assert_eq!(result, vec![good_addr]);
    }

    /// If every candidate is genuinely unreachable, fall back to the full list instead of
    /// pinning an empty resolve map -- the real request still gets attempted (and produces
    /// a normal, diagnosable failure) rather than being silently skipped.
    #[tokio::test]
    async fn reachable_addrs_falls_back_to_full_list_when_nothing_responds() {
        let addrs = vec![
            SocketAddr::from(([127, 0, 0, 1], 1)),
            SocketAddr::from(([127, 0, 0, 1], 2)),
        ];

        let result = reachable_addrs(&addrs).await;

        assert_eq!(result, addrs);
    }

    /// A single-address list has no fallback candidate to select between, so the probe is
    /// skipped entirely and the address is returned as-is, reachable or not.
    #[tokio::test]
    async fn reachable_addrs_skips_the_probe_for_a_single_address() {
        let addr = SocketAddr::from(([127, 0, 0, 1], 1));
        assert_eq!(reachable_addrs(&[addr]).await, vec![addr]);
    }

    #[tokio::test]
    async fn reachable_addrs_returns_empty_for_an_empty_list() {
        assert_eq!(reachable_addrs(&[]).await, Vec::<SocketAddr>::new());
    }
}
