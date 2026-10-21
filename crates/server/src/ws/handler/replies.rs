// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Agent replies to server requests: scan, stage, restore, dry-run and delete
//! results, and the failures that settle a pending request.

use shared::protocol::ServerToUi;

use super::validate_agent_repo;
use crate::{
    AppState,
    db::{self, restore_runs::RestoreOutcome},
    pending::{Claim, PendingRequests},
    restore_runs::{self, Settlement},
};

pub(super) async fn handle_backup_log(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    repo_id: shared::types::RepoId,
    schedule_id: Option<i64>,
    line: String,
) {
    if !validate_agent_repo(&state.pool, agent_id, repo_id.0, hostname, "BackupLog").await {
        return;
    }
    state.ui_broadcast.send(ServerToUi::BackupLog {
        hostname: hostname.to_owned(),
        schedule_id,
        repo_id: repo_id.0,
        line,
    });
}

/// Records the domains an agent reported and hands them to whoever asked for
/// the scan. A scan that failed still resolves the waiter, so the UI reports
/// the reason rather than timing out.
pub(super) async fn handle_vm_scan_result(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    request_id: Option<String>,
    vms: Vec<shared::vm::DiscoveredVm>,
    error: Option<String>,
) {
    if let Some(reason) = error.as_deref() {
        tracing::warn!(
            hostname = %hostname,
            error = %reason,
            "virtual machine scan failed on the agent"
        );
    } else if let Err(e) = db::vms::record_scan(&state.pool, agent_id, &vms).await {
        tracing::error!(
            hostname = %hostname,
            error = %e,
            "failed to record the virtual machines an agent reported"
        );
    }

    if let Some(request_id) = request_id {
        answer_pending(
            &state.pending_vm_scans,
            &request_id,
            agent_id,
            hostname,
            (vms, error),
        )
        .await;
    }
}

/// Records the database workloads an agent found and hands them to whoever
/// asked for the scan. A runtime that could not be searched keeps the sources
/// it had, so a cluster that is briefly unreachable does not wipe them, while
/// what the other runtimes found is still recorded. A failure that names no
/// runtime (an agent too old to say which one failed, or no runtime turned on)
/// records nothing.
pub(super) async fn handle_database_scan_result(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    request_id: String,
    (sources, error): (
        Vec<shared::database::DiscoveredDatabaseSource>,
        Option<String>,
    ),
    failed_runtimes: &[shared::database::DatabaseRuntime],
) {
    if let Some(reason) = error.as_deref() {
        tracing::warn!(hostname = %hostname, error = %reason, "database scan failed on the agent");
    }
    let complete_for_searched = error.is_none() || !failed_runtimes.is_empty();
    if complete_for_searched
        && let Err(e) =
            db::databases::record_database_scan(&state.pool, agent_id, &sources, failed_runtimes)
                .await
    {
        tracing::error!(
            hostname = %hostname,
            error = %e,
            "failed to record the database sources an agent reported"
        );
    }
    answer_pending(
        &state.pending_database_scans,
        &request_id,
        agent_id,
        hostname,
        (sources, error, failed_runtimes.to_vec()),
    )
    .await;
}

/// Records what staging one domain right now did to it, and hands the
/// outcome to whoever asked for the snapshot. Recorded whether or not it
/// carries an error, so the domain's row shows the failure on the next load
/// even though the request itself reports it as failed.
pub(super) async fn handle_vm_stage_result(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    request_id: Option<String>,
    outcome: shared::vm::VmSnapshotOutcome,
) {
    if let Some(reason) = outcome.error.as_deref() {
        tracing::warn!(
            hostname = %hostname,
            domain = %outcome.name,
            error = %reason,
            "manual virtual machine snapshot failed on the agent"
        );
    }
    if let Err(e) =
        db::vms::record_outcomes(&state.pool, agent_id, std::slice::from_ref(&outcome)).await
    {
        tracing::error!(
            hostname = %hostname,
            error = %e,
            "failed to record a manual virtual machine snapshot outcome"
        );
    }

    if let Some(request_id) = request_id {
        answer_pending(
            &state.pending_vm_stages,
            &request_id,
            agent_id,
            hostname,
            outcome,
        )
        .await;
    }
}

/// Hands `answer` to whoever waits on `request_id` in `pending`, provided
/// `agent_id` is the agent the request was sent to. Returns `false` when no
/// request is waiting there under that id.
///
/// An answer to a request that was sent to another agent is dropped with a
/// warning and leaves that request pending for its own agent, so one agent
/// cannot fulfill or fail another agent's operation by sending its request
/// id.
pub(super) async fn answer_pending<T>(
    pending: &PendingRequests<T>,
    request_id: &str,
    agent_id: i64,
    hostname: &str,
    answer: T,
) -> bool {
    match pending.claim(request_id, agent_id).await {
        Claim::Claimed(tx) => {
            let _ = tx.send(answer);
            true
        }
        Claim::WrongAgent { expected_agent_id } => {
            tracing::warn!(
                hostname = %hostname,
                agent_id,
                expected_agent_id,
                request_id = %request_id,
                "ignoring an answer to a request that was sent to another agent"
            );
            true
        }
        Claim::Unknown => false,
    }
}

