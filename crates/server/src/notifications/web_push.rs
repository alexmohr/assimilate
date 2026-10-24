// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::net::SocketAddr;

use base64::Engine as _;
use reqwest::{StatusCode, Url};
use web_push_native::{
    Auth, WebPushBuilder, jwt_simple::algorithms::ES256KeyPair, p256::PublicKey,
};

use super::NotificationError;

/// The contact the VAPID signature names, which a push service may use to
/// reach whoever runs the server.
const VAPID_CONTACT: &str = "mailto:noreply@assimilate.local";

/// The most of a push service's error answer kept in the error message.
const MAX_ERROR_BODY: usize = 500;

/// Why a push service refused a push.
#[derive(Debug, thiserror::Error)]
pub enum PushRejected {
    /// `410 Gone`: the subscription has expired and takes no more pushes.
    #[error("the subscription is gone (410)")]
    Gone,
    /// Any other answer outside `2xx`.
    #[error("push service answered {status}: {body}")]
    Status {
        /// The status the push service answered with.
        status: StatusCode,
        /// The start of its answer, which usually says why.
        body: String,
    },
}

/// Encrypts `payload` for one subscription, signs the request with the
/// server's VAPID key and sends it to the push service at `url`, connecting
/// only to `addrs`, the endpoint's already validated addresses.
///
/// # Errors
///
/// Returns [`NotificationError::Config`] if a key cannot be read or the
/// request cannot be sent, and [`NotificationError::WebPush`] if the push
/// service refuses it.
pub async fn send(
    vapid_private_key: &str,
    endpoint: String,
    p256dh: String,
    auth: String,
    payload: &serde_json::Value,
    url: &Url,
    addrs: &[SocketAddr],
) -> Result<(), NotificationError> {
    let decode = |value: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(value);
    let key_pair = decode(vapid_private_key)
        .map_err(|e| e.to_string())
        .and_then(|bytes| ES256KeyPair::from_bytes(&bytes).map_err(|e| e.to_string()))
        .map_err(|e| NotificationError::Config(format!("VAPID key parse error: {e}")))?;
    let ua_public = decode(&p256dh)
        .map_err(|e| e.to_string())
        .and_then(|bytes| PublicKey::from_sec1_bytes(&bytes).map_err(|e| e.to_string()))
        .map_err(|e| {
            NotificationError::Config(format!("push subscription key parse error: {e}"))
        })?;
    let auth = decode(&auth)
        .ok()
        .filter(|bytes| bytes.len() == 16)
        .map(|bytes| Auth::clone_from_slice(&bytes))
        .ok_or_else(|| {
            NotificationError::Config(
                "push subscription auth secret is not 16 base64url bytes".to_owned(),
            )
        })?;
    let uri: axum::http::Uri = endpoint
        .parse()
        .map_err(|e| NotificationError::Config(format!("push endpoint parse error: {e}")))?;

    let message = WebPushBuilder::new(uri, ua_public, auth)
        .with_vapid(&key_pair, VAPID_CONTACT)
        .build(serde_json::to_vec(payload)?)
        .map_err(|e| NotificationError::Config(format!("web push message build error: {e}")))?;
    let request = reqwest::Request::try_from(message)
        .map_err(|e| NotificationError::Config(format!("web push request error: {e}")))?;

    let host = url
        .host_str()
        .ok_or_else(|| NotificationError::Config("push endpoint URL has no host".to_string()))?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &reachable_addrs(addrs).await)
        .build()
        .map_err(|e| NotificationError::Config(format!("web push client error: {e}")))?;

    let response = match client.execute(request).await {
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
    if status.is_success() {
        return Ok(());
    }
    if status == StatusCode::GONE {
        return Err(PushRejected::Gone.into());
    }
    let body = response.text().await.unwrap_or_default();
    Err(PushRejected::Status {
        status,
        body: body.chars().take(MAX_ERROR_BODY).collect(),
    }
    .into())
}

