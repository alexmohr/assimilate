// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::sync::Arc;

use shared::protocol::ServerToUi;
use tokio::sync::broadcast;

const CHANNEL_CAPACITY: usize = 128;

#[derive(Debug, Clone)]
pub struct UiBroadcast {
    sender: Arc<broadcast::Sender<ServerToUi>>,
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
}
