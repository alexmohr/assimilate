// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

/// Tracks fire-and-forget background tasks (spawned via `tokio::spawn` outside
/// the request/response cycle, e.g. archive stat enrichment and post-backup
/// sync/indexing) that aren't otherwise visible through `RepoOpTracker` or
/// `NotificationService::in_flight_deliveries`. Exists solely so `/api/health`
/// can report `background_ops_in_flight` accurately: e2e coverage teardown
/// polls that field to know it's safe to stop containers, and a task this
/// tracker doesn't know about can still be mid-flight (and non-deterministically
/// hit different branches) when teardown races it.
#[derive(Clone, Default)]
pub struct BackgroundTaskTracker {
    in_flight: Arc<AtomicUsize>,
}

/// Decrements the in-flight counter when the tracked task ends, whether it
/// completes normally or panics.
pub struct BackgroundTaskGuard(Arc<AtomicUsize>);

impl Drop for BackgroundTaskGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl BackgroundTaskTracker {
    /// Mark a background task as started. Keep the returned guard alive for
    /// the duration of the task; dropping it (including via panic unwind)
    /// marks the task as finished.
    #[must_use]
    pub fn begin(&self) -> BackgroundTaskGuard {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        BackgroundTaskGuard(Arc::clone(&self.in_flight))
    }

    /// Claims a guard and spawns `future` holding it, so the task counts as
    /// in-flight for its entire life - including error and panic paths, since
    /// the guard is only dropped when the spawned task ends.
    ///
    /// The guard is claimed here, synchronously, rather than inside the spawned
    /// task: [`Self::any_active`] must read `true` the instant this returns, not
    /// merely once the runtime gets around to polling the new task for the first
    /// time (which depends on incidental yield points in the caller, not on any
    /// synchronization guarantee).
    ///
    /// `future`'s output is dropped - this is for fire-and-forget work, where
    /// nothing is waiting on a result.
    pub fn spawn_tracked<F>(&self, future: F)
    where
        F: Future + Send + 'static,
    {
        let guard = self.begin();
        tokio::spawn(async move {
            let _guard = guard;
            let _output = future.await;
        });
    }

    /// Whether any tracked background task is still running.
    #[must_use]
    pub fn any_active(&self) -> bool {
        self.in_flight.load(Ordering::SeqCst) > 0
    }

    /// Polls `any_active()` until it clears or `timeout_duration` elapses.
    /// Lets a test synchronize with a tracked background task's completion
    /// instead of guessing how long a fixed sleep needs to be, so the test's
    /// own tokio runtime doesn't tear down while the task is still in flight.
    /// Returns `true` if the tracker went idle, `false` on timeout.
    pub async fn wait_until_idle(&self, timeout_duration: Duration) -> bool {
        tokio::time::timeout(timeout_duration, async {
            while self.any_active() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .is_ok()
    }

    /// Test-oriented convenience over [`Self::wait_until_idle`]: panics with a
    /// descriptive message if the tracker doesn't go idle in time, instead of
    /// every call site writing out its own `assert!`.
    ///
    /// # Panics
    ///
    /// Panics if the tracker is still active after `timeout_duration`.
    pub async fn assert_idle(&self, timeout_duration: Duration) {
        assert!(
            self.wait_until_idle(timeout_duration).await,
            "timed out waiting for background tasks to finish"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_active_reflects_in_flight_count_and_clears_on_drop() {
        let tracker = BackgroundTaskTracker::default();
        assert!(!tracker.any_active());

        let guard1 = tracker.begin();
        assert!(tracker.any_active());
        let guard2 = tracker.begin();
        assert!(tracker.any_active());

        drop(guard1);
        assert!(tracker.any_active(), "second guard is still outstanding");
        drop(guard2);
        assert!(!tracker.any_active());
    }

    #[tokio::test]
    async fn wait_until_idle_returns_true_once_the_guard_drops() {
        let tracker = BackgroundTaskTracker::default();
        let guard = tracker.begin();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            drop(guard);
        });

        assert!(tracker.wait_until_idle(Duration::from_secs(5)).await);
        assert!(!tracker.any_active());
    }

    #[tokio::test]
    async fn wait_until_idle_returns_false_on_timeout_while_still_active() {
        let tracker = BackgroundTaskTracker::default();
        let _guard = tracker.begin();

        assert!(!tracker.wait_until_idle(Duration::from_millis(20)).await);
        assert!(tracker.any_active(), "guard was never dropped");
    }

    #[tokio::test]
    async fn spawn_tracked_counts_the_task_before_it_is_ever_polled() {
        let tracker = BackgroundTaskTracker::default();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();

        tracker.spawn_tracked(async move {
            let _ = rx.await;
        });

        // No yield between the spawn and this assertion: the point of claiming
        // the guard synchronously is that the task is visible immediately, not
        // only once the runtime first polls it.
        assert!(tracker.any_active());

        let _ = tx.send(());
        assert!(tracker.wait_until_idle(Duration::from_secs(5)).await);
    }

    #[tokio::test]
    async fn spawn_tracked_releases_the_guard_when_the_task_panics() {
        let tracker = BackgroundTaskTracker::default();
        tracker.spawn_tracked(async {
            panic!("task blew up");
        });

        assert!(tracker.wait_until_idle(Duration::from_secs(5)).await);
    }

    #[tokio::test]
    async fn spawn_tracked_accepts_a_future_whose_output_is_not_unit() {
        let tracker = BackgroundTaskTracker::default();
        // Callers pass dispatch futures that return a value nobody waits on;
        // the output is dropped rather than forcing every call site to bind it.
        tracker.spawn_tracked(async { 42u32 });

        assert!(tracker.wait_until_idle(Duration::from_secs(5)).await);
        assert!(!tracker.any_active());
    }
}
