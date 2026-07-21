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

    let resolve_map = addrs.iter().fold(ResolveMap::new(), |map, addr| {
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
    let mut response = client.send_async(request).await.map_err(|e| {
        NotificationError::Config(format!(
            "web push transport error: {}",
            describe_transport_error(&e)
        ))
    })?;

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
/// `curl::Error`, which is where the actually useful detail lives -- the curl error code,
/// the attempted address, and *why* the connection failed (refused, timed out, no route,
/// TLS handshake failure, ...). Without walking `source()`, every transport failure looks
/// identical and gives an operator nothing to act on.
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
            }
            other => panic!("expected NotificationError::Config, got: {other}"),
        }
    }
}
