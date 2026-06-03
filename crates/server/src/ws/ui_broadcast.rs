// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use shared::protocol::ServerToUi;
use tokio::sync::broadcast;

const CHANNEL_CAPACITY: usize = 128;

#[derive(Debug, Clone)]
pub struct ImportProgressSnapshot {
    pub progress: i32,
    pub total: i32,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UiBroadcast {
    sender: Arc<broadcast::Sender<ServerToUi>>,
    import_progress: Arc<RwLock<HashMap<i64, ImportProgressSnapshot>>>,
}

impl Default for UiBroadcast {
    fn default() -> Self {
        Self::new()
    }
}

impl UiBroadcast {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            sender: Arc::new(sender),
            import_progress: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn send(&self, event: ServerToUi) {
        if let Err(e) = self.sender.send(event) {
            tracing::trace!(error = %e, "ui broadcast: no receivers");
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerToUi> {
        self.sender.subscribe()
    }

    pub fn set_import_progress(&self, repo_id: i64, snapshot: ImportProgressSnapshot) {
        if let Ok(mut map) = self.import_progress.write() {
            map.insert(repo_id, snapshot);
        }
    }

    pub fn clear_import_progress(&self, repo_id: i64) {
        if let Ok(mut map) = self.import_progress.write() {
            map.remove(&repo_id);
        }
    }

    pub fn current_import_snapshots(&self) -> Vec<(i64, ImportProgressSnapshot)> {
        self.import_progress.read().map_or_else(
            |_| Vec::new(),
            |map| {
                map.iter()
                    .map(|(&repo_id, snap)| (repo_id, snap.clone()))
                    .collect()
            },
        )
    }
}
