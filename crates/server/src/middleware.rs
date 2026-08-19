// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    extract::Request,
    http::{HeaderValue, header},
    middleware::Next,
    response::Response,
};

use crate::cookies::CookieSecurity;

const CSP_VALUE: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                         img-src 'self' data:; connect-src 'self'; font-src 'self'; \
                         frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

const CSP_VALUE_APIDOCS: &str = "default-src 'self'; \
     script-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; \
     style-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; \
     img-src 'self' data: blob:; connect-src 'self'; \
     font-src 'self' https://cdn.jsdelivr.net; \
     frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

const X_CONTENT_TYPE_OPTIONS_VALUE: HeaderValue = HeaderValue::from_static("nosniff");
const REFERRER_POLICY_VALUE: HeaderValue = HeaderValue::from_static("no-referrer");
const PERMISSIONS_POLICY_VALUE: HeaderValue =
    HeaderValue::from_static("geolocation=(), camera=(), microphone=()");
const HSTS_VALUE: HeaderValue = HeaderValue::from_static("max-age=63072000; includeSubDomains");

/// Axum middleware that injects standard security-hardening headers into every response:
/// `Content-Security-Policy`, `X-Content-Type-Options`, `Referrer-Policy`, `Permissions-Policy`,
/// and (when `ASSIMILATE_SECURE_COOKIES` is enabled, indicating a TLS deployment)
/// `Strict-Transport-Security`.
pub async fn security_headers(request: Request, next: Next) -> Response {
    let is_apidocs = request.uri().path().starts_with("/api/docs");
    let mut response = next.run(request).await;
    let csp = if is_apidocs {
        CSP_VALUE_APIDOCS
    } else {
        CSP_VALUE
    };
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(csp),
    );
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, X_CONTENT_TYPE_OPTIONS_VALUE);
    headers.insert(header::REFERRER_POLICY, REFERRER_POLICY_VALUE);
    headers.insert("Permissions-Policy", PERMISSIONS_POLICY_VALUE);
    if matches!(
        CookieSecurity::from(std::env::var("ASSIMILATE_SECURE_COOKIES").ok()),
        CookieSecurity::Secure
    ) {
        headers.insert(header::STRICT_TRANSPORT_SECURITY, HSTS_VALUE);
    }
    response
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use axum::{Router, body::Body, http::Request, routing::get};
    use tokio::sync::Mutex;
    use tower::ServiceExt as _;

    use super::*;

    /// Serializes tests that mutate the process-global `ASSIMILATE_SECURE_COOKIES`
    /// env var, since `cargo test` runs tests within a binary concurrently.
    fn secure_cookies_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn app() -> Router {
        Router::new()
            .route("/", get(|| async { "ok" }))
            .route("/api/docs", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(security_headers))
    }

    async fn get_headers(uri: &str) -> axum::http::HeaderMap {
        let response = app()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        response.headers().clone()
    }

    #[tokio::test]
    async fn sets_content_type_options_header() {
        let headers = get_headers("/").await;
        assert_eq!(
            headers.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
            "nosniff"
        );
    }

    #[tokio::test]
    async fn sets_referrer_policy_header() {
        let headers = get_headers("/").await;
        assert_eq!(headers.get(header::REFERRER_POLICY).unwrap(), "no-referrer");
    }

    #[tokio::test]
    async fn sets_permissions_policy_header() {
        let headers = get_headers("/").await;
        assert_eq!(
            headers.get("Permissions-Policy").unwrap(),
            "geolocation=(), camera=(), microphone=()"
        );
    }

    #[tokio::test]
    async fn csp_connect_src_does_not_allow_arbitrary_websocket_hosts() {
        let headers = get_headers("/").await;
        let csp = headers
            .get(header::CONTENT_SECURITY_POLICY)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(csp.contains("connect-src 'self'"));
        assert!(!csp.contains("connect-src 'self' ws:"));
    }

    #[tokio::test]
    async fn apidocs_csp_also_tightens_connect_src() {
        let headers = get_headers("/api/docs").await;
        let csp = headers
            .get(header::CONTENT_SECURITY_POLICY)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(csp.contains("connect-src 'self'"));
        assert!(!csp.contains("ws:"));
        assert!(!csp.contains("wss:"));
    }

    #[tokio::test]
    async fn hsts_present_by_default_secure_fail_safe() {
        let _guard = secure_cookies_env_lock().lock().await;
        // SAFETY: serialized by secure_cookies_env_lock; std::env::set_var is
        // unsafe in edition 2024 because it is process-global.
        unsafe { std::env::remove_var("ASSIMILATE_SECURE_COOKIES") };
        let headers = get_headers("/").await;
        assert!(headers.get(header::STRICT_TRANSPORT_SECURITY).is_some());
    }

    #[tokio::test]
    async fn hsts_absent_when_secure_cookies_explicitly_disabled() {
        let _guard = secure_cookies_env_lock().lock().await;
        // SAFETY: see hsts_present_by_default_secure_fail_safe.
        unsafe { std::env::set_var("ASSIMILATE_SECURE_COOKIES", "false") };
        let headers = get_headers("/").await;
        unsafe { std::env::remove_var("ASSIMILATE_SECURE_COOKIES") };
        assert!(headers.get(header::STRICT_TRANSPORT_SECURITY).is_none());
    }
}
