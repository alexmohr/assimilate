// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{collections::HashMap, sync::Arc};

use chrono::Utc;
use shared::protocol::{ActiveRepoOp, RepoOpKind};
use tokio::sync::RwLock;

#[derive(Clone, Default)]
pub struct RepoOpTracker {
    active: Arc<RwLock<HashMap<i64, ActiveRepoOp>>>,
}

impl RepoOpTracker {
    pub async fn set(&self, repo_id: i64, kind: RepoOpKind, actor: String) {
        self.active.write().await.insert(
            repo_id,
            ActiveRepoOp {
                kind,
                actor,
                started_at: Utc::now(),
            },
        );
    }

    pub async fn clear(&self, repo_id: i64) {
        self.active.write().await.remove(&repo_id);
    }

    pub async fn get(&self, repo_id: i64) -> Option<ActiveRepoOp> {
        self.active.read().await.get(&repo_id).cloned()
    }
}
