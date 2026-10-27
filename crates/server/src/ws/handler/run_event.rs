// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Timeline steps an agent reports for a backup run, such as the filesystem
//! snapshots it took and removed on the source host.

use shared::types::{RunEventTarget, RunEventType};

use super::validate_agent_repo;
use crate::AppState;

/// A step of a backup the agent reports for the run's timeline.
pub(super) struct AgentRunEvent {
    pub(super) run_id: String,
    pub(super) repo_id: i64,
    pub(super) event_type: RunEventType,
    pub(super) message: String,
}

/// Longest message an agent's timeline step keeps; a longer one is cut.
pub(super) const MAX_AGENT_RUN_EVENT_MESSAGE: usize = 1000;

/// Records a timeline step the agent took on the source host. Only the steps
/// an agent takes itself are accepted: the power-management steps are the
/// server's own, and an agent that claims one is ignored.
pub(super) async fn record_agent_run_event(
    state: &AppState,
    agent_id: i64,
    hostname: &str,
    event: AgentRunEvent,
) {
    match event.event_type {
        RunEventType::SnapshotCreated
        | RunEventType::SnapshotRemoved
        | RunEventType::SnapshotFailed => {}
        RunEventType::ReachabilityCheck
        | RunEventType::WakeSent
        | RunEventType::WakeUnavailable
        | RunEventType::HostOnline
        | RunEventType::HostUnreachable
        | RunEventType::AgentStartSent
        | RunEventType::AgentConnected
        | RunEventType::AgentStopSent
        | RunEventType::AgentStopped
        | RunEventType::ShutdownSent
        | RunEventType::HostOffline => {
            tracing::warn!(
                hostname = %hostname,
                event_type = %event.event_type,
                "ignoring a run event only the server records"
            );
            return;
        }
    }
    if !validate_agent_repo(&state.pool, agent_id, event.repo_id, hostname, "RunEvent").await {
        return;
    }
    let message: String = event
        .message
        .chars()
        .take(MAX_AGENT_RUN_EVENT_MESSAGE)
        .collect();
    crate::power::record_run_event(
        crate::power::PowerCtx {
            pool: &state.pool,
            registry: &state.registry,
            ui_broadcast: &state.ui_broadcast,
            power_sessions: &state.power_sessions,
        },
        crate::power::RunEventSite {
            run_id: &event.run_id,
            agent_id,
            repo_id: event.repo_id,
            hostname,
        },
        RunEventTarget::Source,
        event.event_type,
        message,
    )
    .await;
}
