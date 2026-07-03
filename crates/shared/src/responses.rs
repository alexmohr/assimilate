// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use serde::Serialize;
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
pub struct HealthCheckResponse {
    pub status: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct LoginResponse {
    pub user: UserResponse,
    pub session_expires_at: DateTime<Utc>,
    pub remember_me: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct MeResponse {
    #[ts(type = "number")]
    pub id: i64,
    pub username: String,
    pub role: String,
    pub must_change_password: bool,
    pub session_expires_at: Option<DateTime<Utc>>,
    pub remember_me: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RefreshSessionResponse {
    pub session_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct UserResponse {
    #[ts(type = "number")]
    pub id: i64,
    pub username: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub must_change_password: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct UserListResponse {
    pub users: Vec<UserResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct AgentResponse {
    #[ts(type = "number")]
    pub id: i64,
    pub hostname: String,
    pub display_name: Option<String>,
    pub agent_version: Option<String>,
    pub agent_git_sha: Option<String>,
    pub agent_build_time: Option<String>,
    pub agent_commit_count: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub default_backup_paths: Vec<String>,
    pub default_exclude_patterns: Vec<String>,
    pub default_pre_backup_commands: String,
    pub default_post_backup_commands: String,
    pub default_file_change_patterns_raw: String,
    pub is_connected: bool,
    pub is_imported: bool,
    pub is_hidden: bool,
    pub supports_restart: bool,
    #[ts(type = "number | null")]
    pub owner_id: Option<i64>,
    pub visibility: String,
    pub restart_unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct CreateAgentResponse {
    pub agent: AgentResponse,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct AgentListResponse {
    pub agents: Vec<AgentResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct MergeAgentResponse {
    pub merged: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DeleteAgentArchivesResponse {
    pub success: bool,
    pub total_deleted: u32,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct HostnamePatternResponse {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub agent_id: i64,
    pub pattern: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RepoResponse {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub repo_path: String,
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: i32,
    #[ts(type = "string")]
    pub compression: Compression,
    #[ts(type = "string")]
    pub encryption: BorgEncryption,
    pub enabled: bool,
    #[ts(type = "number | null")]
    pub owner_id: Option<i64>,
    pub visibility: String,
    pub sync_schedule: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RepoListResponse {
    pub repos: Vec<RepoResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RepoWithStatsResponse {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub repo_path: String,
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: i32,
    pub ssh_host_key: Option<String>,
    #[ts(type = "string")]
    pub compression: Compression,
    #[ts(type = "string")]
    pub encryption: BorgEncryption,
    pub enabled: bool,
    pub importing: bool,
    pub import_error: Option<String>,
    pub import_progress: i32,
    pub import_total: i32,
    pub import_status_message: Option<String>,
    #[ts(type = "number | null")]
    pub owner_id: Option<i64>,
    pub visibility: String,
    pub sync_schedule: Option<String>,
    pub last_synced_at: Option<DateTime<Utc>>,
    #[ts(type = "number")]
    pub archive_count: i64,
    pub last_backup_at: Option<DateTime<Utc>>,
    #[ts(type = "number")]
    pub total_original_size: i64,
    #[ts(type = "number")]
    pub total_compressed_size: i64,
    #[ts(type = "number")]
    pub total_deduplicated_size: i64,
    #[ts(type = "number")]
    pub agent_count: i64,
    #[ts(type = "number")]
    pub unmatched_count: i64,
    pub relocation_pending: bool,
    #[ts(type = "string | null")]
    pub last_op_kind: Option<RepoOpKind>,
    pub last_op_at: Option<DateTime<Utc>>,
    pub last_op_by: Option<String>,
    pub current_op: Option<crate::protocol::ActiveRepoOp>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RepoWithStatsListResponse {
    pub repos: Vec<RepoWithStatsResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct PassphraseResponse {
    pub passphrase: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RepoHostKeyResponse {
    pub ssh_host_key: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct InitRepoResponse {
    pub repo: RepoResponse,
    pub borg_output: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ConfirmRelocationResponse {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct BreakLockResponse {
    pub message: String,
    pub borg_output: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ExecBorgResponse {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct MigrateEncryptionResponse {
    pub success: bool,
    pub message: String,
    pub migrated_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ScheduleResponse {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number | null")]
    pub repo_id: Option<i64>,
    pub name: String,
    #[ts(type = "string")]
    pub schedule_type: ScheduleType,
    pub cron_expression: String,
    pub enabled: bool,
    pub canary_enabled: bool,
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub exclude_patterns_raw: String,
    pub file_change_patterns_raw: String,
    pub ignore_global_excludes: bool,
    pub keep_hourly: i32,
    pub keep_daily: i32,
    pub keep_weekly: i32,
    pub keep_monthly: i32,
    pub keep_yearly: i32,
    pub compact_enabled: bool,
    pub rate_limit_kbps: Option<i32>,
    pub pre_backup_commands: String,
    pub post_backup_commands: String,
    #[ts(type = "string")]
    pub execution_mode: ExecutionMode,
    #[ts(type = "string")]
    pub on_failure: OnFailure,
    #[ts(type = "number | null")]
    pub owner_id: Option<i64>,
    pub visibility: String,
    pub target_hostnames: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ScheduleListResponse {
    pub schedules: Vec<ScheduleResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ScheduleTargetResponse {
    #[ts(type = "number")]
    pub agent_id: i64,
    pub execution_order: i32,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ScheduleBackupSourcesResponse {
    pub backup_sources: Vec<String>,
    pub backup_sources_per_agent: Vec<PerAgentBackupSourcesResponse>,
    pub exclude_patterns_per_agent: Vec<PerAgentExcludePatternsResponse>,
    pub commands_per_agent: Vec<PerAgentCommandsResponse>,
    pub file_change_patterns_per_agent: Vec<PerAgentFileChangePatternsResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct PerAgentBackupSourcesResponse {
    #[ts(type = "number")]
    pub agent_id: i64,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct PerAgentExcludePatternsResponse {
    #[ts(type = "number")]
    pub agent_id: i64,
    pub raw_text: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct PerAgentCommandsResponse {
    #[ts(type = "number")]
    pub agent_id: i64,
    pub pre_backup_commands: String,
    pub post_backup_commands: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct PerAgentFileChangePatternsResponse {
    #[ts(type = "number")]
    pub agent_id: i64,
    pub raw_text: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ReportResponse {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub agent_id: i64,
    #[ts(type = "number")]
    pub repo_id: i64,
    #[ts(type = "number | null")]
    pub schedule_id: Option<i64>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    #[ts(type = "string")]
    pub status: BackupStatus,
    #[ts(type = "number")]
    pub original_size: i64,
    #[ts(type = "number")]
    pub compressed_size: i64,
    #[ts(type = "number")]
    pub deduplicated_size: i64,
    #[ts(type = "number")]
    pub files_processed: i64,
    #[ts(type = "number")]
    pub duration_secs: i64,
    pub error_message: Option<String>,
    pub warnings: Vec<String>,
    pub borg_version: Option<String>,
    pub archive_name: Option<String>,
    pub borg_command: Option<String>,
    pub hostname: Option<String>,
    pub repo_name: Option<String>,
    pub schedule_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ReportListResponse {
    pub reports: Vec<ReportResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ArchiveEntryResponse {
    pub name: String,
    pub start: String,
    pub hostname: String,
    pub comment: String,
    #[ts(type = "number")]
    pub original_size: i64,
    #[ts(type = "number")]
    pub deduplicated_size: i64,
    pub matched: Option<bool>,
    pub agent_hostname: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ArchiveInfoResponse {
    #[ts(type = "number")]
    pub original_size: i64,
    #[ts(type = "number")]
    pub compressed_size: i64,
    #[ts(type = "number")]
    pub deduplicated_size: i64,
    #[ts(type = "number")]
    pub nfiles: i64,
    pub duration: f64,
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ContentEntryResponse {
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub entry_type: String,
    pub path: String,
    #[ts(type = "number")]
    pub size: i64,
    pub mtime: String,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ContentsResponse {
    pub index_status: String,
    pub entries: Vec<ContentEntryResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ArchiveIndexStatusResponse {
    pub status: String,
    #[ts(type = "number | null")]
    pub file_count: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DeleteArchiveResponse {
    pub success: bool,
    pub archive_name: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DiffResponse {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub modified: Vec<String>,
    #[ts(type = "number")]
    pub total_changes: usize,
    #[ts(type = "number")]
    pub limit: usize,
    #[ts(type = "number")]
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct TagResponse {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub color: String,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct TagListResponse {
    pub tags: Vec<TagResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ArchiveTagResponse {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub repo_id: i64,
    pub archive_name: String,
    pub tag: String,
    #[ts(type = "number | null")]
    pub created_by: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ApiTokenResponse {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub user_id: i64,
    pub name: String,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct CreateApiTokenResponse {
    pub token: ApiTokenResponse,
    pub plaintext: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ListApiTokensResponse {
    pub tokens: Vec<ApiTokenResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DeleteApiTokenResponse {
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct TunnelResponse {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub agent_id: i64,
    pub ssh_host: String,
    pub ssh_user: String,
    pub ssh_port: i32,
    pub tunnel_port: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    #[ts(type = "string")]
    pub status: TunnelStatus,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct GroupResponse {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct GroupListResponse {
    pub groups: Vec<GroupResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct GroupMembersResponse {
    #[ts(type = "number[]")]
    pub user_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RoleResponse {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub can_create_agent: bool,
    pub can_delete_agent: bool,
    pub can_delete_own_agent: bool,
    pub can_create_repo: bool,
    pub can_delete_repo: bool,
    pub can_delete_own_repo: bool,
    pub can_create_schedule: bool,
    pub can_delete_schedule: bool,
    pub can_delete_own_schedule: bool,
    pub can_manage_tags: bool,
    pub can_view_all_repos: bool,
    pub can_manage_tunnels: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RoleListResponse {
    pub roles: Vec<RoleResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RepoPermissionResponse {
    #[ts(type = "number")]
    pub user_id: i64,
    #[ts(type = "number")]
    pub repo_id: i64,
    pub can_view: bool,
    pub can_backup: bool,
    pub can_modify_schedules: bool,
    pub can_extract: bool,
    pub can_delete: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct SshPublicKeyResponse {
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct SettingsResponse {
    #[ts(type = "number")]
    pub retention_days: i64,
    #[ts(type = "number")]
    pub report_retention_days: i64,
    #[ts(type = "number")]
    pub failed_report_retention_days: i64,
    #[ts(type = "number")]
    pub system_event_retention_days: i64,
    pub timezone: String,
    #[ts(type = "number")]
    pub borg_query_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DatabaseStorageResponse {
    #[ts(type = "number")]
    pub database_bytes: i64,
    #[ts(type = "number")]
    pub other_bytes: i64,
    pub relations: Vec<DatabaseRelationSizeResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DatabaseRelationSizeResponse {
    pub relation_name: String,
    #[ts(type = "number")]
    pub total_bytes: i64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct VersionResponse {
    pub server_version: String,
    pub server_git_sha: String,
    pub build_timestamp: String,
    pub server_commit_count: Option<u32>,
    pub agent_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct SystemResetResponse {
    #[ts(type = "number")]
    pub cancelled_backups: u64,
    #[ts(type = "number")]
    pub notified_agents: usize,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct AuditEntryResponse {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number | null")]
    pub user_id: Option<i64>,
    pub username: String,
    pub action: String,
    pub target_type: Option<String>,
    #[ts(type = "number | null")]
    pub target_id: Option<i64>,
    #[ts(type = "any")]
    pub details: Option<serde_json::Value>,
    pub ip_address: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct AuditLogResponse {
    pub items: Vec<AuditEntryResponse>,
    #[ts(type = "number")]
    pub total: i64,
    #[ts(type = "number")]
    pub page: i64,
    #[ts(type = "number")]
    pub per_page: i64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct NotificationChannelResponse {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub channel_type: String,
    #[ts(type = "any")]
    pub config: serde_json::Value,
    pub enabled: bool,
    #[ts(type = "any")]
    pub scope: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct NotificationRuleResponse {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub channel_id: i64,
    pub event_type: String,
    #[ts(type = "number | null")]
    pub repo_id: Option<i64>,
    #[ts(type = "number | null")]
    pub agent_id: Option<i64>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct NotificationDeliveryResponse {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub channel_id: i64,
    pub event_type: String,
    #[ts(type = "any")]
    pub payload: serde_json::Value,
    pub status: String,
    pub error_message: Option<String>,
    pub attempted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct VapidKeyResponse {
    pub public_key: String,
    pub configured: bool,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct PushSubscriptionResponse {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub user_id: i64,
    pub endpoint: String,
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RepoQuotaResponse {
    #[ts(type = "number")]
    pub repo_id: i64,
    #[ts(type = "number | null")]
    pub warn_bytes: Option<i64>,
    #[ts(type = "number | null")]
    pub critical_bytes: Option<i64>,
    pub warn_action: QuotaAction,
    pub critical_action: QuotaAction,
    pub enabled: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ServerQuotaResponse {
    pub ssh_host: String,
    #[ts(type = "number")]
    pub repo_count: i64,
    #[ts(type = "number")]
    pub total_deduplicated_size: i64,
    pub configured: bool,
    #[ts(type = "number | null")]
    pub warn_bytes: Option<i64>,
    #[ts(type = "number | null")]
    pub critical_bytes: Option<i64>,
    pub warn_action: QuotaAction,
    pub critical_action: QuotaAction,
    pub enabled: bool,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct GlobalExcludesResponse {
    pub raw_text: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DashboardSummaryResponse {
    #[ts(type = "number")]
    pub online_agents: usize,
    #[ts(type = "number")]
    pub total_agents: i64,
    #[ts(type = "number")]
    pub total_repos: i64,
    pub last_backup_at: Option<DateTime<Utc>>,
    pub next_backup_at: Option<DateTime<Utc>>,
    #[ts(type = "number | null")]
    pub last_backup_schedule_id: Option<i64>,
    #[ts(type = "number | null")]
    pub last_backup_repo_id: Option<i64>,
    pub last_backup_archive_name: Option<String>,
    #[ts(type = "number | null")]
    pub next_backup_schedule_id: Option<i64>,
    #[ts(type = "number")]
    pub active_schedules: i64,
    #[ts(type = "number")]
    pub total_schedules: i64,
    #[ts(type = "number")]
    pub total_storage_bytes: i64,
    #[ts(type = "number")]
    pub success_30d: i64,
    #[ts(type = "number")]
    pub failed_30d: i64,
    #[ts(type = "number")]
    pub total_30d: i64,
    pub storage_by_repo: Vec<StorageRepoEntryResponse>,
    pub last_failure_at: Option<DateTime<Utc>>,
    pub last_warning_at: Option<DateTime<Utc>>,
    #[ts(type = "number | null")]
    pub last_failure_schedule_id: Option<i64>,
    #[ts(type = "number | null")]
    pub last_warning_schedule_id: Option<i64>,
    pub last_failure_message: Option<String>,
    pub last_warning_message: Option<String>,
    #[ts(type = "number | null")]
    pub last_failure_repo_id: Option<i64>,
    #[ts(type = "number | null")]
    pub last_warning_repo_id: Option<i64>,
    pub last_failure_repo_name: Option<String>,
    pub last_warning_repo_name: Option<String>,
    pub last_failure_schedule_name: Option<String>,
    pub last_warning_schedule_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct StorageRepoEntryResponse {
    pub name: String,
    #[ts(type = "number")]
    pub compressed_size: i64,
    #[ts(type = "number")]
    pub deduplicated_size: i64,
    pub percentage: f64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct HealthSummaryResponse {
    #[ts(type = "number")]
    pub repo_id: i64,
    #[ts(type = "number")]
    pub schedule_id: i64,
    pub hostname: String,
    pub target_name: String,
    #[ts(type = "string | null")]
    pub last_status: Option<BackupStatus>,
    pub last_backup_at: Option<DateTime<Utc>>,
    pub is_overdue: bool,
    pub last_error_message: Option<String>,
    pub cron_expression: Option<String>,
    pub schedule_enabled: Option<bool>,
}

/// Alias so that API handlers can use `HealthResponse` while sharing
/// the same definition as `HealthSummaryResponse`.
pub type HealthResponse = HealthSummaryResponse;

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ActivityEntryResponse {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub repo_id: i64,
    #[ts(type = "number | null")]
    pub schedule_id: Option<i64>,
    pub hostname: String,
    #[ts(type = "string")]
    pub status: BackupStatus,
    pub archive_name: Option<String>,
    #[ts(type = "number")]
    pub original_size: i64,
    #[ts(type = "number")]
    pub compressed_size: i64,
    #[ts(type = "number")]
    pub deduplicated_size: i64,
    #[ts(type = "number")]
    pub files_processed: i64,
    #[ts(type = "number")]
    pub duration_secs: i64,
    pub error_message: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct SystemEventResponse {
    #[ts(type = "number")]
    pub id: i64,
    pub event_type: String,
    #[ts(type = "number | null")]
    pub agent_id: Option<i64>,
    pub message: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct CalendarDayResponse {
    pub date: String,
    pub events: Vec<CalendarEventResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct CalendarEventResponse {
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub event_type: String,
    pub status: String,
    pub repo_name: String,
    pub hostname: String,
    pub time: String,
    #[ts(type = "number | null")]
    pub report_id: Option<i64>,
    #[ts(type = "number | null")]
    pub repo_id: Option<i64>,
    #[ts(type = "number | null")]
    pub schedule_id: Option<i64>,
    pub archive_name: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct TrendEntryResponse {
    pub date: String,
    #[ts(type = "number")]
    pub original_size: i64,
    #[ts(type = "number")]
    pub compressed_size: i64,
    #[ts(type = "number")]
    pub deduplicated_size: i64,
    pub dedup_ratio: f64,
    #[ts(type = "number")]
    pub file_count: i64,
    #[ts(type = "number")]
    pub duration_seconds: i64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct StorageTrendEntryResponse {
    pub date: String,
    #[ts(type = "number")]
    pub original_size: i64,
    #[ts(type = "number")]
    pub compressed_size: i64,
    #[ts(type = "number | null")]
    pub deduplicated_size: Option<i64>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct StorageTrendByRepoEntryResponse {
    pub date: String,
    #[ts(type = "number")]
    pub repo_id: i64,
    pub repo_name: String,
    #[ts(type = "number")]
    pub original_size: i64,
    #[ts(type = "number")]
    pub compressed_size: i64,
    #[ts(type = "number | null")]
    pub deduplicated_size: Option<i64>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct StorageStatResponse {
    pub name: String,
    #[ts(type = "number")]
    pub compressed_size: i64,
    #[ts(type = "number")]
    pub deduplicated_size: i64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ScheduleCountByAgentResponse {
    #[ts(type = "number")]
    pub agent_id: i64,
    #[ts(type = "number")]
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DashboardOverviewResponse {
    pub summary: DashboardSummaryCountersResponse,
    pub findings: Vec<DashboardFindingResponse>,
    pub protection: DashboardProtectionCoverageResponse,
    pub running_operations: Vec<DashboardOperationResponse>,
    pub upcoming_schedules: Vec<DashboardUpcomingScheduleResponse>,
    pub repository_capacity: Vec<DashboardRepositoryCapacityResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DashboardSummaryCountersResponse {
    #[ts(type = "number")]
    pub protected_hosts: i64,
    #[ts(type = "number")]
    pub eligible_hosts: i64,
    #[ts(type = "number")]
    pub needs_attention: usize,
    #[ts(type = "number")]
    pub running_operations: usize,
    #[ts(type = "number")]
    pub total_storage_bytes: i64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DashboardFindingResponse {
    pub id: String,
    pub kind: String,
    pub severity: String,
    pub status: String,
    pub hostname: Option<String>,
    #[ts(type = "number | null")]
    pub schedule_id: Option<i64>,
    pub schedule_name: Option<String>,
    #[ts(type = "number | null")]
    pub repo_id: Option<i64>,
    pub repo_name: Option<String>,
    pub reason: String,
    pub occurred_at: Option<DateTime<Utc>>,
    pub deadline: Option<DateTime<Utc>>,
    pub destination: DashboardDestinationResponse,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
#[serde(tag = "kind")]
pub enum DashboardDestinationResponse {
    #[serde(rename = "host")]
    Host { hostname: String },
    #[serde(rename = "schedule")]
    Schedule {
        #[ts(type = "number")]
        schedule_id: i64,
    },
    #[serde(rename = "repository")]
    Repository {
        #[ts(type = "number")]
        repo_id: i64,
    },
    #[serde(rename = "activity")]
    Activity {
        #[ts(type = "number")]
        report_id: i64,
    },
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DashboardProtectionCoverageResponse {
    #[ts(type = "number")]
    pub protected_hosts: i64,
    #[ts(type = "number")]
    pub eligible_hosts: i64,
    pub protected_agent_links: Vec<DashboardAgentLinkResponse>,
    pub unassigned_agents: Vec<DashboardAgentLinkResponse>,
    #[ts(type = "number")]
    pub never_succeeded_targets: i64,
    pub never_succeeded_agents: Vec<DashboardAgentLinkResponse>,
    pub disabled_only_agents: Vec<DashboardAgentLinkResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DashboardAgentLinkResponse {
    #[ts(type = "number")]
    pub agent_id: i64,
    pub hostname: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DashboardOperationResponse {
    #[ts(type = "number")]
    pub report_id: i64,
    pub status: String,
    pub hostname: String,
    #[ts(type = "number")]
    pub schedule_id: i64,
    pub schedule_name: String,
    #[ts(type = "number")]
    pub repo_id: i64,
    pub repo_name: String,
    pub started_at: DateTime<Utc>,
    pub destination: DashboardDestinationResponse,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DashboardUpcomingScheduleResponse {
    #[ts(type = "number")]
    pub schedule_id: i64,
    pub schedule_name: String,
    #[ts(type = "number")]
    pub repo_id: i64,
    pub repo_name: String,
    pub next_run_at: DateTime<Utc>,
    #[ts(type = "number")]
    pub target_count: i64,
    #[ts(type = "number")]
    pub offline_target_count: usize,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DashboardRepositoryCapacityResponse {
    #[ts(type = "number")]
    pub repo_id: i64,
    pub repo_name: String,
    #[ts(type = "number")]
    pub deduplicated_size: i64,
    #[ts(type = "number | null")]
    pub quota_bytes: Option<i64>,
    pub quota_utilization_percent: Option<f64>,
    pub quota_status: String,
    #[ts(type = "number | null")]
    pub storage_change_bytes: Option<i64>,
    pub threshold_estimate: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct LogEntryResponse {
    pub timestamp: DateTime<Utc>,
    pub level: String,
    pub target: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ConfigExportResponse {
    pub version: u32,
    pub exported_at: DateTime<Utc>,
    pub hosts: Vec<HostExportResponse>,
    pub schedules: Vec<ScheduleExportResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct HostExportResponse {
    pub hostname: String,
    pub display_name: Option<String>,
    pub default_backup_paths: Vec<String>,
    pub default_exclude_patterns: Vec<String>,
    pub default_pre_backup_commands: String,
    pub default_post_backup_commands: String,
    pub hostname_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ScheduleExportResponse {
    pub name: String,
    #[ts(type = "string")]
    pub schedule_type: ScheduleType,
    pub cron_expression: String,
    pub enabled: bool,
    pub canary_enabled: bool,
    #[ts(type = "string")]
    pub execution_mode: ExecutionMode,
    #[ts(type = "string")]
    pub on_failure: OnFailure,
    pub exclude_patterns_raw: String,
    pub file_change_patterns_raw: String,
    pub ignore_global_excludes: bool,
    pub keep_hourly: i32,
    pub keep_daily: i32,
    pub keep_weekly: i32,
    pub keep_monthly: i32,
    pub keep_yearly: i32,
    pub compact_enabled: bool,
    pub rate_limit_kbps: Option<i32>,
    pub pre_backup_commands: String,
    pub post_backup_commands: String,
    pub backup_sources: Vec<String>,
    pub target_hostnames: Vec<String>,
    pub repo_name: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ImportResultResponse {
    pub hosts_created: u32,
    pub hosts_updated: u32,
    pub schedules_created: u32,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DeployAgentResponse {
    pub success: bool,
    pub skipped: bool,
    pub token: Option<String>,
    pub available_version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct FetchServiceUnitResponse {
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RestoreFilesResponse {
    pub success: bool,
    #[ts(type = "number")]
    pub files_restored: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DryRunResponse {
    pub files: Vec<DryRunFileEntryResponse>,
    #[ts(type = "number")]
    pub total_size: i64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct DryRunFileEntryResponse {
    pub path: String,
    #[ts(type = "number")]
    pub size: i64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct SearchResponse {
    pub items: Vec<SearchEntry>,
    #[ts(type = "number")]
    pub total_matched: usize,
    #[ts(type = "number")]
    pub limit: usize,
    #[ts(type = "number")]
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct CrossSearchResponse {
    pub items: Vec<CrossSearchEntryResponse>,
    #[ts(type = "number")]
    pub total_archives_searched: usize,
    #[ts(type = "number")]
    pub limit: usize,
    #[ts(type = "number")]
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct CrossSearchEntryResponse {
    pub path: String,
    #[ts(type = "number")]
    pub size: i64,
    pub mtime: DateTime<Utc>,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub entry_type: String,
    pub archive_name: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
#[serde(transparent)]
pub struct PreferencesResponse {
    #[ts(type = "any")]
    pub inner: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RefreshResponse {
    pub session_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct HostnamePatternListResponse {
    pub patterns: Vec<HostnamePatternResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ScheduleTargetListResponse {
    pub targets: Vec<ScheduleTargetResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RepoPermissionListResponse {
    pub permissions: Vec<RepoPermissionResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ActivityListResponse {
    pub items: Vec<ActivityEntryResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct SystemEventListResponse {
    pub events: Vec<SystemEventResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct StorageStatListResponse {
    pub stats: Vec<StorageStatResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ScheduleCountByAgentListResponse {
    pub counts: Vec<ScheduleCountByAgentResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct ArchiveTagListResponse {
    pub tags: Vec<ArchiveTagResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct AgentTagAssociationResponse {
    #[ts(type = "number")]
    pub tag_id: i64,
    #[ts(type = "number")]
    pub agent_id: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RepoTagAssociationResponse {
    #[ts(type = "number")]
    pub tag_id: i64,
    #[ts(type = "number")]
    pub repo_id: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct AgentTagAssociationListResponse {
    pub associations: Vec<AgentTagAssociationResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RepoTagAssociationListResponse {
    pub associations: Vec<RepoTagAssociationResponse>,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct AgentTagEntryResponse {
    #[ts(type = "number")]
    pub agent_id: i64,
    pub tag_name: String,
    pub tag_color: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RepoTagEntryResponse {
    #[ts(type = "number")]
    pub repo_id: i64,
    pub tag_name: String,
    pub tag_color: String,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct RescanResponse {
    #[ts(type = "number")]
    pub matched: u64,
    #[ts(type = "number")]
    pub remaining_unmatched: u64,
}

#[derive(Debug, Clone, Serialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct SyncResponse {
    #[ts(type = "number")]
    pub imported: u64,
    #[ts(type = "number")]
    pub removed: u64,
    #[ts(type = "number")]
    pub duration_secs: u64,
}
