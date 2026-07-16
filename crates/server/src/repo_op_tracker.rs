// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use chrono::Utc;
use shared::protocol::{ActiveRepoOp, RepoOpKind};
use tokio::sync::RwLock;

#[derive(Default)]
struct RepoOpState {
    active: Option<ActiveRepoOp>,
    queued: u32,
    /// Identifies which call last claimed `active` - `set`, `begin`, and
    /// `set_guarded` all stamp a fresh token whenever they do, so a guard's
    /// deferred clear (see `RepoOpGuard`) can tell whether it's still
    /// clearing its own operation or a later one that reused this `repo_id`.
    /// This has to be updated by every method that claims `active`, not just
    /// `set_guarded`: if a guarded operation's entry survives its own clear
    /// because something is still queued behind it (`clear_if_token` only
    /// removes the map entry, and the token with it, once `queued == 0`), a
    /// later plain `begin()`/`set()` call would otherwise leave the old
    /// token in place - letting the original guard's still-pending deferred
    /// clear match it and wipe out the new operation once it finally runs.
    /// Only ever compared for equality - a plain `u64` counter that survives
    /// the map entry being removed and recreated would work too, but a
    /// per-tracker monotonic token sidesteps having to reason about whether
    /// it could ever wrap back to a value a still-live guard remembers.
    token: u64,
}

/// Tracks the operation currently running against each repository, plus how many
/// further operations are queued behind it. Server-side borg operations are
/// serialised per repository (via `RepoLock`); this exposes that state to the UI.
#[derive(Clone, Default)]
pub struct RepoOpTracker {
    state: Arc<RwLock<HashMap<i64, RepoOpState>>>,
    next_token: Arc<AtomicU64>,
}

impl RepoOpTracker {
    /// Stamp a fresh token onto `state` and record `kind`/`actor` as its
    /// active operation, invalidating whatever token (and thus whichever
    /// guard's deferred clear) was previously associated with this entry.
    /// Shared by every method that claims an entry's active slot - see the
    /// `token` field's doc comment for why they all need to do this.
    fn claim(&self, state: &mut RepoOpState, kind: RepoOpKind, actor: String) -> u64 {
        let token = self.next_token.fetch_add(1, Ordering::SeqCst);
        state.token = token;
        state.active = Some(ActiveRepoOp {
            kind,
            actor,
            started_at: Utc::now(),
            queued: state.queued,
        });
        token
    }

    /// Record that an operation is waiting to run for this repository.
    pub async fn enqueue(&self, repo_id: i64) {
        let mut state = self.state.write().await;
        let entry = state.entry(repo_id).or_default();
        entry.queued = entry.queued.saturating_add(1);
    }

    /// Remove a previously enqueued operation that will no longer run.
    pub async fn dequeue(&self, repo_id: i64) {
        let mut map = self.state.write().await;
        if let Some(state) = map.get_mut(&repo_id) {
            state.queued = state.queued.saturating_sub(1);
            if state.active.is_none() && state.queued == 0 {
                map.remove(&repo_id);
            }
        }
    }

    /// Mark an operation as the one now running for this repository.
    pub async fn set(&self, repo_id: i64, kind: RepoOpKind, actor: String) {
        let mut map = self.state.write().await;
        let state = map.entry(repo_id).or_default();
        self.claim(state, kind, actor);
    }

    /// Transition a queued operation into the running slot.
    pub async fn begin(&self, repo_id: i64, kind: RepoOpKind, actor: String) {
        let mut map = self.state.write().await;
        let state = map.entry(repo_id).or_default();
        state.queued = state.queued.saturating_sub(1);
        self.claim(state, kind, actor);
    }

    /// Clear the running operation. The repository entry is dropped once nothing
    /// is queued behind it.
    pub async fn clear(&self, repo_id: i64) {
        let mut map = self.state.write().await;
        if let Some(state) = map.get_mut(&repo_id) {
            state.active = None;
            if state.queued == 0 {
                map.remove(&repo_id);
            }
        }
    }

