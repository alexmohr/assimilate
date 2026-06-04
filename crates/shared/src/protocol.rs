// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::{
    AgentConfig, AgentStatus, BackupReport, BorgEncryption, DryRunFile, RepoId, SearchEntry,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum ServerToAgent {
    ConfigUpdate(AgentConfig),
    RunBackupNow {
        repo_id: RepoId,
        #[serde(default)]
        schedule_id: Option<i64>,
        #[serde(default)]
        request_id: Option<String>,
    },
    RunCheckNow {
        repo_id: RepoId,
        #[serde(default)]
        request_id: Option<String>,
    },
    RunVerifyNow {
        repo_id: RepoId,
        #[serde(default)]
        request_id: Option<String>,
    },
    InitRepo {
        repo_path: String,
        ssh_user: String,
        ssh_host: String,
        ssh_port: u16,
        passphrase: String,
        encryption: BorgEncryption,
        #[serde(default)]
        request_id: Option<String>,
    },
    SearchArchive {
        request_id: String,
        repo_id: RepoId,
        #[serde(default)]
        archive_name: Option<String>,
        pattern: String,
        #[serde(default)]
        max_archives: Option<u32>,
    },
    RestoreFiles {
        request_id: String,
        repo_id: RepoId,
        archive_name: String,
        paths: Vec<String>,
        target_path: String,
    },
    DryRun {
        request_id: String,
        repo_id: RepoId,
        schedule_id: i64,
    },
    ExportArchive {
        request_id: String,
        repo_id: RepoId,
        archive_name: String,
    },
    KeyExport {
        request_id: String,
        repo_id: RepoId,
    },
    KeyImport {
        request_id: String,
        repo_id: RepoId,
        key_data: String,
    },
    ChangePassphrase {
        request_id: String,
        repo_id: RepoId,
        new_passphrase: String,
    },
    RestartAgent,
    DeleteArchives {
        request_id: String,
        repo_id: RepoId,
        archive_names: Vec<String>,
    },
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum AgentToServer {
    Hello {
        hostname: String,
        token: String,
        agent_version: String,
        #[serde(default)]
        agent_git_sha: Option<String>,
        #[serde(default)]
        agent_build_time: Option<String>,
        #[serde(default)]
        supports_restart: bool,
        #[serde(default)]
        restart_unavailable_reason: Option<String>,
    },
    BackupStarted {
        repo_id: RepoId,
        started_at: DateTime<Utc>,
        #[serde(default)]
        borg_command: Option<String>,
    },
    BackupCompleted {
        report: BackupReport,
    },
    BackupRejected {
        repo_id: RepoId,
        reason: String,
    },
    CheckCompleted {
        repo_id: RepoId,
        success: bool,
        duration_secs: i64,
        error_message: Option<String>,
    },
    VerifyCompleted {
        repo_id: RepoId,
        success: bool,
        duration_secs: i64,
        error_message: Option<String>,
        files_verified: i64,
    },
    CanaryVerified {
        repo_id: RepoId,
        success: bool,
        nonce: String,
        archive_name: String,
        error_message: Option<String>,
    },
    InitRepoCompleted {
        repo_path: String,
        success: bool,
        error_message: Option<String>,
    },
    StatusUpdate {
        repo_id: RepoId,
        status: AgentStatus,
    },
    RestartFailed {
        error_message: String,
    },
    SearchResult {
        request_id: String,
        entries: Vec<SearchEntry>,
        done: bool,
    },
    RestoreCompleted {
        request_id: String,
        success: bool,
        files_restored: u64,
        error_message: Option<String>,
    },
    DryRunResult {
        request_id: String,
        files: Vec<DryRunFile>,
        total_size: i64,
        error_message: Option<String>,
    },
    ExportReady {
        request_id: String,
        success: bool,
        error_message: Option<String>,
    },
    KeyExportResult {
        request_id: String,
        key_data: String,
        error_message: Option<String>,
    },
    KeyImportResult {
        request_id: String,
        success: bool,
        error_message: Option<String>,
    },
    PassphraseChanged {
        request_id: String,
        success: bool,
        error_message: Option<String>,
    },
    OperationProgress {
        request_id: String,
        percent: u8,
        message: String,
    },
    OperationFailed {
        request_id: String,
        error: String,
    },
    MigrateEncryptionCompleted {
        request_id: String,
        success: bool,
        error_message: Option<String>,
    },
    DeleteArchivesResult {
        request_id: String,
        success: bool,
        deleted_count: u32,
        error_message: Option<String>,
    },
    Pong,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TunnelStatus {
    Connected,
    Disconnected,
    Reconnecting,
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum ServerToUi {
    AgentConnected {
        hostname: String,
    },
    AgentDisconnected {
        hostname: String,
    },
    BackupStarted {
        hostname: String,
        target_name: String,
    },
    BackupCompleted {
        hostname: String,
        target_name: String,
        report: BackupReport,
    },
    CheckCompleted {
        hostname: String,
        target_name: String,
        success: bool,
        error_message: Option<String>,
    },
    VerifyCompleted {
        hostname: String,
        target_name: String,
        success: bool,
        error_message: Option<String>,
    },
    CanaryVerified {
        hostname: String,
        target_name: String,
        success: bool,
        error_message: Option<String>,
    },
    ConfigUpdated {
        hostname: String,
    },
    DataChanged,
    ImportProgress {
        repo_id: i64,
        progress: i32,
        total: i32,
        message: Option<String>,
    },
    TunnelStatusChanged {
        client_id: i64,
        hostname: String,
        status: TunnelStatus,
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
            supports_restart: true,
            restart_unavailable_reason: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let msg2: AgentToServer = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&msg2).unwrap();
        assert_eq!(json, json2);
    }
}
