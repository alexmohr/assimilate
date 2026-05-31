// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{extract::Request, middleware::Next, response::Response};

const CSP_VALUE: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                         img-src 'self' data:; connect-src 'self' ws: wss:; font-src 'self'; \
                         frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

const CSP_VALUE_APIDOCS: &str = "default-src 'self'; \
     script-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; \
     style-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; \
     img-src 'self' data: blob:; connect-src 'self' ws: wss:; \
     font-src 'self' https://cdn.jsdelivr.net; \
     frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

pub async fn csp_headers(request: Request, next: Next) -> Response {
    let is_apidocs = request.uri().path().starts_with("/api/docs");
    let mut response = next.run(request).await;
    let csp = if is_apidocs {
        CSP_VALUE_APIDOCS
    } else {
        CSP_VALUE
    };
    response.headers_mut().insert(
        axum::http::header::CONTENT_SECURITY_POLICY,
        axum::http::HeaderValue::from_static(csp),
    );
    response
}
