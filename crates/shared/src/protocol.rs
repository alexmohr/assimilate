// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

use crate::types::{
    AgentConfig, AgentStatus, BackupReport, BorgEncryption, DryRunFile, RepoId, SearchEntry,
};

/// The kind of repository operation being performed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RepoOpKind {
    /// Backup operation initiated by the agent.
    AgentBackup,
    /// Check operation initiated by the agent.
    AgentCheck,
    /// Verify operation initiated by the agent.
    AgentVerify,
    /// Sync operation initiated by the server.
    ServerSync,
    /// Break a lock on the repository.
    BreakLock,
    /// Delete one or more archives from the repository.
    DeleteArchive,
}

impl FromStr for RepoOpKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(serde_json::Value::String(s.to_owned()))
            .map_err(|e| format!("invalid RepoOpKind: {e}"))
    }
}

/// An active operation on a repository, including its queued depth.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema, TS)]
#[ts(export)]
pub struct ActiveRepoOp {
    /// The kind of operation being performed.
    pub kind: RepoOpKind,
    /// The name of the actor (agent hostname) performing the operation.
    pub actor: String,
    /// Timestamp when the operation started.
    pub started_at: DateTime<Utc>,
    /// Number of further operations waiting behind this one for the repository.
    #[serde(default)]
    pub queued: u32,
}

/// Messages sent from the server to an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum ServerToAgent {
    /// Update the agent's configuration.
    ConfigUpdate(AgentConfig),
    /// Instruct the agent to run a backup immediately.
    RunBackupNow {
        /// The repository to back up.
        repo_id: RepoId,
        /// The schedule that triggered this backup, if any.
        #[serde(default)]
        schedule_id: Option<i64>,
        /// Optional opaque request identifier for correlating responses.
        #[serde(default)]
        request_id: Option<String>,
        /// The run identifier for this backup execution.
        #[serde(default)]
        run_id: Option<String>,
    },
    /// Instruct the agent to run a repository check immediately.
    RunCheckNow {
        /// The repository to check.
        repo_id: RepoId,
        /// Optional opaque request identifier for correlating responses.
        #[serde(default)]
        request_id: Option<String>,
    },
    /// Instruct the agent to verify repository data immediately.
    RunVerifyNow {
        /// The repository to verify.
        repo_id: RepoId,
        /// Optional opaque request identifier for correlating responses.
        #[serde(default)]
        request_id: Option<String>,
    },
    /// Instruct the agent to initialize a new borg repository.
    InitRepo {
        /// Remote path for the repository.
        repo_path: String,
        /// SSH username for the repository host.
        ssh_user: String,
        /// SSH hostname for the repository host.
        ssh_host: String,
        /// SSH port for the repository host.
        ssh_port: u16,
        /// Passphrase for repository encryption.
        passphrase: String,
        /// Encryption mode to use for the new repository.
        encryption: BorgEncryption,
        /// Optional opaque request identifier for correlating responses.
        #[serde(default)]
        request_id: Option<String>,
    },
    /// Instruct the agent to search archives on a repository.
    SearchArchive {
        /// Request identifier for correlating results.
        request_id: String,
        /// The repository to search.
        repo_id: RepoId,
        /// Optional archive name filter.
        #[serde(default)]
        archive_name: Option<String>,
        /// Glob pattern to match files against.
        pattern: String,
        /// Maximum number of archives to search.
        #[serde(default)]
        max_archives: Option<u32>,
    },
    /// Instruct the agent to restore files from an archive.
    RestoreFiles {
        /// Request identifier for correlating results.
        request_id: String,
        /// The repository containing the archive.
        repo_id: RepoId,
        /// Name of the archive to restore from.
        archive_name: String,
        /// Paths within the archive to restore.
        paths: Vec<String>,
        /// Local target path for restored files.
        target_path: String,
    },
    /// Instruct the agent to perform a dry-run of a backup schedule.
    DryRun {
        /// Request identifier for correlating results.
        request_id: String,
        /// The repository for the dry run.
        repo_id: RepoId,
        /// The schedule to simulate.
        schedule_id: i64,
    },
    /// Instruct the agent to export an archive from a repository.
    ExportArchive {
        /// Request identifier for correlating results.
        request_id: String,
        /// The repository containing the archive.
        repo_id: RepoId,
        /// Name of the archive to export.
        archive_name: String,
    },
    /// Instruct the agent to export a repository's encryption key.
    KeyExport {
        /// Request identifier for correlating results.
        request_id: String,
        /// The repository to export the key from.
        repo_id: RepoId,
    },
    /// Instruct the agent to import a repository's encryption key.
    KeyImport {
        /// Request identifier for correlating results.
        request_id: String,
        /// The repository to import the key for.
        repo_id: RepoId,
        /// The key data to import.
        key_data: String,
    },
    /// Instruct the agent to change a repository's passphrase.
    ChangePassphrase {
        /// Request identifier for correlating results.
        request_id: String,
        /// The repository to change the passphrase for.
        repo_id: RepoId,
        /// The new passphrase.
        new_passphrase: String,
    },
    /// Instruct the agent to restart (reconnect with updated config).
    RestartAgent,
    /// Instruct the agent to delete archives from a repository.
    DeleteArchives {
        /// Request identifier for correlating results.
        request_id: String,
        /// The repository containing the archives.
        repo_id: RepoId,
        /// Names of the archives to delete.
        archive_names: Vec<String>,
    },
    /// Instruct the agent to cancel a running backup.
    CancelBackup {
        /// The repository whose backup should be cancelled.
        repo_id: RepoId,
    },
    /// Heartbeat ping to check agent connectivity.
    Ping,
    /// Notification that the server is shutting down.
    ShuttingDown,
}

