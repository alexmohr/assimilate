// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::Utc;
use shared::{
    protocol::AgentToServer,
    types::{BackupWarningKind, RepoId},
};
use tokio::sync::mpsc;
use tracing::{error, warn};

use super::{
    BackupTaskContext, send_outbound, transport::setup_ssh_forward, vms::stage_configured_machines,
};
use crate::backup::{BackupEngine, BackupError, BackupResult, BackupTarget, CanaryResult};

pub(super) fn make_failed_report(
    repo_id: RepoId,
    schedule_id: Option<i64>,
    started_at: chrono::DateTime<Utc>,
    error_message: String,
    run_id: Option<String>,
    borg_command: Option<String>,
) -> shared::types::BackupReport {
    let finished_at = Utc::now();
    shared::types::BackupReport {
        id: shared::types::ReportId(0),
        agent_id: shared::types::AgentId(0),
        repo_id,
        schedule_id,
        started_at,
        finished_at,
        status: shared::types::BackupStatus::Failed,
        original_size: 0,
        compressed_size: 0,
        deduplicated_size: 0,
        repo_unique_csize: 0,
        files_processed: 0,
        duration_secs: finished_at.signed_duration_since(started_at).num_seconds(),
        error_message: Some(error_message),
        warnings: Vec::new(),
        borg_version: None,
        archive_name: None,
        borg_command,
        run_id,
    }
}

pub(super) fn spawn_log_forwarder(
    repo_id: RepoId,
    schedule_id: Option<i64>,
    outbound_tx: mpsc::Sender<AgentToServer>,
) -> (mpsc::Sender<String>, tokio::task::JoinHandle<()>) {
    let (log_tx, mut log_rx) = mpsc::channel::<String>(256);
    let handle = tokio::spawn(async move {
        while let Some(line) = log_rx.recv().await {
            let msg = AgentToServer::BackupLog {
                repo_id,
                schedule_id,
                line,
            };
            if outbound_tx.send(msg).await.is_err() {
                break;
            }
        }
    });
    (log_tx, handle)
}

pub(super) async fn run_backup_task(
    repo_id: RepoId,
    mut target: BackupTarget,
    ctx: BackupTaskContext,
    engine: &BackupEngine,
    outbound_tx: &mpsc::Sender<AgentToServer>,
) {
    let BackupTaskContext {
        hostname,
        server_url,
        token,
        run_id,
    } = ctx;
    let started_at = Utc::now();
    let schedule_id = target.schedule_id;
    let borg_command = BackupEngine::preview_create_command(&target);
    let started_msg = AgentToServer::BackupStarted {
        repo_id,
        schedule_id,
        started_at,
        borg_command: Some(borg_command.clone()),
        run_id: run_id.clone(),
    };
    send_outbound(outbound_tx, started_msg).await;

    // Virtual machines are staged before borg runs, so the archive holds the
    // images this run produced. A domain that could not be staged fails the
    // backup: an archive that quietly holds last night's image is worse than a
    // run the operator is told about.
    let staging = stage_configured_machines(&target, schedule_id, outbound_tx, engine).await;
    if let Err(reason) = staging {
        error!(repo_id = ?repo_id, reason = %reason, "virtual machine staging failed");
        report_backup_failure(
            repo_id,
            schedule_id,
            started_at,
            reason,
            run_id,
            borg_command,
            outbound_tx,
        )
        .await;
        return;
    }

    let _ssh_forward = setup_ssh_forward(&mut target, &hostname, &server_url, &token).await;

    let canary = if target.canary_enabled {
        match BackupEngine::write_canary(&target.backup_sources).await {
            Ok(c) => Some(c),
            Err(e) => {
                warn!(repo_id = ?repo_id, error = %e, "canary write failed, proceeding without");
                None
            }
        }
    } else {
        None
    };

    let (log_tx, log_forwarder) = spawn_log_forwarder(repo_id, schedule_id, outbound_tx.clone());
    let completed: CompletedBackup = match engine
        .run_backup(&target, canary.as_ref(), Some(log_tx))
        .await
    {
        Ok(result) => build_backup_report(repo_id, schedule_id, started_at, run_id.clone(), result),
        Err(BackupError::Skipped(reason)) => {
            error!(repo_id = ?repo_id, reason = %reason, "backup skipped, treating as failure");
            make_failed_report(
                repo_id,
                schedule_id,
                started_at,
                reason,
                run_id.clone(),
                Some(borg_command),
            )
            .into()
        }
        Err(e) => {
            error!(repo_id = ?repo_id, error = %e, "backup failed");
            let failed_command = e
                .borg_command()
                .map(std::borrow::ToOwned::to_owned)
                .or_else(|| Some(borg_command.clone()));
            make_failed_report(
                repo_id,
                schedule_id,
                started_at,
                e.to_string(),
                run_id.clone(),
                failed_command,
            )
            .into()
        }
    };

    if let Err(e) = log_forwarder.await {
        tracing::debug!(error = %e, "log forwarder task panicked");
    }

    let (msg, canary_result) = completed.into_message();
    send_outbound(outbound_tx, msg).await;

    if let Some(canary) = &canary {
        send_canary_result(repo_id, canary.nonce.clone(), canary_result, outbound_tx).await;
        BackupEngine::cleanup_canary(canary).await;
    }
}