/// Renders a [`reqwest::Error`] together with its full error-source chain.
///
/// `reqwest::Error`'s own `Display` only says the request could not be sent;
/// the reason (refused, timed out, no route, a TLS failure, ...) sits further
/// down the chain, ending in the operating system's own error.
fn describe_transport_error(error: &reqwest::Error) -> String {
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
/// concurrently. Exists because a pinned client given every DNS-resolved address
/// (including ones from an unreachable address family) has not reliably fallen back to a
/// working one: an IPv6 candidate that fails with a *synchronous* "network unreachable"
/// (no outbound IPv6 route, as opposed to an async connect timeout) made the whole pinned
/// request fail outright instead of falling through to a working IPv4 address, against a
/// real deployment where a plain `curl` trying every candidate itself succeeded. Probing
/// first and pinning only what's verified reachable keeps that from depending on how the
/// HTTP client walks its candidates. Falls back to the full, unfiltered list
/// if nothing responds (e.g. every candidate is genuinely down or this probe itself is
/// blocked), so the real request still gets attempted and produces a normal, diagnosable
/// failure instead of silently being skipped. Single-address lists skip the probe
/// entirely -- there's no fallback candidate to select between.
///
/// Deliberate cost: unlike `probe_tcp_connect` (only reached after the real request has
/// already failed), this runs on every send to a dual-stack host, including the success
/// path. A candidate that's firewalled/silently dropped rather than synchronously refused
/// eats up to `PROBE_TIMEOUT` (5s) here before the real request is even attempted --
/// accepted in exchange for actually delivering to reachable addresses instead of failing
/// outright, per the bug this fixes.
async fn reachable_addrs(addrs: &[SocketAddr]) -> Vec<SocketAddr> {
    if addrs.len() <= 1 {
        return addrs.to_vec();
    }

    let reachable: Vec<SocketAddr> = probe_all(addrs)
        .await
        .into_iter()
        .filter_map(|(addr, result)| result.is_ok().then_some(addr))
        .collect();

    if reachable.is_empty() {
        addrs.to_vec()
    } else {
        reachable
    }
}

/// Runs a bare TCP connect against *every* pinned address used for the failed request,
/// not just one -- a dual-stack endpoint can have some addresses reachable and others not
/// (e.g. no outbound IPv6 route while IPv4 works fine), and the HTTP client's error only
/// describes the attempt it gave up on. Probing every candidate, the same way `curl -v`
/// reports each attempt, makes that split visible instead of hiding it behind whichever
/// address was tried last. Only called after the real request has
/// already failed, so this never adds latency to the success path. All probes run
/// concurrently so the total added latency is bounded by one timeout window, not
/// `addrs.len()` of them.
async fn probe_tcp_connect(addrs: &[SocketAddr]) -> String {
    if addrs.is_empty() {
        return String::new();
    }

    let descriptions: Vec<String> = probe_all(addrs)
        .await
        .into_iter()
        .map(|(addr, result)| describe_probe_outcome(addr, result))
        .collect();

    format!(" (raw TCP probe: {})", descriptions.join("; "))
}

const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Concurrently probes every address with a real TCP connect, bounded by `PROBE_TIMEOUT`
/// each. The shared "connect to every candidate at once" primitive behind both
/// `reachable_addrs` (which only needs to know which candidates are reachable) and
/// `probe_tcp_connect` (which needs a human-readable per-address outcome), so that
/// mechanic lives in one place instead of two independent `join_all` calls.
async fn probe_all(addrs: &[SocketAddr]) -> Vec<(SocketAddr, std::io::Result<()>)> {
    futures_util::future::join_all(addrs.iter().map(|&addr| async move {
        let result = probe_connect(tokio::net::TcpStream::connect(addr), PROBE_TIMEOUT).await;
        (addr, result)
    }))
    .await
}

/// Renders a single address's probe outcome from `probe_all` as a human-readable summary.
fn describe_probe_outcome(addr: SocketAddr, result: std::io::Result<()>) -> String {
    match result {
        Ok(()) => format!("{addr} connected"),
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
            format!("{addr} timed out after {PROBE_TIMEOUT:?}")
        }
        Err(e) => format!("{addr} failed: {e}"),
    }
}