    /// Number of operations waiting to run for this repository.
    pub async fn queued_count(&self, repo_id: i64) -> u32 {
        self.state
            .read()
            .await
            .get(&repo_id)
            .map_or(0, |state| state.queued)
    }

    /// Return the currently active operation for this repository, if any.
    pub async fn get(&self, repo_id: i64) -> Option<ActiveRepoOp> {
        let map = self.state.read().await;
        map.get(&repo_id).and_then(|state| {
            state.active.as_ref().map(|active| ActiveRepoOp {
                queued: state.queued,
                ..active.clone()
            })
        })
    }

    /// Whether any repository has an active or queued operation right now. Used by the
    /// health check so e2e coverage teardown can wait for background repo syncs
    /// (`scheduler.rs`'s spawned sync task) to finish before stopping containers,
    /// instead of racing a fixed timeout against a variable-duration operation.
    pub async fn any_active(&self) -> bool {
        !self.state.read().await.is_empty()
    }

    /// Clear all active operations whose actor matches `hostname` and return
    /// the repo IDs that were affected, so callers can broadcast updates.
    pub async fn clear_for_agent(&self, hostname: &str) -> Vec<i64> {
        let mut map = self.state.write().await;
        let mut cleared = Vec::new();
        for (&repo_id, state) in map.iter_mut() {
            if state.active.as_ref().is_some_and(|a| a.actor == hostname) {
                state.active = None;
                cleared.push(repo_id);
            }
        }
        map.retain(|_, state| state.active.is_some() || state.queued > 0);
        cleared
    }

    /// Forcibly clear all tracked operations and return the affected repo IDs
    /// so callers can broadcast UI updates. Used by the emergency system reset
    /// to unstick repos whose agents will never send a completion signal.
    pub async fn clear_all(&self) -> Vec<i64> {
        let mut map = self.state.write().await;
        let repo_ids: Vec<i64> = map.keys().copied().collect();
        map.clear();
        repo_ids
    }

    /// Like [`Self::set`], but returns a guard that clears this repo's entry
    /// when dropped, including during panic unwind. A task that calls this
    /// and then panics before reaching its own [`RepoOpGuard::clear_now`]
    /// would otherwise leave a permanently "active" entry behind - which
    /// wedges `/api/health`'s `background_ops_in_flight` forever and defeats
    /// the e2e teardown's drain wait (every subsequent poll sees this repo as
    /// busy, even though nothing is actually running).
    ///
    /// `Drop` can't await, so the guard's cleanup runs as a spawned task
    /// rather than inline; call [`RepoOpGuard::clear_now`] on the
    /// normal-completion path so the entry is gone by the time you return,
    /// instead of racing the spawned task. Both paths only ever clear the
    /// exact operation this guard was created for: if something else has
    /// since called `set`/`begin`/`set_guarded` again for the same `repo_id`
    /// (e.g. this operation's own explicit clear already ran, and a
    /// different operation claimed the slot before this guard got dropped),
    /// the stale clear is a no-op instead of wiping out the new operation's
    /// state.
    #[must_use]
    pub async fn set_guarded(&self, repo_id: i64, kind: RepoOpKind, actor: String) -> RepoOpGuard {
        let mut map = self.state.write().await;
        let state = map.entry(repo_id).or_default();
        let token = self.claim(state, kind, actor);
        RepoOpGuard {
            tracker: self.clone(),
            repo_id,
            token,
        }
    }

    /// Clears `repo_id`'s entry only if it's still the one identified by
    /// `token` - see [`Self::set_guarded`].
    async fn clear_if_token(&self, repo_id: i64, token: u64) {
        let mut map = self.state.write().await;
        if let Some(state) = map.get_mut(&repo_id) {
            if state.token != token {
                return;
            }
            state.active = None;
            if state.queued == 0 {
                map.remove(&repo_id);
            }
        }
    }
}

/// See [`RepoOpTracker::set_guarded`].
pub struct RepoOpGuard {
    tracker: RepoOpTracker,
    repo_id: i64,
    token: u64,
}

