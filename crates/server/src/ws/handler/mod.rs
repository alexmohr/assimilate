// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Agent WebSocket endpoint: the upgrade handler and the routing of every
//! [`AgentToServer`] message to its handler.
//!
//! The handlers live in submodules by concern: [`connection`] (handshake,
//! socket loop, reconnect catch-up), [`backup`] and [`completion`] (a backup
//! run from start to finished report), [`failure`] (classifying a failed run
//! and its catch-up), [`quota`] and [`notify`] (what a finished run triggers),
//! [`maintenance`] (check, verify and canary results), [`replies`] (answers
//! to the server's own requests) and [`run_event`] (timeline steps the agent
//! reports, such as filesystem snapshots).

use axum::{
    extract::{State, ws::WebSocketUpgrade},
    response::IntoResponse,
};
use shared::{protocol::AgentToServer, types::SystemEventType};
use sqlx::PgPool;

use self::{
    backup::{
        BackupStartedArgs, handle_backup_cancelled, handle_backup_rejected, handle_backup_started,
    },
    completion::handle_backup_completed,
    connection::handle_socket,
    maintenance::{
        CanaryVerifiedArgs, CheckCompletedArgs, VerifyCompletedArgs, handle_canary_verified,
        handle_check_completed, handle_verify_completed,
    },
    replies::{
        answer_pending, handle_backup_log, handle_database_scan_result,
        handle_delete_archives_result, handle_dry_run_result, handle_live_file_read,
        handle_migrate_encryption_completed, handle_operation_failed, handle_restore_completed,
        handle_unsupported_message, handle_vm_scan_result, handle_vm_stage_result,
    },
    run_event::{AgentRunEvent, record_agent_run_event},
};
use crate::{AppState, db};

mod backup;
mod completion;
mod connection;
mod failure;
mod maintenance;
mod notify;
mod quota;
mod replies;
mod run_event;

pub(crate) use connection::verify_agent_token;

/// WebSocket upgrade handler for agent connections.
///
/// Expects the encrypted channel's handshake, then a `Hello` message inside
/// it; authenticates the agent,
/// registers it in the agent registry, and starts the bidirectional message loop.
pub fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl std::future::Future<Output = impl IntoResponse> {
    std::future::ready(ws.on_upgrade(|socket| handle_socket(socket, state)))
}

pub(crate) async fn validate_agent_repo(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
    hostname: &str,
    msg_type: &str,
) -> bool {
    let Ok(has_access) = db::check_agent_repo_access(pool, agent_id, repo_id).await else {
        tracing::error!(
            hostname = %hostname,
            agent_id = agent_id,
            repo_id = repo_id,
            msg_type = %msg_type,
            "validate_agent_repo: DB error checking agent repo access"
        );
        return false;
    };
    if has_access {
        return true;
    }
    tracing::warn!(
        security = true,
        hostname = %hostname,
        agent_id = agent_id,
        repo_id = repo_id,
        msg_type = %msg_type,
        "agent tried to report on a repo it is not assigned to"
    );
    if let Err(e) = db::insert_system_event(
        pool,
        SystemEventType::SecurityViolation,
        Some(hostname),
        &format!(
            "Agent '{hostname}' (id={agent_id}) tried to report on repo {repo_id} without \
             assignment (msg={msg_type})"
        ),
    )
    .await
    {
        tracing::error!(error = %e, "failed to insert security_violation system event");
    }
    false
}

/// Dispatches a decoded [`AgentToServer`] message to its handler.
///
/// Every variant carrying real logic is delegated to its own `handle_*`
/// function below (each independently under the line-count limit); what
/// remains here is a flat, one-arm-per-variant routing table over the full
/// protocol enum. Splitting that table across multiple functions would not
/// remove any logic -- it would just force a reader chasing "what happens
/// for message X" to jump between several partial matches instead of
/// reading one.
#[allow(
    clippy::too_many_lines,
    reason = "flat 19-variant protocol dispatch table; every arm with real logic already \
              delegates to its own handle_* function, so splitting this further would fragment \
              routing logic across multiple partial matches without removing any code"
)]
async fn handle_agent_message(text: &str, hostname: &str, agent_id: i64, state: &AppState) {
    let msg = match serde_json::from_str::<AgentToServer>(text) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(hostname = %hostname, error = %e, "invalid message from agent");
            return;
        }
    };

    match msg {
        AgentToServer::Pong => {
            if let Err(e) = db::update_last_seen_by_hostname(&state.pool, hostname).await {
                tracing::error!(
                    hostname = %hostname,
                    error = %e,
                    "failed to update last_seen_at"
                );
            }
        }
        AgentToServer::BackupStarted {
            repo_id,
            schedule_id,
            started_at,
            borg_command,
            run_id,
        } => {
            handle_backup_started(BackupStartedArgs {
                hostname,
                agent_id,
                state,
                repo_id,
                schedule_id,
                started_at,
                borg_command,
                run_id,
            })
            .await;
        }
        AgentToServer::BackupCompleted {
            report,
            warning_kind,
        } => {
            handle_backup_completed(hostname, agent_id, state, report, warning_kind).await;
        }
        AgentToServer::StatusUpdate { repo_id, status } => {
            tracing::info!(
                hostname = %hostname,
                repo_id = ?repo_id,
                status = ?status,
                "status update"
            );
            // Guard is applied for future-proofing even though StatusUpdate
            // is currently a no-op.
            let _ = validate_agent_repo(&state.pool, agent_id, repo_id.0, hostname, "StatusUpdate")
                .await;
        }
        AgentToServer::BackupRejected { repo_id, reason } => {
            handle_backup_rejected(hostname, agent_id, state, repo_id, &reason).await;
        }
        AgentToServer::CheckCompleted {
            repo_id,
            success,
            duration_secs,
            error_message,
        } => {
            handle_check_completed(CheckCompletedArgs {
                hostname,
                agent_id,
                state,
                repo_id,
                success,
                duration_secs,
                error_message,
            })
            .await;
        }
        AgentToServer::VerifyCompleted {
            repo_id,
            success,
            duration_secs,
            error_message,
            files_verified,
        } => {
            handle_verify_completed(VerifyCompletedArgs {
                hostname,
                agent_id,
                state,
                repo_id,
                success,
                duration_secs,
                error_message,
                files_verified,
            })
            .await;
        }
        AgentToServer::CanaryVerified {
            repo_id,
            success,
            nonce,
            archive_name,
            error_message,
        } => {
            handle_canary_verified(CanaryVerifiedArgs {
                hostname,
                agent_id,
                state,
                repo_id,
                success,
                nonce,
                archive_name,
                error_message,
            })
            .await;
        }
        AgentToServer::BackupLog {
            repo_id,
            schedule_id,
            line,
        } => {
            handle_backup_log(hostname, agent_id, state, repo_id, schedule_id, line).await;
        }
        AgentToServer::DatabaseScanResult {
            request_id,
            sources,
            error,
            failed_runtimes,
        } => {
            handle_database_scan_result(
                hostname,
                agent_id,
                state,
                request_id,
                (sources, error),
                &failed_runtimes,
            )
            .await;
        }
        AgentToServer::VmScanResult {
            request_id,
            vms,
            error,
        } => {
            handle_vm_scan_result(hostname, agent_id, state, request_id, vms, error).await;
        }
        AgentToServer::RunEvent {
            run_id,
            repo_id,
            event_type,
            message,
        } => {
            record_agent_run_event(
                state,
                agent_id,
                hostname,
                AgentRunEvent {
                    run_id,
                    repo_id: repo_id.0,
                    event_type,
                    message,
                },
            )
            .await;
        }
        AgentToServer::VmSnapshotReport {
            schedule_id,
            outcomes,
        } => {
            let domains = outcomes.len();
            tracing::info!(
                hostname = %hostname,
                schedule_id = ?schedule_id,
                domains,
                "agent staged virtual machines"
            );
            if let Err(e) = db::vms::record_outcomes(&state.pool, agent_id, &outcomes).await {
                tracing::error!(
                    hostname = %hostname,
                    error = %e,
                    "failed to record virtual machine snapshot outcomes"
                );
            }
        }
        AgentToServer::VmBuildResult {
            request_id,
            outcome,
            error,
        } => {
            if let Some(reason) = error.as_deref() {
                tracing::warn!(
                    hostname = %hostname,
                    error = %reason,
                    "virtual machine build failed on the agent"
                );
            }
            answer_pending(
                &state.pending_vm_builds,
                &request_id,
                agent_id,
                hostname,
                (outcome, error),
            )
            .await;
        }
        AgentToServer::VmStageResult {
            request_id,
            outcome,
        } => {
            handle_vm_stage_result(hostname, agent_id, state, request_id, outcome).await;
        }
        AgentToServer::Hello { .. } => {
            tracing::warn!(hostname = %hostname, "unexpected Hello after handshake");
        }
        AgentToServer::RestartFailed { error_message } => {
            tracing::error!(
                hostname = %hostname,
                error = %error_message,
                "agent restart failed"
            );
        }
        AgentToServer::InitRepoCompleted {
            repo_path,
            success,
            error_message,
        } => {
            tracing::info!(
                hostname = %hostname,
                repo_path = %repo_path,
                success,
                error_message = ?error_message,
                "init repo completed"
            );
        }
        AgentToServer::SearchResult { .. }
        | AgentToServer::ExportReady { .. }
        | AgentToServer::KeyExportResult { .. }
        | AgentToServer::KeyImportResult { .. }
        | AgentToServer::PassphraseChanged { .. }
        | AgentToServer::OperationProgress { .. } => {
            tracing::warn!(hostname = %hostname, "unexpected agent response");
        }
        AgentToServer::RestoreCompleted {
            request_id,
            success,
            files_restored,
            error_message,
        } => {
            handle_restore_completed(
                hostname,
                agent_id,
                state,
                request_id,
                success,
                files_restored,
                error_message,
            )
            .await;
        }
        AgentToServer::MigrateEncryptionCompleted {
            request_id,
            success,
            error_message,
        } => {
            handle_migrate_encryption_completed(
                hostname,
                agent_id,
                state,
                request_id,
                success,
                error_message,
            )
            .await;
        }
        AgentToServer::DryRunResult {
            request_id,
            files,
            total_size,
            error_message,
        } => {
            handle_dry_run_result(
                hostname,
                agent_id,
                state,
                request_id,
                files,
                total_size,
                error_message,
            )
            .await;
        }
        AgentToServer::OperationFailed { request_id, error } => {
            handle_operation_failed(hostname, agent_id, state, request_id, error).await;
        }
        AgentToServer::LiveFileRead { request_id, file } => {
            handle_live_file_read(hostname, agent_id, state, request_id, file).await;
        }
        AgentToServer::UnsupportedMessage {
            request_id,
            message_type,
        } => {
            handle_unsupported_message(hostname, agent_id, state, request_id, message_type).await;
        }
        AgentToServer::DeleteArchivesResult {
            request_id,
            success,
            deleted_count,
            error_message,
        } => {
            handle_delete_archives_result(
                hostname,
                agent_id,
                state,
                request_id,
                success,
                deleted_count,
                error_message,
            )
            .await;
        }
        AgentToServer::BackupCancelled { repo_id } => {
            handle_backup_cancelled(hostname, agent_id, state, repo_id).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    use axum::extract::ws::Message;
    use chrono::{TimeZone, Utc};
    use futures_util::{Sink, SinkExt, Stream, StreamExt};
    use shared::{
        crypto::{derive_key, encrypt_passphrase},
        protocol::{AgentToServer, ServerToAgent, ServerToUi},
        types::{
            AcknowledgedFilter, AgentId, BackupReport, BackupStatus, QuotaAction, RepoId, ReportId,
            RunEventTarget, RunEventType, ScheduleType, ScheduleWakeOverride,
        },
    };
    use sqlx::PgPool;
    use tokio::{sync::mpsc, time::timeout};
    use tokio_util::sync::CancellationToken;

    use super::{
        backup::spawn_post_backup_indexing,
        connection::{
            HelloFields, ping_loop, record_agent_seen,
            reenable_system_disabled_schedules_on_reconnect, send_close, send_reconnect_catchup,
            send_ws_message, serve_agent, spawn_restore_catchup,
        },
        failure::{
            classify_failed_backup, clear_repo_catch_up_on_success, mark_repo_catch_up_pending,
            settle_dependency_catch_up,
        },
        notify::warning_event_type,
        quota::{
            check_repo_quota_after_backup, check_server_quota_after_backup, quota_event_type,
            quota_status_label,
        },
        replies::{FailedRequest, fail_restore},
        run_event::MAX_AGENT_RUN_EVENT_MESSAGE,
        *,
    };
    use crate::{catch_up, notifications::EventType, ws::post_backup_sync};

    #[tokio::test]
    async fn ping_loop_exits_promptly_when_shutdown_token_cancelled() {
        let (tx, mut rx) = mpsc::channel::<ServerToAgent>(4);
        let token = CancellationToken::new();

        let handle = tokio::spawn(ping_loop(tx, token.clone()));
        token.cancel();

        let result = timeout(Duration::from_secs(5), handle).await;
        assert!(
            result.is_ok(),
            "ping_loop did not exit within 5s after shutdown_token cancellation"
        );
        assert!(rx.try_recv().is_err(), "no ping should have been sent");
    }

    /// The other way out: the connection's writer is gone. e2e only reaches it
    /// when an agent disconnects between two pings, which made the line's
    /// coverage depend on teardown timing (#353).
    #[tokio::test]
    async fn ping_loop_exits_once_the_connection_is_gone() {
        let (tx, rx) = mpsc::channel::<ServerToAgent>(4);
        drop(rx);

        let result = timeout(
            Duration::from_secs(5),
            ping_loop(tx, CancellationToken::new()),
        )
        .await;

        assert!(
            result.is_ok(),
            "ping_loop kept running after its receiver was dropped"
        );
    }

    #[test]
    fn quota_status_label_covers_every_status() {
        assert_eq!(quota_status_label(db::quota::QuotaStatus::Ok), "ok");
        assert_eq!(
            quota_status_label(db::quota::QuotaStatus::Warning),
            "warning"
        );
        assert_eq!(
            quota_status_label(db::quota::QuotaStatus::Critical),
            "critical"
        );
    }

    #[test]
    fn warning_event_type_sets_file_changes_apart() {
        assert!(matches!(
            warning_event_type(shared::types::BackupWarningKind::General),
            EventType::BackupWarning
        ));
        assert!(matches!(
            warning_event_type(shared::types::BackupWarningKind::FileChanged),
            EventType::BackupFileChanged
        ));
    }

    #[test]
    fn quota_event_type_covers_every_status() {
        assert!(matches!(
            quota_event_type(db::quota::QuotaStatus::Ok),
            EventType::BackupSuccess
        ));
        assert!(matches!(
            quota_event_type(db::quota::QuotaStatus::Warning),
            EventType::BackupWarning
        ));
        assert!(matches!(
            quota_event_type(db::quota::QuotaStatus::Critical),
            EventType::BackupFailed
        ));
    }

    static TEST_BORG_BINARY_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn build_test_state(pool: PgPool) -> AppState {
        crate::test_support::build_test_state(pool, b"handler-test-secret-key")
    }

    async fn write_fake_borg_binary() -> PathBuf {
        let counter = TEST_BORG_BINARY_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "borg-fake-handler-{pid}-{counter}",
            pid = std::process::id()
        ));
        tokio::fs::create_dir_all(&dir)
            .await
            .expect("create fake borg dir");

        let binary = dir.join("borg");
        let script = r#"#!/usr/bin/env bash
set -eu

for arg in "$@"; do
  case "$arg" in
    --json-lines)
      cat <<'JSON'
{
  "type": "file",
  "path": "dir/file.txt",
  "size": 123,
  "mtime": "2026-06-05T12:00:00",
  "mode": "100644"
}
JSON
      exit 0
      ;;
    --glob-archives)
      cat <<'JSON'
{
  "archives": [
    {
      "name": "archive-1",
      "hostname": "agent-1",
      "start": "2026-06-05T12:00:00+00:00",
      "end": "2026-06-05T12:05:00+00:00",
      "duration": 5.0,
      "stats": {
        "original_size": 1000,
        "compressed_size": 500,
        "deduplicated_size": 250,
        "nfiles": 3
      }
    }
  ]
}
JSON
      exit 0
      ;;
  esac
done

if [ "${1:-}" = "info" ]; then
  cat <<'JSON'
{
  "cache": {
    "stats": {
      "total_size": 1000,
      "total_csize": 500,
      "unique_csize": 250,
      "total_chunks": 10,
      "total_unique_chunks": 5
    }
  },
  "encryption": {
    "mode": "repokey"
  }
}
JSON
  exit 0
fi

if [ "${1:-}" = "list" ]; then
  cat <<'JSON'
{
  "archives": [
    {
      "name": "archive-1",
      "hostname": "agent-1",
      "start": "2026-06-05T12:00:00+00:00",
      "end": "2026-06-05T12:05:00+00:00"
    }
  ]
}
JSON
  exit 0
fi

