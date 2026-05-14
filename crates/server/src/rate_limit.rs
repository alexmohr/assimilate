// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct RateLimiter {
    state: Arc<Mutex<RateLimiterState>>,
    max_requests: u32,
    window: Duration,
}

struct RateLimiterState {
    requests: HashMap<IpAddr, Vec<Instant>>,
    last_cleanup: Instant,
}

impl RateLimiter {
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(RateLimiterState {
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

fn extract_client_ip(req: &Request) -> Option<IpAddr> {
    req.headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.trim().parse().ok())
        .or_else(|| {
            req.extensions()
                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                .map(|ci| ci.0.ip())
        })
}

pub async fn rate_limit_middleware(
    axum::extract::State(limiter): axum::extract::State<RateLimiter>,
    req: Request,
    next: Next,
) -> Response {
    let ip = extract_client_ip(&req).unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));

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
