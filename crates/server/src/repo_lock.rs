// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    collections::HashMap,
    fmt,
    sync::{
        Arc, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::{
    sync::{Mutex, OwnedMutexGuard},
    time::Instant,
};

/// How long a caller may wait before its wait is first reported.
const FIRST_LONG_WAIT_REPORT: Duration = Duration::from_mins(10);

/// How often a caller that is still waiting is reported again.
const LONG_WAIT_REPORT_INTERVAL: Duration = Duration::from_hours(1);

/// When a caller still waiting for a repository's lock gets reported.
#[derive(Debug, Clone, Copy)]
struct LongWaitPolicy {
    first_report: Duration,
    report_every: Duration,
}

impl Default for LongWaitPolicy {
    fn default() -> Self {
        Self {
            first_report: FIRST_LONG_WAIT_REPORT,
            report_every: LONG_WAIT_REPORT_INTERVAL,
        }
    }
}

/// A caller that has been queued behind a repository's lock for a long time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LongWait {
    repo_id: i64,
    waited: Duration,
    /// How long the current holder has had the lock, if it's known.
    held_for: Option<Duration>,
    /// Callers waiting for the lock, this one included.
    queued: usize,
}

/// Formats a duration as whole minutes, the resolution a long wait is reported at.
struct Minutes(Duration);

impl fmt::Display for Minutes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}m", self.0.as_secs() / 60)
    }
}

impl fmt::Display for LongWait {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "an operation on repository {} has been waiting {} for another operation on it to \
             finish ({} queued",
            self.repo_id,
            Minutes(self.waited),
            self.queued
        )?;
        if let Some(held_for) = self.held_for {
            write!(f, ", current one running for {}", Minutes(held_for))?;
        }
        write!(
            f,
            "); this is expected behind a long-running backup, but if the running operation is \
             stuck, use Reset under Cancel all running backups on the System page to release it"
        )
    }
}

#[derive(Default)]
struct RepoLockEntry {
    mutex: Arc<Mutex<()>>,
    /// When the current holder acquired `mutex`. Only read while waiting for
    /// `mutex`, i.e. while someone holds it, so it never needs clearing.
    held_since: std::sync::Mutex<Option<Instant>>,
    waiting: AtomicUsize,
}

impl RepoLockEntry {
    fn mark_held(&self) {
        *self
            .held_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    }

    fn held_for(&self) -> Option<Duration> {
        self.held_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .map(|since| since.elapsed())
    }
}

/// Counts a caller as waiting for the lock until it's dropped, so a caller
/// that gives up (e.g. an `acquire` wrapped in a timeout) stops being counted.
struct Waiting<'a>(&'a AtomicUsize);

