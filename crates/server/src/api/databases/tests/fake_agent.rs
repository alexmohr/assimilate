// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! An agent stand-in that answers the server's database scan requests.

use shared::{
    database::{DatabaseRuntime, DiscoveredDatabaseSource},
    protocol::ServerToAgent,
};
use tokio::sync::mpsc;

use crate::{AppState, db, pending::Claim};

/// Plays the agent: takes the scan request and answers it with what
/// `answer` returns, or drops it unanswered when that is `None`.
pub(super) fn answer_with(
    mut requests: mpsc::Receiver<ServerToAgent>,
    state: AppState,
    agent_id: i64,
    answer: Option<(
        Vec<DiscoveredDatabaseSource>,
        Option<String>,
        Vec<DatabaseRuntime>,
    )>,
) {
    tokio::spawn(async move {
        let Some(ServerToAgent::ScanDatabases {
            request_id,
            runtimes,
        }) = requests.recv().await
        else {
            panic!("the agent is asked to scan");
        };
        let runtimes = runtimes.expect("the scan tries the settings in the form");
        assert!(runtimes.kubernetes);
        assert_eq!(
            runtimes.kubeconfig_path.as_deref(),
            Some("/etc/assimilate/kubeconfig")
        );
        if let Some((sources, error, _)) = &answer
            && error.is_none()
        {
            db::databases::record_database_scan(&state.pool, agent_id, sources, &[])
                .await
                .unwrap();
        }
        let Claim::Claimed(tx) = state
            .pending_database_scans
            .claim(&request_id, agent_id)
            .await
        else {
            panic!("the scan is pending for this agent");
        };
        if let Some(answer) = answer {
            let _ = tx.send(answer);
        }
    });
}