/// Messages sent from an agent to the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum AgentToServer {
    /// Initial handshake sent by the agent upon connecting.
    Hello {
        /// Hostname of the agent machine.
        hostname: String,
        /// Authentication token for the agent.
        token: String,
        /// Version string of the agent binary.
        agent_version: String,
        /// Git SHA of the agent build, if available.
        #[serde(default)]
        agent_git_sha: Option<String>,
        /// Build timestamp, if available.
        #[serde(default)]
        agent_build_time: Option<String>,
        /// Number of commits at build time, if available.
        #[serde(default)]
        agent_commit_count: Option<u32>,
        /// Whether this agent version supports graceful restart.
        #[serde(default)]
        supports_restart: bool,
        /// Reason restart is unavailable, if applicable.
        #[serde(default)]
        restart_unavailable_reason: Option<String>,
    },
    /// Notification that a backup has started.
    BackupStarted {
        /// The repository being backed up.
        repo_id: RepoId,
        /// The schedule that triggered this backup, if any.
        #[serde(default)]
        schedule_id: Option<i64>,
        /// Timestamp when the backup started.
        started_at: DateTime<Utc>,
        /// The borg command being executed.
        #[serde(default)]
        borg_command: Option<String>,
        /// The run identifier for this backup.
        #[serde(default)]
        run_id: Option<String>,
    },
    /// Report that a backup completed.
    BackupCompleted {
        /// The backup result report.
        report: BackupReport,
    },
    /// Report that a backup was rejected (e.g., repo locked).
    BackupRejected {
        /// The repository that rejected the backup.
        repo_id: RepoId,
        /// Human-readable reason for rejection.
        reason: String,
    },
    /// Report that a repository check completed.
    CheckCompleted {
        /// The repository that was checked.
        repo_id: RepoId,
        /// Whether the check succeeded.
        success: bool,
        /// Duration of the check in seconds.
        duration_secs: i64,
        /// Error message if the check failed.
        error_message: Option<String>,
    },
    /// Report that a repository verification completed.
    VerifyCompleted {
        /// The repository that was verified.
        repo_id: RepoId,
        /// Whether the verification succeeded.
        success: bool,
        /// Duration of the verification in seconds.
        duration_secs: i64,
        /// Error message if verification failed.
        error_message: Option<String>,
        /// Number of files verified.
        files_verified: i64,
    },
    /// Report that a canary archive was verified.
    CanaryVerified {
        /// The repository where the canary was verified.
        repo_id: RepoId,
        /// Whether the canary verification succeeded.
        success: bool,
        /// The canary nonce that was verified.
        nonce: String,
        /// Name of the archive containing the canary.
        archive_name: String,
        /// Error message if verification failed.
        error_message: Option<String>,
    },
    /// Report that a repository initialization completed.
    InitRepoCompleted {
        /// Path of the initialized repository.
        repo_path: String,
        /// Whether initialization succeeded.
        success: bool,
        /// Error message if initialization failed.
        error_message: Option<String>,
    },
    /// Status update for a repository.
    StatusUpdate {
        /// The repository whose status changed.
        repo_id: RepoId,
        /// The new agent status.
        status: AgentStatus,
    },
    /// Report that the agent failed to restart.
    RestartFailed {
        /// Error message describing the failure.
        error_message: String,
    },
    /// Results of an archive search.
    SearchResult {
        /// Request identifier from the original search request.
        request_id: String,
        /// Matching entries found.
        entries: Vec<SearchEntry>,
        /// Whether this is the final batch of results.
        done: bool,
    },
    /// Report that a file restore operation completed.
    RestoreCompleted {
        /// Request identifier from the original restore request.
        request_id: String,
        /// Whether the restore succeeded.
        success: bool,
        /// Number of files restored.
        files_restored: u64,
        /// Error message if the restore failed.
        error_message: Option<String>,
    },
    /// Results of a dry-run backup simulation.
    DryRunResult {
        /// Request identifier from the original dry-run request.
        request_id: String,
        /// Files that would be included in the backup.
        files: Vec<DryRunFile>,
        /// Total size of files in bytes.
        total_size: i64,
        /// Error message if the dry run failed.
        error_message: Option<String>,
    },
    /// Notification that an archive export is ready for download.
    ExportReady {
        /// Request identifier from the original export request.
        request_id: String,
        /// Whether the export succeeded.
        success: bool,
        /// Error message if the export failed.
        error_message: Option<String>,
    },
    /// Results of a key export operation.
    KeyExportResult {
        /// Request identifier from the original key export request.
        request_id: String,
        /// The exported key data.
        key_data: String,
        /// Error message if the export failed.
        error_message: Option<String>,
    },
    /// Results of a key import operation.
    KeyImportResult {
        /// Request identifier from the original key import request.
        request_id: String,
        /// Whether the import succeeded.
        success: bool,
        /// Error message if the import failed.
        error_message: Option<String>,
    },
    /// Confirmation that a passphrase was changed.
    PassphraseChanged {
        /// Request identifier from the original change passphrase request.
        request_id: String,
        /// Whether the change succeeded.
        success: bool,
        /// Error message if the change failed.
        error_message: Option<String>,
    },
    /// Progress update for a long-running operation.
    OperationProgress {
        /// Request identifier for the operation.
        request_id: String,
        /// Progress percentage (0-100).
        percent: u8,
        /// Human-readable progress message.
        message: String,
    },
    /// Notification that a long-running operation failed.
    OperationFailed {
        /// Request identifier for the operation.
        request_id: String,
        /// Error message describing the failure.
        error: String,
    },
    /// Report that an encryption migration completed.
    MigrateEncryptionCompleted {
        /// Request identifier from the original migration request.
        request_id: String,
        /// Whether the migration succeeded.
        success: bool,
        /// Error message if the migration failed.
        error_message: Option<String>,
    },
    /// Results of an archive deletion operation.
    DeleteArchivesResult {
        /// Request identifier from the original deletion request.
        request_id: String,
        /// Whether the deletion succeeded.
        success: bool,
        /// Number of archives deleted.
        deleted_count: u32,
        /// Error message if the deletion failed.
        error_message: Option<String>,
    },
    /// Notification that a backup was cancelled.
    BackupCancelled {
        /// The repository whose backup was cancelled.
        repo_id: RepoId,
    },
    /// Log line emitted during a backup operation.
    BackupLog {
        /// The repository the log line relates to.
        repo_id: RepoId,
        /// The schedule that triggered the backup, if any.
        #[serde(default)]
        schedule_id: Option<i64>,
        /// The log line content.
        line: String,
    },
    /// Response to a server ping.
    Pong,
}

