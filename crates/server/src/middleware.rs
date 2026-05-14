// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{extract::Request, middleware::Next, response::Response};

const CSP_VALUE: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                         img-src 'self' data:; connect-src 'self' ws: wss:; font-src 'self'; \
                         frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

pub async fn csp_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        axum::http::header::CONTENT_SECURITY_POLICY,
        axum::http::HeaderValue::from_static(CSP_VALUE),
    );
    response
}