/// Reports a backup that failed before borg was even reached, so the run shows
/// up with its reason instead of silently never finishing.
pub(super) async fn report_backup_failure(
    repo_id: RepoId,
    schedule_id: Option<i64>,
    started_at: chrono::DateTime<Utc>,
    reason: String,
    run_id: Option<String>,
    borg_command: String,
    outbound_tx: &mpsc::Sender<AgentToServer>,
) {
    let report = make_failed_report(
        repo_id,
        schedule_id,
        started_at,
        reason,
        run_id,
        Some(borg_command),
    );
    send_outbound(
        outbound_tx,
        AgentToServer::BackupCompleted {
            report,
            warning_kind: BackupWarningKind::General,
        },
    )
    .await;
}

pub(super) async fn send_canary_result(
    repo_id: RepoId,
    nonce: String,
    result: Option<CanaryResult>,
    outbound_tx: &mpsc::Sender<AgentToServer>,
) {
    let Some(result) = result else { return };
    let msg = AgentToServer::CanaryVerified {
        repo_id,
        success: result.success,
        nonce,
        archive_name: result.archive_name,
        error_message: result.error_message,
    };
    send_outbound(outbound_tx, msg).await;
}

/// Everything a finished backup task reports back to the server.
pub(super) struct CompletedBackup {
    report: shared::types::BackupReport,
    warning_kind: BackupWarningKind,
    canary_result: Option<CanaryResult>,
}

impl CompletedBackup {
    /// The message that reports this run, and the canary result that is sent
    /// on its own once the report is out.
    fn into_message(self) -> (AgentToServer, Option<CanaryResult>) {
        let msg = AgentToServer::BackupCompleted {
            report: self.report,
            warning_kind: self.warning_kind,
        };
        (msg, self.canary_result)
    }
}

/// A run that failed warned about nothing and verified no canary.
impl From<shared::types::BackupReport> for CompletedBackup {
    fn from(report: shared::types::BackupReport) -> Self {
        Self {
            report,
            warning_kind: BackupWarningKind::General,
            canary_result: None,
        }
    }
}

pub(super) fn build_backup_report(
    repo_id: RepoId,
    schedule_id: Option<i64>,
    started_at: chrono::DateTime<Utc>,
    run_id: Option<String>,
    result: BackupResult,
) -> CompletedBackup {
    let finished_at = Utc::now();
    let report = shared::types::BackupReport {
        id: shared::types::ReportId(0),
        agent_id: shared::types::AgentId(0),
        repo_id,
        schedule_id,
        started_at,
        finished_at,
        status: result.status,
        original_size: result.original_size,
        compressed_size: result.compressed_size,
        deduplicated_size: result.deduplicated_size,
        repo_unique_csize: result.repo_unique_csize,
        files_processed: result.files_processed,
        duration_secs: result.duration_secs,
        error_message: result.error_message,
        warnings: result.warnings,
        borg_version: None,
        archive_name: result.archive_name,
        borg_command: result.borg_command,
        run_id,
    };
    CompletedBackup {
        report,
        warning_kind: result.warning_kind,
        canary_result: result.canary_result,
    }
}
