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
    BackupTaskContext, databases::prepare_dumps, send_outbound, transport::setup_ssh_forward,
    vms::stage_configured_machines,
};
use crate::{
    backup::{BackupEngine, BackupError, BackupResult, BackupTarget, CanaryResult, CanaryToken},
    database::{PreparedDumps, RuntimeClients},
};

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
        server,
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
    target.run_events = run_id.clone().map(|run_id| crate::snapshot::RunEvents {
        outbound: outbound_tx.clone(),
        run_id,
        repo_id,
    });

    let dumps = match prepare_run(&target, schedule_id, outbound_tx, engine).await {
        Ok(dumps) => dumps,
        Err(reason) => {
            error!(repo_id = ?repo_id, reason = %reason, "backup could not be prepared");
            let report = failed_with_setup_warnings(
                make_failed_report(
                    repo_id,
                    schedule_id,
                    started_at,
                    reason,
                    run_id,
                    Some(borg_command),
                ),
                &target.setup_warnings,
            );
            report_backup_failure(report, outbound_tx).await;
            return;
        }
    };

    let _ssh_forward = setup_ssh_forward(&mut target, &hostname, &server).await;

    let canary = write_canary_if_enabled(repo_id, &target).await;

    let (log_tx, log_forwarder) = spawn_log_forwarder(repo_id, schedule_id, outbound_tx.clone());
    let completed: CompletedBackup = match engine
        .run_backup_with_dumps(&target, canary.as_ref(), Some(log_tx), dumps.as_ref())
        .await
    {
        Ok(result) => build_backup_report(
            repo_id,
            schedule_id,
            started_at,
            run_id.clone(),
            with_setup_warnings(result, &target.setup_warnings),
        ),
        Err(e) => {
            let (reason, command) = match e {
                BackupError::Skipped(reason) => {
                    error!(repo_id = ?repo_id, reason = %reason, "backup skipped, treating as failure");
                    (reason, Some(borg_command))
                }
                e => {
                    error!(repo_id = ?repo_id, error = %e, "backup failed");
                    let command = e
                        .borg_command()
                        .map(ToOwned::to_owned)
                        .or(Some(borg_command));
                    (e.to_string(), command)
                }
            };
            let report = make_failed_report(
                repo_id,
                schedule_id,
                started_at,
                reason,
                run_id.clone(),
                command,
            );
            failed_with_setup_warnings(report, &target.setup_warnings).into()
        }
    };

    if let Some(dumps) = dumps {
        dumps.remove().await;
    }

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

/// Everything that happens before borg runs, returning the database dumps
/// to stream into the archive.
///
/// Virtual machines are staged first, so the archive holds the images this run
/// produced. A domain that could not be staged, or a database that cannot be
/// dumped, fails the backup: an archive that quietly holds last night's image
/// or lacks a database is worse than a run the operator is told about.
async fn prepare_run(
    target: &BackupTarget,
    schedule_id: Option<i64>,
    outbound_tx: &mpsc::Sender<AgentToServer>,
    engine: &BackupEngine,
) -> Result<Option<PreparedDumps<RuntimeClients>>, String> {
    stage_configured_machines(target, schedule_id, outbound_tx, engine).await?;
    prepare_dumps(target).await
}

/// Reports a backup that failed before borg was even reached, so the run shows
/// up with its reason instead of silently never finishing.
pub(super) async fn report_backup_failure(
    report: shared::types::BackupReport,
    outbound_tx: &mpsc::Sender<AgentToServer>,
) {
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

/// Writes the canary file a canary-enabled target checks after the backup;
/// a failure to write it is logged and the backup goes on without one.
async fn write_canary_if_enabled(repo_id: RepoId, target: &BackupTarget) -> Option<CanaryToken> {
    if !target.canary_enabled {
        return None;
    }
    match BackupEngine::write_canary(&target.backup_sources).await {
        Ok(c) => Some(c),
        Err(e) => {
            warn!(repo_id = ?repo_id, error = %e, "canary write failed, proceeding without");
            None
        }
    }
}

/// A run that left out what its schedule asked for says so on its report,
/// and reads as a warning rather than a clean success.
fn with_setup_warnings(mut result: BackupResult, setup_warnings: &[String]) -> BackupResult {
    if !setup_warnings.is_empty() {
        result.warnings.extend(setup_warnings.iter().cloned());
        if result.status == shared::types::BackupStatus::Success {
            result.status = shared::types::BackupStatus::Warning;
        }
    }
    result
}

/// A failed or skipped run still says what it left out by the host's own
/// settings: the operator reading why it failed should also learn that, say,
/// its database dumps were never going to run on this host.
fn failed_with_setup_warnings(
    mut report: shared::types::BackupReport,
    setup_warnings: &[String],
) -> shared::types::BackupReport {
    report.warnings.extend(setup_warnings.iter().cloned());
    report
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

#[cfg(test)]
mod tests {
    use shared::types::BackupStatus;

    use super::*;

    fn result(status: BackupStatus, warnings: &[&str]) -> BackupResult {
        BackupResult {
            status,
            original_size: 0,
            compressed_size: 0,
            deduplicated_size: 0,
            repo_unique_csize: 0,
            files_processed: 0,
            duration_secs: 0,
            error_message: None,
            warnings: warnings.iter().map(|w| (*w).to_owned()).collect(),
            warning_kind: BackupWarningKind::General,
            archive_name: None,
            borg_command: None,
            canary_result: None,
        }
    }

    #[test]
    fn setup_warnings_turn_a_clean_success_into_a_warning() {
        let out = with_setup_warnings(
            result(BackupStatus::Success, &[]),
            &["database dump skipped".to_owned()],
        );
        assert_eq!(out.status, BackupStatus::Warning);
        assert_eq!(out.warnings, vec!["database dump skipped".to_owned()]);
    }

    #[test]
    fn setup_warnings_are_added_without_masking_a_worse_status() {
        let out = with_setup_warnings(
            result(BackupStatus::Failed, &["borg: file vanished"]),
            &["vm skipped".to_owned()],
        );
        assert_eq!(out.status, BackupStatus::Failed);
        assert_eq!(
            out.warnings,
            vec!["borg: file vanished".to_owned(), "vm skipped".to_owned()]
        );
    }

    #[test]
    fn no_setup_warnings_leave_the_result_as_it_was() {
        let out = with_setup_warnings(result(BackupStatus::Success, &[]), &[]);
        assert_eq!(out.status, BackupStatus::Success);
        assert_eq!(out.warnings, Vec::<String>::new());
    }

    fn failed(reason: &str) -> shared::types::BackupReport {
        make_failed_report(
            RepoId(1),
            Some(2),
            Utc::now(),
            reason.to_owned(),
            Some("run-1".to_owned()),
            None,
        )
    }

    #[test]
    fn a_failed_run_still_carries_what_the_host_left_out() {
        let out = failed_with_setup_warnings(
            failed("borg exited with code 2"),
            &[crate::executor::transport::DATABASES_OFF_WARNING.to_owned()],
        );
        assert_eq!(out.status, BackupStatus::Failed);
        assert_eq!(
            out.error_message.as_deref(),
            Some("borg exited with code 2")
        );
        assert_eq!(
            out.warnings,
            vec![crate::executor::transport::DATABASES_OFF_WARNING.to_owned()]
        );
    }

    #[test]
    fn a_failed_run_without_setup_warnings_has_none() {
        let out = failed_with_setup_warnings(failed("staging failed"), &[]);
        assert_eq!(out.status, BackupStatus::Failed);
        assert_eq!(out.warnings, Vec::<String>::new());
    }
}
