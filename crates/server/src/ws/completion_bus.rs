// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub struct OperationOutcome {
    pub hostname: String,
    pub repo_id: i64,
    pub success: bool,
}

#[derive(Clone, Debug)]
pub struct CompletionBus {
    tx: broadcast::Sender<OperationOutcome>,
}

impl CompletionBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx }
    }

    pub fn publish(&self, outcome: OperationOutcome) {
        let _ = self.tx.send(outcome);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<OperationOutcome> {
        self.tx.subscribe()
    }
}

impl Default for CompletionBus {
    fn default() -> Self {
        Self::new()
    }
}