impl RepoOpGuard {
    /// Clears this operation's entry immediately, awaiting the clear instead
    /// of leaving it to the guard's deferred `Drop` cleanup. Safe to call even
    /// if a later operation has already reclaimed this `repo_id` - only clears
    /// if the entry still matches this guard's token.
    pub async fn clear_now(&self) {
        self.tracker.clear_if_token(self.repo_id, self.token).await;
    }
}

impl Drop for RepoOpGuard {
    fn drop(&mut self) {
        let tracker = self.tracker.clone();
        let repo_id = self.repo_id;
        let token = self.token;
        tokio::spawn(async move {
            tracker.clear_if_token(repo_id, token).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn clear_for_agent_removes_matching_ops() {
        let tracker = RepoOpTracker::default();
        tracker
            .set(1, RepoOpKind::AgentBackup, "gremlin".to_owned())
            .await;
        tracker
            .set(2, RepoOpKind::AgentBackup, "gremlin".to_owned())
            .await;
        tracker
            .set(3, RepoOpKind::AgentBackup, "other-host".to_owned())
            .await;

        let mut cleared = tracker.clear_for_agent("gremlin").await;
        cleared.sort_unstable();

        assert_eq!(cleared, vec![1, 2]);
        assert!(tracker.get(1).await.is_none());
        assert!(tracker.get(2).await.is_none());
        assert!(tracker.get(3).await.is_some());
    }

    #[tokio::test]
    async fn clear_for_agent_preserves_queued_entries() {
        let tracker = RepoOpTracker::default();
        tracker.enqueue(1).await;
        tracker
            .set(1, RepoOpKind::AgentBackup, "gremlin".to_owned())
            .await;

        let cleared = tracker.clear_for_agent("gremlin").await;
        assert_eq!(cleared, vec![1]);
        assert!(tracker.get(1).await.is_none());
        assert_eq!(tracker.queued_count(1).await, 1);
    }

    #[tokio::test]
    async fn clear_for_agent_is_idempotent_on_no_match() {
        let tracker = RepoOpTracker::default();
        tracker
            .set(1, RepoOpKind::AgentBackup, "some-host".to_owned())
            .await;

        let cleared = tracker.clear_for_agent("gremlin").await;
        assert!(cleared.is_empty());
        assert!(tracker.get(1).await.is_some());
    }

    #[tokio::test]
    async fn any_active_reflects_active_and_queued_operations() {
        let tracker = RepoOpTracker::default();
        assert!(!tracker.any_active().await);

        tracker
            .set(1, RepoOpKind::AgentBackup, "some-host".to_owned())
            .await;
        assert!(tracker.any_active().await);

        tracker.clear(1).await;
        assert!(!tracker.any_active().await);

        tracker.enqueue(2).await;
        assert!(tracker.any_active().await);

        tracker.dequeue(2).await;
        assert!(!tracker.any_active().await);
    }

    /// The guard's cleanup runs as a spawned task (`Drop` can't await), so
    /// tests observe it by polling instead of asserting immediately after drop.
    async fn wait_until_cleared(tracker: &RepoOpTracker, repo_id: i64) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while tracker.get(repo_id).await.is_some() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("guard's spawned cleanup never cleared the entry");
    }

    #[tokio::test]
    async fn guard_clears_entry_when_dropped_without_an_explicit_clear() {
        let tracker = RepoOpTracker::default();
        let guard = tracker
            .set_guarded(1, RepoOpKind::AgentBackup, "some-host".to_owned())
            .await;
        assert!(tracker.any_active().await);

        drop(guard);
        wait_until_cleared(&tracker, 1).await;
    }

    #[tokio::test]
    async fn guard_cleanup_is_a_harmless_no_op_after_clear_now() {
        let tracker = RepoOpTracker::default();
        let guard = tracker
            .set_guarded(1, RepoOpKind::AgentBackup, "some-host".to_owned())
            .await;

        guard.clear_now().await;
        assert!(tracker.get(1).await.is_none());

        drop(guard);
        wait_until_cleared(&tracker, 1).await;
    }

    /// The exact race a review on this PR flagged: a guard's deferred `Drop`
    /// cleanup must not clobber a *different*, later operation that reused the
    /// same `repo_id` after this guard's own operation already cleared itself.
    #[tokio::test]
    async fn guard_drop_does_not_clobber_a_newer_operation_on_the_same_repo() {
        let tracker = RepoOpTracker::default();
        let first = tracker
            .set_guarded(1, RepoOpKind::ServerSync, "server".to_owned())
            .await;
        first.clear_now().await;
        assert!(tracker.get(1).await.is_none());

        // A different operation claims the same repo_id before the old guard
        // gets dropped - e.g. a manual break-lock racing a scheduled sync's
        // task teardown.
        let second = tracker
            .set_guarded(1, RepoOpKind::BreakLock, "admin".to_owned())
            .await;
        assert_eq!(
            tracker.get(1).await.map(|op| op.kind),
            Some(RepoOpKind::BreakLock)
        );

        // The first guard's deferred cleanup must not wipe out the second
        // operation's still-active entry.
        drop(first);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(
            tracker.get(1).await.map(|op| op.kind),
            Some(RepoOpKind::BreakLock),
            "a stale guard's deferred clear wiped out a newer operation's entry"
        );

        drop(second);
        wait_until_cleared(&tracker, 1).await;
    }

    /// The exact interleaving a follow-up review flagged: a guarded
    /// operation's entry survives its own `clear_now()` because something is
    /// queued behind it, so a later plain `begin()` (the path
    /// `run_archive_deletion` uses) must stamp a fresh token - otherwise the
    /// original guard's still-pending deferred `Drop` cleanup would match the
    /// unchanged stale token and wipe out the new operation once it runs.
    #[tokio::test]
    async fn plain_begin_after_guarded_clear_is_immune_to_the_stale_guards_deferred_clear() {
        let tracker = RepoOpTracker::default();
        let guard = tracker
            .set_guarded(1, RepoOpKind::ServerSync, "server".to_owned())
            .await;

        // A second operation queues behind the guarded one before it finishes.
        tracker.enqueue(1).await;

        // The guarded operation finishes and clears itself explicitly - but
        // the entry (and its now-stale token) survives because something is
        // still queued.
        guard.clear_now().await;
        assert!(tracker.get(1).await.is_none());

        // The queued operation transitions into the running slot via the
        // plain, non-guarded `begin()`.
        tracker
            .begin(1, RepoOpKind::DeleteArchive, "user".to_owned())
            .await;
        assert_eq!(
            tracker.get(1).await.map(|op| op.kind),
            Some(RepoOpKind::DeleteArchive)
        );

        // The stale guard's deferred Drop cleanup fires with its now-stale
        // token and must not clobber the new operation.
        drop(guard);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(
            tracker.get(1).await.map(|op| op.kind),
            Some(RepoOpKind::DeleteArchive),
            "a stale guard's deferred clear wiped out a plain begin()'s active operation"
        );
    }

    /// Same interleaving as above but through `set()` - the path
    /// `break_lock` uses instead of `begin()`.
    #[tokio::test]
    async fn plain_set_after_guarded_clear_is_immune_to_the_stale_guards_deferred_clear() {
        let tracker = RepoOpTracker::default();
        let guard = tracker
            .set_guarded(1, RepoOpKind::ServerSync, "server".to_owned())
            .await;
        tracker.enqueue(1).await;
        guard.clear_now().await;
        assert!(tracker.get(1).await.is_none());

        tracker
            .set(1, RepoOpKind::BreakLock, "admin".to_owned())
            .await;
        assert_eq!(
            tracker.get(1).await.map(|op| op.kind),
            Some(RepoOpKind::BreakLock)
        );

        drop(guard);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(
            tracker.get(1).await.map(|op| op.kind),
            Some(RepoOpKind::BreakLock),
            "a stale guard's deferred clear wiped out a plain set()'s active operation"
        );
    }
}
