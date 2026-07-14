// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    protocol::{RepoOpKind, TunnelStatus},
    types::{
        BackupStatus, BorgEncryption, Compression, ExecutionMode, OnFailure, QuotaAction,
        ScheduleType, SearchEntry,
    },
};

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing health check.
pub struct HealthCheckResponse {
    /// Current status.
    pub status: String,
    /// Whether any background operation (repo sync, notification delivery) is
    /// currently in flight. Polled by e2e coverage teardown so it can wait for
    /// these to finish before stopping containers, rather than racing a fixed
    /// timeout against variable-duration background work.
    pub background_ops_in_flight: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing login.
pub struct LoginResponse {
    /// User details.
    pub user: UserResponse,
    /// Timestamp of when the session expires occurred.
    pub session_expires_at: DateTime<Utc>,
    /// Whether the session should be remembered beyond the current browser session.
    pub remember_me: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing me.
pub struct MeResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    /// Username.
    pub username: String,
    /// Role assigned to the user.
    pub role: String,
    /// Whether the user must change their password on next login.
    pub must_change_password: bool,
    /// Timestamp of when the session expires occurred.
    pub session_expires_at: Option<DateTime<Utc>>,
    /// Whether the session should be remembered beyond the current browser session.
    pub remember_me: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing refresh session.
pub struct RefreshSessionResponse {
    /// Timestamp of when the session expires occurred.
    pub session_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing user.
pub struct UserResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    /// Username.
    pub username: String,
    /// Role assigned to the user.
    pub role: String,
    /// Timestamp of when the created occurred.
    pub created_at: DateTime<Utc>,
    /// Timestamp of when the last login occurred.
    pub last_login_at: Option<DateTime<Utc>>,
    /// Whether the user must change their password on next login.
    pub must_change_password: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing user list.
pub struct UserListResponse {
    /// List of users.
    pub users: Vec<UserResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states; \
              splitting into enums or sub-structs would break the frontend TS bindings and RBAC \
              field names for no correctness benefit"
)]
/// Response containing agent.
pub struct AgentResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    /// Hostname of the machine.
    pub hostname: String,
    /// Human-readable display name.
    pub display_name: Option<String>,
    /// Version of the agent software.
    pub agent_version: Option<String>,
    /// agent git sha.
    pub agent_git_sha: Option<String>,
    /// agent build time.
    pub agent_build_time: Option<String>,
    /// agent commit count.
    pub agent_commit_count: Option<i32>,
    /// Timestamp of when the created occurred.
    pub created_at: DateTime<Utc>,
    /// Timestamp of when the last seen occurred.
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Default paths to include in backups.
    pub default_backup_paths: Vec<String>,
    /// Default exclude patterns.
    pub default_exclude_patterns: Vec<String>,
    /// Default commands to run before backups.
    pub default_pre_backup_commands: String,
    /// Default commands to run after backups.
    pub default_post_backup_commands: String,
    /// Default file change patterns.
    pub default_file_change_patterns_raw: String,
    /// Whether the agent is currently connected.
    pub is_connected: bool,
    /// Whether this agent was imported from an external source.
    pub is_imported: bool,
    /// Whether this agent is hidden from the UI.
    pub is_hidden: bool,
    /// Whether the agent supports restarting.
    pub supports_restart: bool,
    #[ts(type = "number | null")]
    /// Identifier of the associated owner.
    pub owner_id: Option<i64>,
    /// Visibility scope of this entity.
    pub visibility: String,
    /// Reason why restart is unavailable, if applicable.
    pub restart_unavailable_reason: Option<String>,
    /// SSH username last used to deploy/upgrade this agent.
    pub last_ssh_user: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing create agent.
pub struct CreateAgentResponse {
    /// Agent details.
    pub agent: AgentResponse,
    /// Authentication token.
    pub token: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing agent list.
pub struct AgentListResponse {
    /// agents.
    pub agents: Vec<AgentResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing merge agent.
pub struct MergeAgentResponse {
    /// Whether the merge was successful.
    pub merged: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing delete agent archives.
pub struct DeleteAgentArchivesResponse {
    /// Whether the operation was successful.
    pub success: bool,
    /// Total number of items deleted.
    pub total_deleted: u32,
    /// List of error messages.
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing hostname pattern.
pub struct HostnamePatternResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    #[ts(type = "number")]
    /// Identifier of the associated agent.
    pub agent_id: i64,
    /// Pattern string.
    pub pattern: String,
    /// Timestamp of when the created occurred.
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing repo.
pub struct RepoResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    /// Display name.
    pub name: String,
    /// Path to the repository on the remote host.
    pub repo_path: String,
    /// SSH user for connecting to the repository host.
    pub ssh_user: String,
    /// SSH host for connecting to the repository host.
    pub ssh_host: String,
    /// SSH port for connecting to the repository host.
    pub ssh_port: i32,
    #[ts(type = "string")]
    /// Compression algorithm used.
    pub compression: Compression,
    #[ts(type = "string")]
    /// Encryption method used.
    pub encryption: BorgEncryption,
    /// Whether this entity is enabled.
    pub enabled: bool,
    #[ts(type = "number | null")]
    /// Identifier of the associated owner.
    pub owner_id: Option<i64>,
    /// Visibility scope of this entity.
    pub visibility: String,
    /// sync schedule.
    pub sync_schedule: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing repo list.
pub struct RepoListResponse {
    /// List of repositories.
    pub repos: Vec<RepoResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing repo with stats.
pub struct RepoWithStatsResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    /// Display name.
    pub name: String,
    /// Path to the repository on the remote host.
    pub repo_path: String,
    /// SSH user for connecting to the repository host.
    pub ssh_user: String,
    /// SSH host for connecting to the repository host.
    pub ssh_host: String,
    /// SSH port for connecting to the repository host.
    pub ssh_port: i32,
    /// SSH host key of the repository server.
    pub ssh_host_key: Option<String>,
    #[ts(type = "string")]
    /// Compression algorithm used.
    pub compression: Compression,
    #[ts(type = "string")]
    /// Encryption method used.
    pub encryption: BorgEncryption,
    /// Whether this entity is enabled.
    pub enabled: bool,
    /// Whether the repository is currently being imported.
    pub importing: bool,
    /// Error message from the import process, if any.
    pub import_error: Option<String>,
    /// Import progress count.
    pub import_progress: i32,
    /// Total items to import.
    pub import_total: i32,
    /// Status message from the import process.
    pub import_status_message: Option<String>,
    #[ts(type = "number | null")]
    /// Identifier of the associated owner.
    pub owner_id: Option<i64>,
    /// Visibility scope of this entity.
    pub visibility: String,
    /// sync schedule.
    pub sync_schedule: Option<String>,
    /// Timestamp of when the last synced occurred.
    pub last_synced_at: Option<DateTime<Utc>>,
    #[ts(type = "number")]
    /// Number of archives in the repository.
    pub archive_count: i64,
    /// Timestamp of when the last backup occurred.
    pub last_backup_at: Option<DateTime<Utc>>,
    #[ts(type = "number")]
    /// Total original size across all archives.
    pub total_original_size: i64,
    #[ts(type = "number")]
    /// Total compressed size across all archives.
    pub total_compressed_size: i64,
    #[ts(type = "number")]
    /// Total deduplicated size across all archives.
    pub total_deduplicated_size: i64,
    #[ts(type = "number")]
    /// Number of associated agents.
    pub agent_count: i64,
    #[ts(type = "number")]
    /// Number of unmatched archives.
    pub unmatched_count: i64,
    /// Whether a repository relocation is pending.
    pub relocation_pending: bool,
    #[ts(type = "string | null")]
    /// Kind of the last operation performed.
    pub last_op_kind: Option<RepoOpKind>,
    /// Timestamp of when the last op occurred.
    pub last_op_at: Option<DateTime<Utc>>,
    /// User who performed the last operation.
    pub last_op_by: Option<String>,
    /// Currently active operation, if any.
    pub current_op: Option<crate::protocol::ActiveRepoOp>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing repo with stats list.
pub struct RepoWithStatsListResponse {
    /// List of repositories.
    pub repos: Vec<RepoWithStatsResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing passphrase.
pub struct PassphraseResponse {
    /// Repository passphrase.
    pub passphrase: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing repo host key.
pub struct RepoHostKeyResponse {
    /// SSH host key of the repository server.
    pub ssh_host_key: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing init repo.
pub struct InitRepoResponse {
    /// repo.
    pub repo: RepoResponse,
    /// Output from the borg command.
    pub borg_output: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing confirm relocation.
pub struct ConfirmRelocationResponse {
    /// Response message.
    pub message: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing break lock.
pub struct BreakLockResponse {
    /// Response message.
    pub message: String,
    /// Output from the borg command.
    pub borg_output: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing exec borg.
pub struct ExecBorgResponse {
    /// Standard output from the command.
    pub stdout: String,
    /// Standard error from the command.
    pub stderr: String,
    /// Exit code of the command.
    pub exit_code: i32,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing migrate encryption.
pub struct MigrateEncryptionResponse {
    /// Whether the operation was successful.
    pub success: bool,
    /// Response message.
    pub message: String,
    /// Path of the migrated data, if applicable.
    pub migrated_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states; \
              splitting into enums or sub-structs would break the frontend TS bindings and RBAC \
              field names for no correctness benefit"
)]
/// Response containing schedule.
pub struct ScheduleResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    #[ts(type = "number | null")]
    /// Identifier of the associated repo.
    pub repo_id: Option<i64>,
    /// Display name.
    pub name: String,
    #[ts(type = "string")]
    /// Type of schedule.
    pub schedule_type: ScheduleType,
    /// Cron expression defining the schedule.
    pub cron_expression: String,
    /// Whether this entity is enabled.
    pub enabled: bool,
    /// Whether canary deployments are enabled.
    pub canary_enabled: bool,
    /// Timestamp of when the last run occurred.
    pub last_run_at: Option<DateTime<Utc>>,
    /// Timestamp of when the next run occurred.
    pub next_run_at: Option<DateTime<Utc>>,
    /// Raw exclude patterns.
    pub exclude_patterns_raw: String,
    /// Raw file change detection patterns.
    pub file_change_patterns_raw: String,
    /// Whether global exclude patterns are ignored.
    pub ignore_global_excludes: bool,
    /// Number of hourly backups to retain.
    pub keep_hourly: i32,
    /// Number of daily backups to retain.
    pub keep_daily: i32,
    /// Number of weekly backups to retain.
    pub keep_weekly: i32,
    /// Number of monthly backups to retain.
    pub keep_monthly: i32,
    /// Number of yearly backups to retain.
    pub keep_yearly: i32,
    /// Whether automatic compaction is enabled.
    pub compact_enabled: bool,
    /// Rate limit in kilobytes per second.
    pub rate_limit_kbps: Option<i32>,
    /// Commands to run before the backup.
    pub pre_backup_commands: String,
    /// Commands to run after the backup.
    pub post_backup_commands: String,
    #[ts(type = "string")]
    /// Execution mode for the schedule.
    pub execution_mode: ExecutionMode,
    #[ts(type = "string")]
    /// Action to take on failure.
    pub on_failure: OnFailure,
    #[ts(type = "number | null")]
    /// Identifier of the associated owner.
    pub owner_id: Option<i64>,
    /// Visibility scope of this entity.
    pub visibility: String,
    /// Hostnames targeted by this schedule.
    pub target_hostnames: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing schedule list.
pub struct ScheduleListResponse {
    /// List of schedules.
    pub schedules: Vec<ScheduleResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing schedule target.
pub struct ScheduleTargetResponse {
    #[ts(type = "number")]
    /// Identifier of the associated agent.
    pub agent_id: i64,
    /// Order in which agents execute the schedule.
    pub execution_order: i32,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing schedule backup sources.
pub struct ScheduleBackupSourcesResponse {
    /// Backup source paths.
    pub backup_sources: Vec<String>,
    /// Backup sources per agent.
    pub backup_sources_per_agent: Vec<PerAgentBackupSourcesResponse>,
    /// Exclude patterns per agent.
    pub exclude_patterns_per_agent: Vec<PerAgentExcludePatternsResponse>,
    /// Commands per agent.
    pub commands_per_agent: Vec<PerAgentCommandsResponse>,
    /// File change patterns per agent.
    pub file_change_patterns_per_agent: Vec<PerAgentFileChangePatternsResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing per agent backup sources.
pub struct PerAgentBackupSourcesResponse {
    #[ts(type = "number")]
    /// Identifier of the associated agent.
    pub agent_id: i64,
    /// List of paths.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing per agent exclude patterns.
pub struct PerAgentExcludePatternsResponse {
    #[ts(type = "number")]
    /// Identifier of the associated agent.
    pub agent_id: i64,
    /// Raw text content.
    pub raw_text: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing per agent commands.
pub struct PerAgentCommandsResponse {
    #[ts(type = "number")]
    /// Identifier of the associated agent.
    pub agent_id: i64,
    /// Commands to run before the backup.
    pub pre_backup_commands: String,
    /// Commands to run after the backup.
    pub post_backup_commands: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing per agent file change patterns.
pub struct PerAgentFileChangePatternsResponse {
    #[ts(type = "number")]
    /// Identifier of the associated agent.
    pub agent_id: i64,
    /// Raw text content.
    pub raw_text: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing report.
pub struct ReportResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    #[ts(type = "number")]
    /// Identifier of the associated agent.
    pub agent_id: i64,
    #[ts(type = "number")]
    /// Identifier of the associated repo.
    pub repo_id: i64,
    #[ts(type = "number | null")]
    /// Identifier of the associated schedule.
    pub schedule_id: Option<i64>,
    /// Timestamp of when the started occurred.
    pub started_at: DateTime<Utc>,
    /// Timestamp of when the finished occurred.
    pub finished_at: DateTime<Utc>,
    #[ts(type = "string")]
    /// Current status.
    pub status: BackupStatus,
    #[ts(type = "number")]
    /// Original size of the data before compression.
    pub original_size: i64,
    #[ts(type = "number")]
    /// Compressed size of the data.
    pub compressed_size: i64,
    #[ts(type = "number")]
    /// Deduplicated size of the data.
    pub deduplicated_size: i64,
    #[ts(type = "number")]
    /// Number of files processed.
    pub files_processed: i64,
    #[ts(type = "number")]
    /// Duration of the operation in seconds.
    pub duration_secs: i64,
    /// Error message, if any.
    pub error_message: Option<String>,
    /// Warning messages.
    pub warnings: Vec<String>,
    /// Version of borg used.
    pub borg_version: Option<String>,
    /// Name of the archive.
    pub archive_name: Option<String>,
    /// The borg command that was executed.
    pub borg_command: Option<String>,
    /// Hostname of the machine.
    pub hostname: Option<String>,
    /// Name of the repository.
    pub repo_name: Option<String>,
    /// Name of the schedule.
    pub schedule_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing report list.
pub struct ReportListResponse {
    /// reports.
    pub reports: Vec<ReportResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing archive entry.
pub struct ArchiveEntryResponse {
    /// Display name.
    pub name: String,
    /// Start timestamp.
    pub start: String,
    /// Hostname of the machine.
    pub hostname: String,
    /// Comment associated with the archive.
    pub comment: String,
    #[ts(type = "number")]
    /// Original size of the data before compression.
    pub original_size: i64,
    #[ts(type = "number")]
    /// Deduplicated size of the data.
    pub deduplicated_size: i64,
    /// Whether a matching host was found.
    pub matched: Option<bool>,
    /// Hostname of the agent.
    pub agent_hostname: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing archive info.
pub struct ArchiveInfoResponse {
    #[ts(type = "number")]
    /// Original size of the data before compression.
    pub original_size: i64,
    #[ts(type = "number")]
    /// Compressed size of the data.
    pub compressed_size: i64,
    #[ts(type = "number")]
    /// Deduplicated size of the data.
    pub deduplicated_size: i64,
    #[ts(type = "number")]
    /// Number of files in the archive.
    pub nfiles: i64,
    /// Duration in seconds.
    pub duration: f64,
    /// Start timestamp.
    pub start: String,
    /// End timestamp.
    pub end: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing content entry.
pub struct ContentEntryResponse {
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    /// Type of the entry (file or directory).
    pub entry_type: String,
    /// File path.
    pub path: String,
    #[ts(type = "number")]
    /// Size in bytes.
    pub size: i64,
    /// Modification timestamp.
    pub mtime: String,
    /// File mode/permissions.
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing contents.
pub struct ContentsResponse {
    /// Status of the archive index.
    pub index_status: String,
    /// List of entries.
    pub entries: Vec<ContentEntryResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing archive index status.
pub struct ArchiveIndexStatusResponse {
    /// Current status.
    pub status: String,
    #[ts(type = "number | null")]
    /// Number of files in the index.
    pub file_count: Option<i64>,
    /// Error message, if any.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing delete archive.
pub struct DeleteArchiveResponse {
    /// Whether the operation was successful.
    pub success: bool,
    /// Name of the archive.
    pub archive_name: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing diff.
pub struct DiffResponse {
    /// Files that were added.
    pub added: Vec<String>,
    /// Files that were removed.
    pub removed: Vec<String>,
    /// Files that were modified.
    pub modified: Vec<String>,
    #[ts(type = "number")]
    /// Total number of changes.
    pub total_changes: usize,
    #[ts(type = "number")]
    /// Maximum number of items to return.
    pub limit: usize,
    #[ts(type = "number")]
    /// Number of items to skip.
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing tag.
pub struct TagResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    /// Display name.
    pub name: String,
    /// Color associated with the tag.
    pub color: String,
    /// Scope of the notification channel.
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing tag list.
pub struct TagListResponse {
    /// List of tags.
    pub tags: Vec<TagResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing archive tag.
pub struct ArchiveTagResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    #[ts(type = "number")]
    /// Identifier of the associated repo.
    pub repo_id: i64,
    /// Name of the archive.
    pub archive_name: String,
    /// Tag string.
    pub tag: String,
    #[ts(type = "number | null")]
    /// Identifier of the user who created this.
    pub created_by: Option<i64>,
    /// Timestamp of when the created occurred.
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing api token.
pub struct ApiTokenResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    #[ts(type = "number")]
    /// Identifier of the associated user.
    pub user_id: i64,
    /// Display name.
    pub name: String,
    /// Timestamp of when the last used occurred.
    pub last_used_at: Option<DateTime<Utc>>,
    /// Timestamp of when the created occurred.
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing create api token.
pub struct CreateApiTokenResponse {
    /// Authentication token.
    pub token: ApiTokenResponse,
    /// Plaintext token value.
    pub plaintext: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing list api tokens.
pub struct ListApiTokensResponse {
    /// List of API tokens.
    pub tokens: Vec<ApiTokenResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing delete api token.
pub struct DeleteApiTokenResponse {
    /// Whether the deletion was successful.
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing tunnel.
pub struct TunnelResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    #[ts(type = "number")]
    /// Identifier of the associated agent.
    pub agent_id: i64,
    /// SSH host for connecting to the repository host.
    pub ssh_host: String,
    /// SSH user for connecting to the repository host.
    pub ssh_user: String,
    /// SSH port for connecting to the repository host.
    pub ssh_port: i32,
    /// Port number for the tunnel.
    pub tunnel_port: i32,
    /// Whether this entity is enabled.
    pub enabled: bool,
    /// Timestamp of when the created occurred.
    pub created_at: DateTime<Utc>,
    #[ts(type = "string")]
    /// Current status.
    pub status: TunnelStatus,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing group.
pub struct GroupResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    /// Display name.
    pub name: String,
    /// Description of the entity.
    pub description: Option<String>,
    /// Timestamp of when the created occurred.
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing group list.
pub struct GroupListResponse {
    /// List of groups.
    pub groups: Vec<GroupResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing group members.
pub struct GroupMembersResponse {
    #[ts(type = "number[]")]
    /// List of user identifiers.
    pub user_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states; \
              splitting into enums or sub-structs would break the frontend TS bindings and RBAC \
              field names for no correctness benefit"
)]
/// Response containing role.
pub struct RoleResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    /// Display name.
    pub name: String,
    /// Timestamp of when the created occurred.
    pub created_at: DateTime<Utc>,
    /// Whether the role can create agents.
    pub can_create_agent: bool,
    /// Whether the role can delete any agent.
    pub can_delete_agent: bool,
    /// Whether the role can delete their own agents.
    pub can_delete_own_agent: bool,
    /// Whether the role can create repositories.
    pub can_create_repo: bool,
    /// Whether the role can delete any repository.
    pub can_delete_repo: bool,
    /// Whether the role can delete their own repositories.
    pub can_delete_own_repo: bool,
    /// Whether the role can create schedules.
    pub can_create_schedule: bool,
    /// Whether the role can delete any schedule.
    pub can_delete_schedule: bool,
    /// Whether the role can delete their own schedules.
    pub can_delete_own_schedule: bool,
    /// Whether the role can manage tags.
    pub can_manage_tags: bool,
    /// Whether the role can view all repositories.
    pub can_view_all_repos: bool,
    /// Whether the role can manage tunnels.
    pub can_manage_tunnels: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing role list.
pub struct RoleListResponse {
    /// List of roles.
    pub roles: Vec<RoleResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states; \
              splitting into enums or sub-structs would break the frontend TS bindings and RBAC \
              field names for no correctness benefit"
)]
/// Response containing repo permission.
pub struct RepoPermissionResponse {
    #[ts(type = "number")]
    /// Identifier of the associated user.
    pub user_id: i64,
    #[ts(type = "number")]
    /// Identifier of the associated repo.
    pub repo_id: i64,
    /// Whether the user can view this.
    pub can_view: bool,
    /// Whether backups are allowed.
    pub can_backup: bool,
    /// Whether schedules can be modified.
    pub can_modify_schedules: bool,
    /// Whether extraction is allowed.
    pub can_extract: bool,
    /// Whether deletion is allowed.
    pub can_delete: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing ssh public key.
pub struct SshPublicKeyResponse {
    /// Public key.
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing settings.
pub struct SettingsResponse {
    #[ts(type = "number")]
    /// Number of days to retain backup data.
    pub retention_days: i64,
    #[ts(type = "number")]
    /// Number of days to retain backup reports.
    pub report_retention_days: i64,
    #[ts(type = "number")]
    /// Number of days to retain failed backup reports.
    pub failed_report_retention_days: i64,
    #[ts(type = "number")]
    /// Number of days to retain system events.
    pub system_event_retention_days: i64,
    /// Timezone setting.
    pub timezone: String,
    #[ts(type = "number")]
    /// Timeout for borg queries in seconds.
    pub borg_query_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing database storage.
pub struct DatabaseStorageResponse {
    #[ts(type = "number")]
    /// Size of the database in bytes.
    pub database_bytes: i64,
    #[ts(type = "number")]
    /// Size of non-database storage in bytes.
    pub other_bytes: i64,
    /// Per-relation size breakdown.
    pub relations: Vec<DatabaseRelationSizeResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing database relation size.
pub struct DatabaseRelationSizeResponse {
    /// Name of the database relation.
    pub relation_name: String,
    #[ts(type = "number")]
    /// Total bytes used by the relation.
    pub total_bytes: i64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing version.
pub struct VersionResponse {
    /// Version of the server software.
    pub server_version: String,
    /// Git SHA of the server build.
    pub server_git_sha: String,
    /// Timestamp of the build.
    pub build_timestamp: String,
    /// Number of commits in the server build.
    pub server_commit_count: Option<u32>,
    /// Version of the agent software.
    pub agent_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing system reset.
pub struct SystemResetResponse {
    #[ts(type = "number")]
    /// Number of backups that were cancelled.
    pub cancelled_backups: u64,
    #[ts(type = "number")]
    /// Number of agents that were notified.
    pub notified_agents: usize,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing audit entry.
pub struct AuditEntryResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    #[ts(type = "number | null")]
    /// Identifier of the associated user.
    pub user_id: Option<i64>,
    /// Username.
    pub username: String,
    /// Action that was performed.
    pub action: String,
    /// Type of the target entity.
    pub target_type: Option<String>,
    #[ts(type = "number | null")]
    /// Identifier of the associated target.
    pub target_id: Option<i64>,
    #[ts(type = "any")]
    /// Additional details about the audit entry.
    pub details: Option<serde_json::Value>,
    /// IP address of the user who performed the action.
    pub ip_address: Option<String>,
    /// Timestamp of when the created occurred.
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing audit log.
pub struct AuditLogResponse {
    /// List of items.
    pub items: Vec<AuditEntryResponse>,
    #[ts(type = "number")]
    /// Total number of items.
    pub total: i64,
    #[ts(type = "number")]
    /// Current page number.
    pub page: i64,
    #[ts(type = "number")]
    /// Number of items per page.
    pub per_page: i64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing notification channel.
pub struct NotificationChannelResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    /// Display name.
    pub name: String,
    /// Type of notification channel.
    pub channel_type: String,
    #[ts(type = "any")]
    /// Configuration for the notification channel.
    pub config: serde_json::Value,
    /// Whether this entity is enabled.
    pub enabled: bool,
    #[ts(type = "any")]
    /// Scope of the notification channel.
    pub scope: serde_json::Value,
    /// Timestamp of when the created occurred.
    pub created_at: DateTime<Utc>,
    /// Timestamp of when the updated occurred.
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing notification rule.
pub struct NotificationRuleResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    #[ts(type = "number")]
    /// Identifier of the associated channel.
    pub channel_id: i64,
    /// Type of event that triggers this rule.
    pub event_type: String,
    #[ts(type = "number | null")]
    /// Identifier of the associated repo.
    pub repo_id: Option<i64>,
    #[ts(type = "number | null")]
    /// Identifier of the associated agent.
    pub agent_id: Option<i64>,
    /// Whether this entity is enabled.
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing notification delivery.
pub struct NotificationDeliveryResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    #[ts(type = "number")]
    /// Identifier of the associated channel.
    pub channel_id: i64,
    /// Type of event that triggers this rule.
    pub event_type: String,
    #[ts(type = "any")]
    /// Payload of the notification delivery.
    pub payload: serde_json::Value,
    /// Current status.
    pub status: String,
    /// Error message, if any.
    pub error_message: Option<String>,
    /// Timestamp of when the attempted occurred.
    pub attempted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing vapid key.
pub struct VapidKeyResponse {
    /// Public key.
    pub public_key: String,
    /// Whether the key is configured.
    pub configured: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing push subscription.
pub struct PushSubscriptionResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    #[ts(type = "number")]
    /// Identifier of the associated user.
    pub user_id: i64,
    /// Endpoint URL.
    pub endpoint: String,
    /// User agent string.
    pub user_agent: Option<String>,
    /// Timestamp of when the created occurred.
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing repo quota.
pub struct RepoQuotaResponse {
    #[ts(type = "number")]
    /// Identifier of the associated repo.
    pub repo_id: i64,
    #[ts(type = "number | null")]
    /// Threshold for warning in bytes.
    pub warn_bytes: Option<i64>,
    #[ts(type = "number | null")]
    /// Threshold for critical in bytes.
    pub critical_bytes: Option<i64>,
    /// Action to take when warning threshold is exceeded.
    pub warn_action: QuotaAction,
    /// Action to take when critical threshold is exceeded.
    pub critical_action: QuotaAction,
    /// Whether this entity is enabled.
    pub enabled: bool,
    /// Timestamp of when the updated occurred.
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing server quota.
pub struct ServerQuotaResponse {
    /// SSH host for connecting to the repository host.
    pub ssh_host: String,
    #[ts(type = "number")]
    /// Number of repositories on this server.
    pub repo_count: i64,
    #[ts(type = "number")]
    /// Total deduplicated size across all archives.
    pub total_deduplicated_size: i64,
    /// Whether the key is configured.
    pub configured: bool,
    #[ts(type = "number | null")]
    /// Threshold for warning in bytes.
    pub warn_bytes: Option<i64>,
    #[ts(type = "number | null")]
    /// Threshold for critical in bytes.
    pub critical_bytes: Option<i64>,
    /// Action to take when warning threshold is exceeded.
    pub warn_action: QuotaAction,
    /// Action to take when critical threshold is exceeded.
    pub critical_action: QuotaAction,
    /// Whether this entity is enabled.
    pub enabled: bool,
    /// Timestamp of when the updated occurred.
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing global excludes.
pub struct GlobalExcludesResponse {
    /// Raw text content.
    pub raw_text: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing dashboard summary.
pub struct DashboardSummaryResponse {
    #[ts(type = "number")]
    /// Number of currently connected agents.
    pub online_agents: usize,
    #[ts(type = "number")]
    /// Total number of registered agents.
    pub total_agents: i64,
    #[ts(type = "number")]
    /// Total number of repositories.
    pub total_repos: i64,
    /// Timestamp of when the last backup occurred.
    pub last_backup_at: Option<DateTime<Utc>>,
    /// Timestamp of when the next backup occurred.
    pub next_backup_at: Option<DateTime<Utc>>,
    #[ts(type = "number | null")]
    /// Identifier of the associated last backup schedule.
    pub last_backup_schedule_id: Option<i64>,
    #[ts(type = "number | null")]
    /// Identifier of the associated last backup repo.
    pub last_backup_repo_id: Option<i64>,
    /// Archive name of the last backup.
    pub last_backup_archive_name: Option<String>,
    #[ts(type = "number | null")]
    /// Identifier of the associated next backup schedule.
    pub next_backup_schedule_id: Option<i64>,
    #[ts(type = "number")]
    /// Number of active schedules.
    pub active_schedules: i64,
    #[ts(type = "number")]
    /// Total number of schedules.
    pub total_schedules: i64,
    #[ts(type = "number")]
    /// Total storage used in bytes.
    pub total_storage_bytes: i64,
    #[ts(type = "number")]
    /// Number of successful backups in the last 30 days.
    pub success_30d: i64,
    #[ts(type = "number")]
    /// Number of failed backups in the last 30 days.
    pub failed_30d: i64,
    #[ts(type = "number")]
    /// Total number of backups in the last 30 days.
    pub total_30d: i64,
    /// Storage usage broken down by repository.
    pub storage_by_repo: Vec<StorageRepoEntryResponse>,
    /// Timestamp of when the last failure occurred.
    pub last_failure_at: Option<DateTime<Utc>>,
    /// Timestamp of when the last warning occurred.
    pub last_warning_at: Option<DateTime<Utc>>,
    #[ts(type = "number | null")]
    /// Identifier of the associated last failure schedule.
    pub last_failure_schedule_id: Option<i64>,
    #[ts(type = "number | null")]
    /// Identifier of the associated last warning schedule.
    pub last_warning_schedule_id: Option<i64>,
    /// Error message of the last failure.
    pub last_failure_message: Option<String>,
    /// Warning message of the last warning.
    pub last_warning_message: Option<String>,
    #[ts(type = "number | null")]
    /// Identifier of the associated last failure repo.
    pub last_failure_repo_id: Option<i64>,
    #[ts(type = "number | null")]
    /// Identifier of the associated last warning repo.
    pub last_warning_repo_id: Option<i64>,
    /// Repository name of the last failure.
    pub last_failure_repo_name: Option<String>,
    /// Repository name of the last warning.
    pub last_warning_repo_name: Option<String>,
    /// Schedule name of the last failure.
    pub last_failure_schedule_name: Option<String>,
    /// Schedule name of the last warning.
    pub last_warning_schedule_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing storage repo entry.
pub struct StorageRepoEntryResponse {
    /// Display name.
    pub name: String,
    #[ts(type = "number")]
    /// Compressed size of the data.
    pub compressed_size: i64,
    #[ts(type = "number")]
    /// Deduplicated size of the data.
    pub deduplicated_size: i64,
    /// Percentage value.
    pub percentage: f64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing health summary.
pub struct HealthSummaryResponse {
    #[ts(type = "number")]
    /// Identifier of the associated repo.
    pub repo_id: i64,
    #[ts(type = "number")]
    /// Identifier of the associated schedule.
    pub schedule_id: i64,
    /// Hostname of the machine.
    pub hostname: String,
    /// target name.
    pub target_name: String,
    #[ts(type = "string | null")]
    /// last status.
    pub last_status: Option<BackupStatus>,
    /// Timestamp of when the last backup occurred.
    pub last_backup_at: Option<DateTime<Utc>>,
    /// Whether the schedule is overdue.
    pub is_overdue: bool,
    /// Last error message.
    pub last_error_message: Option<String>,
    /// Cron expression defining the schedule.
    pub cron_expression: Option<String>,
    /// Whether the schedule is enabled.
    pub schedule_enabled: Option<bool>,
}

/// Alias so that API handlers can use `HealthResponse` while sharing
/// the same definition as `HealthSummaryResponse`.
pub type HealthResponse = HealthSummaryResponse;

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing activity entry.
pub struct ActivityEntryResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    #[ts(type = "number")]
    /// Identifier of the associated repo.
    pub repo_id: i64,
    #[ts(type = "number | null")]
    /// Identifier of the associated schedule.
    pub schedule_id: Option<i64>,
    /// Hostname of the machine.
    pub hostname: String,
    #[ts(type = "string")]
    /// Current status.
    pub status: BackupStatus,
    /// Name of the archive.
    pub archive_name: Option<String>,
    #[ts(type = "number")]
    /// Original size of the data before compression.
    pub original_size: i64,
    #[ts(type = "number")]
    /// Compressed size of the data.
    pub compressed_size: i64,
    #[ts(type = "number")]
    /// Deduplicated size of the data.
    pub deduplicated_size: i64,
    #[ts(type = "number")]
    /// Number of files processed.
    pub files_processed: i64,
    #[ts(type = "number")]
    /// Duration of the operation in seconds.
    pub duration_secs: i64,
    /// Error message, if any.
    pub error_message: Option<String>,
    /// Timestamp of when the started occurred.
    pub started_at: DateTime<Utc>,
    /// Timestamp of when the finished occurred.
    pub finished_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing system event.
pub struct SystemEventResponse {
    #[ts(type = "number")]
    /// Unique identifier.
    pub id: i64,
    /// Type of event that triggers this rule.
    pub event_type: String,
    #[ts(type = "number | null")]
    /// Identifier of the associated agent.
    pub agent_id: Option<i64>,
    /// Response message.
    pub message: String,
    /// Timestamp of when the created occurred.
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing calendar day.
pub struct CalendarDayResponse {
    /// Date string.
    pub date: String,
    /// List of events for this day.
    pub events: Vec<CalendarEventResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing calendar event.
pub struct CalendarEventResponse {
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    /// Type of event that triggers this rule.
    pub event_type: String,
    /// Current status.
    pub status: String,
    /// Name of the repository.
    pub repo_name: String,
    /// Hostname of the machine.
    pub hostname: String,
    /// Time of the event.
    pub time: String,
    #[ts(type = "number | null")]
    /// Identifier of the associated report.
    pub report_id: Option<i64>,
    #[ts(type = "number | null")]
    /// Identifier of the associated repo.
    pub repo_id: Option<i64>,
    #[ts(type = "number | null")]
    /// Identifier of the associated schedule.
    pub schedule_id: Option<i64>,
    /// Name of the archive.
    pub archive_name: Option<String>,
    /// Error message, if any.
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing trend entry.
pub struct TrendEntryResponse {
    /// Date string.
    pub date: String,
    #[ts(type = "number")]
    /// Original size of the data before compression.
    pub original_size: i64,
    #[ts(type = "number")]
    /// Compressed size of the data.
    pub compressed_size: i64,
    #[ts(type = "number")]
    /// Deduplicated size of the data.
    pub deduplicated_size: i64,
    /// Deduplication ratio.
    pub dedup_ratio: f64,
    #[ts(type = "number")]
    /// Number of files in the index.
    pub file_count: i64,
    #[ts(type = "number")]
    /// Duration in seconds.
    pub duration_seconds: i64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing storage trend entry.
pub struct StorageTrendEntryResponse {
    /// Date string.
    pub date: String,
    #[ts(type = "number")]
    /// Original size of the data before compression.
    pub original_size: i64,
    #[ts(type = "number")]
    /// Compressed size of the data.
    pub compressed_size: i64,
    #[ts(type = "number | null")]
    /// Deduplicated size of the data.
    pub deduplicated_size: Option<i64>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing storage trend by repo entry.
pub struct StorageTrendByRepoEntryResponse {
    /// Date string.
    pub date: String,
    #[ts(type = "number")]
    /// Identifier of the associated repo.
    pub repo_id: i64,
    /// Name of the repository.
    pub repo_name: String,
    #[ts(type = "number")]
    /// Original size of the data before compression.
    pub original_size: i64,
    #[ts(type = "number")]
    /// Compressed size of the data.
    pub compressed_size: i64,
    #[ts(type = "number | null")]
    /// Deduplicated size of the data.
    pub deduplicated_size: Option<i64>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing storage stat.
pub struct StorageStatResponse {
    /// Display name.
    pub name: String,
    #[ts(type = "number")]
    /// Compressed size of the data.
    pub compressed_size: i64,
    #[ts(type = "number")]
    /// Deduplicated size of the data.
    pub deduplicated_size: i64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing schedule count by agent.
pub struct ScheduleCountByAgentResponse {
    #[ts(type = "number")]
    /// Identifier of the associated agent.
    pub agent_id: i64,
    #[ts(type = "number")]
    /// Count of items.
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing dashboard overview.
pub struct DashboardOverviewResponse {
    /// Summary counters for the dashboard overview.
    pub summary: DashboardSummaryCountersResponse,
    /// Dashboard findings.
    pub findings: Vec<DashboardFindingResponse>,
    /// Protection coverage data.
    pub protection: DashboardProtectionCoverageResponse,
    /// Currently running operations.
    pub running_operations: Vec<DashboardOperationResponse>,
    /// Upcoming scheduled backups.
    pub upcoming_schedules: Vec<DashboardUpcomingScheduleResponse>,
    /// Repository capacity information.
    pub repository_capacity: Vec<DashboardRepositoryCapacityResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing dashboard summary counters.
pub struct DashboardSummaryCountersResponse {
    #[ts(type = "number")]
    /// Number of protected hosts.
    pub protected_hosts: i64,
    #[ts(type = "number")]
    /// Number of eligible hosts.
    pub eligible_hosts: i64,
    #[ts(type = "number")]
    /// Number of items needing attention.
    pub needs_attention: usize,
    #[ts(type = "number")]
    /// Currently running operations.
    pub running_operations: usize,
    #[ts(type = "number")]
    /// Total storage used in bytes.
    pub total_storage_bytes: i64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing dashboard finding.
pub struct DashboardFindingResponse {
    /// Unique identifier.
    pub id: String,
    /// Kind of the finding.
    pub kind: String,
    /// Severity level.
    pub severity: String,
    /// Current status.
    pub status: String,
    /// Hostname of the machine.
    pub hostname: Option<String>,
    #[ts(type = "number | null")]
    /// Identifier of the associated schedule.
    pub schedule_id: Option<i64>,
    /// Name of the schedule.
    pub schedule_name: Option<String>,
    #[ts(type = "number | null")]
    /// Identifier of the associated repo.
    pub repo_id: Option<i64>,
    /// Name of the repository.
    pub repo_name: Option<String>,
    /// Reason for the finding.
    pub reason: String,
    /// Timestamp of when the occurred occurred.
    pub occurred_at: Option<DateTime<Utc>>,
    /// Deadline for addressing the finding.
    pub deadline: Option<DateTime<Utc>>,
    /// Destination details for navigation.
    pub destination: DashboardDestinationResponse,
}

/// Dashboard destination response.
#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
#[serde(tag = "kind")]
pub enum DashboardDestinationResponse {
    /// Navigation to a specific host.
    #[serde(rename = "host")]
    Host {
        /// Hostname of the agent.
        hostname: String,
    },
    /// Navigation to a specific schedule.
    #[serde(rename = "schedule")]
    Schedule {
        /// Identifier of the schedule.
        #[ts(type = "number")]
        schedule_id: i64,
    },
    /// Navigation to a specific repository.
    #[serde(rename = "repository")]
    Repository {
        /// Identifier of the repository.
        #[ts(type = "number")]
        repo_id: i64,
    },
    /// Navigation to a specific activity/report.
    #[serde(rename = "activity")]
    Activity {
        /// Identifier of the report.
        #[ts(type = "number")]
        report_id: i64,
    },
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing dashboard protection coverage.
pub struct DashboardProtectionCoverageResponse {
    #[ts(type = "number")]
    /// Number of protected hosts.
    pub protected_hosts: i64,
    #[ts(type = "number")]
    /// Number of eligible hosts.
    pub eligible_hosts: i64,
    /// Links to protected agents.
    pub protected_agent_links: Vec<DashboardAgentLinkResponse>,
    /// Links to unassigned agents.
    pub unassigned_agents: Vec<DashboardAgentLinkResponse>,
    #[ts(type = "number")]
    /// Number of targets that have never successfully backed up.
    pub never_succeeded_targets: i64,
    /// Links to agents that have never successfully backed up.
    pub never_succeeded_agents: Vec<DashboardAgentLinkResponse>,
    /// Links to agents with only disabled schedules.
    pub disabled_only_agents: Vec<DashboardAgentLinkResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing dashboard agent link.
pub struct DashboardAgentLinkResponse {
    #[ts(type = "number")]
    /// Identifier of the associated agent.
    pub agent_id: i64,
    /// Hostname of the machine.
    pub hostname: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing dashboard operation.
pub struct DashboardOperationResponse {
    #[ts(type = "number")]
    /// Identifier of the associated report.
    pub report_id: i64,
    /// Current status.
    pub status: String,
    /// Hostname of the machine.
    pub hostname: String,
    #[ts(type = "number")]
    /// Identifier of the associated schedule.
    pub schedule_id: i64,
    /// Name of the schedule.
    pub schedule_name: String,
    #[ts(type = "number")]
    /// Identifier of the associated repo.
    pub repo_id: i64,
    /// Name of the repository.
    pub repo_name: String,
    /// Timestamp of when the started occurred.
    pub started_at: DateTime<Utc>,
    /// Destination details for navigation.
    pub destination: DashboardDestinationResponse,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing dashboard upcoming schedule.
pub struct DashboardUpcomingScheduleResponse {
    #[ts(type = "number")]
    /// Identifier of the associated schedule.
    pub schedule_id: i64,
    /// Name of the schedule.
    pub schedule_name: String,
    #[ts(type = "number")]
    /// Identifier of the associated repo.
    pub repo_id: i64,
    /// Name of the repository.
    pub repo_name: String,
    /// Timestamp of when the next run occurred.
    pub next_run_at: DateTime<Utc>,
    #[ts(type = "number")]
    /// Number of targets.
    pub target_count: i64,
    #[ts(type = "number")]
    /// Number of offline targets.
    pub offline_target_count: usize,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing dashboard repository capacity.
pub struct DashboardRepositoryCapacityResponse {
    #[ts(type = "number")]
    /// Identifier of the associated repo.
    pub repo_id: i64,
    /// Name of the repository.
    pub repo_name: String,
    #[ts(type = "number")]
    /// Deduplicated size of the data.
    pub deduplicated_size: i64,
    #[ts(type = "number | null")]
    /// Quota limit in bytes.
    pub quota_bytes: Option<i64>,
    /// Quota utilization as a percentage.
    pub quota_utilization_percent: Option<f64>,
    /// Status of the quota.
    pub quota_status: String,
    #[ts(type = "number | null")]
    /// Storage change in bytes.
    pub storage_change_bytes: Option<i64>,
    /// Estimated time when the quota threshold will be reached.
    pub threshold_estimate: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing log entry.
pub struct LogEntryResponse {
    /// Timestamp of the log entry.
    pub timestamp: DateTime<Utc>,
    /// Log level.
    pub level: String,
    /// Log target/module.
    pub target: String,
    /// Response message.
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing repo export.
///
/// Passphrases are never exported - they are encrypted at rest with a
/// server-specific key and must be set manually on the target server
/// after import.
pub struct RepoExportResponse {
    /// Name of the repository.
    pub name: String,
    /// Path to the repository on the remote host.
    pub repo_path: String,
    /// SSH user for connecting to the repository host.
    pub ssh_user: String,
    /// SSH host for connecting to the repository host.
    pub ssh_host: String,
    /// SSH port for connecting to the repository host.
    pub ssh_port: i32,
    /// Compression algorithm.
    pub compression: String,
    /// Encryption algorithm.
    pub encryption: String,
    /// Whether the repository is enabled.
    pub enabled: bool,
    /// Sync schedule for the repository.
    pub sync_schedule: Option<String>,
    /// SSH host key fingerprint.
    pub ssh_host_key: Option<String>,
    /// Warning quota threshold in bytes.
    #[serde(default)]
    pub quota_warn_bytes: Option<i64>,
    /// Critical quota threshold in bytes.
    #[serde(default)]
    pub quota_critical_bytes: Option<i64>,
    /// Action when warning threshold is exceeded.
    #[serde(default)]
    pub quota_warn_action: String,
    /// Action when critical threshold is exceeded.
    #[serde(default)]
    pub quota_critical_action: String,
    /// Tags associated with the repository.
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing config export.
pub struct ConfigExportResponse {
    /// Version number.
    pub version: u32,
    /// Timestamp of when the exported occurred.
    pub exported_at: DateTime<Utc>,
    /// Exported hosts.
    pub hosts: Vec<HostExportResponse>,
    /// List of schedules.
    pub schedules: Vec<ScheduleExportResponse>,
    /// Exported repositories.
    #[serde(default)]
    pub repos: Vec<RepoExportResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing host export.
pub struct HostExportResponse {
    /// Hostname of the machine.
    pub hostname: String,
    /// Human-readable display name.
    pub display_name: Option<String>,
    /// Default paths to include in backups.
    pub default_backup_paths: Vec<String>,
    /// Default exclude patterns.
    pub default_exclude_patterns: Vec<String>,
    /// Default commands to run before backups.
    pub default_pre_backup_commands: String,
    /// Default commands to run after backups.
    pub default_post_backup_commands: String,
    /// Default file change detection patterns.
    #[serde(default)]
    pub default_file_change_patterns_raw: String,
    /// Hostname pattern aliases.
    pub hostname_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing schedule target export.
pub struct ScheduleTargetExportResponse {
    /// Hostname of the target machine.
    pub hostname: String,
    /// Execution order for the target.
    pub execution_order: i32,
    /// Backup source paths for the target.
    pub backup_sources: Vec<String>,
    /// Exclude patterns for the target.
    pub exclude_patterns: String,
    /// File change detection patterns.
    #[serde(default)]
    pub file_change_patterns: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, utoipa::ToSchema)]
#[ts(export)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states; \
              splitting into enums or sub-structs would break the frontend TS bindings and RBAC \
              field names for no correctness benefit"
)]
/// Response containing schedule export.
pub struct ScheduleExportResponse {
    /// Display name.
    pub name: String,
    #[ts(type = "string")]
    /// Type of schedule.
    pub schedule_type: ScheduleType,
    /// Cron expression defining the schedule.
    pub cron_expression: String,
    /// Whether this entity is enabled.
    pub enabled: bool,
    /// Whether canary deployments are enabled.
    pub canary_enabled: bool,
    #[ts(type = "string")]
    /// Execution mode for the schedule.
    pub execution_mode: ExecutionMode,
    #[ts(type = "string")]
    /// Action to take on failure.
    pub on_failure: OnFailure,
    /// Raw exclude patterns.
    pub exclude_patterns_raw: String,
    /// Raw file change detection patterns.
    #[serde(default)]
    pub file_change_patterns_raw: String,
    /// Whether global exclude patterns are ignored.
    pub ignore_global_excludes: bool,
    /// Number of hourly backups to retain.
    pub keep_hourly: i32,
    /// Number of daily backups to retain.
    pub keep_daily: i32,
    /// Number of weekly backups to retain.
    pub keep_weekly: i32,
    /// Number of monthly backups to retain.
    pub keep_monthly: i32,
    /// Number of yearly backups to retain.
    pub keep_yearly: i32,
    /// Whether automatic compaction is enabled.
    pub compact_enabled: bool,
    /// Rate limit in kilobytes per second.
    pub rate_limit_kbps: Option<i32>,
    /// Commands to run before the backup.
    pub pre_backup_commands: Vec<String>,
    /// Commands to run after the backup.
    pub post_backup_commands: Vec<String>,
    /// Backup source paths.
    pub backup_sources: Vec<String>,
    /// Per-target overrides.
    pub targets: Vec<ScheduleTargetExportResponse>,
    /// Name of the repository.
    pub repo_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing import result.
pub struct ImportResultResponse {
    /// Number of hosts created during import.
    pub hosts_created: u32,
    /// Number of hosts updated during import.
    pub hosts_updated: u32,
    /// Number of schedules created during import.
    pub schedules_created: u32,
    /// Number of repositories created during import.
    pub repos_created: u32,
    /// Number of repositories updated during import.
    pub repos_updated: u32,
    /// Warning messages.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing deploy agent.
pub struct DeployAgentResponse {
    /// Whether the operation was successful.
    pub success: bool,
    /// Whether the deployment was skipped.
    pub skipped: bool,
    /// Authentication token.
    pub token: Option<String>,
    /// Version available for deployment.
    pub available_version: Option<String>,
    /// Error message, if any.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing fetch service unit.
pub struct FetchServiceUnitResponse {
    /// File content.
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing restore files.
pub struct RestoreFilesResponse {
    /// Whether the operation was successful.
    pub success: bool,
    #[ts(type = "number")]
    /// Number of files restored.
    pub files_restored: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Error message, if any.
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing dry run.
pub struct DryRunResponse {
    /// List of files.
    pub files: Vec<DryRunFileEntryResponse>,
    #[ts(type = "number")]
    /// Total size in bytes.
    pub total_size: i64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing dry run file entry.
pub struct DryRunFileEntryResponse {
    /// File path.
    pub path: String,
    #[ts(type = "number")]
    /// Size in bytes.
    pub size: i64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing search.
pub struct SearchResponse {
    /// List of items.
    pub items: Vec<SearchEntry>,
    #[ts(type = "number")]
    /// Total number of matching entries.
    pub total_matched: usize,
    #[ts(type = "number")]
    /// Maximum number of items to return.
    pub limit: usize,
    #[ts(type = "number")]
    /// Number of items to skip.
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing cross search.
pub struct CrossSearchResponse {
    /// List of items.
    pub items: Vec<CrossSearchEntryResponse>,
    #[ts(type = "number")]
    /// Total number of archives searched.
    pub total_archives_searched: usize,
    #[ts(type = "number")]
    /// Maximum number of items to return.
    pub limit: usize,
    #[ts(type = "number")]
    /// Number of items to skip.
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing cross search entry.
pub struct CrossSearchEntryResponse {
    /// File path.
    pub path: String,
    #[ts(type = "number")]
    /// Size in bytes.
    pub size: i64,
    /// Modification timestamp.
    pub mtime: DateTime<Utc>,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    /// Type of the entry (file or directory).
    pub entry_type: String,
    /// Name of the archive.
    pub archive_name: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
#[serde(transparent)]
/// Response containing preferences.
pub struct PreferencesResponse {
    #[ts(type = "any")]
    /// Inner JSON value.
    pub inner: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing refresh.
pub struct RefreshResponse {
    /// Timestamp of when the session expires occurred.
    pub session_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing hostname pattern list.
pub struct HostnamePatternListResponse {
    /// List of patterns.
    pub patterns: Vec<HostnamePatternResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing schedule target list.
pub struct ScheduleTargetListResponse {
    /// List of targets.
    pub targets: Vec<ScheduleTargetResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing repo permission list.
pub struct RepoPermissionListResponse {
    /// List of permissions.
    pub permissions: Vec<RepoPermissionResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing activity list.
pub struct ActivityListResponse {
    /// List of items.
    pub items: Vec<ActivityEntryResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing system event list.
pub struct SystemEventListResponse {
    /// List of events for this day.
    pub events: Vec<SystemEventResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing storage stat list.
pub struct StorageStatListResponse {
    /// List of statistics entries.
    pub stats: Vec<StorageStatResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing schedule count by agent list.
pub struct ScheduleCountByAgentListResponse {
    /// List of count entries.
    pub counts: Vec<ScheduleCountByAgentResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing archive tag list.
pub struct ArchiveTagListResponse {
    /// List of tags.
    pub tags: Vec<ArchiveTagResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing agent tag association.
pub struct AgentTagAssociationResponse {
    #[ts(type = "number")]
    /// Identifier of the associated tag.
    pub tag_id: i64,
    #[ts(type = "number")]
    /// Identifier of the associated agent.
    pub agent_id: i64,
    /// Timestamp of when the created occurred.
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing repo tag association.
pub struct RepoTagAssociationResponse {
    #[ts(type = "number")]
    /// Identifier of the associated tag.
    pub tag_id: i64,
    #[ts(type = "number")]
    /// Identifier of the associated repo.
    pub repo_id: i64,
    /// Timestamp of when the created occurred.
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing agent tag association list.
pub struct AgentTagAssociationListResponse {
    /// List of associations.
    pub associations: Vec<AgentTagAssociationResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing repo tag association list.
pub struct RepoTagAssociationListResponse {
    /// List of associations.
    pub associations: Vec<RepoTagAssociationResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing agent tag entry.
pub struct AgentTagEntryResponse {
    #[ts(type = "number")]
    /// Identifier of the associated agent.
    pub agent_id: i64,
    /// Name of the tag.
    pub tag_name: String,
    /// Color of the tag.
    pub tag_color: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing repo tag entry.
pub struct RepoTagEntryResponse {
    #[ts(type = "number")]
    /// Identifier of the associated repo.
    pub repo_id: i64,
    /// Name of the tag.
    pub tag_name: String,
    /// Color of the tag.
    pub tag_color: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing rescan.
pub struct RescanResponse {
    #[ts(type = "number")]
    /// Whether a matching host was found.
    pub matched: u64,
    #[ts(type = "number")]
    /// Number of items still unmatched.
    pub remaining_unmatched: u64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
/// Response containing sync.
pub struct SyncResponse {
    #[ts(type = "number")]
    /// Number of items imported.
    pub imported: u64,
    #[ts(type = "number")]
    /// Files that were removed.
    pub removed: u64,
    #[ts(type = "number")]
    /// Duration of the operation in seconds.
    pub duration_secs: u64,
}
