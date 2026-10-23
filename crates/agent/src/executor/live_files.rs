// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use shared::protocol::AgentToServer;
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::{Executor, LiveFileRequest};
use crate::live_file;

impl Executor {
    /// Reads a file for the server's live comparison off the command loop,
    /// limited to the sources of the configuration current right now.
    pub(super) async fn handle_read_live_file(
        &self,
        LiveFileRequest { request_id, path }: LiveFileRequest,
        outbound_tx: &mpsc::Sender<AgentToServer>,
    ) {
        let sources = live_file::configured_sources(self.current_config.lock().await.as_ref());
        let outbound = outbound_tx.clone();
        self.task_registry.register(tokio::spawn(async move {
            let file = live_file::read(sources, &path).await;
            info!(request_id = %request_id, "read a file for a live comparison");
            if outbound
                .send(AgentToServer::LiveFileRead { request_id, file })
                .await
                .is_err()
            {
                warn!("Outbound channel closed; live file answer dropped");
            }
        }));
    }
}
