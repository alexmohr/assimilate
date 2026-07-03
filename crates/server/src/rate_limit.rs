// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
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
    resolver: ClientIpResolver,
}

struct IpRateLimiterState {
    requests: HashMap<IpAddr, Vec<Instant>>,
    last_cleanup: Instant,
}

impl IpRateLimiter {
    pub fn new(max_requests: u32, window: Duration, resolver: ClientIpResolver) -> Self {
        Self {
            state: Arc::new(Mutex::new(IpRateLimiterState {
                requests: HashMap::new(),
                last_cleanup: Instant::now(),
            })),
            max_requests,
            window,
            resolver,
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

/// Wraps the auth extractor to populate request extensions with the authenticated
/// user's ID so downstream middleware (e.g. rate limiting) can read it.
pub async fn auth_tracking_middleware(
    axum::extract::State(state): axum::extract::State<crate::AppState>,
    req: Request,
    next: Next,
) -> Response {
    // Try to extract AuthUser — this duplicates the extraction that the handler
    // will do, but it's cheap (reads headers + one DB lookup, cached by sqlx).
    let (mut parts, body) = req.into_parts();
    let auth_user = crate::api::auth::AuthUser::from_request_parts(&mut parts, &state)
        .await
        .ok();
    let req = Request::from_parts(parts, body);

    if let Some(user) = auth_user {
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