/// Races `connect` against `timeout`, folding an elapsed timeout into
/// [`std::io::ErrorKind::TimedOut`] so callers see a single `io::Result` regardless of
/// which layer gave up. Generic over the connect future (rather than calling
/// `TcpStream::connect` directly) so the timeout branch -- not reliably reproducible with
/// a real socket in a fast, deterministic test -- can be exercised with an injected future
/// that simply never resolves.
async fn probe_connect<F>(connect: F, timeout: std::time::Duration) -> std::io::Result<()>
where
    F: std::future::Future<Output = std::io::Result<tokio::net::TcpStream>>,
{
    match tokio::time::timeout(timeout, connect).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("timed out after {timeout:?}"),
        )),
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    // Public example VAPID key and subscription fixture from the `web-push` crate's own
    // test suite (`vapid/builder.rs`) -- shape-valid but not tied to any real subscriber,
    // safe to use as a throwaway key/subscription for exercising the send path.
    pub(in crate::notifications) const VAPID_PRIVATE_KEY: &str =
        "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";
    pub(in crate::notifications) const P256DH: &str =
        "BH1HTeKM7-NwaLGHEqxeu2IamQaVVLkcsFHPIHmsCnqxcBHPQBprF41bEMOr3O1hUQ2jU1opNEm1F_lZV_sxMP8";
    pub(in crate::notifications) const AUTH: &str = "sBXU5_tIYz-5w7G2B25BEw";

    /// A local stand-in for a push service: answers a push to `/{path}` with the status
    /// line and body `answers` give that path, and stops after one push per entry.
    pub(in crate::notifications) async fn push_service(
        answers: &'static [(&'static str, &'static str, &'static str)],
    ) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in answers {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 4096];
                let mut received = Vec::new();
                while !received.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = socket.read(&mut buf).await.unwrap();
                    received.extend_from_slice(buf.get(..n).expect("read() length is within buf"));
                }
                let request = String::from_utf8_lossy(&received);
                let (_, status, body) = answers
                    .iter()
                    .find(|(path, _, _)| request.starts_with(&format!("POST /{path} ")))
                    .expect("a push to a path the stand-in answers");
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (addr, server)
    }

    /// Sends a push to `addr` with the fixture keys, or the given replacements.
    async fn send_to(
        addr: SocketAddr,
        vapid_private_key: &str,
        p256dh: &str,
        auth: &str,
    ) -> Result<(), NotificationError> {
        let endpoint = format!("http://127.0.0.1:{}/push", addr.port());
        let url: Url = endpoint.parse().unwrap();
        send(
            vapid_private_key,
            endpoint,
            p256dh.to_owned(),
            auth.to_owned(),
            &serde_json::json!({"title": "t"}),
            &url,
            &[addr],
        )
        .await
    }

    /// A key that cannot be read stops the push before anything is sent, with the
    /// key named in the error.
    #[tokio::test]
    async fn an_unreadable_key_is_reported_before_sending() {
        let nowhere = SocketAddr::from(([127, 0, 0, 1], 1));
        for (vapid, p256dh, auth, expected) in [
            ("not base64!", P256DH, AUTH, "VAPID key parse error"),
            (
                VAPID_PRIVATE_KEY,
                "AAAA",
                AUTH,
                "push subscription key parse error",
            ),
            (
                VAPID_PRIVATE_KEY,
                P256DH,
                "AAAA",
                "auth secret is not 16 base64url bytes",
            ),
        ] {
            let Err(NotificationError::Config(message)) =
                send_to(nowhere, vapid, p256dh, auth).await
            else {
                panic!("an unreadable key must be a configuration error");
            };
            assert!(message.contains(expected), "{message}");
        }
    }

    /// A push service that refuses a push for any reason but `410` is reported with its
    /// status and the start of its answer, so the delivery log says why.
    #[tokio::test]
    async fn a_refused_push_reports_the_status_and_the_answer() {
        let (addr, server) =
            push_service(&[("push", "500 Internal Server Error", "try later")]).await;

        let result = send_to(addr, VAPID_PRIVATE_KEY, P256DH, AUTH).await;

        server.await.unwrap();
        assert!(
            matches!(
                &result,
                Err(NotificationError::WebPush(PushRejected::Status { status, body }))
                    if *status == StatusCode::INTERNAL_SERVER_ERROR && body == "try later"
            ),
            "a 500 must be reported as a refused push with its answer, got {result:?}"
        );
    }

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
    /// error`, because the web push client of the time discarded the underlying transport
    /// error entirely. Connecting to a closed local port exercises exactly that failure mode,
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
                // The operating system's own reason ends reqwest's source chain; its
                // presence proves `describe_transport_error` walked past reqwest's generic
                // "error sending request" down to the actual cause.
                assert!(
                    msg.contains("Connection refused"),
                    "expected the underlying connect error to be included, got: {msg}"
                );
                // The raw TCP probe reports every pinned address, not only the one the
                // client gave up on.
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
    /// one address family is unreachable and another works. The client's own error only
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
    /// deterministically with a real socket, so this drives `probe_connect` directly with
    /// a connect future that never resolves.
    #[tokio::test]
    async fn probe_connect_times_out_when_connect_never_completes() {
        let never_completes = std::future::pending::<std::io::Result<tokio::net::TcpStream>>();

        let result = probe_connect(never_completes, std::time::Duration::from_millis(1)).await;

        assert_eq!(
            result
                .expect_err("a never-completing connect must time out")
                .kind(),
            std::io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn describe_probe_outcome_formats_a_timeout_distinctly_from_other_errors() {
        let addr: SocketAddr = "127.0.0.1:1".parse().unwrap();

        // The inner message is irrelevant for a TimedOut error -- only the ErrorKind is
        // inspected, and the rendered text always reports PROBE_TIMEOUT itself.
        let timeout_err = std::io::Error::new(std::io::ErrorKind::TimedOut, "ignored inner text");
        assert_eq!(
            describe_probe_outcome(addr, Err(timeout_err)),
            format!("{addr} timed out after {PROBE_TIMEOUT:?}")
        );

        let other_err = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        assert_eq!(
            describe_probe_outcome(addr, Err(other_err)),
            format!("{addr} failed: refused")
        );

        assert_eq!(
            describe_probe_outcome(addr, Ok(())),
            format!("{addr} connected")
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
