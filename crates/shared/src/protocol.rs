// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::{AgentConfig, AgentStatus, BackupReport, BorgEncryption, RepoId};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum ServerToAgent {
    ConfigUpdate(AgentConfig),
    RunBackupNow {
        repo_id: RepoId,
    },
    RunCheckNow {
        repo_id: RepoId,
    },
    RunVerifyNow {
        repo_id: RepoId,
    },
    InitRepo {
        repo_path: String,
        ssh_user: String,
        ssh_host: String,
        ssh_port: u16,
        passphrase: String,
        encryption: BorgEncryption,
    },
    RestartAgent,
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
        supports_restart: bool,
        #[serde(default)]
        restart_unavailable_reason: Option<String>,
    },
    BackupStarted {
        repo_id: RepoId,
        started_at: DateTime<Utc>,
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
    TunnelStatusChanged {
        client_id: i64,
        hostname: String,
        status: TunnelStatus,
    },
}

#[cfg(test)]
mod tests {
    use super::TunnelStatus;

    #[test]
    fn tunnel_status_connected_round_trips_json() {
        let status = TunnelStatus::Connected;
        let json = serde_json::to_string(&status);
        assert!(json.is_ok());
        let json = json.unwrap_or_default();
        let status2 = serde_json::from_str::<TunnelStatus>(&json);
        assert!(status2.is_ok());
        let status2 = status2.unwrap_or(TunnelStatus::Disconnected);
        assert_eq!(status, status2);
    }

    #[test]
    fn tunnel_status_error_round_trips_json() {
        let status = TunnelStatus::Error {
            message: String::from("AllowTcpForwarding disabled"),
        };
        let json = serde_json::to_string(&status);
        assert!(json.is_ok());
        let json = json.unwrap_or_default();
        let status2 = serde_json::from_str::<TunnelStatus>(&json);
        assert!(status2.is_ok());
        let status2 = status2.unwrap_or(TunnelStatus::Disconnected);
        assert_eq!(status, status2);
    }
}
