// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use shared::protocol::AgentToServer;
use tokio::sync::mpsc;
use tracing::warn;

use super::Executor;

impl Executor {
    /// Looks for borg again and reports what it found. It runs on its own
    /// task: the probe can take as long as a stuck borg lets it, and the
    /// connection must keep answering pings meanwhile.
    pub(super) fn handle_check_borg(&self, outbound_tx: &mpsc::Sender<AgentToServer>) {
        let outbound = outbound_tx.clone();
        self.task_registry.register(tokio::spawn(async move {
            let status = crate::borg::probe().await;
            if outbound
                .send(AgentToServer::BorgStatus { status })
                .await
                .is_err()
            {
                warn!("Outbound channel closed; borg status answer dropped");
            }
        }));
    }
}