/// The connection status of a reverse SSH tunnel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum TunnelStatus {
    /// The tunnel is connected and operational.
    Connected,
    /// The tunnel is disconnected.
    Disconnected,
    /// The tunnel is attempting to reconnect.
    Reconnecting,
    /// The tunnel encountered an error.
    Error {
        /// Description of the error.
        message: String,
    },
}

/// Messages sent from the server to the UI (web client) via WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "payload")]
#[ts(export)]
pub enum ServerToUi {
    /// Notification that an agent has connected.
    AgentConnected {
        /// The hostname of the agent that connected.
        hostname: String,
    },
    /// Notification that an agent has disconnected.
    AgentDisconnected {
        /// The hostname of the agent that disconnected.
        hostname: String,
    },
    /// Notification that a backup has started on an agent.
    BackupStarted {
        /// The hostname of the agent running the backup.
        hostname: String,
        /// The name of the backup target (repository).
        target_name: String,
        /// The archive name being created, if known.
        #[serde(default)]
        archive_name: Option<String>,
        /// The schedule that triggered this backup, if any.
        #[serde(default)]
        #[ts(type = "number | null")]
        schedule_id: Option<i64>,
    },
    /// Notification that a backup has completed.
    BackupCompleted {
        /// The hostname of the agent that ran the backup.
        hostname: String,
        /// The name of the backup target (repository).
        target_name: String,
        /// The backup result report.
        report: Box<BackupReport>,
    },
    /// Notification that a repository check has completed.
    CheckCompleted {
        /// The hostname of the agent that ran the check.
        hostname: String,
        /// The name of the target that was checked.
        target_name: String,
        /// Whether the check succeeded.
        success: bool,
        /// Error message if the check failed.
        error_message: Option<String>,
    },
    /// Notification that a repository verification has completed.
    VerifyCompleted {
        /// The hostname of the agent that ran the verification.
        hostname: String,
        /// The name of the target that was verified.
        target_name: String,
        /// Whether the verification succeeded.
        success: bool,
        /// Error message if verification failed.
        error_message: Option<String>,
    },
    /// Notification that a canary archive has been verified.
    CanaryVerified {
        /// The hostname of the agent that verified the canary.
        hostname: String,
        /// The name of the target where the canary resides.
        target_name: String,
        /// Whether the canary verification succeeded.
        success: bool,
        /// Error message if verification failed.
        error_message: Option<String>,
    },
    /// Notification that an agent's configuration has been updated.
    ConfigUpdated {
        /// The hostname of the agent whose config was updated.
        hostname: String,
    },
    /// Generic signal that data has changed and the UI should refresh.
    DataChanged,
    /// Progress update for an import operation.
    ImportProgress {
        /// The repository being imported.
        #[ts(type = "number")]
        repo_id: i64,
        /// Current progress count.
        progress: i32,
        /// Total items to import.
        total: i32,
        /// Optional human-readable progress message.
        message: Option<String>,
    },
    /// Notification that an SSH tunnel's status changed.
    TunnelStatusChanged {
        /// The agent ID whose tunnel status changed.
        #[ts(type = "number")]
        agent_id: i64,
        /// The hostname of the agent.
        hostname: String,
        /// The new tunnel status.
        status: TunnelStatus,
    },
    /// Notification that a repository operation has changed.
    RepoOpChanged {
        /// The repository whose operation changed.
        #[ts(type = "number")]
        repo_id: i64,
        /// The current operation, or `None` if no operation is active.
        op: Option<ActiveRepoOp>,
    },
    /// Log line emitted during a backup.
    BackupLog {
        /// The hostname of the agent generating the log.
        hostname: String,
        /// The schedule that triggered the backup, if any.
        #[serde(default)]
        #[ts(type = "number | null")]
        schedule_id: Option<i64>,
        /// The repository the log line relates to.
        #[ts(type = "number")]
        repo_id: i64,
        /// The log line content.
        line: String,
    },
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::types::{DryRunFile, SearchEntry};

