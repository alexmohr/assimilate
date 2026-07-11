// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
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

    /// Whether any tracked background task is still running.
    #[must_use]
    pub fn any_active(&self) -> bool {
        self.in_flight.load(Ordering::SeqCst) > 0
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
}
