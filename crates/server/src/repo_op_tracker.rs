// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{collections::HashMap, sync::Arc};

use chrono::Utc;
use shared::protocol::{ActiveRepoOp, RepoOpKind};
use tokio::sync::RwLock;

#[derive(Default)]
struct RepoOpState {
    active: Option<ActiveRepoOp>,
    queued: u32,
}

/// Tracks the operation currently running against each repository, plus how many
/// further operations are queued behind it. Server-side borg operations are
/// serialised per repository (via `RepoLock`); this exposes that state to the UI.
#[derive(Clone, Default)]
pub struct RepoOpTracker {
    state: Arc<RwLock<HashMap<i64, RepoOpState>>>,
}

impl RepoOpTracker {
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
        state.active = Some(ActiveRepoOp {
            kind,
            actor,
            started_at: Utc::now(),
            queued: state.queued,
        });
    }

    /// Transition a queued operation into the running slot.
    pub async fn begin(&self, repo_id: i64, kind: RepoOpKind, actor: String) {
        let mut map = self.state.write().await;
        let state = map.entry(repo_id).or_default();
        state.queued = state.queued.saturating_sub(1);
        state.active = Some(ActiveRepoOp {
            kind,
            actor,
            started_at: Utc::now(),
            queued: state.queued,
        });
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
}
