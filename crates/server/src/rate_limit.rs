// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::{ConnectInfo, FromRequestParts, Request},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use tokio::sync::Mutex;

use crate::client_ip::ClientIpResolver;

#[derive(Clone)]
pub struct IpRateLimiter {
    state: Arc<Mutex<IpRateLimiterState>>,
    max_requests: u32,
    window: Duration,
}

struct IpRateLimiterState {
    requests: HashMap<IpAddr, Vec<Instant>>,
    last_cleanup: Instant,
}

impl IpRateLimiter {
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(IpRateLimiterState {
                requests: HashMap::new(),
                last_cleanup: Instant::now(),
            })),
            max_requests,
            window,
        }
    }

    async fn check(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut state = self.state.lock().await;

        if now.duration_since(state.last_cleanup) > self.window * 2 {
            state.requests.retain(|_, timestamps| {
                timestamps
                    .iter()
                    .any(|t| now.duration_since(*t) < self.window)
            });
            state.last_cleanup = now;
        }

        let timestamps = state.requests.entry(ip).or_default();
        timestamps.retain(|t| now.duration_since(*t) < self.window);

        if u32::try_from(timestamps.len()).unwrap_or(u32::MAX) >= self.max_requests {
            return false;
        }

        timestamps.push(now);
        true
    }
}

#[derive(Clone)]
pub struct IpRateLimitMiddlewareState {
    pub limiter: IpRateLimiter,
    pub resolver: ClientIpResolver,
}

pub async fn ip_rate_limit_middleware(
    axum::extract::State(state): axum::extract::State<IpRateLimitMiddlewareState>,
    req: Request,
    next: Next,
) -> Response {
    let peer_ip = req
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), |ci| ci.0.ip());

    let ip = state.resolver.resolve(peer_ip, req.headers());
    if state.limiter.check(ip).await {
        next.run(req).await
    } else {
        (
            StatusCode::TOO_MANY_REQUESTS,
            "too many requests, please try again later",
        )
            .into_response()
    }
}

/// Per-user sliding-window rate limiter for mutating / expensive endpoints.
#[derive(Clone)]
pub struct UserRateLimiter {
    state: Arc<Mutex<UserRateLimiterState>>,
    max_requests: u32,
    window: Duration,
}

struct UserRateLimiterState {
    requests: HashMap<i64, Vec<Instant>>,
    last_cleanup: Instant,
}

impl UserRateLimiter {
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(UserRateLimiterState {
                requests: HashMap::new(),
                last_cleanup: Instant::now(),
            })),
            max_requests,
            window,
        }
    }

    async fn check(&self, user_id: i64) -> bool {
        let now = Instant::now();
        let mut state = self.state.lock().await;

        if now.duration_since(state.last_cleanup) > self.window * 2 {
            state.requests.retain(|_, timestamps| {
                timestamps
                    .iter()
                    .any(|t| now.duration_since(*t) < self.window)
            });
            state.last_cleanup = now;
        }

        let timestamps = state.requests.entry(user_id).or_default();
        timestamps.retain(|t| now.duration_since(*t) < self.window);

        if u32::try_from(timestamps.len()).unwrap_or(u32::MAX) >= self.max_requests {
            return false;
        }

        timestamps.push(now);
        true
    }
}

/// Wraps the auth extractor to populate request extensions with the
/// authenticated user so the handler's own extractor can reuse it,
/// and to apply per-user rate limiting to mutating requests.
pub async fn auth_tracking_middleware(
    axum::extract::State(state): axum::extract::State<crate::AppState>,
    req: Request,
    next: Next,
) -> Response {
    // Extract AuthUser and stash it in request extensions so the handler's
    // own `AuthUser` extractor can reuse it instead of doing a second DB
    // round trip.
    let (mut parts, body) = req.into_parts();
    let auth_user = crate::api::auth::AuthUser::from_request_parts(&mut parts, &state)
        .await
        .ok();
    if let Some(ref user) = auth_user {
        parts.extensions.insert(user.clone());
    }
    let req = Request::from_parts(parts, body);

    // Only rate-limit mutating requests (POST, PUT, PATCH, DELETE).
    // Reads (GET, HEAD, OPTIONS) are not throttled so E2E test suites
    // and UI polling are not blocked.
    let is_mutating = matches!(
        req.method(),
        &axum::http::Method::POST
            | &axum::http::Method::PUT
            | &axum::http::Method::PATCH
            | &axum::http::Method::DELETE
    );

    if let Some(user) = auth_user
        && is_mutating
    {
        let user_id = user.user_id;
        if state.user_rate_limiter.check(user_id).await {
            next.run(req).await
        } else {
            (
                StatusCode::TOO_MANY_REQUESTS,
                "too many requests, please try again later",
            )
                .into_response()
        }
    } else {
        next.run(req).await
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    #[tokio::test]
    async fn ip_rate_limiter_accepts_first_request() {
        let limiter = IpRateLimiter::new(5, Duration::from_secs(60));
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        assert!(limiter.check(ip).await);
    }

    #[tokio::test]
    async fn ip_rate_limiter_rejects_excess_requests() {
        let limiter = IpRateLimiter::new(2, Duration::from_secs(60));
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        assert!(limiter.check(ip).await);
        assert!(limiter.check(ip).await);
        assert!(!limiter.check(ip).await);
    }

    #[tokio::test]
    async fn ip_rate_limiter_allows_different_ips() {
        let limiter = IpRateLimiter::new(2, Duration::from_secs(60));
        let ip1 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        assert!(limiter.check(ip1).await);
        assert!(limiter.check(ip1).await);
        assert!(limiter.check(ip2).await);
        assert!(!limiter.check(ip1).await);
    }

    #[tokio::test]
    async fn user_rate_limiter_accepts_first_request() {
        let limiter = UserRateLimiter::new(5, Duration::from_secs(60));
        assert!(limiter.check(42).await);
    }

    #[tokio::test]
    async fn user_rate_limiter_rejects_excess_requests() {
        let limiter = UserRateLimiter::new(2, Duration::from_secs(60));
        assert!(limiter.check(1).await);
        assert!(limiter.check(1).await);
        assert!(!limiter.check(1).await);
    }

    #[tokio::test]
    async fn user_rate_limiter_allows_different_users() {
        let limiter = UserRateLimiter::new(2, Duration::from_secs(60));
        assert!(limiter.check(10).await);
        assert!(limiter.check(10).await);
        assert!(limiter.check(20).await);
        assert!(!limiter.check(10).await);
    }
}