    #[test]
    fn tunnel_status_connected_round_trips_json() {
        let status = TunnelStatus::Connected;
        let json = serde_json::to_string(&status).unwrap();
        let status2: TunnelStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, status2);
    }

    #[test]
    fn tunnel_status_error_round_trips_json() {
        let status = TunnelStatus::Error {
            message: String::from("AllowTcpForwarding disabled"),
        };
        let json = serde_json::to_string(&status).unwrap();
        let status2: TunnelStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, status2);
    }

    #[test]
    fn server_to_agent_search_archive_round_trips() {
        let msg = ServerToAgent::SearchArchive {
            request_id: "req-1".into(),
            repo_id: RepoId(1),
            archive_name: Some("archive-2024".into()),
            pattern: "*.log".into(),
            max_archives: Some(5),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: ServerToAgent = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn server_to_agent_restore_files_round_trips() {
        let msg = ServerToAgent::RestoreFiles {
            request_id: "req-2".into(),
            repo_id: RepoId(3),
            archive_name: "backup-2024-01-01".into(),
            paths: vec!["/etc/hosts".into(), "/var/log".into()],
            target_path: "/tmp/restore".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: ServerToAgent = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn server_to_agent_dry_run_round_trips() {
        let msg = ServerToAgent::DryRun {
            request_id: "req-3".into(),
            repo_id: RepoId(2),
            schedule_id: 42,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: ServerToAgent = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn server_to_agent_export_archive_round_trips() {
        let msg = ServerToAgent::ExportArchive {
            request_id: "req-4".into(),
            repo_id: RepoId(1),
            archive_name: "daily-2024-01-01".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: ServerToAgent = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn server_to_agent_key_export_round_trips() {
        let msg = ServerToAgent::KeyExport {
            request_id: "req-5".into(),
            repo_id: RepoId(7),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: ServerToAgent = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn server_to_agent_key_import_round_trips() {
        let msg = ServerToAgent::KeyImport {
            request_id: "req-6".into(),
            repo_id: RepoId(7),
            key_data: "BORG_KEY abc123".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: ServerToAgent = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn server_to_agent_change_passphrase_round_trips() {
        let msg = ServerToAgent::ChangePassphrase {
            request_id: "req-7".into(),
            repo_id: RepoId(2),
            new_passphrase: "new-secret".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: ServerToAgent = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn server_to_agent_run_backup_now_with_request_id_round_trips() {
        let msg = ServerToAgent::RunBackupNow {
            repo_id: RepoId(1),
            schedule_id: Some(42),
            request_id: Some("req-8".into()),
            run_id: Some("run-abc".into()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: ServerToAgent = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn server_to_agent_run_backup_now_without_request_id_backward_compat() {
        let json = r#"{"type":"RunBackupNow","payload":{"repo_id":1}}"#;
        let msg: ServerToAgent = serde_json::from_str(json).unwrap();
        let json2 = serde_json::to_string(&msg).unwrap();
        let msg2: ServerToAgent = serde_json::from_str(&json2).unwrap();
        let json3 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json2, json3);
    }

    #[test]
    fn server_to_agent_run_backup_now_with_schedule_id_round_trips() {
        let msg = ServerToAgent::RunBackupNow {
            repo_id: RepoId(1),
            schedule_id: Some(7),
            request_id: None,
            run_id: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: ServerToAgent = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn server_to_agent_run_backup_now_without_schedule_id_backward_compat() {
        let json = r#"{"type":"RunBackupNow","payload":{"repo_id":5}}"#;
        let msg: ServerToAgent = serde_json::from_str(json).unwrap();
        match msg {
            ServerToAgent::RunBackupNow {
                repo_id,
                schedule_id,
                ..
            } => {
                assert_eq!(repo_id.0, 5);
                assert!(schedule_id.is_none());
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn agent_to_server_search_result_round_trips() {
        let msg = AgentToServer::SearchResult {
            request_id: "req-1".into(),
            entries: vec![SearchEntry {
                path: "/etc/hosts".into(),
                size: 1024,
                mtime: Utc::now(),
                entry_type: "file".into(),
                archive_name: Some("archive-1".into()),
            }],
            done: true,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: AgentToServer = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn agent_to_server_restore_completed_round_trips() {
        let msg = AgentToServer::RestoreCompleted {
            request_id: "req-2".into(),
            success: true,
            files_restored: 42,
            error_message: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: AgentToServer = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn agent_to_server_dry_run_result_round_trips() {
        let msg = AgentToServer::DryRunResult {
            request_id: "req-3".into(),
            files: vec![DryRunFile {
                path: "/var/log/syslog".into(),
                size: 2048,
            }],
            total_size: 2048,
            error_message: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: AgentToServer = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn agent_to_server_export_ready_round_trips() {
        let msg = AgentToServer::ExportReady {
            request_id: "req-4".into(),
            success: true,
            error_message: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: AgentToServer = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn agent_to_server_key_export_result_round_trips() {
        let msg = AgentToServer::KeyExportResult {
            request_id: "req-5".into(),
            key_data: "BORG_KEY abc".into(),
            error_message: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: AgentToServer = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn agent_to_server_key_import_result_round_trips() {
        let msg = AgentToServer::KeyImportResult {
            request_id: "req-6".into(),
            success: true,
            error_message: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: AgentToServer = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn agent_to_server_passphrase_changed_round_trips() {
        let msg = AgentToServer::PassphraseChanged {
            request_id: "req-7".into(),
            success: true,
            error_message: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: AgentToServer = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn agent_to_server_operation_progress_round_trips() {
        let msg = AgentToServer::OperationProgress {
            request_id: "req-8".into(),
            percent: 50,
            message: "Extracting files...".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: AgentToServer = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn agent_to_server_operation_failed_round_trips() {
        let msg = AgentToServer::OperationFailed {
            request_id: "req-9".into(),
            error: "Repository locked".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: AgentToServer = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn server_to_agent_delete_archives_round_trips() {
        let msg = ServerToAgent::DeleteArchives {
            request_id: "req-del-1".into(),
            repo_id: RepoId(5),
            archive_names: vec!["daily-2026-01-01".into(), "daily-2026-01-02".into()],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: ServerToAgent = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn agent_to_server_delete_archives_result_round_trips() {
        let msg = AgentToServer::DeleteArchivesResult {
            request_id: "req-del-1".into(),
            success: true,
            deleted_count: 2,
            error_message: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: AgentToServer = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn agent_to_server_migrate_encryption_completed_round_trips() {
        let msg = AgentToServer::MigrateEncryptionCompleted {
            request_id: "req-mig-1".into(),
            success: false,
            error_message: Some("key mismatch".into()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: AgentToServer = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn agent_to_server_hello_with_new_fields_round_trips() {
        let msg = AgentToServer::Hello {
            hostname: "web-01".into(),
            token: "abc".into(),
            agent_version: "1.0.0".into(),
            agent_git_sha: Some("deadbeef".into()),
            agent_build_time: Some("2026-01-01".into()),
            agent_commit_count: Some(42),
            supports_restart: true,
            restart_unavailable_reason: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: AgentToServer = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn server_to_agent_cancel_backup_round_trips() {
        let msg = ServerToAgent::CancelBackup {
            repo_id: RepoId(42),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: ServerToAgent = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn agent_to_server_backup_cancelled_round_trips() {
        let msg = AgentToServer::BackupCancelled {
            repo_id: RepoId(42),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: AgentToServer = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn agent_to_server_backup_log_round_trips() {
        let msg = AgentToServer::BackupLog {
            repo_id: RepoId(7),
            schedule_id: Some(3),
            line: r#"{"type":"log_message","levelname":"INFO","message":"Creating archive"}"#
                .to_owned(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: AgentToServer = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn server_to_ui_backup_log_round_trips() {
        let msg = ServerToUi::BackupLog {
            hostname: "web-01".to_owned(),
            schedule_id: Some(3),
            repo_id: 7,
            line: r#"{"type":"log_message","levelname":"WARNING","message":"File changed"}"#
                .to_owned(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: ServerToUi = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }
}
