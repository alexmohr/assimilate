// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::{ConnectInfo, Request},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use tokio::sync::Mutex;

use crate::client_ip::ClientIpResolver;

#[derive(Clone)]
pub struct RateLimiter {
    state: Arc<Mutex<RateLimiterState>>,
    max_requests: u32,
    window: Duration,
    resolver: ClientIpResolver,
}

struct RateLimiterState {
    requests: HashMap<IpAddr, Vec<Instant>>,
    last_cleanup: Instant,
}

impl RateLimiter {
    #[must_use]
    pub fn new(max_requests: u32, window: Duration, resolver: ClientIpResolver) -> Self {
        Self {
            state: Arc::new(Mutex::new(RateLimiterState {
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

        if now.duration_since(state.last_cleanup) > self.window.saturating_mul(2) {
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

pub async fn rate_limit_middleware(
    axum::extract::State(limiter): axum::extract::State<RateLimiter>,
    req: Request,
    next: Next,
) -> Response {
    let peer_ip = req
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), |ci| ci.0.ip());

    let ip = limiter.resolver.resolve(peer_ip, req.headers());

    if limiter.check(ip).await {
        next.run(req).await
    } else {
        (
            StatusCode::TOO_MANY_REQUESTS,
            "too many requests, please try again later",
        )
            .into_response()
    }
}