pub(super) async fn handle_restore_completed(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    request_id: String,
    success: bool,
    files_restored: u64,
    error_message: Option<String>,
) {
    let outcome = RestoreOutcome {
        success,
        files_restored: i64::try_from(files_restored).unwrap_or(i64::MAX),
        error_message,
    };
    match restore_runs::settle(state, agent_id, &request_id, outcome).await {
        Settlement::Recorded | Settlement::Retrying => {}
        Settlement::NotRunning => tracing::warn!(
            hostname = %hostname,
            request_id = %request_id,
            "unexpected RestoreCompleted for no restore running on this agent"
        ),
    }
}

/// Fails the restore `request_id` names with `error`, if it is one this
/// agent is running.
pub(super) async fn fail_restore(
    agent_id: i64,
    state: &AppState,
    request_id: &str,
    error: String,
) -> FailedRequest {
    let outcome = RestoreOutcome {
        success: false,
        files_restored: 0,
        error_message: Some(error),
    };
    match restore_runs::settle(state, agent_id, request_id, outcome).await {
        Settlement::Recorded => FailedRequest::Answered,
        Settlement::Retrying => FailedRequest::Unrecorded,
        Settlement::NotRunning => FailedRequest::Unknown,
    }
}

pub(super) async fn handle_migrate_encryption_completed(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    request_id: String,
    success: bool,
    error_message: Option<String>,
) {
    if !answer_pending(
        &state.pending_migrations,
        &request_id,
        agent_id,
        hostname,
        (success, error_message),
    )
    .await
    {
        tracing::warn!(
            hostname = %hostname,
            request_id = %request_id,
            "unexpected MigrateEncryptionCompleted with no pending request"
        );
    }
}

pub(super) async fn handle_dry_run_result(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    request_id: String,
    files: Vec<shared::types::DryRunFile>,
    total_size: i64,
    error_message: Option<String>,
) {
    if !answer_pending(
        &state.pending_dryruns,
        &request_id,
        agent_id,
        hostname,
        (files, total_size, error_message),
    )
    .await
    {
        tracing::warn!(
            hostname = %hostname,
            request_id = %request_id,
            "unexpected DryRunResult with no pending request"
        );
    }
}

pub(super) async fn handle_operation_failed(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    request_id: String,
    error: String,
) {
    if fail_pending(hostname, agent_id, state, &request_id, error).await == FailedRequest::Unknown {
        tracing::warn!(
            hostname = %hostname,
            request_id = %request_id,
            "unexpected OperationFailed with no pending request"
        );
    }
}

/// Fails whatever request `request_id` named, since the agent could not
/// handle it, so a request type added later fails fast on an agent that
/// predates it.
pub(super) async fn handle_unsupported_message(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    request_id: String,
    message_type: String,
) {
    tracing::warn!(
        hostname = %hostname,
        request_id = %request_id,
        message_type = %message_type,
        "agent does not support a message it was sent"
    );
    let error = format!(
        "the agent on {hostname} does not support {message_type}; update the agent to a version \
         that matches the server"
    );
    if fail_pending(hostname, agent_id, state, &request_id, error).await == FailedRequest::Unknown {
        tracing::warn!(
            hostname = %hostname,
            request_id = %request_id,
            "unexpected UnsupportedMessage with no pending request"
        );
    }
}

/// What became of a failure an agent reported for one of its requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FailedRequest {
    /// Whoever waited on the request has its answer.
    Answered,
    /// The failure is for a restore whose outcome the database rejected;
    /// it is retried in the background until recorded.
    Unrecorded,
    /// No request waits on that id.
    Unknown,
}

/// Fails the request waiting on `request_id` with `error`, in whichever
/// registry holds it among those whose answer can carry an error.
async fn fail_pending(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    request_id: &str,
    error: String,
) -> FailedRequest {
    let answered = answer_pending(
        &state.pending_dryruns,
        request_id,
        agent_id,
        hostname,
        (Vec::new(), 0, Some(error.clone())),
    )
    .await
        || answer_pending(
            &state.pending_deletes,
            request_id,
            agent_id,
            hostname,
            (false, 0, Some(error.clone())),
        )
        .await
        || answer_pending(
            &state.pending_migrations,
            request_id,
            agent_id,
            hostname,
            (false, Some(error.clone())),
        )
        .await
        || answer_pending(
            &state.pending_vm_scans,
            request_id,
            agent_id,
            hostname,
            (Vec::new(), Some(error.clone())),
        )
        .await
        || answer_pending(
            &state.pending_database_scans,
            request_id,
            agent_id,
            hostname,
            (Vec::new(), Some(error.clone()), Vec::new()),
        )
        .await
        || answer_pending(
            &state.pending_vm_builds,
            request_id,
            agent_id,
            hostname,
            (None, Some(error.clone())),
        )
        .await;
    if answered {
        return FailedRequest::Answered;
    }
    fail_restore(agent_id, state, request_id, error).await
}

pub(super) async fn handle_delete_archives_result(
    hostname: &str,
    agent_id: i64,
    state: &AppState,
    request_id: String,
    success: bool,
    deleted_count: u32,
    error_message: Option<String>,
) {
    if !answer_pending(
        &state.pending_deletes,
        &request_id,
        agent_id,
        hostname,
        (success, deleted_count, error_message),
    )
    .await
    {
        tracing::warn!(
            hostname = %hostname,
            request_id = %request_id,
            "unexpected DeleteArchivesResult with no pending request"
        );
    }
}
