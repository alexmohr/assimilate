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
        self.state.write().await.entry(repo_id).or_default().queued += 1;
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

    pub async fn get(&self, repo_id: i64) -> Option<ActiveRepoOp> {
        let map = self.state.read().await;
        map.get(&repo_id).and_then(|state| {
            state.active.as_ref().map(|active| ActiveRepoOp {
                queued: state.queued,
                ..active.clone()
            })
        })
    }
}