impl<'a> Waiting<'a> {
    fn start(count: &'a AtomicUsize) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self(count)
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Per-repository mutex that serialises borg operations on the same repo.
///
/// Waiting for it is deliberately unbounded: an operation holding it can
/// legitimately run for many hours (e.g. a large backup to a remote host), and
/// whatever is queued behind it must still run afterwards. A caller that has
/// been waiting unusually long is logged as a warning instead, so a stuck
/// holder is visible and can be released with the System page's reset (see
/// [`Self::force_reset`]).
#[derive(Clone, Default)]
pub struct RepoLock {
    locks: Arc<Mutex<HashMap<i64, Arc<RepoLockEntry>>>>,
    policy: LongWaitPolicy,
}

impl RepoLock {
    /// Acquire the per-repo lock, waiting until the current holder releases
    /// it. A long wait is logged periodically but never gives up.
    pub async fn acquire(&self, repo_id: i64) -> OwnedMutexGuard<()> {
        self.acquire_reporting(repo_id, |wait| tracing::warn!(repo_id, "{wait}"))
            .await
    }

    async fn acquire_reporting(
        &self,
        repo_id: i64,
        mut report: impl FnMut(LongWait),
    ) -> OwnedMutexGuard<()> {
        let entry = Arc::clone(self.locks.lock().await.entry(repo_id).or_default());
        let started = Instant::now();
        let waiting = Waiting::start(&entry.waiting);
        let lock = Arc::clone(&entry.mutex).lock_owned();
        tokio::pin!(lock);

        let mut next_report = started.checked_add(self.policy.first_report);
        let guard = loop {
            let Some(deadline) = next_report else {
                break lock.await;
            };
            if let Ok(guard) = tokio::time::timeout_at(deadline, &mut lock).await {
                break guard;
            }
            report(LongWait {
                repo_id,
                waited: started.elapsed(),
                held_for: entry.held_for(),
                queued: entry.waiting.load(Ordering::SeqCst),
            });
            next_report = deadline.checked_add(self.policy.report_every);
        };
        drop(waiting);
        entry.mark_held();
        guard
    }

    /// Drop all per-repo mutex entries so subsequent `acquire` calls get fresh,
    /// unlocked mutexes. Stuck tasks that hold an `OwnedMutexGuard` from before
    /// this call continue to own their (now orphaned) guard; they cannot block
    /// any new operations because new callers will receive a different `Arc`.
    pub async fn force_reset(&self) {
        self.locks.lock().await.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use super::*;

    fn fast_lock() -> RepoLock {
        RepoLock {
            policy: LongWaitPolicy {
                first_report: Duration::from_millis(50),
                report_every: Duration::from_millis(50),
            },
            ..RepoLock::default()
        }
    }

    type Reports = Arc<StdMutex<Vec<LongWait>>>;

    /// Spawns a caller waiting for `repo_id`'s lock that records every long-wait report.
    fn spawn_reporting_waiter(
        lock: &RepoLock,
        repo_id: i64,
    ) -> (tokio::task::JoinHandle<OwnedMutexGuard<()>>, Reports) {
        let reports = Reports::default();
        let waiter = tokio::spawn({
            let lock = lock.clone();
            let reports = Arc::clone(&reports);
            async move {
                lock.acquire_reporting(repo_id, |wait| reports.lock().unwrap().push(wait))
                    .await
            }
        });
        (waiter, reports)
    }

    #[tokio::test]
    async fn uncontended_acquire_reports_nothing() {
        let lock = fast_lock();
        let reports = StdMutex::new(Vec::new());
        let guard = lock
            .acquire_reporting(1, |wait| reports.lock().unwrap().push(wait))
            .await;
        drop(guard);
        assert!(reports.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn long_wait_is_reported_repeatedly_and_still_acquires() {
        let lock = fast_lock();
        let held = lock.acquire(1).await;
        let (waiter, reports) = spawn_reporting_waiter(&lock, 1);

        tokio::time::sleep(Duration::from_millis(180)).await;
        assert!(
            !waiter.is_finished(),
            "waiter must keep waiting, not time out"
        );
        drop(held);
        let guard = waiter.await.unwrap();
        drop(guard);

        let reports = reports.lock().unwrap();
        assert!(
            reports.len() >= 2,
            "expected repeated reports, got {reports:?}"
        );
        assert!(
            reports
                .iter()
                .all(|wait| wait.repo_id == 1 && wait.queued == 1)
        );
        assert!(reports.iter().all(|wait| wait.held_for.is_some()));
        assert!(
            reports
                .iter()
                .zip(reports.iter().skip(1))
                .all(|(earlier, later)| earlier.waited < later.waited)
        );
        assert!(
            reports
                .first()
                .is_some_and(|first| first.waited >= Duration::from_millis(50))
        );
    }

    #[tokio::test]
    async fn waiting_on_another_repo_is_not_blocked() {
        let lock = fast_lock();
        let _held = lock.acquire(1).await;
        let reports = StdMutex::new(Vec::new());
        let guard = tokio::time::timeout(
            Duration::from_secs(1),
            lock.acquire_reporting(2, |wait| reports.lock().unwrap().push(wait)),
        )
        .await;
        assert!(guard.is_ok());
        assert!(reports.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn abandoned_waiter_stops_counting_as_queued() {
        let lock = fast_lock();
        let held = lock.acquire(1).await;

        let gave_up = tokio::time::timeout(Duration::from_millis(10), lock.acquire(1)).await;
        assert!(gave_up.is_err());

        let (waiter, reports) = spawn_reporting_waiter(&lock, 1);
        tokio::time::sleep(Duration::from_millis(80)).await;
        drop(held);
        drop(waiter.await.unwrap());

        let reports = reports.lock().unwrap();
        assert!(!reports.is_empty());
        assert!(reports.iter().all(|wait| wait.queued == 1));
    }

    #[tokio::test]
    async fn force_reset_unblocks_new_callers() {
        let lock = fast_lock();
        let _stuck = lock.acquire(1).await;
        lock.force_reset().await;
        let fresh = tokio::time::timeout(Duration::from_secs(1), lock.acquire(1)).await;
        assert!(fresh.is_ok());
    }

    #[test]
    fn long_wait_message_names_repo_durations_and_reset() {
        let wait = LongWait {
            repo_id: 7,
            waited: Duration::from_mins(125),
            held_for: Some(Duration::from_hours(12)),
            queued: 3,
        };
        let message = wait.to_string();
        assert!(message.contains("repository 7"));
        assert!(message.contains("waiting 125m"));
        assert!(message.contains("3 queued"));
        assert!(message.contains("running for 720m"));
        assert!(message.contains("Cancel all running backups"));
    }

    #[test]
    fn long_wait_message_without_known_holder_duration() {
        let wait = LongWait {
            repo_id: 7,
            waited: Duration::from_mins(10),
            held_for: None,
            queued: 1,
        };
        assert!(!wait.to_string().contains("running for"));
    }
}
