// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The Prometheus `/metrics` endpoint's data: what it collects, how it renders
//! it, and how long one rendering is reused.

/// Turns what the server knows into metric families.
pub mod collect;
/// The text exposition format.
pub mod exposition;
/// Whether `/metrics` is served and who may scrape it without a token.
pub mod settings;

use std::{
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use tokio::sync::Mutex;

use crate::{AppState, db, error::ApiError};

/// How long one rendering is served before the next scrape renders afresh.
/// Short enough that a 15-second scrape interval sees every change, long
/// enough that several scrapers - or one hammering the endpoint - cost one
/// set of queries between them.
pub const CACHE_TTL: Duration = Duration::from_secs(10);

#[derive(Debug)]
struct Rendered {
    at: Instant,
    body: Arc<str>,
}

/// State the `/metrics` endpoint keeps between scrapes.
#[derive(Debug, Clone, Default)]
pub struct MetricsState {
    cache: Arc<Mutex<Option<Rendered>>>,
    /// Unix seconds of the scheduler's last finished tick; `0` until it has
    /// finished one.
    scheduler_last_tick: Arc<AtomicI64>,
}

impl MetricsState {
    /// Records that the scheduler finished a tick at `at`.
    pub fn record_scheduler_tick(&self, at: DateTime<Utc>) {
        self.scheduler_last_tick
            .store(at.timestamp(), Ordering::Relaxed);
    }

    /// When the scheduler last finished a tick, if it has since start-up.
    #[must_use]
    pub fn scheduler_last_tick(&self) -> Option<DateTime<Utc>> {
        match self.scheduler_last_tick.load(Ordering::Relaxed) {
            0 => None,
            secs => DateTime::from_timestamp(secs, 0),
        }
    }

    /// The current rendering, reused while younger than `ttl` and otherwise
    /// produced by `render`. The lock is held while rendering, so concurrent
    /// scrapes that find the cache stale wait for one rendering instead of
    /// each running the queries.
    ///
    /// # Errors
    ///
    /// Returns whatever `render` returns; a failed rendering is not cached.
    pub async fn cached<F, Fut>(&self, ttl: Duration, render: F) -> Result<Arc<str>, ApiError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<String, ApiError>>,
    {
        let mut cache = self.cache.lock().await;
        if let Some(rendered) = cache.as_ref()
            && rendered.at.elapsed() < ttl
        {
            return Ok(Arc::clone(&rendered.body));
        }
        let body: Arc<str> = Arc::from(render().await?);
        *cache = Some(Rendered {
            at: Instant::now(),
            body: Arc::clone(&body),
        });
        Ok(body)
    }
}

async fn snapshot(state: &AppState) -> Result<collect::Snapshot, ApiError> {
    // Concurrent scrapes wait on this under the cache lock, so the queries run
    // side by side rather than one after another.
    let (queries, connected_agents, jobs) = tokio::join!(
        async {
            tokio::try_join!(
                db::metrics::schedule_targets(&state.pool),
                db::metrics::repositories(&state.pool),
                db::metrics::agents(&state.pool),
                db::get_schedule_timezone(&state.pool),
            )
        },
        state.registry.connected_agents(),
        state.repo_op_tracker.totals(),
    );
    let (targets, repositories, agents, timezone) = queries?;
    Ok(collect::Snapshot {
        now: Utc::now(),
        timezone,
        targets,
        repositories,
        agents,
        connected_agents: connected_agents.into_iter().collect(),
        scheduler_last_tick: state.metrics.scheduler_last_tick(),
        jobs,
        server_version: env!("APP_VERSION").to_owned(),
    })
}

/// The exposition text for a scrape, from the cache when it is fresh.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a query fails.
pub async fn render(state: &AppState) -> Result<Arc<str>, ApiError> {
    state
        .metrics
        .cached(CACHE_TTL, || async {
            let snapshot = snapshot(state).await?;
            Ok(exposition::render(&collect::families(&snapshot)))
        })
        .await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    #[tokio::test]
    async fn reuses_a_fresh_rendering() {
        let metrics = MetricsState::default();
        let calls = AtomicUsize::new(0);
        let render = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok("body".to_owned())
        };

        let first = metrics
            .cached(Duration::from_mins(1), render)
            .await
            .unwrap();
        let second = metrics
            .cached(Duration::from_mins(1), render)
            .await
            .unwrap();

        assert_eq!(&*first, "body");
        assert_eq!(&*second, "body");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn renders_again_once_the_ttl_has_passed() {
        let metrics = MetricsState::default();
        let calls = AtomicUsize::new(0);
        let render = || async {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            Ok(format!("body {n}"))
        };

        metrics.cached(Duration::ZERO, render).await.unwrap();
        let second = metrics.cached(Duration::ZERO, render).await.unwrap();

        assert_eq!(&*second, "body 1");
    }

    #[tokio::test]
    async fn does_not_cache_a_failed_rendering() {
        let metrics = MetricsState::default();

        let failed = metrics
            .cached(Duration::from_mins(1), || async {
                Err(ApiError::Internal("boom".to_owned()))
            })
            .await;
        let next = metrics
            .cached(Duration::from_mins(1), || async { Ok("ok".to_owned()) })
            .await
            .unwrap();

        assert!(failed.is_err());
        assert_eq!(&*next, "ok");
    }

    #[test]
    fn the_scheduler_has_not_ticked_until_it_records_one() {
        let metrics = MetricsState::default();
        assert_eq!(metrics.scheduler_last_tick(), None);

        let at = DateTime::from_timestamp(1_760_000_000, 0).unwrap();
        metrics.record_scheduler_tick(at);

        assert_eq!(metrics.scheduler_last_tick(), Some(at));
    }
}
