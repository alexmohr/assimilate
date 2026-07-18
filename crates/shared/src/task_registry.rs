// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use futures_util::StreamExt;
use tokio::task::JoinHandle;

/// Collects the `JoinHandle`s of fire-and-forget work spawned outside the
/// request/response cycle - queued borg operations, a borg child's
/// SIGKILL-escalation/break-lock cleanup (`borg::GracefulChild`'s reaper,
/// spawned from `Drop`), a `RepoOpGuard`'s deferred clear (also spawned from
/// `Drop`), and the server's long-lived background loops (scheduler, tunnel
/// manager, interrupted-import resume) - so shutdown can wait for them to
/// actually finish instead of silently dropping them when the process exits.
/// `register` is synchronous (a `std::sync::Mutex`, not `tokio::sync::Mutex`) specifically
/// so `Drop` impls can call it directly - `Drop` can't `.await`. Without
/// this, whether a task's remaining lines run before the process exits
/// (SIGTERM, or a test's tokio runtime tearing down) is a scheduling race
/// rather than something callers can join on. Shared between `agent` and
/// `server` since both spawn borg child processes through `shared::borg`.
#[derive(Clone, Default)]
pub struct TaskRegistry {
    handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl TaskRegistry {
    /// Registers a spawned task's handle so [`Self::shutdown`] can join it.
    pub fn register(&self, handle: JoinHandle<()>) {
        self.handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(handle);
    }

    /// Drains every registered handle and awaits them, bounded by
    /// `timeout_duration`. Tasks that haven't finished when the deadline
    /// passes are left running (their handles are simply dropped, not
    /// aborted - forcibly killing a borg operation mid-write is exactly what
    /// this codebase avoids elsewhere, see `GracefulChild`) rather than
    /// blocking shutdown indefinitely. Returns the number of tasks that were
    /// still outstanding at the deadline, so the caller can log it. Handles
    /// are joined individually against a shared deadline (rather than via a
    /// single `join_all` gated by an overall timeout) so that tasks which
    /// finish before the deadline are counted as done even if a straggler
    /// times out - otherwise every registered handle would be reported as
    /// outstanding just because one of them was slow.
    ///
    /// Re-checks `self.handles` between completions rather than taking a single
    /// snapshot up front, so a handle registered *during* the drain (e.g. a
    /// `GracefulChild` reaper spawned because some unrelated in-flight borg call
    /// got cancelled by its own caller-level timeout while shutdown is already in
    /// progress) still gets folded in and joined instead of being silently
    /// orphaned in an already-drained registry.
    pub async fn shutdown(&self, timeout_duration: Duration) -> usize {
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout_duration)
            .unwrap_or_else(tokio::time::Instant::now);

        let mut joins: futures_util::stream::FuturesUnordered<JoinHandle<()>> =
            futures_util::stream::FuturesUnordered::new();
        let mut remaining = 0usize;

        loop {
            let newly_registered = std::mem::take(
                &mut *self
                    .handles
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            remaining = remaining.saturating_add(newly_registered.len());
            joins.extend(newly_registered);

            if remaining == 0 {
                break;
            }

            let Ok(Some(result)) = tokio::time::timeout_at(deadline, joins.next()).await else {
                break;
            };
            remaining = remaining.saturating_sub(1);
            if let Err(e) = result {
                if e.is_panic() {
                    tracing::warn!(error = %e, "background task panicked during shutdown");
                } else {
                    tracing::warn!(error = %e, "background task was cancelled during shutdown");
                }
            }
        }

        remaining
    }

    /// Whether any registered task hasn't been drained by [`Self::shutdown`]
    /// yet. Test-oriented: lets a test (in this crate or a dependent one) assert
    /// that a spawn actually reached the registry before joining it, without a
    /// fixed sleep. Not `#[cfg(test)]` because that gate is crate-local - it
    /// would disappear for `agent`/`server`'s own tests, which depend on
    /// `shared` as an ordinary (non-test) dependency.
    pub fn pending_count(&self) -> usize {
        self.handles
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn shutdown_joins_registered_tasks_and_reports_none_outstanding() {
        let registry = TaskRegistry::default();
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ran_clone = Arc::clone(&ran);

        registry.register(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            ran_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        }));
        assert_eq!(registry.pending_count(), 1);

        let outstanding = registry.shutdown(Duration::from_secs(5)).await;

        assert_eq!(outstanding, 0);
        assert!(ran.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(registry.pending_count(), 0);
    }

    #[tokio::test]
    async fn shutdown_reports_outstanding_tasks_on_timeout() {
        let registry = TaskRegistry::default();
        registry.register(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }));

        let outstanding = registry.shutdown(Duration::from_millis(20)).await;

        assert_eq!(outstanding, 1);
    }

    #[tokio::test]
    async fn shutdown_with_no_registered_tasks_returns_immediately() {
        let registry = TaskRegistry::default();
        let outstanding = registry.shutdown(Duration::from_secs(5)).await;
        assert_eq!(outstanding, 0);
    }

    #[tokio::test]
    async fn shutdown_does_not_count_a_finished_task_as_outstanding_when_a_straggler_times_out() {
        let registry = TaskRegistry::default();
        registry.register(tokio::spawn(async {}));
        registry.register(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }));

        let outstanding = registry.shutdown(Duration::from_millis(50)).await;

        assert_eq!(outstanding, 1);
    }

    #[tokio::test]
    async fn shutdown_logs_a_panicked_task_and_still_reports_none_outstanding() {
        let registry = TaskRegistry::default();
        registry.register(tokio::spawn(async {
            panic!("boom");
        }));

        let outstanding = registry.shutdown(Duration::from_secs(5)).await;

        assert_eq!(outstanding, 0);
    }

    #[tokio::test]
    async fn shutdown_joins_a_task_registered_during_its_own_drain() {
        let registry = TaskRegistry::default();
        let ran_late = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ran_late_clone = Arc::clone(&ran_late);

        // Already registered when shutdown() starts, so it's in the initial snapshot.
        registry.register(tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(80)).await;
        }));

        // Registers a second handle mid-drain, simulating a GracefulChild reaper
        // spawned because some unrelated in-flight borg call got cancelled by its
        // own caller-level timeout while shutdown() is already awaiting the first
        // handle. A single up-front snapshot would never see this one.
        let registry_clone = registry.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            registry_clone.register(tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(40)).await;
                ran_late_clone.store(true, std::sync::atomic::Ordering::SeqCst);
            }));
        });

        let outstanding = registry.shutdown(Duration::from_secs(5)).await;

        assert_eq!(
            outstanding, 0,
            "both the pre-registered and mid-drain-registered tasks must be joined"
        );
        assert!(
            ran_late.load(std::sync::atomic::Ordering::SeqCst),
            "the task registered during shutdown's drain must run to completion, not be orphaned \
             in an already-drained registry"
        );
    }

    #[tokio::test]
    async fn shutdown_logs_a_cancelled_task_and_still_reports_none_outstanding() {
        let registry = TaskRegistry::default();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        handle.abort();
        registry.register(handle);

        let outstanding = registry.shutdown(Duration::from_secs(5)).await;

        assert_eq!(outstanding, 0);
    }
}
