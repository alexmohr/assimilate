// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::task::JoinHandle;

/// Collects the `JoinHandle`s of fire-and-forget work spawned outside the
/// request/response cycle - queued borg operations, and a borg child's
/// SIGKILL-escalation/break-lock cleanup (`borg::GracefulChild`'s reaper,
/// spawned from `Drop`) - so shutdown can wait for them to actually finish
/// instead of silently dropping them when the process exits. `register` is
/// synchronous (a `std::sync::Mutex`, not `tokio::sync::Mutex`) specifically
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
    /// still outstanding at the deadline, so the caller can log it.
    pub async fn shutdown(&self, timeout_duration: Duration) -> usize {
        let handles = std::mem::take(
            &mut *self
                .handles
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        let total = handles.len();

        let joined =
            tokio::time::timeout(timeout_duration, futures_util::future::join_all(handles)).await;

        match joined {
            Ok(results) => {
                results.iter().filter_map(|r| r.as_ref().err()).for_each(
                    |e| tracing::warn!(error = %e, "background task panicked during shutdown"),
                );
                0
            }
            Err(_) => total,
        }
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
}