exit 0
"#;

        tokio::fs::write(&binary, script)
            .await
            .expect("write fake borg binary");
        let mut permissions = tokio::fs::metadata(&binary)
            .await
            .expect("read fake borg metadata")
            .permissions();
        permissions.set_mode(0o755);
        tokio::fs::set_permissions(&binary, permissions)
            .await
            .expect("mark fake borg executable");
        binary
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn backup_completed_queues_archive_indexing(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "agent-1", None, "token-hash", None, None)
            .await
            .expect("insert agent");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        let repo = crate::db::insert_repo(
            &pool,
            &crate::db::InsertRepoParams {
                name: "handler-repo",
                repo_path: "/backups/handler",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");

        // Link agent to repo via a schedule so the validate_agent_repo guard passes
        let schedule = crate::db::insert_schedule(
            &pool,
            repo.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "test-schedule",
                schedule_type: shared::types::ScheduleType::Backup,
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                database_backup_enabled: false,
                fs_snapshot: shared::snapshot::FsSnapshotMode::Off,
                lvm_snapshot_percent: 10,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: shared::types::OnFailure::Stop,
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(&pool, schedule.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule target");

        let _gate = crate::borg::acquire_test_binary_gate().await;
        let borg_binary = write_fake_borg_binary().await;
        let _borg_guard = crate::borg::override_binary_for_tests(borg_binary);
        let state = build_test_state(pool.clone());

        let started_at = Utc
            .with_ymd_and_hms(2026, 6, 5, 12, 0, 0)
            .single()
            .expect("valid timestamp");
        let report = BackupReport {
            id: ReportId(1),
            agent_id: AgentId(agent.id),
            repo_id: RepoId(repo.id),
            schedule_id: None,
            started_at,
            finished_at: started_at
                .checked_add_signed(chrono::Duration::minutes(5))
                .unwrap(),
            status: BackupStatus::Success,
            original_size: 1_000,
            compressed_size: 500,
            deduplicated_size: 250,
            repo_unique_csize: 250,
            files_processed: 3,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.0.0".to_string()),
            archive_name: Some("archive-2".to_string()),
            borg_command: Some("borg create".to_string()),
            run_id: None,
        };
        let msg = serde_json::to_string(&AgentToServer::BackupCompleted {
            report,
            warning_kind: shared::types::BackupWarningKind::General,
        })
        .expect("serialize message");

        handle_agent_message(&msg, &agent.hostname, agent.id, &state).await;

        timeout(Duration::from_secs(5), async {
            loop {
                let archive_2 = crate::archive_index::get_index_status(&pool, repo.id, "archive-2")
                    .await
                    .expect("archive-2 status query");
                if matches!(archive_2, Some(shared::types::IndexStatus::Done)) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("timed out waiting for archive indexing");

        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;
    }

    /// Create a test agent+repo+schedule triple linked via `schedule_targets`.
    /// Returns (agent, repo, schedule).
    async fn create_agent_repo_schedule(
        pool: &PgPool,
    ) -> (
        crate::db::AgentRow,
        crate::db::RepoRow,
        crate::db::ScheduleRow,
    ) {
        let agent = crate::db::insert_agent(pool, "test-handler-host", None, "hash", None, None)
            .await
            .expect("insert agent");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        let repo = crate::db::insert_repo(
            pool,
            &crate::db::InsertRepoParams {
                name: "handler-test-repo",
                repo_path: "/backups/handler-test",
                ssh_user: "user",
                ssh_host: "host.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");
        let schedule = add_schedule(
            pool,
            repo.id,
            agent.id,
            shared::types::ScheduleType::Backup,
            "test-schedule",
        )
        .await;
        (agent, repo, schedule)
    }

    /// Adds a schedule of `schedule_type` named `name` for `repo_id`, with
    /// `agent_id` as its only target.
    async fn add_schedule(
        pool: &PgPool,
        repo_id: i64,
        agent_id: i64,
        schedule_type: shared::types::ScheduleType,
        name: &str,
    ) -> crate::db::ScheduleRow {
        let schedule = crate::db::insert_schedule(
            pool,
            repo_id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name,
                schedule_type,
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                database_backup_enabled: false,
                fs_snapshot: shared::snapshot::FsSnapshotMode::Off,
                lvm_snapshot_percent: 10,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: shared::types::OnFailure::Stop,
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(pool, schedule.id, &[(agent_id, 0)])
            .await
            .expect("insert schedule target");
        schedule
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn validate_agent_repo_rejects_rogue_agent(pool: PgPool) {
        let (assigned_agent, assigned_repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue_agent =
            crate::db::insert_agent(&pool, "rogue-agent", None, "rogue-hash", None, None)
                .await
                .expect("insert rogue agent");

        // Assigned agent passes
        assert!(
            validate_agent_repo(
                &pool,
                assigned_agent.id,
                assigned_repo.id,
                &assigned_agent.hostname,
                "Test",
            )
            .await
        );

        // Rogue agent is rejected
        assert!(
            !validate_agent_repo(
                &pool,
                rogue_agent.id,
                assigned_repo.id,
                &rogue_agent.hostname,
                "BackupStarted",
            )
            .await
        );

        // A security_violation event was logged
        let events = crate::db::get_system_events(&pool, 10, AcknowledgedFilter::All)
            .await
            .expect("get system events");
        let security_events: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == SystemEventType::SecurityViolation)
            .collect();
        assert_eq!(security_events.len(), 1);
        assert!(
            security_events
                .first()
                .unwrap()
                .message
                .contains("rogue-agent")
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_backup_started_rejects_rogue_agent(pool: PgPool) {
        let (_assigned_agent, assigned_repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue_agent =
            crate::db::insert_agent(&pool, "rogue-backup-agent", None, "hash", None, None)
                .await
                .expect("insert rogue agent");

        let state = build_test_state(pool.clone());

        let msg = serde_json::to_string(&AgentToServer::BackupStarted {
            repo_id: RepoId(assigned_repo.id),
            schedule_id: None,
            started_at: chrono::Utc::now(),
            borg_command: Some("borg create --compression lz4 ::archive-name".into()),
            run_id: None,
        })
        .expect("serialize");

        handle_agent_message(&msg, &rogue_agent.hostname, rogue_agent.id, &state).await;

        // Verify no backup_report was created for the rogue agent
        let reports = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM backup_reports WHERE agent_id = $1",
            rogue_agent.id,
        )
        .fetch_one(&pool)
        .await
        .expect("query reports");
        assert_eq!(reports.unwrap_or(0), 0);
    }

    /// A manual snapshot's result is recorded into `agent_vms` and handed to
    /// whoever is waiting on the request, the same way a scheduled backup's
    /// `VmSnapshotReport` is recorded - but resolving the one-shot besides,
    /// since a manual snapshot has a caller actually waiting on the answer.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_vm_stage_result_records_and_resolves(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "vm-stage-test-host", None, "hash", None, None)
            .await
            .expect("insert agent");

        let state = build_test_state(pool.clone());
        let (tx, rx) = tokio::sync::oneshot::channel();
        state
            .pending_vm_stages
            .insert("req-stage-test".to_owned(), agent.id, tx)
            .await;

        let msg = serde_json::to_string(&AgentToServer::VmStageResult {
            request_id: Some("req-stage-test".into()),
            outcome: shared::vm::VmSnapshotOutcome {
                name: "web01".into(),
                action: shared::vm::VmRunAction::Increment,
                mode: shared::vm::VmSnapshotMode::Incremental,
                staged_bytes: 4096,
                chain_length: 2,
                error: None,
            },
        })
        .expect("serialize");

        handle_agent_message(&msg, &agent.hostname, agent.id, &state).await;

        let outcome = rx.await.expect("the waiter is resolved");
        assert_eq!(outcome.name, "web01");
        assert_eq!(outcome.staged_bytes, 4096);

        let row = sqlx::query!(
            "SELECT staged_bytes, chain_length FROM agent_vms WHERE agent_id = $1 AND name = $2",
            agent.id,
            "web01",
        )
        .fetch_one(&pool)
        .await
        .expect("outcome recorded");
        assert_eq!(row.staged_bytes, 4096);
        assert_eq!(row.chain_length, 2);
    }

    /// An answer carrying another agent's request id - a result or an
    /// `OperationFailed` - must not resolve that agent's pending request,
    /// and must leave it pending for the agent it was sent to.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_ignores_answers_from_another_agent(pool: PgPool) {
        let target = crate::db::insert_agent(&pool, "answer-target-host", None, "hash", None, None)
            .await
            .expect("insert target agent");
        let other = crate::db::insert_agent(&pool, "answer-other-host", None, "hash", None, None)
            .await
            .expect("insert other agent");

        let state = build_test_state(pool);
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        state
            .pending_dryruns
            .insert("req-dry-run".to_owned(), target.id, tx)
            .await;

        let forged_result = serde_json::to_string(&AgentToServer::DryRunResult {
            request_id: "req-dry-run".into(),
            files: Vec::new(),
            total_size: 1,
            error_message: None,
        })
        .expect("serialize");
        handle_agent_message(&forged_result, &other.hostname, other.id, &state).await;
        let forged_failure = serde_json::to_string(&AgentToServer::OperationFailed {
            request_id: "req-dry-run".into(),
            error: "forged".into(),
        })
        .expect("serialize");
        handle_agent_message(&forged_failure, &other.hostname, other.id, &state).await;
        assert!(
            rx.try_recv().is_err(),
            "another agent's answers must not resolve the request"
        );

        let genuine = serde_json::to_string(&AgentToServer::DryRunResult {
            request_id: "req-dry-run".into(),
            files: Vec::new(),
            total_size: 42,
            error_message: None,
        })
        .expect("serialize");
        handle_agent_message(&genuine, &target.hostname, target.id, &state).await;
        let (_, total_size, error) = rx.await.expect("the target agent resolves the request");
        assert_eq!(total_size, 42);
        assert_eq!(error, None);
    }

    /// A database scan result is recorded and handed to whoever asked; an
    /// agent too old to scan fails the request at once instead of letting it
    /// time out; and a failed scan records nothing.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_answers_database_scans(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "db-scan-host", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool.clone());
        let source = shared::database::DiscoveredDatabaseSource {
            id: shared::database::DatabaseSourceId {
                runtime: shared::database::DatabaseRuntime::Kubernetes,
                namespace: "apps".to_owned(),
                name: "pg".to_owned(),
            },
            kind: shared::database::DatabaseSourceKind::CnpgCluster,
            engine: shared::database::DatabaseEngine::Postgresql,
            pod: "pg-2".to_owned(),
            container: "postgres".to_owned(),
            databases: vec!["app".to_owned()],
            error: None,
        };

        let (tx, rx) = tokio::sync::oneshot::channel();
        state
            .pending_database_scans
            .insert("req-db-scan".to_owned(), agent.id, tx)
            .await;
        let result = serde_json::to_string(&AgentToServer::DatabaseScanResult {
            request_id: "req-db-scan".into(),
            sources: vec![source.clone()],
            error: None,
            failed_runtimes: Vec::new(),
        })
        .expect("serialize");
        handle_agent_message(&result, &agent.hostname, agent.id, &state).await;
        let (sources, error, _) = rx.await.expect("the scan is answered");
        assert_eq!(sources, vec![source.clone()]);
        assert_eq!(error, None);
        let rows = crate::db::databases::list_agent_database_sources(&pool, agent.id)
            .await
            .expect("list sources");
        assert_eq!(rows.len(), 1);

        let (tx, rx) = tokio::sync::oneshot::channel();
        state
            .pending_database_scans
            .insert("req-db-failed".to_owned(), agent.id, tx)
            .await;
        let failed = serde_json::to_string(&AgentToServer::DatabaseScanResult {
            request_id: "req-db-failed".into(),
            sources: Vec::new(),
            error: Some("forbidden".into()),
            failed_runtimes: Vec::new(),
        })
        .expect("serialize");
        handle_agent_message(&failed, &agent.hostname, agent.id, &state).await;
        let (_, error, _) = rx.await.expect("the failed scan is answered");
        assert_eq!(error.as_deref(), Some("forbidden"));
        let rows = crate::db::databases::list_agent_database_sources(&pool, agent.id)
            .await
            .expect("list sources");
        assert_eq!(rows.len(), 1, "a failed scan must not wipe the sources");

        // One runtime failed while another answered: what the healthy runtime
        // found is recorded, and the failed runtime's source is kept.
        let docker_source = shared::database::DiscoveredDatabaseSource {
            id: shared::database::DatabaseSourceId {
                runtime: shared::database::DatabaseRuntime::Docker,
                namespace: String::new(),
                name: "shop-db".to_owned(),
            },
            kind: shared::database::DatabaseSourceKind::Container,
            pod: "shop-db-1".to_owned(),
            ..source.clone()
        };
        let (tx, rx) = tokio::sync::oneshot::channel();
        state
            .pending_database_scans
            .insert("req-db-partial".to_owned(), agent.id, tx)
            .await;
        let partial = serde_json::to_string(&AgentToServer::DatabaseScanResult {
            request_id: "req-db-partial".into(),
            sources: vec![docker_source.clone()],
            error: Some("the cluster is unreachable".into()),
            failed_runtimes: vec![shared::database::DatabaseRuntime::Kubernetes],
        })
        .expect("serialize");
        handle_agent_message(&partial, &agent.hostname, agent.id, &state).await;
        let (sources, error, failed) = rx.await.expect("the partial scan is answered");
        assert_eq!(failed, [shared::database::DatabaseRuntime::Kubernetes]);
        assert_eq!(sources, vec![docker_source]);
        assert!(error.is_some());
        let rows = crate::db::databases::list_agent_database_sources(&pool, agent.id)
            .await
            .expect("list sources");
        let mut runtimes: Vec<String> = rows.iter().map(|row| row.runtime.to_string()).collect();
        runtimes.sort();
        assert_eq!(
            runtimes,
            ["docker", "kubernetes"],
            "a partial scan records the healthy runtime and keeps the failed one's sources"
        );

        let (tx, rx) = tokio::sync::oneshot::channel();
        state
            .pending_database_scans
            .insert("req-db-old".to_owned(), agent.id, tx)
            .await;
        let unsupported = serde_json::to_string(&AgentToServer::UnsupportedMessage {
            request_id: "req-db-old".into(),
            message_type: "ScanDatabases".into(),
        })
        .expect("serialize");
        handle_agent_message(&unsupported, &agent.hostname, agent.id, &state).await;
        let (_, error, _) = rx.await.expect("the scan is failed at once");
        assert!(error.is_some_and(|e| e.contains("ScanDatabases")));
    }

    /// Both ways a scan result can go wrong on the server are logged: an
    /// agent that could not scan, and a result the database will not take.
    /// The second is still answered, so the operator sees the sources even
    /// though they could not be kept.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_logs_database_scans_it_cannot_use(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "db-log-host", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool.clone());
        let result = |request_id: &str, error: Option<&str>| {
            serde_json::to_string(&AgentToServer::DatabaseScanResult {
                request_id: request_id.into(),
                sources: Vec::new(),
                error: error.map(str::to_owned),
                failed_runtimes: Vec::new(),
            })
            .expect("serialize")
        };

        let failed = result("req-failed", Some("pods is forbidden"));
        let logs = crate::test_support::warnings_while(Box::pin(handle_agent_message(
            &failed,
            &agent.hostname,
            agent.id,
            &state,
        )))
        .await;
        assert!(crate::test_support::logged(
            &logs,
            "database scan failed on the agent"
        ));

        let (tx, rx) = tokio::sync::oneshot::channel();
        state
            .pending_database_scans
            .insert("req-unrecorded".to_owned(), agent.id, tx)
            .await;
        pool.close().await;
        let unrecorded = result("req-unrecorded", None);
        let logs = crate::test_support::warnings_while(Box::pin(handle_agent_message(
            &unrecorded,
            &agent.hostname,
            agent.id,
            &state,
        )))
        .await;
        assert!(crate::test_support::logged(
            &logs,
            "failed to record the database sources an agent reported"
        ));
        let (_, error, _) = rx.await.expect("the scan is still answered");
        assert_eq!(error, None);
    }

    /// An agent that cannot handle a request answers `UnsupportedMessage`;
    /// that must fail the pending request at once with an error naming the
    /// message type, and only for the agent the request was sent to.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_unsupported_message_fails_the_pending_request(pool: PgPool) {
        let target =
            crate::db::insert_agent(&pool, "unsupported-target-host", None, "hash", None, None)
                .await
                .expect("insert target agent");
        let other =
            crate::db::insert_agent(&pool, "unsupported-other-host", None, "hash", None, None)
                .await
                .expect("insert other agent");

        let state = build_test_state(pool);
        let (dry_run_tx, mut dry_run_rx) = tokio::sync::oneshot::channel();
        state
            .pending_dryruns
            .insert("req-dry-run".to_owned(), target.id, dry_run_tx)
            .await;
        let (scan_tx, scan_rx) = tokio::sync::oneshot::channel();
        state
            .pending_vm_scans
            .insert("req-scan".to_owned(), target.id, scan_tx)
            .await;

        let unsupported = |request_id: &str| {
            serde_json::to_string(&AgentToServer::UnsupportedMessage {
                request_id: request_id.into(),
                message_type: "SomeFutureRequest".into(),
            })
            .expect("serialize")
        };

        handle_agent_message(
            &unsupported("req-dry-run"),
            &other.hostname,
            other.id,
            &state,
        )
        .await;
        assert!(
            dry_run_rx.try_recv().is_err(),
            "another agent must not fail the request"
        );

        handle_agent_message(
            &unsupported("req-dry-run"),
            &target.hostname,
            target.id,
            &state,
        )
        .await;
        let (files, total_size, error) = dry_run_rx.await.expect("the dry run is failed");
        assert_eq!(files.len(), 0);
        assert_eq!(total_size, 0);
        assert!(
            error.is_some_and(|e| e.contains("SomeFutureRequest")),
            "the error names the unsupported message type"
        );

        handle_agent_message(
            &unsupported("req-scan"),
            &target.hostname,
            target.id,
            &state,
        )
        .await;
        let (vms, error) = scan_rx.await.expect("the scan is failed");
        assert_eq!(vms.len(), 0);
        assert!(error.is_some_and(|e| e.contains("SomeFutureRequest")));

        let (build_tx, build_rx) = tokio::sync::oneshot::channel();
        state
            .pending_vm_builds
            .insert("req-build".to_owned(), target.id, build_tx)
            .await;
        handle_agent_message(
            &unsupported("req-build"),
            &target.hostname,
            target.id,
            &state,
        )
        .await;
        let (outcome, error) = build_rx.await.expect("the build is failed");
        assert_eq!(outcome, None);
        assert!(error.is_some_and(|e| e.contains("SomeFutureRequest")));

        // Nothing waits on this id, so the answer is dropped without effect.
        handle_agent_message(
            &unsupported("req-gone"),
            &target.hostname,
            target.id,
            &state,
        )
        .await;
    }

    /// `OperationFailed` fails any pending request whose answer can carry an
    /// error, not only dry runs, restores and deletes.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_operation_failed_fails_a_pending_migration(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "op-failed-host", None, "hash", None, None)
            .await
            .expect("insert agent");

        let state = build_test_state(pool);
        let (tx, rx) = tokio::sync::oneshot::channel();
        state
            .pending_migrations
            .insert("req-migrate".to_owned(), agent.id, tx)
            .await;

        let failure = serde_json::to_string(&AgentToServer::OperationFailed {
            request_id: "req-migrate".into(),
            error: "repository locked".into(),
        })
        .expect("serialize");
        handle_agent_message(&failure, &agent.hostname, agent.id, &state).await;

        let (success, error) = rx.await.expect("the migration is failed");
        assert!(!success);
        assert_eq!(error.as_deref(), Some("repository locked"));

        // Nothing waits on this id any more, so a repeat is dropped without effect.
        handle_agent_message(&failure, &agent.hostname, agent.id, &state).await;
    }

    async fn insert_restore_repo(pool: &PgPool) -> i64 {
        crate::db::insert_repo(
            pool,
            &crate::db::InsertRepoParams {
                name: "restore-repo",
                repo_path: "/backups/restore",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: b"encrypted",
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo")
        .id
    }

    /// Records a restore onto `agent_id` and hands it to that agent, as the
    /// restore API does for a connected agent.
    async fn running_restore(state: &AppState, agent_id: i64) -> uuid::Uuid {
        let repo_id = insert_restore_repo(&state.pool).await;
        let id = crate::db::restore_runs::insert_restore_run(
            &state.pool,
            &crate::db::restore_runs::NewRestoreRun {
                agent_id,
                repo_id,
                archive_name: "nightly",
                paths: &[],
                target_path: "/restore",
                requested_by: "admin",
            },
        )
        .await
        .expect("insert restore");
        crate::db::restore_runs::claim_restore_run(&state.pool, id, None)
            .await
            .expect("claim restore");
        id
    }

    async fn restore_run(state: &AppState, id: uuid::Uuid) -> shared::types::RestoreRun {
        crate::db::restore_runs::get_restore_run(&state.pool, id)
            .await
            .expect("get restore")
            .expect("the restore exists")
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn restore_completed_settles_the_restore(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "restore-done-host", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool);
        let id = running_restore(&state, agent.id).await;

        let completed = serde_json::to_string(&AgentToServer::RestoreCompleted {
            request_id: id.to_string(),
            success: true,
            files_restored: 3,
            error_message: None,
        })
        .expect("serialize");
        handle_agent_message(&completed, &agent.hostname, agent.id, &state).await;

        let run = restore_run(&state, id).await;
        assert_eq!(run.status, shared::types::RestoreRunStatus::Success);
        assert_eq!(run.files_restored, Some(3));

        // The restore is over, so a repeat changes nothing.
        let repeat = serde_json::to_string(&AgentToServer::RestoreCompleted {
            request_id: id.to_string(),
            success: false,
            files_restored: 0,
            error_message: Some("late".into()),
        })
        .expect("serialize");
        handle_agent_message(&repeat, &agent.hostname, agent.id, &state).await;
        assert_eq!(
            restore_run(&state, id).await.status,
            shared::types::RestoreRunStatus::Success
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn operation_failed_fails_a_running_restore(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "restore-failed-host", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool);
        let id = running_restore(&state, agent.id).await;

        let failure = serde_json::to_string(&AgentToServer::OperationFailed {
            request_id: id.to_string(),
            error: "borg extract timed out".into(),
        })
        .expect("serialize");
        handle_agent_message(&failure, &agent.hostname, agent.id, &state).await;

        let run = restore_run(&state, id).await;
        assert_eq!(run.status, shared::types::RestoreRunStatus::Failed);
        assert_eq!(run.error_message.as_deref(), Some("borg extract timed out"));
    }

    /// A restore whose outcome cannot be recorded is logged, not panicked
    /// over, and an `OperationFailed` for it is told apart from one for no
    /// request: it is still being recorded, in the background, until the
    /// server shuts down.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restore_outcome_the_database_rejects_is_logged(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "restore-db-down", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool.clone());
        let id = running_restore(&state, agent.id).await;
        pool.close().await;

        handle_restore_completed(
            &agent.hostname,
            agent.id,
            &state,
            id.to_string(),
            true,
            1,
            None,
        )
        .await;
        let failed = fail_restore(agent.id, &state, &id.to_string(), "failed".to_owned()).await;
        let unknown = fail_restore(agent.id, &state, "dry-run-17", "failed".to_owned()).await;

        assert_eq!(failed, FailedRequest::Unrecorded);
        assert_eq!(unknown, FailedRequest::Unknown);
        assert!(state.background_task_tracker.any_active());
        state.shutdown_token.cancel();
        assert!(
            state
                .background_task_tracker
                .wait_until_idle(Duration::from_secs(10))
                .await
        );
    }

    /// An `OperationFailed` the database rejected at first fails the restore
    /// once the database takes it, instead of leaving it running.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_rejected_restore_failure_is_recorded_once_the_database_takes_it(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "restore-db-flaky", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool.clone());
        let id = running_restore(&state, agent.id).await;
        crate::restore_runs::test_support::reject_restore_outcomes(&pool).await;

        let failure = serde_json::to_string(&AgentToServer::OperationFailed {
            request_id: id.to_string(),
            error: "borg extract timed out".into(),
        })
        .expect("serialize");
        handle_agent_message(&failure, &agent.hostname, agent.id, &state).await;
        assert_eq!(
            restore_run(&state, id).await.status,
            shared::types::RestoreRunStatus::Running
        );
        crate::restore_runs::test_support::accept_restore_outcomes(&pool).await;

        assert!(
            state
                .background_task_tracker
                .wait_until_idle(Duration::from_secs(10))
                .await
        );
        let run = restore_run(&state, id).await;
        assert_eq!(run.status, shared::types::RestoreRunStatus::Failed);
        assert_eq!(run.error_message.as_deref(), Some("borg extract timed out"));
    }

    /// A failure catching an agent up on its restores is logged, and the
    /// background task still ends.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_restore_catchup_the_database_rejects_ends(pool: PgPool) {
        let state = build_test_state(pool.clone());
        pool.close().await;

        spawn_restore_catchup(&state, 1, "catchup-host", None);

        assert!(
            state
                .background_task_tracker
                .wait_until_idle(Duration::from_secs(10))
                .await
        );
    }

    /// Recording a connection the database rejects is logged; the agent
    /// stays connected all the same.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_connection_the_database_cannot_record_is_logged(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "seen-db-down", None, "hash", None, None)
            .await
            .expect("insert agent");
        pool.close().await;
        let hello = HelloFields {
            hostname: agent.hostname,
            token: String::new(),
            agent_version: "1.0.0".into(),
            agent_git_sha: None,
            agent_build_time: None,
            agent_commit_count: Some(u32::MAX),
            supports_restart: false,
            restart_unavailable_reason: None,
            instance_id: None,
            snapshot_tools: Vec::new(),
        };

        record_agent_seen(&pool, agent.id, &hello).await;
    }

    /// An agent from before the encrypted channel reaches the real route,
    /// sends its Hello in plain text, and is told to update.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_plain_text_agent_on_the_real_route_is_refused(pool: PgPool) {
        use futures_util::{SinkExt as _, StreamExt as _};
        use tokio_tungstenite::tungstenite::Message as WsMessage;

        let app = axum::Router::new()
            .route("/ws/agent", axum::routing::get(ws_handler))
            .with_state(build_test_state(pool));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("address");
        tokio::spawn(async move { axum::serve(listener, app).await });

        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/agent"))
            .await
            .expect("connect");
        ws.send(WsMessage::Text(r#"{"type":"Hello"}"#.into()))
            .await
            .expect("send Hello");
        let frames: Vec<WsMessage> = timeout(
            Duration::from_secs(10),
            ws.filter_map(|m| async { m.ok() }).collect(),
        )
        .await
        .expect("the server ends the connection");

        assert!(
            frames.iter().any(|frame| matches!(
                frame,
                WsMessage::Close(Some(close))
                    if close.reason.as_str() == crate::ws::channel::UNENCRYPTED_REASON
            )),
            "{frames:?}"
        );
    }

    /// End to end over a real agent WebSocket: the Hello's instance id is
    /// recorded with the connection, a restore that waited for the agent is
    /// handed over once it has connected, and its answer settles it.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_connecting_agent_runs_the_restore_that_waited_for_it(pool: PgPool) {
        use futures_util::{SinkExt as _, StreamExt as _};
        use tokio_tungstenite::tungstenite::Message as WsMessage;

        let token = "restore-agent-token";
        let hash = bcrypt::hash(token, 4).expect("hash token");
        let agent = crate::db::insert_agent(&pool, "restore-ws-host", None, &hash, None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool);
        let repo_id = insert_restore_repo(&state.pool).await;
        let id = crate::db::restore_runs::insert_restore_run(
            &state.pool,
            &crate::db::restore_runs::NewRestoreRun {
                agent_id: agent.id,
                repo_id,
                archive_name: "nightly",
                paths: &["etc/hosts".to_owned()],
                target_path: "/restore",
                requested_by: "admin",
            },
        )
        .await
        .expect("insert restore");

        let app = axum::Router::new()
            .route("/ws/agent", axum::routing::get(ws_handler))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("address");
        tokio::spawn(async move { axum::serve(listener, app).await });

        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/agent"))
            .await
            .expect("connect");
        let (mut sink, mut stream) = ws.split();
        let key = crate::test_support::test_channel_key()
            .public_key()
            .expect("public key");
        let (sealer, opener) = shared::agent_channel::ws::initiate::<
            shared::agent_channel::ws::TungsteniteCodec,
            _,
            _,
            _,
        >(&mut sink, &mut stream, &key)
        .await
        .expect("handshake");
        let mut sink = shared::agent_channel::ws::EncryptedSink::<
            shared::agent_channel::ws::TungsteniteCodec,
            _,
        >::new(sink, sealer);
        let mut stream = shared::agent_channel::ws::DecryptedStream::<
            shared::agent_channel::ws::TungsteniteCodec,
            _,
        >::new(stream, opener);
        let hello = AgentToServer::Hello {
            hostname: "restore-ws-host".into(),
            token: token.into(),
            agent_version: "1.0.0".into(),
            agent_git_sha: None,
            agent_build_time: None,
            agent_commit_count: None,
            supports_restart: false,
            restart_unavailable_reason: None,
            instance_id: Some("instance-a".into()),
            snapshot_tools: Vec::new(),
        };
        sink.send(WsMessage::Text(
            serde_json::to_string(&hello).expect("serialize").into(),
        ))
        .await
        .expect("send Hello");

        let request_id = timeout(Duration::from_secs(10), async {
            loop {
                let message = stream
                    .next()
                    .await
                    .expect("the connection is open")
                    .expect("a frame");
                let text = message.into_text().expect("a text frame");
                if let Ok(ServerToAgent::RestoreFiles { request_id, .. }) =
                    serde_json::from_str(&text)
                {
                    return request_id;
                }
            }
        })
        .await
        .expect("the restore is handed over");
        assert_eq!(request_id, id.to_string());
        assert_eq!(
            state.registry.instance_id(agent.id).await,
            Some(Some("instance-a".to_owned()))
        );

        let completed = AgentToServer::RestoreCompleted {
            request_id,
            success: true,
            files_restored: 1,
            error_message: None,
        };
        let mut ui = state.ui_broadcast.subscribe();
        sink.send(WsMessage::Text(
            serde_json::to_string(&completed).expect("serialize").into(),
        ))
        .await
        .expect("send RestoreCompleted");

        // Handing the restore over and settling it each push a change. The
        // hand-over's can arrive either side of the answer, so the pushes
        // are read until the settled one.
        let success = serde_json::json!("success");
        let mut pushed = Vec::new();
        while !pushed.contains(&success) {
            let change = timeout(Duration::from_secs(10), ui.recv())
                .await
                .expect("the restore settles")
                .expect("the UI channel is open");
            let change = serde_json::to_value(change).expect("serialize");
            pushed.push(
                change
                    .pointer("/payload/status")
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        assert_eq!(
            restore_run(&state, id).await.status,
            shared::types::RestoreRunStatus::Success
        );
    }

    /// `post_backup_sync::spawn` must mark the task in flight before it returns.
    /// Claiming the guard as the first statement of the sync task's own
    /// body looked equivalent but wasn't: calling an async fn runs none of it, so
    /// `any_active()` only turned true once the runtime first polled the spawned
    /// task - and whether that happens before a caller (or a test's runtime
    /// teardown) looks is a scheduling race, not a guarantee.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn spawn_post_backup_sync_is_tracked_before_the_task_is_first_polled(pool: PgPool) {
        let state = build_test_state(pool.clone());

        // No repo with this id exists, so the task fails out immediately without
        // running borg - what it does is irrelevant here, only when it starts
        // counting is.
        post_backup_sync::spawn(
            &state,
            post_backup_sync::FinishedRun {
                repo_id: 987_654,
                agent_id: 987_654,
                hostname: "no-such-host".to_owned(),
                archive_name: None,
                host_hold: None,
            },
        );

        // Deliberately no await between the spawn and this assertion.
        assert!(
            state.background_task_tracker.any_active(),
            "post-backup sync must be tracked the moment it is spawned"
        );

        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;
    }

    /// Records an archive the fake borg's listing does not contain, as one
    /// the run's own `borg prune` has just removed would be.
    #[cfg(test)]
    async fn record_pruned_archive(pool: &PgPool, agent_id: i64, repo_id: i64) {
        let at = Utc
            .with_ymd_and_hms(2026, 6, 1, 12, 0, 0)
            .single()
            .expect("valid timestamp");
        crate::db::insert_backup_report(
            pool,
            &crate::db::InsertReportParams {
                agent_id,
                repo_id,
                schedule_id: None,
                started_at: at,
                finished_at: at,
                status: BackupStatus::Success,
                original_size: 0,
                compressed_size: 0,
                deduplicated_size: 0,
                repo_unique_csize: 0,
                files_processed: 0,
                duration_secs: 0,
                error_message: None,
                warnings: Vec::new(),
                borg_version: None,
                matched: true,
                archive_name: Some("pruned-archive".to_owned()),
                borg_command: None,
                run_id: None,
            },
        )
        .await
        .expect("insert pruned archive's report");
    }

    #[cfg(test)]
    async fn known_archives(pool: &PgPool, repo_id: i64) -> Vec<String> {
        let mut names: Vec<String> = crate::db::list_archive_names_for_repo(pool, repo_id)
            .await
            .expect("list archive names")
            .into_iter()
            .collect();
        names.sort();
        names
    }

    /// A failed run still changed the repository (its create may have landed
    /// before prune, compact or a hook failed it, and prune itself removes
    /// archives), so the sync that follows it runs anyway, and reconciles
    /// both ways: the new archive is imported, the pruned one dropped.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_backup_still_syncs_the_repository_both_ways(pool: PgPool) {
        let (agent, repo, schedule) = create_agent_repo_schedule(&pool).await;
        record_pruned_archive(&pool, agent.id, repo.id).await;

        let _gate = crate::borg::acquire_test_binary_gate().await;
        let _borg_guard = crate::borg::override_binary_for_tests(write_fake_borg_binary().await);
        let state = build_test_state(pool.clone());

        let msg = backup_failed_message(agent.id, repo.id, Some(schedule.id), None);
        handle_agent_message(&msg, &agent.hostname, agent.id, &state).await;
        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(30))
            .await;

        assert_eq!(
            known_archives(&pool, repo.id).await,
            vec!["archive-1".to_owned()],
            "the sync must import the archive borg lists and drop the one it no longer does"
        );
        let repo_after = crate::db::get_repo_with_stats(&pool, repo.id)
            .await
            .expect("load repo");
        assert!(
            repo_after.last_synced_at.is_some(),
            "the sync must be recorded"
        );
        assert!(
            !repo_after.importing,
            "importing must be cleared afterwards"
        );
        assert_eq!(repo_after.import_error, None);
    }

    /// The archive the run reported writing is never treated as stale by the
    /// sync that follows it, even when the listing does not show it.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn the_post_backup_sync_keeps_the_archive_the_run_wrote(pool: PgPool) {
        let (agent, repo, schedule) = create_agent_repo_schedule(&pool).await;

        let _gate = crate::borg::acquire_test_binary_gate().await;
        let _borg_guard = crate::borg::override_binary_for_tests(write_fake_borg_binary().await);
        let state = build_test_state(pool.clone());

        let mut msg: serde_json::Value = serde_json::from_str(&backup_report_message(
            agent.id,
            repo.id,
            Some(schedule.id),
            None,
            BackupStatus::Success,
        ))
        .unwrap();
        *msg.pointer_mut("/payload/report/archive_name").unwrap() =
            serde_json::json!("just-written");
        handle_agent_message(&msg.to_string(), &agent.hostname, agent.id, &state).await;
        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(30))
            .await;

        assert_eq!(
            known_archives(&pool, repo.id).await,
            vec!["archive-1".to_owned(), "just-written".to_owned()],
        );
    }

    /// The sync queues behind whatever holds the repository lock (the run
    /// itself, until its completion is handled) rather than racing it for
    /// borg's own lock, and the completion is never held up waiting for it.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn the_post_backup_sync_waits_for_the_repository_lock(pool: PgPool) {
        let (agent, repo, schedule) = create_agent_repo_schedule(&pool).await;

        let _gate = crate::borg::acquire_test_binary_gate().await;
        let _borg_guard = crate::borg::override_binary_for_tests(write_fake_borg_binary().await);
        let state = build_test_state(pool.clone());
        let mut completions = state.completion_bus.subscribe();

        let held = state.repo_lock.acquire(repo.id).await;
        let msg = backup_report_message(
            agent.id,
            repo.id,
            Some(schedule.id),
            None,
            BackupStatus::Success,
        );
        timeout(
            Duration::from_secs(10),
            handle_agent_message(&msg, &agent.hostname, agent.id, &state),
        )
        .await
        .expect("handling the completion must not wait for the sync");
        let outcome = completions.try_recv().expect("the completion is published");
        assert_eq!(outcome.repo_id, repo.id);

        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            known_archives(&pool, repo.id).await.is_empty(),
            "nothing may be synced while another operation holds the repository"
        );
        assert!(state.background_task_tracker.any_active());

        drop(held);
        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(30))
            .await;
        assert_eq!(
            known_archives(&pool, repo.id).await,
            vec!["archive-1".to_owned()]
        );
    }

    /// A dispatched run's teardown, which may shut a host it woke down, runs as
    /// soon as the completion is published - before the sync, queued behind it
    /// on the repository lock, has listed the repository. The sync holds the
    /// host's power session open until it is done, and then lets it go.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn the_post_backup_sync_holds_the_repository_host_up_until_it_is_done(pool: PgPool) {
        let (agent, repo, schedule) = create_agent_repo_schedule(&pool).await;

        let _gate = crate::borg::acquire_test_binary_gate().await;
        let _borg_guard = crate::borg::override_binary_for_tests(write_fake_borg_binary().await);
        let state = build_test_state(pool.clone());
        let host = crate::power::PowerHostKey::RepoHost(repo.repo_host_id);
        // The dispatcher's own reservation for the run.
        state
            .power_sessions
            .reserve(host, crate::power::TEST_HOLD)
            .await;

        let held = state.repo_lock.acquire(repo.id).await;
        let msg = backup_report_message(
            agent.id,
            repo.id,
            Some(schedule.id),
            Some("run-holds-host"),
            BackupStatus::Success,
        );
        handle_agent_message(&msg, &agent.hostname, agent.id, &state).await;

        assert_eq!(
            state
                .power_sessions
                .end(host, crate::power::TEST_HOLD)
                .await,
            None,
            "the run's teardown must not be the last one out while the sync is pending"
        );

        drop(held);
        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(30))
            .await;
        assert_eq!(
            state
                .power_sessions
                .end(host, crate::power::TEST_HOLD)
                .await,
            None,
            "the sync must have ended the session once it was done"
        );
        assert_eq!(
            known_archives(&pool, repo.id).await,
            vec!["archive-1".to_owned()]
        );
    }

    /// A run the agent aborted may still have written an archive before the
    /// abort, and its prune may have removed others, so the cancellation is
    /// followed by the same sync - queued on the repository lock, and without
    /// holding the repository host, since a cancellation carries no `run_id`.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_cancelled_backup_still_syncs_the_repository(pool: PgPool) {
        let (agent, repo, _schedule) = create_agent_repo_schedule(&pool).await;
        record_pruned_archive(&pool, agent.id, repo.id).await;

        let _gate = crate::borg::acquire_test_binary_gate().await;
        let _borg_guard = crate::borg::override_binary_for_tests(write_fake_borg_binary().await);
        let state = build_test_state(pool.clone());
        let host = crate::power::PowerHostKey::RepoHost(repo.repo_host_id);
        state
            .power_sessions
            .reserve(host, crate::power::TEST_HOLD)
            .await;

        let held = state.repo_lock.acquire(repo.id).await;
        let msg = serde_json::to_string(&AgentToServer::BackupCancelled {
            repo_id: RepoId(repo.id),
        })
        .expect("serialize");
        handle_agent_message(&msg, &agent.hostname, agent.id, &state).await;

        assert!(
            state.background_task_tracker.any_active(),
            "the cancellation must spawn the sync"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            known_archives(&pool, repo.id).await,
            vec!["pruned-archive".to_owned()],
            "nothing may be synced while another operation holds the repository"
        );

        drop(held);
        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(30))
            .await;
        assert_eq!(
            known_archives(&pool, repo.id).await,
            vec!["archive-1".to_owned()],
            "the sync must import the archive borg lists and drop the one it no longer does"
        );
        let repo_after = crate::db::get_repo_with_stats(&pool, repo.id)
            .await
            .expect("load repo");
        assert!(repo_after.last_synced_at.is_some());
        assert!(!repo_after.importing);
        assert_eq!(
            state
                .power_sessions
                .end(host, crate::power::TEST_HOLD)
                .await,
            Some((false, false)),
            "a cancelled run's sync must not take a hold on the repository host"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_backup_log_rejects_rogue_agent(pool: PgPool) {
        let (_assigned_agent, assigned_repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue_agent =
            crate::db::insert_agent(&pool, "rogue-log-agent", None, "hash", None, None)
                .await
                .expect("insert rogue agent");

        let state = build_test_state(pool.clone());

        let msg = serde_json::to_string(&AgentToServer::BackupLog {
            repo_id: RepoId(assigned_repo.id),
            schedule_id: None,
            line: "some log line".into(),
        })
        .expect("serialize");

        handle_agent_message(&msg, &rogue_agent.hostname, rogue_agent.id, &state).await;

        // Verify a security_violation was logged (BackupLog is guarded)
        let events = crate::db::get_system_events(&pool, 10, AcknowledgedFilter::All)
            .await
            .expect("get system events");
        let security_events: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == SystemEventType::SecurityViolation)
            .collect();
        assert_eq!(security_events.len(), 1);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_backup_cancelled_rejects_rogue_agent(pool: PgPool) {
        let (_assigned_agent, assigned_repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue_agent =
            crate::db::insert_agent(&pool, "rogue-cancel-agent", None, "hash", None, None)
                .await
                .expect("insert rogue agent");

        let state = build_test_state(pool.clone());

        let msg = serde_json::to_string(&AgentToServer::BackupCancelled {
            repo_id: RepoId(assigned_repo.id),
        })
        .expect("serialize");

        handle_agent_message(&msg, &rogue_agent.hostname, rogue_agent.id, &state).await;

        let events = crate::db::get_system_events(&pool, 10, AcknowledgedFilter::All)
            .await
            .expect("get system events");
        let security_events: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == SystemEventType::SecurityViolation)
            .collect();
        assert_eq!(security_events.len(), 1);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_backup_rejected_rejects_rogue_agent(pool: PgPool) {
        let (_assigned_agent, assigned_repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue_agent =
            crate::db::insert_agent(&pool, "rogue-reject-agent", None, "hash", None, None)
                .await
                .expect("insert rogue agent");

        let state = build_test_state(pool.clone());

        let msg = serde_json::to_string(&AgentToServer::BackupRejected {
            repo_id: RepoId(assigned_repo.id),
            reason: "security test".into(),
        })
        .expect("serialize");

        handle_agent_message(&msg, &rogue_agent.hostname, rogue_agent.id, &state).await;

        let events = crate::db::get_system_events(&pool, 10, AcknowledgedFilter::All)
            .await
            .expect("get system events");
        let security_events: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == SystemEventType::SecurityViolation)
            .collect();
        assert_eq!(security_events.len(), 1);
    }

    /// A filesystem snapshot step lands on the run's timeline as the source
    /// host's, cut to its longest message; a power-management step an agent
    /// claims is the server's own and is ignored.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_run_event_records_only_an_agents_own_steps(pool: PgPool) {
        let (agent, repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let state = build_test_state(pool.clone());
        let event = |event_type, message: String| {
            serde_json::to_string(&AgentToServer::RunEvent {
                run_id: "run-snap".into(),
                repo_id: RepoId(repo.id),
                event_type,
                message,
            })
            .expect("serialize")
        };
        handle_agent_message(
            &event(RunEventType::SnapshotCreated, "x".repeat(1500)),
            &agent.hostname,
            agent.id,
            &state,
        )
        .await;
        handle_agent_message(
            &event(RunEventType::AgentConnected, "forged".into()),
            &agent.hostname,
            agent.id,
            &state,
        )
        .await;

        let events = crate::db::run_events::list_run_events(&pool, "run-snap", agent.id, repo.id)
            .await
            .expect("list run events");
        assert_eq!(events.len(), 1);
        let recorded = events.first().unwrap();
        assert_eq!(recorded.event_type, RunEventType::SnapshotCreated);
        assert_eq!(recorded.target, RunEventTarget::Source);
        assert_eq!(
            recorded.message.chars().count(),
            MAX_AGENT_RUN_EVENT_MESSAGE
        );
    }

    /// An agent that reports a step for a repo it is not scheduled against
    /// records nothing, the same as its other reports.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn handle_agent_message_run_event_rejects_rogue_agent(pool: PgPool) {
        let (_assigned_agent, assigned_repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue_agent =
            crate::db::insert_agent(&pool, "rogue-snapshot-agent", None, "hash", None, None)
                .await
                .expect("insert rogue agent");
        let state = build_test_state(pool.clone());

        let msg = serde_json::to_string(&AgentToServer::RunEvent {
            run_id: "run-rogue".into(),
            repo_id: RepoId(assigned_repo.id),
            event_type: RunEventType::SnapshotCreated,
            message: "forged".into(),
        })
        .expect("serialize");
        handle_agent_message(&msg, &rogue_agent.hostname, rogue_agent.id, &state).await;

        let events = crate::db::run_events::list_run_events(
            &pool,
            "run-rogue",
            rogue_agent.id,
            assigned_repo.id,
        )
        .await
        .expect("list run events");
        assert!(events.is_empty());
    }

    fn backup_completed_message(agent_id: i64, repo_id: i64, deduplicated_size: i64) -> String {
        backup_completed_message_with_repo_size(
            agent_id,
            repo_id,
            deduplicated_size,
            deduplicated_size,
        )
    }

    /// Like [`backup_completed_message`], but lets the archive-level `deduplicated_size`
    /// (this backup's own new unique data) and the repo-wide `repo_unique_csize` (the
    /// repo's total current usage) diverge, matching what a real backup with prior
    /// archives reports.
    fn backup_completed_message_with_repo_size(
        agent_id: i64,
        repo_id: i64,
        deduplicated_size: i64,
        repo_unique_csize: i64,
    ) -> String {
        let started_at = Utc
            .with_ymd_and_hms(2026, 6, 5, 12, 0, 0)
            .single()
            .expect("valid timestamp");
        let report = BackupReport {
            id: ReportId(1),
            agent_id: AgentId(agent_id),
            repo_id: RepoId(repo_id),
            schedule_id: None,
            started_at,
            finished_at: started_at
                .checked_add_signed(chrono::Duration::minutes(5))
                .unwrap(),
            status: BackupStatus::Success,
            original_size: deduplicated_size,
            compressed_size: deduplicated_size,
            deduplicated_size,
            repo_unique_csize,
            files_processed: 3,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.0.0".to_string()),
            archive_name: None,
            borg_command: None,
            run_id: None,
        };
        serde_json::to_string(&AgentToServer::BackupCompleted {
            report,
            warning_kind: shared::types::BackupWarningKind::General,
        })
        .expect("serialize message")
    }

    /// A failed backup report, the shape a run against a host that is not
    /// there actually produces.
    fn backup_failed_message(
        agent_id: i64,
        repo_id: i64,
        schedule_id: Option<i64>,
        run_id: Option<&str>,
    ) -> String {
        backup_report_message(agent_id, repo_id, schedule_id, run_id, BackupStatus::Failed)
    }

    /// A completed backup report with the given outcome; a failed one carries
    /// the connection error a run against an absent host produces.
    #[cfg(test)]
    fn backup_report_message(
        agent_id: i64,
        repo_id: i64,
        schedule_id: Option<i64>,
        run_id: Option<&str>,
        status: BackupStatus,
    ) -> String {
        let started_at = Utc
            .with_ymd_and_hms(2026, 6, 5, 12, 0, 0)
            .single()
            .expect("valid timestamp");
        let report = BackupReport {
            id: ReportId(1),
            agent_id: AgentId(agent_id),
            repo_id: RepoId(repo_id),
            schedule_id,
            started_at,
            finished_at: started_at
                .checked_add_signed(chrono::Duration::minutes(1))
                .unwrap(),
            status,
            original_size: 0,
            compressed_size: 0,
            deduplicated_size: 0,
            repo_unique_csize: 0,
            files_processed: 0,
            duration_secs: 60,
            error_message: matches!(status, BackupStatus::Failed)
                .then(|| "Connection closed by remote host".to_owned()),
            warnings: vec![],
            borg_version: Some("1.0.0".to_string()),
            archive_name: None,
            borg_command: None,
            run_id: run_id.map(str::to_owned),
        };
        serde_json::to_string(&AgentToServer::BackupCompleted {
            report,
            warning_kind: shared::types::BackupWarningKind::General,
        })
        .expect("serialize message")
    }

    /// An agent owning one repository whose host never answers, through one
    /// schedule. `intermittent` is the repository's own "host is not always
    /// online" switch, which is the only thing the tests below differ on - and
    /// far too much setup to write out for each.
    #[cfg(test)]
    async fn absent_repo_fixture(
        pool: &PgPool,
        intermittent: bool,
    ) -> (
        crate::db::AgentRow,
        crate::db::RepoRow,
        crate::db::ScheduleRow,
    ) {
        let agent = crate::db::insert_agent(pool, "agent-1", None, "token-hash", None, None)
            .await
            .expect("insert agent");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        // `storage.local` never answers from a test, which is exactly the
        // state this reports on.
        let repo = crate::db::insert_repo(
            pool,
            &crate::db::InsertRepoParams {
                name: "absent-repo",
                repo_path: "/backups/absent",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");
        if intermittent {
            sqlx::query!(
                "UPDATE machines SET intermittent = true WHERE id = (SELECT machine_id FROM \
                 repo_hosts WHERE id = $1)",
                repo.repo_host_id
            )
            .execute(pool)
            .await
            .expect("mark the repository as not always online");
        }
        // The agent has to own this repository through a schedule, or the
        // report is rejected before any of this is reached.
        let schedule = crate::db::insert_schedule(
            pool,
            repo.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "absent-repo-schedule",
                schedule_type: shared::types::ScheduleType::Backup,
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                database_backup_enabled: false,
                fs_snapshot: shared::snapshot::FsSnapshotMode::Off,
                lvm_snapshot_percent: 10,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: shared::types::OnFailure::Stop,
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(pool, schedule.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule targets");
        (agent, repo, schedule)
    }

    /// Reports a failed backup of `repo` from `agent` - through `schedule_id`, or
    /// as a manual run when it is `None` - with a webhook subscribed to both a
    /// plain failure and the repo-offline skip, and returns the channel and the
    /// event types delivered to it once every background task has finished.
    ///
    /// Subscribed to both so a double-fire shows up as two rows rather than
    /// being masked by only one of them having a rule.
    #[cfg(test)]
    async fn report_failed_backup(
        pool: &PgPool,
        agent: &crate::db::AgentRow,
        repo_id: i64,
        schedule_id: Option<i64>,
    ) -> (i64, Vec<String>) {
        report_failed_run(pool, agent, repo_id, schedule_id, None).await
    }

    /// [`report_failed_backup`] for a run that carries its own `run_id`, as a
    /// scheduled one does.
    #[cfg(test)]
    async fn report_failed_run(
        pool: &PgPool,
        agent: &crate::db::AgentRow,
        repo_id: i64,
        schedule_id: Option<i64>,
        run_id: Option<&str>,
    ) -> (i64, Vec<String>) {
        let msg = backup_failed_message(agent.id, repo_id, schedule_id, run_id);
        deliver_agent_message(
            pool,
            agent,
            &msg,
            &["backup_failed", "backup_skipped_repo_offline"],
        )
        .await
    }

    /// Handles `msg` from `agent` with a webhook subscribed to `event_types`,
    /// and returns the channel and the event types delivered to it once every
    /// background task has finished.
    #[cfg(test)]
    async fn deliver_agent_message(
        pool: &PgPool,
        agent: &crate::db::AgentRow,
        msg: &str,
        event_types: &[&str],
    ) -> (i64, Vec<String>) {
        deliver_agent_message_with_state(&build_test_state(pool.clone()), agent, msg, event_types)
            .await
    }

    /// [`deliver_agent_message`] on a caller-built `state`, so the caller can
    /// also watch that state's UI and completion channels.
    async fn deliver_agent_message_with_state(
        state: &AppState,
        agent: &crate::db::AgentRow,
        msg: &str,
        event_types: &[&str],
    ) -> (i64, Vec<String>) {
        let pool = &state.pool;
        let channel_id: i64 = sqlx::query_scalar!(
            "INSERT INTO notification_channels (name, channel_type, config, enabled) VALUES ($1, \
             'webhook', $2, true) RETURNING id",
            "test-webhook",
            serde_json::json!({ "url": "http://127.0.0.1:1/unreachable" }),
        )
        .fetch_one(pool)
        .await
        .unwrap();
        for event_type in event_types {
            sqlx::query!(
                "INSERT INTO notification_rules (channel_id, event_type, enabled) VALUES ($1, $2, \
                 true)",
                channel_id,
                event_type,
            )
            .execute(pool)
            .await
            .unwrap();
        }

        handle_agent_message(msg, &agent.hostname, agent.id, state).await;

        // The notification is spawned on the background tracker, and only the
        // delivery attempt itself lands on the task registry, so both have to
        // be drained before the rows exist.
        assert!(
            state
                .background_task_tracker
                .wait_until_idle(std::time::Duration::from_mins(1))
                .await,
            "the backup-completed background work must finish"
        );
        let outstanding = state
            .task_registry
            .shutdown(std::time::Duration::from_secs(30))
            .await;
        assert_eq!(
            outstanding, 0,
            "notification delivery must have been joined"
        );

        let deliveries: Vec<String> = sqlx::query_scalar!(
            "SELECT event_type FROM notification_deliveries WHERE channel_id = $1",
            channel_id,
        )
        .fetch_all(pool)
        .await
        .unwrap();
        (channel_id, deliveries)
    }

    /// A warning-status report whose warnings were all files changing goes out
    /// as `backup_file_changed` alone, and any other warning still goes out as
    /// `backup_warning` alone - so a channel subscribed to both hears about a
    /// run once, and one subscribed to only one of them can tell them apart.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_file_changed_warning_is_delivered_as_its_own_event(pool: PgPool) {
        let (agent, repo, schedule) = absent_repo_fixture(&pool, false).await;
        for (kind, expected) in [
            ("file_changed", "backup_file_changed"),
            ("general", "backup_warning"),
        ] {
            let mut msg: serde_json::Value = serde_json::from_str(&backup_report_message(
                agent.id,
                repo.id,
                Some(schedule.id),
                None,
                BackupStatus::Warning,
            ))
            .unwrap();
            msg.pointer_mut("/payload")
                .and_then(serde_json::Value::as_object_mut)
                .unwrap()
                .insert("warning_kind".to_owned(), serde_json::json!(kind));

            let (_, deliveries) = deliver_agent_message(
                &pool,
                &agent,
                &msg.to_string(),
                &["backup_warning", "backup_file_changed"],
            )
            .await;

            assert_eq!(deliveries, vec![expected.to_owned()], "warning_kind {kind}");
        }
    }

    #[cfg(test)]
    async fn skipped_repo_offline_events(pool: &PgPool) -> usize {
        sqlx::query_scalar!(
            "SELECT event_type FROM system_events WHERE event_type = 'backup_skipped_repo_offline'",
        )
        .fetch_all(pool)
        .await
        .unwrap()
        .len()
    }

    #[cfg(test)]
    async fn pending_repo_catch_ups(
        pool: &PgPool,
    ) -> Vec<crate::db::catch_up::RepoCatchUpCandidate> {
        crate::db::catch_up::list_repo_catch_up_candidates(
            pool,
            crate::db::catch_up::RepoCatchUpFilter::All,
        )
        .await
        .unwrap()
    }

    /// A backup that failed against a repository marked as not always online,
    /// whose host is not answering, must be reported as a repo-offline skip -
    /// and as *only* that. The failure notification has to become the skip
    /// rather than fire alongside it, or one backup leaves two alerts and two
    /// activity rows.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_backup_against_an_absent_repository_host_is_reported_as_skipped_once(
        pool: PgPool,
    ) {
        let (agent, repo, schedule) = absent_repo_fixture(&pool, true).await;
        let (channel_id, deliveries) =
            report_failed_backup(&pool, &agent, repo.id, Some(schedule.id)).await;

        assert_eq!(
            deliveries,
            vec!["backup_skipped_repo_offline".to_owned()],
            "exactly one notification may describe the run, and it must be the skip rather than a \
             bare failure"
        );

        // Asserted on the delivered payload, not on a hand-built one: the
        // point of collapsing to a single event is that the alert itself says
        // why, so it must carry the reason rather than repeat the raw borg
        // error a plain failure would have shown.
        let payload: serde_json::Value = sqlx::query_scalar!(
            r#"SELECT payload AS "payload!" FROM notification_deliveries WHERE channel_id = $1"#,
            channel_id,
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            payload
                .get("error_message")
                .and_then(serde_json::Value::as_str),
            Some("the host for repository 'absent-repo' did not answer SSH"),
            "the outbound notification must explain the absent host, not echo borg's own \
             connection error"
        );
        assert_eq!(
            skipped_repo_offline_events(&pool).await,
            1,
            "the Activity Log must carry the reason the backup had nowhere to write"
        );
    }

    /// A failure against a host the machine expects to be online, but that this
    /// schedule is set to wait for, is a skip too - reported with the event of
    /// its own that no existing rule follows, and caught up.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_backup_against_a_host_the_schedule_waits_for_is_reported_as_waiting(
        pool: PgPool,
    ) {
        let (agent, repo, schedule) = absent_repo_fixture(&pool, false).await;
        crate::db::machines::replace_schedule_machine_wake(
            &pool,
            schedule.id,
            &[crate::db::machines::ScheduleMachineWake {
                machine_id: repo.machine_id,
                mode: shared::types::MachineWakeMode::Wait,
            }],
        )
        .await
        .unwrap();
        let msg = backup_failed_message(agent.id, repo.id, Some(schedule.id), None);
        let (_, deliveries) = deliver_agent_message(
            &pool,
            &agent,
            &msg,
            &["backup_failed", "backup_waiting_for_host"],
        )
        .await;

        assert_eq!(deliveries, vec!["backup_waiting_for_host".to_owned()]);
        let events: Vec<String> = sqlx::query_scalar!(
            "SELECT event_type FROM system_events WHERE event_type LIKE 'backup_%'"
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(events, vec!["backup_waiting_for_host".to_owned()]);
        assert_eq!(
            pending_repo_catch_ups(&pool).await.len(),
            1,
            "the schedule waits for the host to come back"
        );
    }

    /// The same failure against a repository that is *not* marked as not always
    /// online: a server that should have been there, so an unreachable host is
    /// exactly the failure borg reported. No skip, no probe-driven relabelling,
    /// and nothing waiting for it to come back.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_backup_against_an_always_online_repository_stays_a_failure(pool: PgPool) {
        let (agent, repo, schedule) = absent_repo_fixture(&pool, false).await;
        let (_, deliveries) = report_failed_backup(&pool, &agent, repo.id, Some(schedule.id)).await;

        assert_eq!(
            deliveries,
            vec!["backup_failed".to_owned()],
            "an always-online repository's unreachable host is a failed backup, never a skip"
        );
        assert_eq!(skipped_repo_offline_events(&pool).await, 0);
        assert!(
            pending_repo_catch_ups(&pool).await.is_empty(),
            "nothing waits for a repository that is expected to always be there"
        );
    }

    /// The other half of the skip: the repository is also waited for, so the
    /// run happens once its host answers again instead of at the schedule's
    /// next occurrence.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_backup_against_an_absent_repository_leaves_a_catch_up_pending(pool: PgPool) {
        let (agent, repo, schedule) = absent_repo_fixture(&pool, true).await;
        // The occurrence the run stood in for, which is what the marker names
        // and what the give-up window is measured from. Truncated to what
        // Postgres itself stores, so the round-trip below is exact whichever
        // way the server version handles the sub-microsecond tail.
        let missed = chrono::SubsecRound::trunc_subsecs(chrono::Utc::now(), 6)
            .checked_sub_signed(chrono::Duration::hours(2))
            .unwrap();
        sqlx::query!(
            "UPDATE schedules SET last_run_at = $2 WHERE id = $1",
            schedule.id,
            missed,
        )
        .execute(&pool)
        .await
        .unwrap();

        report_failed_backup(&pool, &agent, repo.id, Some(schedule.id)).await;

        let pending = pending_repo_catch_ups(&pool).await;
        assert_eq!(pending.len(), 1, "the absent repository must be waited on");
        let candidate = pending.first().unwrap();
        assert_eq!(candidate.repo_id, repo.id);
        assert_eq!(candidate.schedule_id, schedule.id);
        assert_eq!(
            candidate.pending_for, missed,
            "the marker names the occurrence the run stood in for, not the moment it gave up"
        );
        assert_eq!(candidate.last_probe_at, None);
    }

    /// An ordinary run that writes the repository successfully settles its
    /// pending catch-up, so the poller does not re-run a covered occurrence the
    /// next time it finds the host answering.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_successful_backup_settles_the_repositorys_pending_catch_up(pool: PgPool) {
        let (agent, repo, schedule) = absent_repo_fixture(&pool, true).await;
        crate::db::catch_up::mark_repo_catch_up_pending(
            &pool,
            schedule.id,
            repo.id,
            chrono::Utc::now(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(pending_repo_catch_ups(&pool).await.len(), 1);

        let state = build_test_state(pool.clone());
        let msg = backup_report_message(
            agent.id,
            repo.id,
            Some(schedule.id),
            None,
            BackupStatus::Success,
        );
        handle_agent_message(&msg, &agent.hostname, agent.id, &state).await;
        assert!(
            state
                .background_task_tracker
                .wait_until_idle(std::time::Duration::from_mins(1))
                .await
        );

        assert!(
            pending_repo_catch_ups(&pool).await.is_empty(),
            "a run that reached the repository leaves nothing to catch up"
        );
    }

    /// A failure that comes back slowly, after the same schedule has already
    /// run again, must still name the occurrence it stood for - its own run's
    /// start - rather than whatever `last_run_at` says by then.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_slow_failure_names_its_own_run_not_the_schedules_latest(pool: PgPool) {
        let (agent, repo, schedule) = absent_repo_fixture(&pool, true).await;
        // The run that failed, as the scheduler recorded it when it dispatched.
        sqlx::query!(
            "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
             status, run_id) VALUES ($1, $2, $3, NOW() - interval '2 hours', NOW() - interval '2 \
             hours', 'pending', 'slow-run')",
            agent.id,
            repo.id,
            schedule.id,
        )
        .execute(&pool)
        .await
        .unwrap();
        // ...and the schedule has moved on to a later run since.
        sqlx::query!(
            "UPDATE schedules SET last_run_at = NOW() WHERE id = $1",
            schedule.id,
        )
        .execute(&pool)
        .await
        .unwrap();

        report_failed_run(&pool, &agent, repo.id, Some(schedule.id), Some("slow-run")).await;

        let run_started = crate::db::catch_up::run_started_at(&pool, "slow-run", repo.id)
            .await
            .unwrap()
            .expect("the run keeps its report");
        let pending = pending_repo_catch_ups(&pool).await;
        let candidate = pending
            .first()
            .expect("the absent repository must be waited on");
        assert_eq!(
            candidate.pending_for, run_started,
            "the marker must name the run that failed, not the schedule's latest one"
        );
        let last_run_at = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .unwrap()
            .last_run_at;
        assert_ne!(Some(candidate.pending_for), last_run_at);
    }

    /// A manual Run now has no occurrence behind it, so there is nothing to
    /// catch up to - only a schedule that was due leaves a marker.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_manual_run_against_an_absent_repository_leaves_nothing_pending(pool: PgPool) {
        let (agent, repo, _) = absent_repo_fixture(&pool, true).await;

        report_failed_backup(&pool, &agent, repo.id, None).await;

        assert!(
            pending_repo_catch_ups(&pool).await.is_empty(),
            "a run with no schedule behind it has no occurrence to catch up"
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn backup_completed_disables_schedule_on_repo_quota_breach(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "agent-1", None, "token-hash", None, None)
            .await
            .expect("insert agent");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        let repo = crate::db::insert_repo(
            &pool,
            &crate::db::InsertRepoParams {
                name: "quota-repo",
                repo_path: "/backups/quota",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");
        let schedule = crate::db::insert_schedule(
            &pool,
            repo.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "quota-schedule",
                schedule_type: shared::types::ScheduleType::Backup,
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                database_backup_enabled: false,
                fs_snapshot: shared::snapshot::FsSnapshotMode::Off,
                lvm_snapshot_percent: 10,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: shared::types::OnFailure::Stop,
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(&pool, schedule.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule targets");
        db::quota::upsert_quota(
            &pool,
            repo.id,
            Some(50),
            Some(100),
            QuotaAction::NotifyOnly,
            QuotaAction::BlockBackups,
            true,
        )
        .await
        .expect("upsert quota");

        let state = build_test_state(pool.clone());
        let msg = backup_completed_message(agent.id, repo.id, 200);
        handle_agent_message(&msg, &agent.hostname, agent.id, &state).await;

        let updated = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .expect("get schedule");
        assert!(!updated.enabled);

        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;
    }

    /// Regression test for the archive-level-vs-repo-wide size mixup: a backup that adds
    /// almost no new unique data (`deduplicated_size`) to an already-large repo must still
    /// breach the repo's quota, because enforcement is decided by `repo_unique_csize` (the
    /// repo's actual current total), not by how much this one archive contributed.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn backup_completed_uses_repo_unique_csize_not_archive_delta_for_repo_quota(
        pool: PgPool,
    ) {
        let agent = crate::db::insert_agent(&pool, "agent-1", None, "token-hash", None, None)
            .await
            .expect("insert agent");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        let repo = crate::db::insert_repo(
            &pool,
            &crate::db::InsertRepoParams {
                name: "quota-repo",
                repo_path: "/backups/quota",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");
        let schedule = crate::db::insert_schedule(
            &pool,
            repo.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "quota-schedule",
                schedule_type: shared::types::ScheduleType::Backup,
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                database_backup_enabled: false,
                fs_snapshot: shared::snapshot::FsSnapshotMode::Off,
                lvm_snapshot_percent: 10,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: shared::types::OnFailure::Stop,
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(&pool, schedule.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule targets");
        db::quota::upsert_quota(
            &pool,
            repo.id,
            Some(50),
            Some(100),
            QuotaAction::NotifyOnly,
            QuotaAction::BlockBackups,
            true,
        )
        .await
        .expect("upsert quota");

        let state = build_test_state(pool.clone());
        // This archive itself only added 5 bytes of new unique data (well under the
        // warn/critical thresholds), but the repo's actual current total is 150 bytes,
        // over the critical threshold of 100.
        let msg = backup_completed_message_with_repo_size(agent.id, repo.id, 5, 150);
        handle_agent_message(&msg, &agent.hostname, agent.id, &state).await;

        let updated = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .expect("get schedule");
        assert!(!updated.enabled);

        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn backup_completed_disables_schedule_on_server_quota_breach(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "agent-1", None, "token-hash", None, None)
            .await
            .expect("insert agent");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        let repo_a = crate::db::insert_repo(
            &pool,
            &crate::db::InsertRepoParams {
                name: "shared-repo-a",
                repo_path: "/backups/shared-a",
                ssh_user: "backup",
                ssh_host: "shared.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo a");
        let repo_b = crate::db::insert_repo(
            &pool,
            &crate::db::InsertRepoParams {
                name: "shared-repo-b",
                repo_path: "/backups/shared-b",
                ssh_user: "backup",
                ssh_host: "shared.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo b");
        let schedule_a = crate::db::insert_schedule(
            &pool,
            repo_a.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "shared-schedule-a",
                schedule_type: shared::types::ScheduleType::Backup,
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                database_backup_enabled: false,
                fs_snapshot: shared::snapshot::FsSnapshotMode::Off,
                lvm_snapshot_percent: 10,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: shared::types::OnFailure::Stop,
            },
            None,
        )
        .await
        .expect("insert schedule a");
        crate::db::insert_schedule_targets(&pool, schedule_a.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule targets");
        let schedule_b = crate::db::insert_schedule(
            &pool,
            repo_b.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "shared-schedule-b",
                schedule_type: shared::types::ScheduleType::Backup,
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                database_backup_enabled: false,
                fs_snapshot: shared::snapshot::FsSnapshotMode::Off,
                lvm_snapshot_percent: 10,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: shared::types::OnFailure::Stop,
            },
            None,
        )
        .await
        .expect("insert schedule b");
        crate::db::insert_schedule_targets(&pool, schedule_b.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule targets");
        db::server_quota::upsert_server_quota(
            &pool,
            "shared.local",
            Some(50),
            Some(100),
            QuotaAction::NotifyOnly,
            QuotaAction::BlockBackups,
            true,
        )
        .await
        .expect("upsert server quota");

        let state = build_test_state(pool.clone());
        let msg = backup_completed_message(agent.id, repo_a.id, 200);
        handle_agent_message(&msg, &agent.hostname, agent.id, &state).await;

        let updated = crate::db::get_schedule_by_id(&pool, schedule_b.id)
            .await
            .expect("get schedule");
        assert!(!updated.enabled);

        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;
    }

    /// Regression test for the archive-level-vs-repo-wide size mixup in the *server* quota
    /// path: repo A's own backup only added 5 bytes of new unique data (its archive-level
    /// `deduplicated_size`), well under what's needed to breach the shared host quota when
    /// combined with sibling repo B's 60-byte snapshot. But repo A's actual current total
    /// (`repo_unique_csize`) is 45 bytes, which combined with B's 60 bytes breaches the
    /// critical threshold of 100. Enforcement must use `repo_unique_csize`, not
    /// `deduplicated_size`, for the just-completed repo's contribution.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn backup_completed_uses_repo_unique_csize_not_archive_delta_for_server_quota(
        pool: PgPool,
    ) {
        let agent = crate::db::insert_agent(&pool, "agent-1", None, "token-hash", None, None)
            .await
            .expect("insert agent");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        let repo_a = crate::db::insert_repo(
            &pool,
            &crate::db::InsertRepoParams {
                name: "shared-repo-a",
                repo_path: "/backups/shared-a",
                ssh_user: "backup",
                ssh_host: "shared.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo a");
        let repo_b = crate::db::insert_repo(
            &pool,
            &crate::db::InsertRepoParams {
                name: "shared-repo-b",
                repo_path: "/backups/shared-b",
                ssh_user: "backup",
                ssh_host: "shared.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo b");
        // Repo B's sibling snapshot, as if left over from an earlier sync/rescan.
        crate::db::update_repo_info_stats(
            &pool,
            repo_b.id,
            &crate::db::RepoInfoStats {
                deduplicated_size: 60,
                ..Default::default()
            },
        )
        .await
        .expect("seed repo b stats");
        let schedule_a = crate::db::insert_schedule(
            &pool,
            repo_a.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "shared-schedule-a",
                schedule_type: shared::types::ScheduleType::Backup,
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                database_backup_enabled: false,
                fs_snapshot: shared::snapshot::FsSnapshotMode::Off,
                lvm_snapshot_percent: 10,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: shared::types::OnFailure::Stop,
            },
            None,
        )
        .await
        .expect("insert schedule a");
        crate::db::insert_schedule_targets(&pool, schedule_a.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule targets");
        db::server_quota::upsert_server_quota(
            &pool,
            "shared.local",
            Some(50),
            Some(100),
            QuotaAction::NotifyOnly,
            QuotaAction::BlockBackups,
            true,
        )
        .await
        .expect("upsert server quota");

        let state = build_test_state(pool.clone());
        // Archive-level delta (5) + sibling snapshot (60) = 65, under the critical
        // threshold of 100. Only the repo-wide total (45) + sibling snapshot (60) = 105
        // breaches it.
        let msg = backup_completed_message_with_repo_size(agent.id, repo_a.id, 5, 45);
        handle_agent_message(&msg, &agent.hostname, agent.id, &state).await;

        let updated = crate::db::get_schedule_by_id(&pool, schedule_a.id)
            .await
            .expect("get schedule");
        assert!(!updated.enabled);

        state
            .background_task_tracker
            .assert_idle(Duration::from_secs(5))
            .await;
    }

    /// The agent reconnecting must both re-enable a schedule the scheduler auto-disabled
    /// for it, and record a `ScheduleReenabled` system event, so the reconnect isn't
    /// invisible outside server logs - matching the `ScheduleAutoDisabled` event the
    /// scheduler already records when it disables the schedule in the first place.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn reconnect_reenables_schedule_and_records_system_event(pool: PgPool) {
        let agent =
            crate::db::insert_agent(&pool, "reconnect-event-agent", None, "hash", None, None)
                .await
                .expect("insert agent");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        let repo = crate::db::insert_repo(
            &pool,
            &crate::db::InsertRepoParams {
                name: "reconnect-event-repo",
                repo_path: "/backups/reconnect-event",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");
        let schedule = crate::db::insert_schedule(
            &pool,
            repo.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "reconnect-event-schedule",
                schedule_type: shared::types::ScheduleType::Backup,
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                database_backup_enabled: false,
                fs_snapshot: shared::snapshot::FsSnapshotMode::Off,
                lvm_snapshot_percent: 10,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: shared::types::OnFailure::Stop,
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(&pool, schedule.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule targets");

        let next = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::days(1))
            .unwrap();
        for _ in 0..3 {
            crate::db::record_schedule_failure(&pool, schedule.id, agent.id, next, 3, true)
                .await
                .expect("record schedule failure");
        }
        let disabled = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .expect("get schedule");
        assert!(!disabled.enabled && disabled.auto_disabled_agent_unreachable);

        let state = build_test_state(pool.clone());
        let mut ui_rx = state.ui_broadcast.subscribe();
        // The real connection handler registers the agent before calling this - the
        // reconnect check now requires every target of a candidate schedule to be
        // currently connected, which for this single-target schedule means just the
        // reconnecting agent itself.
        let (tx, _rx) = mpsc::channel(1);
        state
            .registry
            .register(agent.id, tx, false, None, None)
            .await;
        reenable_system_disabled_schedules_on_reconnect(&state, agent.id, &agent.hostname).await;

        let reenabled = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .expect("get schedule");
        assert!(reenabled.enabled);
        assert!(!reenabled.auto_disabled_agent_unreachable);

        let events = crate::db::get_system_events(&pool, 10, AcknowledgedFilter::All)
            .await
            .expect("get system events");
        let event = events
            .iter()
            .find(|e| matches!(e.event_type, SystemEventType::ScheduleReenabled))
            .expect("a ScheduleReenabled system event was recorded");
        assert_eq!(event.hostname.as_deref(), Some(agent.hostname.as_str()));
        assert!(event.message.contains(&schedule.id.to_string()));

        // Without this, SchedulesView.vue (which only refetches on mount or on a
        // DataChanged message) would never show the re-enabled schedule until a
        // manual reload.
        let mut saw_data_changed = false;
        while let Ok(msg) = ui_rx.try_recv() {
            if matches!(msg, ServerToUi::DataChanged) {
                saw_data_changed = true;
            }
        }
        assert!(
            saw_data_changed,
            "re-enabling a schedule on reconnect must broadcast DataChanged so it shows up live \
             in the UI"
        );
    }

    /// `auto_disabled_by_agent_id` isn't exposed on `ScheduleRow` (it's internal
    /// bookkeeping, not part of the schedule API), so tests that need to assert on it
    /// read it directly.
    async fn auto_disabled_by_agent_id(pool: &PgPool, schedule_id: i64) -> Option<i64> {
        sqlx::query_scalar!(
            "SELECT auto_disabled_by_agent_id FROM schedules WHERE id = $1",
            schedule_id,
        )
        .fetch_one(pool)
        .await
        .expect("select auto_disabled_by_agent_id")
    }

    /// Reproduces the exact bug the registry-connectivity check exists to prevent: for a
    /// multi-target schedule, `auto_disabled_by_agent_id` only ever names whichever
    /// target happened to be first-recorded when the threshold was crossed, not every
    /// target that contributed to the (schedule-wide) failure streak. Here "flaky"
    /// reconnects routinely while "broken" stays down the whole time, but the failure
    /// that actually crosses the threshold is recorded against "flaky" - so a reconnect
    /// handler that only checked `auto_disabled_by_agent_id` would re-enable the
    /// schedule the moment flaky reconnects, immediately reproducing the unbounded-retry
    /// problem for "broken", which this PR exists to fix.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn reconnect_only_reenables_multi_target_schedule_once_every_target_is_connected(
        pool: PgPool,
    ) {
        let flaky = crate::db::insert_agent(&pool, "flaky-agent", None, "hash", None, None)
            .await
            .expect("insert flaky agent");
        let broken = crate::db::insert_agent(&pool, "broken-agent", None, "hash", None, None)
            .await
            .expect("insert broken agent");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        let repo = crate::db::insert_repo(
            &pool,
            &crate::db::InsertRepoParams {
                name: "multi-target-repo",
                repo_path: "/backups/multi-target",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");
        let schedule = crate::db::insert_schedule(
            &pool,
            repo.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "multi-target-schedule",
                schedule_type: shared::types::ScheduleType::Backup,
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                database_backup_enabled: false,
                fs_snapshot: shared::snapshot::FsSnapshotMode::Off,
                lvm_snapshot_percent: 10,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: shared::types::OnFailure::Continue,
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(&pool, schedule.id, &[(flaky.id, 0), (broken.id, 1)])
            .await
            .expect("insert schedule targets");

        let next = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::days(1))
            .unwrap();
        // Two failures attributed to "broken", then the third (threshold-crossing) one
        // attributed to "flaky" - so `auto_disabled_by_agent_id` ends up naming flaky,
        // even though broken is the target that's actually still down.
        crate::db::record_schedule_failure(&pool, schedule.id, broken.id, next, 3, true)
            .await
            .expect("record schedule failure 1");
        crate::db::record_schedule_failure(&pool, schedule.id, broken.id, next, 3, true)
            .await
            .expect("record schedule failure 2");
        crate::db::record_schedule_failure(&pool, schedule.id, flaky.id, next, 3, true)
            .await
            .expect("record schedule failure 3");
        let disabled = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .expect("get schedule");
        assert!(!disabled.enabled && disabled.auto_disabled_agent_unreachable);
        assert_eq!(
            auto_disabled_by_agent_id(&pool, schedule.id).await,
            Some(flaky.id)
        );

        let state = build_test_state(pool.clone());

        // Flaky reconnects, but broken is still down - must not re-enable the schedule.
        let (flaky_tx, _flaky_rx) = mpsc::channel(1);
        state
            .registry
            .register(flaky.id, flaky_tx, false, None, None)
            .await;
        reenable_system_disabled_schedules_on_reconnect(&state, flaky.id, &flaky.hostname).await;

        let still_disabled = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .expect("get schedule");
        assert!(
            !still_disabled.enabled && still_disabled.auto_disabled_agent_unreachable,
            "must not re-enable a multi-target schedule while another target is still \
             unreachable, even if the reconnecting agent is the one named by \
             auto_disabled_by_agent_id"
        );

        // Broken reconnects too - now every target is connected, so the schedule
        // resumes.
        let (broken_tx, _broken_rx) = mpsc::channel(1);
        state
            .registry
            .register(broken.id, broken_tx, false, None, None)
            .await;
        reenable_system_disabled_schedules_on_reconnect(&state, flaky.id, &flaky.hostname).await;

        let reenabled = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .expect("get schedule");
        assert!(reenabled.enabled);
        assert!(!reenabled.auto_disabled_agent_unreachable);
    }

    /// `auto_disabled_by_agent_id` only ever names whichever target happened to be
    /// first-recorded on the disabling tick - not necessarily the target that's
    /// actually been down the whole streak. If reconnect handling only reconsidered a
    /// schedule when *that specific* agent reconnects, a schedule could stay disabled
    /// forever once the credited agent has already recovered and stopped generating
    /// new reconnect events, even after the truly-broken target comes back too.
    /// Reconnect handling must reconsider a candidate schedule on *any* of its
    /// targets reconnecting - here "broken" reconnects (not "flaky", the one credited
    /// via `auto_disabled_by_agent_id`), and that alone must be enough to re-enable
    /// the schedule once every target, including the already-connected "flaky", is up.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn reconnect_from_uncredited_target_reenables_schedule_once_all_connected(pool: PgPool) {
        let flaky = crate::db::insert_agent(&pool, "flaky-agent-2", None, "hash", None, None)
            .await
            .expect("insert flaky agent");
        let broken = crate::db::insert_agent(&pool, "broken-agent-2", None, "hash", None, None)
            .await
            .expect("insert broken agent");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        let repo = crate::db::insert_repo(
            &pool,
            &crate::db::InsertRepoParams {
                name: "uncredited-target-repo",
                repo_path: "/backups/uncredited-target",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");
        let schedule = crate::db::insert_schedule(
            &pool,
            repo.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: "uncredited-target-schedule",
                schedule_type: shared::types::ScheduleType::Backup,
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                database_backup_enabled: false,
                fs_snapshot: shared::snapshot::FsSnapshotMode::Off,
                lvm_snapshot_percent: 10,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: shared::types::OnFailure::Continue,
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(&pool, schedule.id, &[(flaky.id, 0), (broken.id, 1)])
            .await
            .expect("insert schedule targets");

        let next = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::days(1))
            .unwrap();
        // Same attribution as the sibling test: the threshold-crossing failure is
        // recorded against "flaky", so auto_disabled_by_agent_id ends up naming flaky
        // even though "broken" is the one reconnecting below.
        crate::db::record_schedule_failure(&pool, schedule.id, broken.id, next, 3, true)
            .await
            .expect("record schedule failure 1");
        crate::db::record_schedule_failure(&pool, schedule.id, broken.id, next, 3, true)
            .await
            .expect("record schedule failure 2");
        crate::db::record_schedule_failure(&pool, schedule.id, flaky.id, next, 3, true)
            .await
            .expect("record schedule failure 3");
        assert_eq!(
            auto_disabled_by_agent_id(&pool, schedule.id).await,
            Some(flaky.id)
        );

        let state = build_test_state(pool.clone());

        // Flaky already recovered and is connected, but generates no new reconnect
        // event here - only "broken" does.
        let (flaky_tx, _flaky_rx) = mpsc::channel(1);
        state
            .registry
            .register(flaky.id, flaky_tx, false, None, None)
            .await;
        let (broken_tx, _broken_rx) = mpsc::channel(1);
        state
            .registry
            .register(broken.id, broken_tx, false, None, None)
            .await;
        reenable_system_disabled_schedules_on_reconnect(&state, broken.id, &broken.hostname).await;

        let reenabled = crate::db::get_schedule_by_id(&pool, schedule.id)
            .await
            .expect("get schedule");
        assert!(
            reenabled.enabled,
            "the uncredited target's own reconnect must still re-enable the schedule once every \
             target is connected"
        );
        assert!(!reenabled.auto_disabled_agent_unreachable);
    }

    /// Agent, repository and catch-up-enabled schedule for the reconnect catch-up
    /// tests below, with `agent_id` already carrying a missed occurrence.
    async fn insert_catch_up_fixture(
        pool: &PgPool,
        name: &str,
        next_run_at: chrono::DateTime<chrono::Utc>,
    ) -> (crate::db::AgentRow, i64) {
        let agent = crate::db::insert_agent(pool, name, None, "hash", None, None)
            .await
            .expect("insert agent");
        // Only a host marked as not always online is waited for.
        sqlx::query!(
            "UPDATE machines SET intermittent = true WHERE id = (SELECT machine_id FROM agents \
             WHERE id = $1)",
            agent.id
        )
        .execute(pool)
        .await
        .expect("mark the agent as not always online");
        let passphrase_encrypted = encrypt_passphrase(
            "test-passphrase",
            &derive_key(b"handler-test-secret-key").unwrap(),
        )
        .expect("encrypt passphrase");
        let repo = crate::db::insert_repo(
            pool,
            &crate::db::InsertRepoParams {
                name: &format!("{name}-repo"),
                repo_path: "/backups/catch-up",
                ssh_user: "backup",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");
        crate::db::update_repo_ssh_host_key(pool, repo.id, "ssh-ed25519 AAAACATCHUP")
            .await
            .expect("set repo host key");
        let schedule = crate::db::insert_schedule(
            pool,
            repo.id,
            &crate::db::ScheduleParams {
                wake_override: ScheduleWakeOverride::HostDefault,
                name: &format!("{name}-schedule"),
                schedule_type: shared::types::ScheduleType::Backup,
                cron_expression: "0 2 * * *",
                enabled: true,
                canary_enabled: false,
                vm_snapshot_enabled: false,
                database_backup_enabled: false,
                fs_snapshot: shared::snapshot::FsSnapshotMode::Off,
                lvm_snapshot_percent: 10,
                exclude_patterns_raw: "",
                include_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: &[],
                post_backup_commands: &[],
                hook_timeout_seconds: 60,
                missed_backup_threshold: 3,
                catch_up_min_lead_minutes: 120,
                on_failure: shared::types::OnFailure::Stop,
            },
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(pool, schedule.id, &[(agent.id, 0)])
            .await
            .expect("insert schedule targets");
        crate::db::set_next_run_at(pool, schedule.id, next_run_at)
            .await
            .expect("set next run");
        crate::db::catch_up::mark_catch_up_pending(
            pool,
            schedule.id,
            agent.id,
            chrono::Utc::now()
                .checked_sub_signed(chrono::Duration::days(35))
                .unwrap(),
        )
        .await
        .expect("mark catch-up pending");
        (agent, schedule.id)
    }

    async fn pending_catch_ups(pool: &PgPool, agent_id: i64) -> usize {
        crate::db::catch_up::list_catch_up_candidates_for_agent(pool, agent_id)
            .await
            .expect("list catch-up candidates")
            .len()
    }

    async fn recorded_catch_up_events(pool: &PgPool) -> usize {
        crate::db::get_system_events(pool, 20, AcknowledgedFilter::All)
            .await
            .expect("get system events")
            .iter()
            .filter(|e| matches!(e.event_type, SystemEventType::ScheduleCatchUp))
            .count()
    }

    /// The happy path: a host that missed a run comes back with plenty of time before
    /// the next scheduled one, so the run it missed is dispatched to it.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn reconnect_runs_the_occurrence_the_host_missed(pool: PgPool) {
        let next_run = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::hours(17))
            .unwrap();
        let (agent, schedule_id) = insert_catch_up_fixture(&pool, "catch-up-runs", next_run).await;

        let state = build_test_state(pool.clone());
        let (tx, mut rx) = mpsc::channel(8);
        state
            .registry
            .register(agent.id, tx, false, None, None)
            .await;

        catch_up::run_catch_ups_on_reconnect(&state, agent.id, &agent.hostname).await;

        let first = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("a catch-up run must be dispatched on reconnect")
            .expect("registry channel open");
        assert!(
            matches!(first, ServerToAgent::ConfigUpdate(_)),
            "the catch-up must push a fresh config first, got: {first:?}"
        );
        let second = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("the run trigger must follow the config push")
            .expect("registry channel open");
        assert!(
            matches!(second, ServerToAgent::RunBackupNow { .. }),
            "the catch-up must trigger the missed backup, got: {second:?}"
        );

        assert_eq!(
            pending_catch_ups(&pool, agent.id).await,
            0,
            "the marker must be cleared, so a second reconnect cannot run it again"
        );
        assert_eq!(recorded_catch_up_events(&pool).await, 1);
        assert!(
            crate::db::get_schedule_by_id(&pool, schedule_id)
                .await
                .expect("get schedule")
                .enabled
        );

        // Lets the dispatch task's completion wait finish instead of outliving the test.
        state.registry.unregister(agent.id).await;
    }

    /// The regular run is 30 minutes out and the floor is two hours: catching up now
    /// would only duplicate it, so the miss is dropped instead.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn reconnect_skips_a_catch_up_when_the_next_run_is_close(pool: PgPool) {
        let next_run = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::minutes(30))
            .unwrap();
        let (agent, _) = insert_catch_up_fixture(&pool, "catch-up-skips", next_run).await;

        let state = build_test_state(pool.clone());
        let (tx, mut rx) = mpsc::channel(8);
        state
            .registry
            .register(agent.id, tx, false, None, None)
            .await;

        catch_up::run_catch_ups_on_reconnect(&state, agent.id, &agent.hostname).await;

        assert!(
            rx.try_recv().is_err(),
            "nothing may be dispatched when the next scheduled run is inside the floor"
        );
        assert_eq!(
            pending_catch_ups(&pool, agent.id).await,
            0,
            "a skipped miss is dropped, not carried forward to the next reconnect"
        );
        assert_eq!(recorded_catch_up_events(&pool).await, 0);
    }

    /// Marking the agent as always online between the miss and the reconnect means
    /// the run is no longer wanted - the marker still goes, so it cannot resurface
    /// later.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn reconnect_drops_a_catch_up_the_agent_no_longer_wants(pool: PgPool) {
        let next_run = chrono::Utc::now()
            .checked_add_signed(chrono::Duration::hours(17))
            .unwrap();
        let (agent, _) = insert_catch_up_fixture(&pool, "catch-up-off", next_run).await;
        sqlx::query!(
            "UPDATE machines SET intermittent = false WHERE id = (SELECT machine_id FROM agents \
             WHERE id = $1)",
            agent.id,
        )
        .execute(&pool)
        .await
        .expect("mark the agent as always online");

        let state = build_test_state(pool.clone());
        let (tx, mut rx) = mpsc::channel(8);
        state
            .registry
            .register(agent.id, tx, false, None, None)
            .await;

        catch_up::run_catch_ups_on_reconnect(&state, agent.id, &agent.hostname).await;

        assert!(
            rx.try_recv().is_err(),
            "an agent no longer marked as not always online runs nothing"
        );
        let still_pending = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM schedule_targets WHERE agent_id = $1 AND catch_up_pending_for \
             IS NOT NULL",
            agent.id,
        )
        .fetch_one(&pool)
        .await
        .expect("count pending");
        assert_eq!(still_pending, Some(0), "the stale marker must be cleared");
    }

    /// Serialises `msg` for [`handle_agent_message`].
    fn wire(msg: &AgentToServer) -> String {
        serde_json::to_string(msg).expect("serialize")
    }

    /// Every [`ServerToUi`] event pushed so far on `ui`.
    fn drain_ui(ui: &mut tokio::sync::broadcast::Receiver<ServerToUi>) -> Vec<ServerToUi> {
        std::iter::from_fn(|| ui.try_recv().ok()).collect()
    }

    fn check_completed(repo_id: i64, success: bool) -> String {
        wire(&AgentToServer::CheckCompleted {
            repo_id: RepoId(repo_id),
            success,
            duration_secs: 42,
            error_message: (!success).then(|| "segment 7 is corrupt".to_owned()),
        })
    }

    /// A `borg check` result from the agent that owns the repository settles
    /// whoever waits on it, is pushed to the UI under the repository's name
    /// and goes out as a check notification - succeeded or failed - named
    /// after the check schedule, or after the repository when that schedule
    /// has no name, or with no schedule at all.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_check_result_is_published_pushed_and_notified(pool: PgPool) {
        let (agent, repo, _schedule) = create_agent_repo_schedule(&pool).await;

        // No check schedule yet, then a blank-named one, then a named one.
        for (schedule_name, success, event_type) in [
            (None, true, "check_success"),
            (Some(" "), false, "check_failed"),
            (Some("weekly check"), true, "check_success"),
        ] {
            if let Some(name) = schedule_name {
                sqlx::query!("DELETE FROM schedules WHERE schedule_type = 'check'")
                    .execute(&pool)
                    .await
                    .expect("clear check schedules");
                add_schedule(&pool, repo.id, agent.id, ScheduleType::Check, name).await;
            }
            let state = build_test_state(pool.clone());
            let mut outcomes = state.completion_bus.subscribe();
            let mut ui = state.ui_broadcast.subscribe();

            let (_channel, deliveries) = deliver_agent_message_with_state(
                &state,
                &agent,
                &check_completed(repo.id, success),
                &["check_success", "check_failed"],
            )
            .await;

            let outcome = outcomes.try_recv().expect("the result is published");
            assert_eq!(
                (outcome.agent_id, outcome.repo_id, outcome.success),
                (agent.id, repo.id, success)
            );
            assert!(drain_ui(&mut ui).iter().any(|event| matches!(
                event,
                ServerToUi::CheckCompleted { target_name, success: pushed, .. }
                    if target_name == "handler-test-repo" && *pushed == success
            )));
            assert_eq!(deliveries, vec![event_type.to_owned()]);
        }
    }

    /// A check result for a repository the agent does not back up is
    /// dropped: nothing waiting on it hears of it.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_check_result_from_a_rogue_agent_is_dropped(pool: PgPool) {
        let (_agent, repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue = crate::db::insert_agent(&pool, "check-rogue", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool);
        let mut outcomes = state.completion_bus.subscribe();

        handle_agent_message(
            &check_completed(repo.id, true),
            &rogue.hostname,
            rogue.id,
            &state,
        )
        .await;

        assert!(outcomes.try_recv().is_err());
    }

    /// A verify result from the agent that owns the repository settles
    /// whoever waits on it and is pushed to the UI; one from another agent is
    /// dropped.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_verify_result_is_published_and_pushed_only_for_the_owner(pool: PgPool) {
        let (agent, repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue = crate::db::insert_agent(&pool, "verify-rogue", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool);
        let mut outcomes = state.completion_bus.subscribe();
        let mut ui = state.ui_broadcast.subscribe();
        let verified = wire(&AgentToServer::VerifyCompleted {
            repo_id: RepoId(repo.id),
            success: false,
            duration_secs: 9,
            error_message: Some("archive data mismatch".into()),
            files_verified: 12,
        });

        handle_agent_message(&verified, &rogue.hostname, rogue.id, &state).await;
        assert!(outcomes.try_recv().is_err());
        assert!(drain_ui(&mut ui).is_empty());

        handle_agent_message(&verified, &agent.hostname, agent.id, &state).await;
        let outcome = outcomes.try_recv().expect("the result is published");
        assert!(!outcome.success);
        let pushed = drain_ui(&mut ui);
        assert!(pushed.iter().any(|event| matches!(
            event,
            ServerToUi::VerifyCompleted { target_name, error_message, .. }
                if target_name == "handler-test-repo"
                    && error_message.as_deref() == Some("archive data mismatch")
        )));
        assert!(matches!(pushed.last(), Some(ServerToUi::DataChanged)));
    }

    /// A status update is only logged, but still checked against the
    /// repositories the agent backs up: one for another agent's repository
    /// records a security violation.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_status_update_for_a_foreign_repository_is_a_violation(pool: PgPool) {
        let (agent, repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue = crate::db::insert_agent(&pool, "status-rogue", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool.clone());
        let update = |repo_id| {
            wire(&AgentToServer::StatusUpdate {
                repo_id: RepoId(repo_id),
                status: shared::types::AgentStatus::Online,
            })
        };

        handle_agent_message(&update(repo.id), &agent.hostname, agent.id, &state).await;
        handle_agent_message(&update(repo.id), &rogue.hostname, rogue.id, &state).await;

        let violations: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM system_events WHERE event_type = \
             'security_violation'"
        )
        .fetch_one(&pool)
        .await
        .expect("count violations");
        assert_eq!(violations, 1);
    }

    fn vm(name: &str) -> shared::vm::DiscoveredVm {
        shared::vm::DiscoveredVm {
            name: name.into(),
            state: shared::vm::VmState::Running,
            mode: shared::vm::VmSnapshotMode::Incremental,
            disk_count: 1,
            disk_bytes: 4096,
        }
    }

    /// A VM scan is recorded and handed to whoever asked for it; a failed
    /// scan is handed over as it is without replacing what was recorded, and
    /// a scan nobody asked for (sent on connect) is only recorded.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_vm_scan_is_recorded_and_answered(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "vm-scan-host", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool.clone());

        let (tx, rx) = tokio::sync::oneshot::channel();
        state
            .pending_vm_scans
            .insert("scan-1".to_owned(), agent.id, tx)
            .await;
        let scan = wire(&AgentToServer::VmScanResult {
            request_id: Some("scan-1".into()),
            vms: vec![vm("web01")],
            error: None,
        });
        handle_agent_message(&scan, &agent.hostname, agent.id, &state).await;
        let (vms, error) = rx.await.expect("the scan is answered");
        assert_eq!((vms.len(), error), (1, None));

        let (tx, rx) = tokio::sync::oneshot::channel();
        state
            .pending_vm_scans
            .insert("scan-2".to_owned(), agent.id, tx)
            .await;
        let failed = wire(&AgentToServer::VmScanResult {
            request_id: Some("scan-2".into()),
            vms: Vec::new(),
            error: Some("libvirt is not running".into()),
        });
        handle_agent_message(&failed, &agent.hostname, agent.id, &state).await;
        let (_, error) = rx.await.expect("the failure is answered");
        assert_eq!(error.as_deref(), Some("libvirt is not running"));

        let unprompted = wire(&AgentToServer::VmScanResult {
            request_id: None,
            vms: vec![vm("web01"), vm("db01")],
            error: None,
        });
        handle_agent_message(&unprompted, &agent.hostname, agent.id, &state).await;

        let recorded: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM agent_vms WHERE agent_id = $1",
            agent.id
        )
        .fetch_one(&pool)
        .await
        .expect("count VMs");
        assert_eq!(recorded, 2);
    }

    fn snapshot_outcome(name: &str, error: Option<&str>) -> shared::vm::VmSnapshotOutcome {
        shared::vm::VmSnapshotOutcome {
            name: name.into(),
            action: shared::vm::VmRunAction::Increment,
            mode: shared::vm::VmSnapshotMode::Incremental,
            staged_bytes: 2048,
            chain_length: 3,
            error: error.map(str::to_owned),
        }
    }

    /// A VM snapshot report from a backup run records every outcome.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_vm_snapshot_report_is_recorded(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "vm-report-host", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool.clone());
        let report = wire(&AgentToServer::VmSnapshotReport {
            schedule_id: None,
            outcomes: vec![
                snapshot_outcome("web01", None),
                snapshot_outcome("db01", Some("disk busy")),
            ],
        });

        handle_agent_message(&report, &agent.hostname, agent.id, &state).await;

        let staged: Vec<i64> = sqlx::query_scalar!(
            "SELECT staged_bytes FROM agent_vms WHERE agent_id = $1",
            agent.id
        )
        .fetch_all(&pool)
        .await
        .expect("read outcomes");
        assert_eq!(staged, vec![2048, 2048]);
    }

    /// A failed manual VM snapshot is still handed to whoever asked for it,
    /// error and all.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_vm_stage_is_answered(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "vm-stage-fail", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool);
        let (tx, rx) = tokio::sync::oneshot::channel();
        state
            .pending_vm_stages
            .insert("stage-1".to_owned(), agent.id, tx)
            .await;

        let stage = wire(&AgentToServer::VmStageResult {
            request_id: Some("stage-1".into()),
            outcome: snapshot_outcome("web01", Some("no space left")),
        });
        handle_agent_message(&stage, &agent.hostname, agent.id, &state).await;

        let outcome = rx.await.expect("the stage is answered");
        assert_eq!(outcome.error.as_deref(), Some("no space left"));
    }

    /// When the database is gone, VM scans, snapshot reports and stages are
    /// still answered; only recording them fails, and that is logged.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn vm_results_the_database_rejects_are_still_answered(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "vm-db-down", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool.clone());
        let (scan_tx, scan_rx) = tokio::sync::oneshot::channel();
        state
            .pending_vm_scans
            .insert("scan".to_owned(), agent.id, scan_tx)
            .await;
        let (stage_tx, stage_rx) = tokio::sync::oneshot::channel();
        state
            .pending_vm_stages
            .insert("stage".to_owned(), agent.id, stage_tx)
            .await;
        pool.close().await;

        let messages = [
            AgentToServer::VmScanResult {
                request_id: Some("scan".into()),
                vms: vec![vm("web01")],
                error: None,
            },
            AgentToServer::VmSnapshotReport {
                schedule_id: Some(1),
                outcomes: vec![snapshot_outcome("web01", None)],
            },
            AgentToServer::VmStageResult {
                request_id: Some("stage".into()),
                outcome: snapshot_outcome("web01", None),
            },
        ];
        for msg in &messages {
            handle_agent_message(&wire(msg), &agent.hostname, agent.id, &state).await;
        }

        assert_eq!(scan_rx.await.expect("the scan is answered").0.len(), 1);
        assert_eq!(stage_rx.await.expect("the stage is answered").name, "web01");
    }

    /// Encryption-migration and delete-archives results reach whoever waits
    /// on them; a result nobody waits on (a repeat, or a request that timed
    /// out) is dropped.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn migration_and_delete_results_are_answered_once(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "answers-host", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool);
        let (migrated_tx, migrated_rx) = tokio::sync::oneshot::channel();
        state
            .pending_migrations
            .insert("migrate-1".to_owned(), agent.id, migrated_tx)
            .await;
        let (deleted_tx, deleted_rx) = tokio::sync::oneshot::channel();
        state
            .pending_deletes
            .insert("delete-1".to_owned(), agent.id, deleted_tx)
            .await;
        let migrated = wire(&AgentToServer::MigrateEncryptionCompleted {
            request_id: "migrate-1".into(),
            success: true,
            error_message: None,
        });
        let deleted = wire(&AgentToServer::DeleteArchivesResult {
            request_id: "delete-1".into(),
            success: true,
            deleted_count: 3,
            error_message: None,
        });

        for msg in [&migrated, &deleted, &migrated, &deleted] {
            handle_agent_message(msg, &agent.hostname, agent.id, &state).await;
        }

        assert_eq!(
            migrated_rx.await.expect("the migration is answered"),
            (true, None)
        );
        assert_eq!(
            deleted_rx.await.expect("the delete is answered"),
            (true, 3, None)
        );
    }

    /// A dry-run result nobody waits on is dropped without effect.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_unexpected_dry_run_result_is_dropped(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "dry-run-host", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool);
        let mut ui = state.ui_broadcast.subscribe();

        let result = wire(&AgentToServer::DryRunResult {
            request_id: "never-asked".into(),
            files: Vec::new(),
            total_size: 0,
            error_message: None,
        });
        handle_agent_message(&result, &agent.hostname, agent.id, &state).await;

        assert!(drain_ui(&mut ui).is_empty());
    }

    /// A live file nobody asked for is dropped with a warning.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_unexpected_live_file_is_dropped(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "live-host", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool);

        let read = wire(&AgentToServer::LiveFileRead {
            request_id: "never-asked".into(),
            file: shared::live_file::LiveFile::Binary,
        });
        let logs = crate::test_support::warnings_while(Box::pin(handle_agent_message(
            &read,
            &agent.hostname,
            agent.id,
            &state,
        )))
        .await;

        assert!(crate::test_support::logged(
            &logs,
            "unexpected LiveFileRead with no pending request"
        ));
        // With nothing listening for warnings, it is dropped all the same.
        handle_agent_message(&read, &agent.hostname, agent.id, &state).await;
    }

    /// Messages the server only logs - a second Hello, a failed restart, a
    /// repository initialised on the agent's own initiative, answers that
    /// belong to an HTTP request's own channel - and text that is not a
    /// message at all change nothing.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn messages_the_server_only_logs_change_nothing(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "log-only-host", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool.clone());
        let mut ui = state.ui_broadcast.subscribe();
        let mut outcomes = state.completion_bus.subscribe();

        let messages = [
            wire(&AgentToServer::Hello {
                hostname: agent.hostname.clone(),
                token: "ignored".into(),
                agent_version: "1.0.0".into(),
                agent_git_sha: None,
                agent_build_time: None,
                agent_commit_count: None,
                supports_restart: false,
                restart_unavailable_reason: None,
                instance_id: None,
                snapshot_tools: Vec::new(),
            }),
            wire(&AgentToServer::RestartFailed {
                error_message: "systemd refused".into(),
            }),
            wire(&AgentToServer::InitRepoCompleted {
                repo_path: "/backup/new".into(),
                success: true,
                error_message: None,
            }),
            wire(&AgentToServer::SearchResult {
                request_id: "search-1".into(),
                entries: Vec::new(),
                done: true,
            }),
            "not a message".to_owned(),
        ];
        for msg in &messages {
            handle_agent_message(msg, &agent.hostname, agent.id, &state).await;
        }

        assert!(drain_ui(&mut ui).is_empty());
        assert!(outcomes.try_recv().is_err());
        let events: i64 = sqlx::query_scalar!("SELECT COUNT(*) AS \"count!\" FROM system_events")
            .fetch_one(&pool)
            .await
            .expect("count events");
        assert_eq!(events, 0);
    }

    /// Frames a fake agent socket has been sent, in order.
    type SentFrames = std::sync::Arc<std::sync::Mutex<Vec<Message>>>;

    /// The sending half of a fake agent socket that records every frame.
    fn recording_sink() -> (
        impl Sink<Message, Error = std::io::Error> + Unpin,
        SentFrames,
    ) {
        let sent = SentFrames::default();
        let sink = Box::pin(futures_util::sink::unfold(
            std::sync::Arc::clone(&sent),
            |sent: SentFrames, frame: Message| async move {
                sent.lock().unwrap().push(frame);
                Ok::<_, std::io::Error>(sent)
            },
        ));
        (sink, sent)
    }

    /// The sending half of a fake agent socket whose peer has gone: every
    /// send fails, and keeps failing.
    fn failing_sink() -> impl Sink<Message, Error = std::io::Error> + Unpin {
        futures_util::sink::drain::<Message>()
            .sink_map_err(|never| match never {})
            .with_flat_map(|_frame: Message| {
                futures_util::stream::iter([Err(std::io::Error::other("connection reset by peer"))])
            })
    }

    /// The receiving half of a fake agent socket: `frames`, then the end of
    /// the connection.
    fn frames(frames: Vec<Message>) -> impl Stream<Item = Result<Message, std::io::Error>> + Unpin {
        futures_util::stream::iter(frames.into_iter().map(Ok))
    }

    /// The receiving half of a fake agent socket that sends `frames` and then
    /// stays open.
    fn frames_then_open(
        frames: Vec<Message>,
    ) -> impl Stream<Item = Result<Message, std::io::Error>> + Unpin {
        futures_util::stream::iter(frames.into_iter().map(Ok))
            .chain(futures_util::stream::pending())
    }

    /// The `ServerToAgent` messages among `sent`, decoded.
    fn sent_messages(sent: &SentFrames) -> Vec<ServerToAgent> {
        sent.lock()
            .unwrap()
            .iter()
            .filter_map(|frame| match frame {
                Message::Text(text) => serde_json::from_str(text.as_str()).ok(),
                _ => None,
            })
            .collect()
    }

    /// The reason of the close frame among `sent`, if any.
    fn close_reason(sent: &SentFrames) -> Option<String> {
        sent.lock().unwrap().iter().find_map(|frame| match frame {
            Message::Close(Some(close)) => Some(close.reason.to_string()),
            _ => None,
        })
    }

    fn hello_frame(hostname: &str, token: &str) -> Message {
        Message::Text(
            wire(&AgentToServer::Hello {
                hostname: hostname.into(),
                token: token.into(),
                agent_version: "1.0.0".into(),
                agent_git_sha: None,
                agent_build_time: None,
                agent_commit_count: None,
                supports_restart: true,
                restart_unavailable_reason: None,
                instance_id: Some("instance-1".into()),
                snapshot_tools: Vec::new(),
            })
            .into(),
        )
    }

    const SOCKET_TOKEN: &str = "socket-test-token";

    /// An agent whose token is [`SOCKET_TOKEN`].
    async fn socket_agent(pool: &PgPool, hostname: &str) -> crate::db::AgentRow {
        let hash = bcrypt::hash(SOCKET_TOKEN, 4).expect("hash token");
        crate::db::insert_agent(pool, hostname, None, &hash, None, None)
            .await
            .expect("insert agent")
    }

    /// A connection whose first frame is not a Hello - another message, a
    /// binary frame, or nothing at all - is closed with the reason why.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_connection_that_does_not_start_with_hello_is_closed(pool: PgPool) {
        let state = build_test_state(pool);
        for first in [
            vec![Message::Text(wire(&AgentToServer::Pong).into())],
            vec![Message::Binary(vec![1, 2, 3].into())],
            Vec::new(),
        ] {
            let (sink, sent) = recording_sink();
            Box::pin(serve_agent(sink, frames(first), state.clone())).await;
            assert_eq!(
                close_reason(&sent).as_deref(),
                Some("expected Hello message")
            );
        }
    }

    /// A close frame the peer can no longer take is only logged.
    #[tokio::test]
    async fn a_close_frame_that_cannot_be_sent_is_dropped() {
        let (output, logs) = crate::test_support::formatted_logs_while(Box::pin(send_close(
            &mut failing_sink(),
            "expected Hello message",
        )))
        .await;
        assert_eq!(output, ());
        assert!(logs.contains("ws send failed"), "{logs}");
    }

    /// A Hello with a token that matches no agent registered under its
    /// hostname - a wrong token, or a hostname nobody registered - is refused
    /// with a close frame and recorded as a failed authentication.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_hello_with_a_wrong_token_is_refused(pool: PgPool) {
        let agent = socket_agent(&pool, "known-host").await;
        let state = build_test_state(pool.clone());

        for (hostname, token) in [
            ("known-host", "wrong-token"),
            ("unknown-host", SOCKET_TOKEN),
        ] {
            let (sink, sent) = recording_sink();
            Box::pin(serve_agent(
                sink,
                frames(vec![hello_frame(hostname, token)]),
                state.clone(),
            ))
            .await;
            assert_eq!(
                close_reason(&sent).as_deref(),
                Some("authentication failed")
            );
            assert!(sent_messages(&sent).is_empty());
        }
        assert!(!state.registry.is_connected(agent.id).await);

        let failures: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM system_events WHERE event_type = 'auth_failed'"
        )
        .fetch_one(&pool)
        .await
        .expect("count failures");
        assert_eq!(failures, 2);
    }

    /// With the database gone an agent cannot be authenticated; it is
    /// refused, and the failure to record that is logged.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_agent_cannot_connect_while_the_database_is_gone(pool: PgPool) {
        socket_agent(&pool, "db-down-host").await;
        let state = build_test_state(pool.clone());
        pool.close().await;

        let (sink, sent) = recording_sink();
        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(serve_agent(
            sink,
            frames(vec![hello_frame("db-down-host", SOCKET_TOKEN)]),
            state,
        )))
        .await;

        assert_eq!(
            close_reason(&sent).as_deref(),
            Some("authentication failed")
        );
        assert!(logs.contains("failed to insert system event"), "{logs}");
    }

    /// A connected agent's session: pings and binary frames are ignored, its
    /// messages are handled, the operation it was running is released when
    /// it goes, and the UI hears it connect and disconnect.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_agent_session_runs_until_the_agent_goes(pool: PgPool) {
        let agent = socket_agent(&pool, "session-host").await;
        let state = build_test_state(pool.clone());
        let mut ui = state.ui_broadcast.subscribe();
        state
            .repo_op_tracker
            .set(
                7,
                shared::protocol::RepoOpKind::AgentBackup,
                agent.hostname.clone(),
                Some(agent.id),
            )
            .await;

        let (sink, _sent) = recording_sink();
        Box::pin(serve_agent(
            sink,
            frames(vec![
                hello_frame("session-host", SOCKET_TOKEN),
                Message::Ping(Vec::new().into()),
                Message::Binary(vec![0].into()),
                Message::Text(wire(&AgentToServer::Pong).into()),
            ]),
            state.clone(),
        ))
        .await;

        let pushed = drain_ui(&mut ui);
        assert!(
            pushed
                .iter()
                .any(|e| matches!(e, ServerToUi::AgentConnected { .. }))
        );
        assert!(pushed.iter().any(|e| matches!(
            e,
            ServerToUi::RepoOpChanged {
                repo_id: 7,
                op: None
            }
        )));
        assert!(matches!(
            pushed.last(),
            Some(ServerToUi::AgentDisconnected { .. })
        ));
        assert!(!state.registry.is_connected(agent.id).await);
        assert!(state.repo_op_tracker.get(7).await.is_none());
    }

    /// Shutting the server down ends every agent session.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn shutdown_ends_an_agent_session(pool: PgPool) {
        let agent = socket_agent(&pool, "shutdown-host").await;
        let state = build_test_state(pool);
        let (sink, _sent) = recording_sink();
        let session = tokio::spawn(serve_agent(
            sink,
            frames_then_open(vec![hello_frame("shutdown-host", SOCKET_TOKEN)]),
            state.clone(),
        ));
        timeout(Duration::from_secs(10), async {
            while !state.registry.is_connected(agent.id).await {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the agent connects");

        state.shutdown_token.cancel();

        timeout(Duration::from_secs(10), session)
            .await
            .expect("the session ends")
            .expect("the session does not panic");
        assert!(!state.registry.is_connected(agent.id).await);
    }

    /// A session whose agent can no longer be written to ends at the first
    /// message the server has for it.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_session_ends_when_the_agent_cannot_be_written_to(pool: PgPool) {
        let agent = socket_agent(&pool, "gone-host").await;
        let state = build_test_state(pool);
        let session = tokio::spawn(serve_agent(
            failing_sink(),
            frames_then_open(vec![hello_frame("gone-host", SOCKET_TOKEN)]),
            state.clone(),
        ));
        timeout(Duration::from_secs(10), async {
            while !state.registry.is_connected(agent.id).await {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the agent connects");

        // Whichever message comes first ends it: this one, or the session's
        // own first ping.
        let _ = state.registry.send_to(agent.id, ServerToAgent::Ping).await;

        timeout(Duration::from_secs(10), session)
            .await
            .expect("the session ends")
            .expect("the session does not panic");
        assert!(!state.registry.is_connected(agent.id).await);
    }

    /// An agent that connects while its previous session is still registered
    /// replaces it, and every backup the old session had running is failed
    /// and settled for whoever waits on it.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_replacing_session_fails_the_old_sessions_backups(pool: PgPool) {
        let agent = socket_agent(&pool, "replaced-host").await;
        let (_owner, repo, _schedule) = create_agent_repo_schedule(&pool).await;
        crate::db::insert_backup_started(&pool, agent.id, repo.id, None, Utc::now(), None, None)
            .await
            .expect("insert started backup");
        let state = build_test_state(pool.clone());
        let (old_tx, _old_rx) = mpsc::channel(1);
        state
            .registry
            .register(agent.id, old_tx, false, None, None)
            .await;
        let mut outcomes = state.completion_bus.subscribe();

        let (sink, _sent) = recording_sink();
        Box::pin(serve_agent(
            sink,
            frames(vec![hello_frame("replaced-host", SOCKET_TOKEN)]),
            state,
        ))
        .await;

        let outcome = outcomes.try_recv().expect("the old backup is settled");
        assert_eq!((outcome.repo_id, outcome.success), (repo.id, false));
        let started: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM backup_reports WHERE agent_id = $1 AND status = \
             'started'",
            agent.id
        )
        .fetch_one(&pool)
        .await
        .expect("count started");
        assert_eq!(started, 0);
    }

    /// When the old session's backups cannot be failed, the new session
    /// carries on and the failure is logged.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_replacing_session_survives_a_database_that_rejects_the_cleanup(pool: PgPool) {
        let agent = socket_agent(&pool, "replaced-db-host").await;
        let (_owner, repo, _schedule) = create_agent_repo_schedule(&pool).await;
        crate::db::insert_backup_started(&pool, agent.id, repo.id, None, Utc::now(), None, None)
            .await
            .expect("insert started backup");
        reject_writes(&pool, WriteRejecting::BackupReports).await;
        let state = build_test_state(pool);
        let (old_tx, _old_rx) = mpsc::channel(1);
        state
            .registry
            .register(agent.id, old_tx, false, None, None)
            .await;

        let (sink, _sent) = recording_sink();
        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(serve_agent(
            sink,
            frames(vec![hello_frame("replaced-db-host", SOCKET_TOKEN)]),
            state,
        )))
        .await;

        assert!(
            logs.contains("failed to abandon stale in-flight backups on agent reconnect"),
            "{logs}"
        );
    }

    /// On connecting, an agent is handed its config, told about backups the
    /// user cancelled while it was away, and given the runs it missed.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_reconnecting_agent_is_caught_up(pool: PgPool) {
        let (agent, repo, schedule) = create_agent_repo_schedule(&pool).await;
        sqlx::query!(
            "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
             status, run_id) VALUES ($1, $2, $3, now(), now(), 'pending', 'run-pending'), ($1, \
             $2, $3, now() - interval '1 hour', now(), 'cancelled', NULL)",
            agent.id,
            repo.id,
            schedule.id
        )
        .execute(&pool)
        .await
        .expect("insert pending and cancelled runs");
        // A repository the server has reached before has its host key pinned.
        crate::db::update_repo_ssh_host_key(&pool, repo.id, "ssh-ed25519 AAAATEST")
            .await
            .expect("pin host key");
        let state = build_test_state(pool);

        let (mut sink, sent) = recording_sink();
        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(
            send_reconnect_catchup(&state, &mut sink, &agent.hostname, agent.id, None),
        ))
        .await;

        // Catching up leaves the connection open.
        assert_eq!(close_reason(&sent), None);
        let messages = sent_messages(&sent);
        assert!(
            matches!(messages.first(), Some(ServerToAgent::ConfigUpdate(_))),
            "{messages:?}\n{logs}"
        );
        assert!(messages.iter().any(|m| matches!(
            m,
            ServerToAgent::CancelBackup { repo_id } if repo_id.0 == repo.id
        )));
        assert!(messages.iter().any(|m| matches!(
            m,
            ServerToAgent::RunBackupNow { run_id: Some(run_id), .. } if run_id == "run-pending"
        )));
    }

    /// Catching up an agent that cannot be written to, or whose config cannot
    /// be assembled, logs each message it could not hand over.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_catchup_that_cannot_be_delivered_is_logged(pool: PgPool) {
        let (agent, repo, schedule) = create_agent_repo_schedule(&pool).await;
        sqlx::query!(
            "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
             status) VALUES ($1, $2, $3, now(), now(), 'pending'), ($1, $2, $3, now() - interval \
             '1 hour', now(), 'cancelled')",
            agent.id,
            repo.id,
            schedule.id
        )
        .execute(&pool)
        .await
        .expect("insert pending and cancelled runs");
        // A repository the server has reached before has its host key pinned.
        crate::db::update_repo_ssh_host_key(&pool, repo.id, "ssh-ed25519 AAAATEST")
            .await
            .expect("pin host key");
        let state = build_test_state(pool.clone());

        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(
            send_reconnect_catchup(&state, &mut failing_sink(), &agent.hostname, agent.id, None),
        ))
        .await;
        assert!(logs.contains("ws send failed"), "{logs}");
        assert!(
            logs.contains("failed to notify agent of cancelled backup on reconnect"),
            "{logs}"
        );
        assert!(
            logs.contains("failed to send pending RunBackupNow on reconnect"),
            "{logs}"
        );

        pool.close().await;
        let (mut sink, sent) = recording_sink();
        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(
            send_reconnect_catchup(&state, &mut sink, &agent.hostname, agent.id, None),
        ))
        .await;
        assert!(
            logs.contains("failed to assemble config on connect"),
            "{logs}"
        );
        assert!(sent_messages(&sent).is_empty());
    }

    /// A message that cannot be serialised is not sent.
    #[tokio::test]
    async fn a_message_that_cannot_be_serialised_is_not_sent() {
        // JSON object keys must be strings; serde_json refuses tuple keys.
        let unserialisable = std::collections::HashMap::from([((1, 2), "value")]);
        let (mut sink, sent) = recording_sink();

        assert!(!send_ws_message(&mut sink, &unserialisable).await);
        assert!(sent.lock().unwrap().is_empty());
    }

    /// Tables a test makes the database refuse every write to.
    #[derive(Clone, Copy)]
    enum WriteRejecting {
        BackupReports,
        CanaryResults,
        RepoLastOp,
        ScheduleRepos,
        Schedules,
        SystemEvents,
    }

    /// Makes every insert into `table`, and every update of one of its rows,
    /// fail from now on, as a database refusing the write would.
    async fn reject_writes(pool: &PgPool, table: WriteRejecting) {
        match table {
            WriteRejecting::BackupReports => sqlx::query!(
                "ALTER TABLE backup_reports ADD CONSTRAINT test_rejects_writes CHECK (false) NOT \
                 VALID"
            ),
            WriteRejecting::CanaryResults => sqlx::query!(
                "ALTER TABLE canary_results ADD CONSTRAINT test_rejects_writes CHECK (false) NOT \
                 VALID"
            ),
            WriteRejecting::RepoLastOp => sqlx::query!(
                "ALTER TABLE repo_last_op ADD CONSTRAINT test_rejects_writes CHECK (false) NOT \
                 VALID"
            ),
            WriteRejecting::ScheduleRepos => sqlx::query!(
                "ALTER TABLE schedule_repos ADD CONSTRAINT test_rejects_writes CHECK (false) NOT \
                 VALID"
            ),
            WriteRejecting::Schedules => sqlx::query!(
                "ALTER TABLE schedules ADD CONSTRAINT test_rejects_writes CHECK (false) NOT VALID"
            ),
            WriteRejecting::SystemEvents => sqlx::query!(
                "ALTER TABLE system_events ADD CONSTRAINT test_rejects_writes CHECK (false) NOT \
                 VALID"
            ),
        }
        .execute(pool)
        .await
        .expect("reject writes");
    }

    /// Tables a test hides from every query.
    #[derive(Clone, Copy)]
    enum Hidden {
        Agents,
        NotificationRules,
        RepoRelocationPendingHosts,
        RepoStats,
        ScheduleRepos,
    }

    /// Hides `table` from every query from now on, as a database missing it
    /// would.
    async fn hide_table(pool: &PgPool, table: Hidden) {
        match table {
            Hidden::Agents => sqlx::query!("ALTER TABLE agents RENAME TO agents_hidden"),
            Hidden::NotificationRules => {
                sqlx::query!("ALTER TABLE notification_rules RENAME TO notification_rules_hidden")
            }
            Hidden::RepoRelocationPendingHosts => sqlx::query!(
                "ALTER TABLE repo_relocation_pending_hosts RENAME TO \
                 repo_relocation_pending_hosts_hidden"
            ),
            Hidden::RepoStats => sqlx::query!("ALTER TABLE repo_stats RENAME TO repo_stats_hidden"),
            Hidden::ScheduleRepos => {
                sqlx::query!("ALTER TABLE schedule_repos RENAME TO schedule_repos_hidden")
            }
        }
        .execute(pool)
        .await
        .expect("hide table");
    }

    /// Makes every update of `schedules` quietly leave its row as it was, as a
    /// concurrent writer that got there first would.
    async fn skip_schedule_updates(pool: &PgPool) {
        sqlx::query!(
            "CREATE FUNCTION test_skip_update() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN \
             RETURN NULL; END $$"
        )
        .execute(pool)
        .await
        .expect("create trigger function");
        sqlx::query!(
            "CREATE TRIGGER test_skip_update BEFORE UPDATE ON schedules FOR EACH ROW EXECUTE \
             FUNCTION test_skip_update()"
        )
        .execute(pool)
        .await
        .expect("create trigger");
    }

    /// A schedule auto-disabled while `agent` was unreachable, as the
    /// scheduler leaves it.
    async fn auto_disabled_schedule(
        pool: &PgPool,
        agent: &crate::db::AgentRow,
        repo_id: i64,
    ) -> i64 {
        let schedule = add_schedule(pool, repo_id, agent.id, ScheduleType::Backup, "nightly").await;
        sqlx::query!(
            "UPDATE schedules SET enabled = false, auto_disabled_agent_unreachable = true, \
             auto_disabled_by_agent_id = $2 WHERE id = $1",
            schedule.id,
            agent.id
        )
        .execute(pool)
        .await
        .expect("auto-disable schedule");
        schedule.id
    }

    /// Re-enabling the schedules an agent's outage disabled stops at the
    /// first lookup the database fails, logging it, and reports a re-enable
    /// it could not record; a schedule someone else re-enabled first is left
    /// alone.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn reenabling_on_reconnect_survives_the_database(pool: PgPool) {
        let (agent, repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let schedule_id = auto_disabled_schedule(&pool, &agent, repo.id).await;
        let state = build_test_state(pool.clone());
        let (tx, _rx) = mpsc::channel(1);
        state
            .registry
            .register(agent.id, tx, false, None, None)
            .await;
        let reenable = || {
            crate::test_support::formatted_logs_while(Box::pin(
                reenable_system_disabled_schedules_on_reconnect(&state, agent.id, &agent.hostname),
            ))
        };

        // Someone else got there first: nothing to do, nothing recorded.
        skip_schedule_updates(&pool).await;
        let ((), logs) = reenable().await;
        assert!(!logs.contains("re-enabling schedules"), "{logs}");
        sqlx::query!("DROP TRIGGER test_skip_update ON schedules")
            .execute(&pool)
            .await
            .expect("drop trigger");

        reject_writes(&pool, WriteRejecting::SystemEvents).await;
        let ((), logs) = reenable().await;
        assert!(
            logs.contains("failed to record schedule-reenabled system event"),
            "{logs}"
        );
        let enabled: bool =
            sqlx::query_scalar!("SELECT enabled FROM schedules WHERE id = $1", schedule_id)
                .fetch_one(&pool)
                .await
                .expect("read schedule");
        assert!(enabled);

        auto_disabled_schedule(&pool, &agent, repo.id).await;
        reject_writes(&pool, WriteRejecting::Schedules).await;
        let ((), logs) = reenable().await;
        assert!(
            logs.contains("failed to re-enable auto-disabled schedules on agent reconnect"),
            "{logs}"
        );

        hide_table(&pool, Hidden::Agents).await;
        let ((), logs) = reenable().await;
        assert!(
            logs.contains("failed to look up target agents for auto-disabled schedules"),
            "{logs}"
        );

        pool.close().await;
        let ((), logs) = reenable().await;
        assert!(
            logs.contains("failed to list auto-disabled schedules on agent reconnect"),
            "{logs}"
        );
    }

    /// A message from an agent that cannot be checked against its
    /// repositories, because the database is gone, is refused; a violation
    /// the database will not record is still refused, and logged.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn repository_checks_fail_closed(pool: PgPool) {
        let (agent, repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let rogue = crate::db::insert_agent(&pool, "violation-rogue", None, "hash", None, None)
            .await
            .expect("insert agent");

        reject_writes(&pool, WriteRejecting::SystemEvents).await;
        let (allowed, logs) = crate::test_support::formatted_logs_while(Box::pin(
            validate_agent_repo(&pool, rogue.id, repo.id, &rogue.hostname, "BackupStarted"),
        ))
        .await;
        assert!(!allowed);
        assert!(
            logs.contains("failed to insert security_violation system event"),
            "{logs}"
        );

        pool.close().await;
        let (allowed, logs) = crate::test_support::formatted_logs_while(Box::pin(
            validate_agent_repo(&pool, agent.id, repo.id, &agent.hostname, "BackupStarted"),
        ))
        .await;
        assert!(!allowed);
        assert!(
            logs.contains("DB error checking agent repo access"),
            "{logs}"
        );
    }

    /// A pong the database cannot record is logged.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_pong_the_database_cannot_record_is_logged(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "pong-host", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool.clone());
        pool.close().await;

        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(handle_agent_message(
            &wire(&AgentToServer::Pong),
            &agent.hostname,
            agent.id,
            &state,
        )))
        .await;

        assert!(logs.contains("failed to update last_seen_at"), "{logs}");
    }

    /// A VM build result is handed to whoever asked for it, with the agent's
    /// error if the build failed; a snapshot report logs how many domains it
    /// covers; a manual snapshot nobody waits on is only recorded.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn vm_builds_reports_and_unprompted_stages_are_handled(pool: PgPool) {
        let agent = crate::db::insert_agent(&pool, "vm-build-host", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool.clone());
        let (tx, rx) = tokio::sync::oneshot::channel();
        state
            .pending_vm_builds
            .insert("build-1".to_owned(), agent.id, tx)
            .await;
        let (built_tx, built_rx) = tokio::sync::oneshot::channel();
        state
            .pending_vm_builds
            .insert("build-2".to_owned(), agent.id, built_tx)
            .await;
        let built = shared::vm::VmBuildOutcome {
            name: "web01-restored".into(),
            images: vec!["/var/lib/libvirt/images/web01-restored.qcow2".into()],
            merged_increments: 2,
            defined: true,
            started: false,
        };

        let messages = [
            AgentToServer::VmBuildResult {
                request_id: "build-1".into(),
                outcome: None,
                error: Some("virt-install failed".into()),
            },
            AgentToServer::VmBuildResult {
                request_id: "build-2".into(),
                outcome: Some(built.clone()),
                error: None,
            },
            AgentToServer::VmSnapshotReport {
                schedule_id: None,
                outcomes: vec![snapshot_outcome("web01", None)],
            },
            AgentToServer::VmStageResult {
                request_id: None,
                outcome: snapshot_outcome("db01", None),
            },
        ];
        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(async {
            for msg in &messages {
                handle_agent_message(&wire(msg), &agent.hostname, agent.id, &state).await;
            }
        }))
        .await;

        let (outcome, error) = rx.await.expect("the build is answered");
        assert!(outcome.is_none());
        assert_eq!(error.as_deref(), Some("virt-install failed"));
        assert_eq!(
            built_rx.await.expect("the second build is answered"),
            (Some(built), None)
        );
        assert!(logs.contains("domains=1"), "{logs}");
        let recorded: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM agent_vms WHERE agent_id = $1",
            agent.id
        )
        .fetch_one(&pool)
        .await
        .expect("count VMs");
        assert_eq!(recorded, 2);
    }

    /// A backup start the database will not record is logged, and so is the
    /// clean-up of the run it replaces; the UI still hears the backup start.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_backup_start_the_database_rejects_is_logged(pool: PgPool) {
        let (agent, repo, _schedule) = create_agent_repo_schedule(&pool).await;
        // An earlier run the agent abandoned when it restarted, and the
        // pending row "Run now" left for the run that is starting.
        sqlx::query!(
            "INSERT INTO backup_reports (agent_id, repo_id, started_at, finished_at, status, \
             run_id) VALUES ($1, $2, now() - interval '1 hour', now(), 'started', 'run-orphan'), \
             ($1, $2, now(), now(), 'pending', 'run-new')",
            agent.id,
            repo.id
        )
        .execute(&pool)
        .await
        .expect("insert pending run");
        reject_writes(&pool, WriteRejecting::BackupReports).await;
        let state = build_test_state(pool);
        let mut ui = state.ui_broadcast.subscribe();

        let started = wire(&AgentToServer::BackupStarted {
            repo_id: RepoId(repo.id),
            schedule_id: None,
            started_at: Utc::now(),
            borg_command: Some("borg create ::nightly-1".into()),
            run_id: Some("run-new".into()),
        });
        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(handle_agent_message(
            &started,
            &agent.hostname,
            agent.id,
            &state,
        )))
        .await;

        assert!(
            logs.contains("failed to insert backup started row"),
            "{logs}"
        );
        assert!(
            logs.contains("failed to clean up orphaned backup rows"),
            "{logs}"
        );
        assert!(drain_ui(&mut ui).iter().any(|e| matches!(
            e,
            ServerToUi::BackupStarted { archive_name: Some(name), .. } if name == "nightly-1"
        )));
    }

    /// Indexing a finished backup's archive that cannot be queued is logged.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn indexing_that_cannot_be_queued_is_logged(pool: PgPool) {
        let state = build_test_state(pool.clone());
        pool.close().await;

        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(async {
            spawn_post_backup_indexing(&state, 1, "nightly-1".into());
            assert!(
                state
                    .background_task_tracker
                    .wait_until_idle(Duration::from_secs(10))
                    .await
            );
        }))
        .await;

        assert!(
            logs.contains("failed to queue archive indexing after backup"),
            "{logs}"
        );
    }

    /// A repository quota breach is logged with its level and notified even
    /// when the quota takes no action; with no repository, or a server quota
    /// whose usage cannot be read, the server quota check gives up quietly.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn quota_checks_without_an_action_only_notify(pool: PgPool) {
        let (agent, repo, schedule) = create_agent_repo_schedule(&pool).await;
        crate::db::quota::upsert_quota(
            &pool,
            repo.id,
            Some(10),
            None,
            QuotaAction::NotifyOnly,
            QuotaAction::NotifyOnly,
            true,
        )
        .await
        .expect("set repo quota");
        crate::db::server_quota::upsert_server_quota(
            &pool,
            "host.local",
            Some(10),
            None,
            QuotaAction::NotifyOnly,
            QuotaAction::NotifyOnly,
            true,
        )
        .await
        .expect("set server quota");
        let state = build_test_state(pool.clone());

        // Under both quotas: nothing to report.
        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(async {
            check_repo_quota_after_backup(
                &state,
                &agent.hostname,
                agent.id,
                repo.id,
                Some(schedule.id),
                5,
                &repo.name,
            )
            .await;
            check_server_quota_after_backup(
                &state,
                &agent.hostname,
                agent.id,
                repo.id,
                Some(schedule.id),
                5,
                &repo.name,
            )
            .await;
        }))
        .await;
        assert!(!logs.contains("quota exceeded"), "{logs}");

        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(async {
            check_repo_quota_after_backup(
                &state,
                &agent.hostname,
                agent.id,
                repo.id,
                Some(schedule.id),
                100,
                &repo.name,
            )
            .await;
            check_server_quota_after_backup(
                &state,
                &agent.hostname,
                agent.id,
                repo.id,
                Some(schedule.id),
                100,
                &repo.name,
            )
            .await;
        }))
        .await;
        assert!(logs.contains("repository quota exceeded"), "{logs}");
        assert!(logs.contains("server quota exceeded"), "{logs}");
        assert!(logs.contains("quota_status=\"warning\""), "{logs}");

        // No such repository: no host to look up a quota for.
        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(
            check_server_quota_after_backup(&state, "h", agent.id, -1, None, 100, "gone"),
        ))
        .await;
        assert!(!logs.contains("server quota exceeded"), "{logs}");

        hide_table(&pool, Hidden::RepoStats).await;
        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(
            check_server_quota_after_backup(&state, "h", agent.id, repo.id, None, 100, &repo.name),
        ))
        .await;
        assert!(!logs.contains("server quota exceeded"), "{logs}");
    }

    /// A failed backup of a repository that no longer exists is a plain
    /// failure; one of an away repository whose skip cannot be recorded is
    /// still reported as skipped.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn classifying_a_failure_survives_the_database(pool: PgPool) {
        let (agent, repo, schedule) = absent_repo_fixture(&pool, true).await;

        let gone = classify_failed_backup(&pool, -1, "gone", &agent.hostname, None, None).await;
        assert_eq!(gone.reason, None);

        reject_writes(&pool, WriteRejecting::SystemEvents).await;
        let (away, logs) =
            crate::test_support::formatted_logs_while(Box::pin(classify_failed_backup(
                &pool,
                repo.id,
                &repo.name,
                &agent.hostname,
                Some((schedule.id, schedule.name.as_str())),
                None,
            )))
            .await;
        assert!(away.reason.is_some());
        assert!(
            logs.contains("failed to record backup-skipped-repo-offline system event"),
            "{logs}"
        );
    }

    /// A repository catch-up for a schedule that does not exist is not
    /// recorded; one the database rejects is logged.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn marking_a_catch_up_survives_the_database(pool: PgPool) {
        let (_agent, repo, schedule) = create_agent_repo_schedule(&pool).await;

        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(
            mark_repo_catch_up_pending(&pool, -1, repo.id, &repo.name, None),
        ))
        .await;
        assert!(!logs.contains("will be caught up"), "{logs}");

        reject_writes(&pool, WriteRejecting::ScheduleRepos).await;
        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(
            mark_repo_catch_up_pending(&pool, schedule.id, repo.id, &repo.name, None),
        ))
        .await;
        assert!(
            logs.contains("failed to record a pending repository catch-up"),
            "{logs}"
        );
    }

    /// A reported backup settles its target's pending dependency catch-up;
    /// settling either kind of catch-up when the database is gone is logged.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn settling_catch_ups(pool: PgPool) {
        let (agent, repo, schedule) = create_agent_repo_schedule(&pool).await;
        let dependency_host_id: i64 = sqlx::query_scalar!(
            "INSERT INTO dependency_hosts (name, address, port) VALUES ('nas', 'nas.local', 445) \
             RETURNING id"
        )
        .fetch_one(&pool)
        .await
        .expect("insert dependency host");
        sqlx::query!(
            "INSERT INTO dependency_catch_ups (schedule_id, agent_id, dependency_host_id, \
             pending_for) VALUES ($1, $2, $3, now())",
            schedule.id,
            agent.id,
            dependency_host_id
        )
        .execute(&pool)
        .await
        .expect("insert dependency catch-up");

        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(
            settle_dependency_catch_up(&pool, schedule.id, agent.id),
        ))
        .await;
        assert!(
            logs.contains("settled the target's pending dependency catch-up"),
            "{logs}"
        );

        pool.close().await;
        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(async {
            settle_dependency_catch_up(&pool, schedule.id, agent.id).await;
            clear_repo_catch_up_on_success(&pool, schedule.id, repo.id).await;
        }))
        .await;
        assert!(
            logs.contains("failed to clear a dependency catch-up"),
            "{logs}"
        );
        assert!(
            logs.contains("failed to clear a repository catch-up"),
            "{logs}"
        );
    }

    /// When the notification channels cannot be read, a check result's and a
    /// backup's notifications are dropped and logged.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn notifications_that_cannot_be_dispatched_are_logged(pool: PgPool) {
        let (agent, repo, schedule) = create_agent_repo_schedule(&pool).await;
        let state = build_test_state(pool.clone());
        hide_table(&pool, Hidden::NotificationRules).await;

        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(async {
            handle_agent_message(
                &check_completed(repo.id, true),
                &agent.hostname,
                agent.id,
                &state,
            )
            .await;
            handle_agent_message(
                &backup_report_message(
                    agent.id,
                    repo.id,
                    Some(schedule.id),
                    None,
                    BackupStatus::Success,
                ),
                &agent.hostname,
                agent.id,
                &state,
            )
            .await;
            assert!(
                state
                    .background_task_tracker
                    .wait_until_idle(Duration::from_mins(1))
                    .await
            );
        }))
        .await;

        assert!(
            logs.matches("notification dispatch failed").count() >= 2,
            "{logs}"
        );
    }

    /// A finished backup the database will not record is logged at every
    /// step that needs it - the report, the repository's last operation, the
    /// host's relocation flag - and the UI still hears it finish.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_finished_backup_the_database_rejects_is_logged(pool: PgPool) {
        let (agent, repo, schedule) = create_agent_repo_schedule(&pool).await;
        reject_writes(&pool, WriteRejecting::BackupReports).await;
        reject_writes(&pool, WriteRejecting::RepoLastOp).await;
        hide_table(&pool, Hidden::RepoRelocationPendingHosts).await;
        let state = build_test_state(pool);
        let mut ui = state.ui_broadcast.subscribe();

        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(async {
            handle_agent_message(
                &backup_report_message(
                    agent.id,
                    repo.id,
                    Some(schedule.id),
                    None,
                    BackupStatus::Success,
                ),
                &agent.hostname,
                agent.id,
                &state,
            )
            .await;
            state
                .background_task_tracker
                .wait_until_idle(Duration::from_mins(1))
                .await;
        }))
        .await;

        assert!(logs.contains("failed to persist backup report"), "{logs}");
        assert!(
            logs.contains("failed to persist last_op for backup"),
            "{logs}"
        );
        assert!(
            logs.contains("failed to clear relocation_pending for host"),
            "{logs}"
        );
        assert!(
            drain_ui(&mut ui)
                .iter()
                .any(|e| matches!(e, ServerToUi::BackupCompleted { .. }))
        );
    }

    /// A finished backup reported by an agent that does not back up the
    /// repository is dropped: no report is stored.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_finished_backup_from_a_rogue_agent_is_dropped(pool: PgPool) {
        let (_agent, repo, schedule) = create_agent_repo_schedule(&pool).await;
        let rogue = crate::db::insert_agent(&pool, "completed-rogue", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool.clone());

        handle_agent_message(
            &backup_report_message(
                rogue.id,
                repo.id,
                Some(schedule.id),
                None,
                BackupStatus::Success,
            ),
            &rogue.hostname,
            rogue.id,
            &state,
        )
        .await;

        let reports: i64 = sqlx::query_scalar!("SELECT COUNT(*) AS \"count!\" FROM backup_reports")
            .fetch_one(&pool)
            .await
            .expect("count reports");
        assert_eq!(reports, 0);
    }

    /// A backup the agent refused to start settles whoever waits on it as a
    /// failure.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_rejected_backup_settles_as_a_failure(pool: PgPool) {
        let (agent, repo, _schedule) = create_agent_repo_schedule(&pool).await;
        let state = build_test_state(pool);
        let mut outcomes = state.completion_bus.subscribe();

        let rejected = wire(&AgentToServer::BackupRejected {
            repo_id: RepoId(repo.id),
            reason: "another backup is running".into(),
        });
        handle_agent_message(&rejected, &agent.hostname, agent.id, &state).await;

        let outcome = outcomes.try_recv().expect("the rejection is published");
        assert_eq!((outcome.repo_id, outcome.success), (repo.id, false));
    }

    fn canary_verified(repo_id: i64, archive_name: &str) -> String {
        wire(&AgentToServer::CanaryVerified {
            repo_id: RepoId(repo_id),
            success: true,
            nonce: "nonce-1".into(),
            archive_name: archive_name.into(),
            error_message: None,
        })
    }

    /// A canary result is stored against the repository's backup schedule,
    /// with no archive when the agent named none; with no backup schedule, a
    /// schedule lookup the database fails, or a store the database rejects,
    /// it is logged and still pushed to the UI. One from another agent is
    /// dropped.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn canary_results_are_stored_or_logged(pool: PgPool) {
        let (agent, repo, schedule) = create_agent_repo_schedule(&pool).await;
        let rogue = crate::db::insert_agent(&pool, "canary-rogue", None, "hash", None, None)
            .await
            .expect("insert agent");
        let state = build_test_state(pool.clone());
        let mut ui = state.ui_broadcast.subscribe();
        let deliver = |msg: String, hostname: String, agent_id: i64| {
            let state = state.clone();
            crate::test_support::formatted_logs_while(Box::pin(async move {
                handle_agent_message(&msg, &hostname, agent_id, &state).await;
            }))
        };

        deliver(
            canary_verified(repo.id, ""),
            rogue.hostname.clone(),
            rogue.id,
        )
        .await;
        assert!(drain_ui(&mut ui).is_empty());

        deliver(
            canary_verified(repo.id, ""),
            agent.hostname.clone(),
            agent.id,
        )
        .await;
        let archive: Option<String> = sqlx::query_scalar!(
            "SELECT archive_name FROM canary_results WHERE schedule_id = $1",
            schedule.id
        )
        .fetch_one(&pool)
        .await
        .expect("read canary result");
        assert_eq!(archive, None);

        reject_writes(&pool, WriteRejecting::CanaryResults).await;
        let ((), logs) = deliver(
            canary_verified(repo.id, "nightly-1"),
            agent.hostname.clone(),
            agent.id,
        )
        .await;
        assert!(logs.contains("failed to insert canary result"), "{logs}");

        hide_table(&pool, Hidden::ScheduleRepos).await;
        let ((), logs) = deliver(
            canary_verified(repo.id, "nightly-1"),
            agent.hostname.clone(),
            agent.id,
        )
        .await;
        assert!(
            logs.contains("failed to look up schedule for canary result"),
            "{logs}"
        );

        sqlx::query!("ALTER TABLE schedule_repos_hidden RENAME TO schedule_repos")
            .execute(&pool)
            .await
            .expect("restore table");
        sqlx::query!(
            "UPDATE schedules SET schedule_type = 'check' WHERE id = $1",
            schedule.id
        )
        .execute(&pool)
        .await
        .expect("make it a check schedule");
        let ((), logs) = deliver(
            canary_verified(repo.id, "nightly-1"),
            agent.hostname.clone(),
            agent.id,
        )
        .await;
        assert!(
            logs.contains("no backup schedule found for canary result"),
            "{logs}"
        );
        assert!(
            drain_ui(&mut ui)
                .iter()
                .any(|e| matches!(e, ServerToUi::CanaryVerified { .. }))
        );
    }

    /// A cancellation the database will not record is logged, and the
    /// backup still settles as a failure.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_cancellation_the_database_rejects_is_logged(pool: PgPool) {
        let (agent, repo, schedule) = create_agent_repo_schedule(&pool).await;
        sqlx::query!(
            "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
             status) VALUES ($1, $2, $3, now(), now(), 'started'), ($1, $2, $3, now() - interval \
             '1 hour', now(), 'cancelled')",
            agent.id,
            repo.id,
            schedule.id
        )
        .execute(&pool)
        .await
        .expect("insert started and cancelled runs");
        reject_writes(&pool, WriteRejecting::BackupReports).await;
        let state = build_test_state(pool);
        let mut outcomes = state.completion_bus.subscribe();

        let cancelled = wire(&AgentToServer::BackupCancelled {
            repo_id: RepoId(repo.id),
        });
        let ((), logs) = crate::test_support::formatted_logs_while(Box::pin(handle_agent_message(
            &cancelled,
            &agent.hostname,
            agent.id,
            &state,
        )))
        .await;

        assert!(
            logs.contains("failed to update cancelled backup report"),
            "{logs}"
        );
        assert!(
            logs.contains("failed to acknowledge cancellation"),
            "{logs}"
        );
        assert!(
            !outcomes
                .try_recv()
                .expect("the cancellation is published")
                .success
        );
        state.shutdown_token.cancel();
        state
            .background_task_tracker
            .wait_until_idle(Duration::from_secs(10))
            .await;
    }
}
