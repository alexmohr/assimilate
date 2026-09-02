// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/// Audit log database queries.
pub mod audit;
/// Dashboard summary queries.
pub mod dashboard;
/// Hostname pattern-matching queries.
pub mod patterns;
/// Quota database queries.
pub mod quota;
/// Backup run power-management event log queries.
pub mod run_events;
/// Server-level quota database queries.
pub mod server_quota;
/// Tag database queries.
pub mod tags;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared::types::{
    AcknowledgedFilter, BackupStatus, ScheduleType, SystemEventSeverity, SystemEventType,
};
use sqlx::PgPool;

use crate::error::ApiError;

/// Exponential backoff durations for account lockout (indexed by escalation level).
pub const LOCKOUT_DURATIONS: &[i64] = &[1, 5, 15, 60, 1440];

/// Sentinel `agent_token_hash` value for imported placeholder agents that have
/// no real authentication token.
pub const IMPORTED_TOKEN_HASH: &str = "imported:no-auth";

/// Result of resolving an agent for a given hostname.
#[derive(Debug, Clone, Serialize)]
pub enum ResolveResult {
    /// An exact hostname match was found.
    ExactMatch(AgentRow),
    /// A glob-pattern match was found.
    PatternMatch(AgentRow),
    /// No matching agent was found.
    Unmatched,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
///
/// A hostname shared by more than one non-placeholder agent (agents in
/// different domains reporting the same OS hostname) can't be resolved from
/// the hostname alone -- callers that need such a report or archive
/// attributed correctly must resolve it manually (hostname patterns or the
/// merge-agent flow), so an ambiguous exact match is treated as
/// [`ResolveResult::Unmatched`] rather than guessing.
pub async fn resolve_agent_for_hostname(
    pool: &PgPool,
    hostname: &str,
) -> Result<ResolveResult, ApiError> {
    let exact_matches = sqlx::query_as!(
        AgentRow,
        "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands AS \
         \"default_pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
         default_post_backup_commands AS \"default_post_backup_commands: \
         sqlx::types::Json<Vec<String>>\", default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user, domain, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup, \
         start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, agent_service_name \
         FROM agents WHERE hostname = $1 AND agent_token_hash != 'imported:no-auth'",
        hostname,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    if let Ok([agent]) = <[AgentRow; 1]>::try_from(exact_matches) {
        return Ok(ResolveResult::ExactMatch(agent));
    }

    if let Some(agent) = patterns::find_agent_by_pattern(pool, hostname).await? {
        return Ok(ResolveResult::PatternMatch(agent));
    }

    Ok(ResolveResult::Unmatched)
}

/// Takes a `SELECT ... FOR UPDATE` lock on every schedule that currently targets
/// `agent_id`, so a concurrent single-statement `UPDATE schedules ...` for one of
/// them - most importantly `record_schedule_failure`, run by an in-flight scheduler
/// tick - blocks until this transaction commits or rolls back, instead of racing it.
///
/// Without this, a schedule that crosses the auto-disable threshold for this agent
/// (setting `auto_disabled_by_agent_id = agent_id`, `auto_disabled_agent_unreachable =
/// true`, `consecutive_failures = N`) in a transaction that commits after this
/// transaction's own bookkeeping-reset check ran but before its destructive step
/// (deleting or merging away the agent) would end up permanently stale: the
/// destructive step's `ON DELETE SET NULL` FK only clears `auto_disabled_by_agent_id`,
/// leaving `auto_disabled_agent_unreachable`/`consecutive_failures` stuck, and since
/// the causing agent is gone, no reconnect can ever un-stick it.
///
/// Must run before anything in the same transaction that would stop `agent_id` from
/// still being a target of these schedules (e.g. retargeting `schedule_targets`) -
/// otherwise the lookup that finds which rows to lock races the same way.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
async fn lock_schedules_targeting_agent(
    tx: &mut sqlx::PgConnection,
    agent_id: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "SELECT s.id FROM schedules s JOIN schedule_targets st ON st.schedule_id = s.id WHERE \
         st.agent_id = $1 FOR UPDATE OF s",
        agent_id,
    )
    .fetch_all(tx)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::BadRequest`]: the request is invalid
pub async fn merge_agent(pool: &PgPool, source_id: i64, target_id: i64) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    let source = sqlx::query_as!(
        AgentRow,
        "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands AS \
         \"default_pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
         default_post_backup_commands AS \"default_post_backup_commands: \
         sqlx::types::Json<Vec<String>>\", default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user, domain, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup, \
         start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, agent_service_name \
         FROM agents WHERE id = $1",
        source_id,
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    let Some(source) = source else {
        return Err(ApiError::NotFound(format!(
            "source agent {source_id} not found"
        )));
    };

    let has_imported_token = sqlx::query_scalar!(
        "SELECT agent_token_hash FROM agents WHERE id = $1",
        source.id
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    if has_imported_token != IMPORTED_TOKEN_HASH {
        return Err(ApiError::BadRequest(
            "source agent does not have imported:no-auth token".to_string(),
        ));
    }

    // Must run before the schedule_targets retarget below, while source_id can still be
    // found via a schedule_targets join - see lock_schedules_targeting_agent's doc comment.
    lock_schedules_targeting_agent(&mut tx, source_id).await?;

    sqlx::query!(
        "UPDATE backup_reports SET agent_id = $1, matched = true WHERE agent_id = $2",
        target_id,
        source_id,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    sqlx::query!(
        "UPDATE schedule_targets SET agent_id = $1 WHERE agent_id = $2",
        target_id,
        source_id,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    sqlx::query!(
        "INSERT INTO agent_tags (agent_id, tag_id) SELECT $1, tag_id FROM agent_tags WHERE \
         agent_id = $2 ON CONFLICT DO NOTHING",
        target_id,
        source_id,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    sqlx::query!("DELETE FROM agent_tags WHERE agent_id = $1", source_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

    // See delete_agent's comment: same auto-disable bookkeeping must be cleared here too,
    // since the source agent's row disappears the same way (FK only nulls
    // auto_disabled_by_agent_id, leaving the rest stale).
    sqlx::query!(
        "UPDATE schedules SET auto_disabled_agent_unreachable = false, auto_disabled_by_agent_id \
         = NULL, consecutive_failures = 0, failure_streak_pure_connectivity = true WHERE \
         auto_disabled_by_agent_id = $1 AND auto_disabled_agent_unreachable = true",
        source_id
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    sqlx::query!("DELETE FROM agents WHERE id = $1", source_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

/// A row from the `agents` table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the agents table's columns via query_as!, not \
              mutually-exclusive states; splitting into enums or sub-structs would require \
              restructuring every query_as! call site around these columns for no correctness \
              benefit -- the API layer (AgentResponse) already nests the power-related ones"
)]
pub struct AgentRow {
    /// Unique identifier.
    pub id: i64,
    /// Agent hostname.
    pub hostname: String,
    /// Optional display name.
    pub display_name: Option<String>,
    /// Agent binary version string.
    pub agent_version: Option<String>,
    /// Git SHA of the agent build.
    pub agent_git_sha: Option<String>,
    /// Agent build timestamp.
    pub agent_build_time: Option<String>,
    /// Git commit count at build time.
    pub agent_commit_count: Option<i32>,
    /// When the agent was first registered.
    pub created_at: DateTime<Utc>,
    /// When the agent last connected.
    pub last_seen_at: Option<DateTime<Utc>>,
    /// Owning user ID, if any.
    pub owner_id: Option<i64>,
    /// Visibility scope.
    pub visibility: String,
    /// Default backup paths for schedules targeting this agent.
    #[serde(default)]
    pub default_backup_paths: Vec<String>,
    /// Default exclude patterns for schedules targeting this agent.
    #[serde(default)]
    pub default_exclude_patterns: Vec<String>,
    /// Default pre-backup commands.
    #[schema(value_type = Vec<String>)]
    pub default_pre_backup_commands: sqlx::types::Json<Vec<String>>,
    /// Default post-backup commands.
    #[schema(value_type = Vec<String>)]
    pub default_post_backup_commands: sqlx::types::Json<Vec<String>>,
    /// Default file-change detection patterns (raw text).
    #[serde(default)]
    pub default_file_change_patterns_raw: String,
    /// Hash of the agent's authentication token (never serialized).
    #[serde(skip)]
    pub agent_token_hash: String,
    /// Whether the agent is hidden from the UI.
    #[serde(default)]
    pub is_hidden: bool,
    /// SSH username last used to deploy/upgrade this agent.
    pub last_ssh_user: Option<String>,
    /// Optional DNS domain, set by an admin to disambiguate agents that
    /// share an OS hostname across different networks.
    pub domain: Option<String>,
    /// Whether to send a Wake-on-LAN packet before a backup if the agent
    /// isn't already reachable.
    pub wake_enabled: bool,
    /// MAC address to wake, required when `wake_enabled`.
    pub wake_mac_address: Option<String>,
    /// Broadcast address the magic packet is sent to (defaults to the
    /// global broadcast address when unset).
    pub wake_broadcast_address: Option<String>,
    /// How long to wait for the host to come online after waking it.
    pub wake_timeout_seconds: i32,
    /// Whether to shut the host down after the backup, but only if this run
    /// is what woke it.
    pub shutdown_after_backup: bool,
    /// Whether to start the agent process over SSH before a backup if it
    /// isn't already connected once the host is up.
    pub start_agent_enabled: bool,
    /// Whether to stop the agent process after the backup, but only if this
    /// run is what started it.
    pub stop_agent_after_backup: bool,
    /// SSH hostname used to start/stop the agent process and to shut the
    /// host down, required when `start_agent_enabled`.
    pub ssh_host: Option<String>,
    /// SSH port for `ssh_host`.
    pub ssh_port: i32,
    /// Name of the systemd unit managing the agent process.
    pub agent_service_name: String,
}

/// A row from the `repos` table (sensitive fields excluded).
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct RepoRow {
    /// Unique identifier.
    pub id: i64,
    /// Repository display name.
    pub name: String,
    /// Borg repository path on the remote host.
    pub repo_path: String,
    /// SSH user for the remote host.
    pub ssh_user: String,
    /// SSH hostname for the remote host.
    pub ssh_host: String,
    /// SSH port for the remote host.
    pub ssh_port: i32,
    /// Compression algorithm (e.g. "lz4", "zstd").
    pub compression: String,
    /// Encryption mode (e.g. "repokey-blake2").
    pub encryption: String,
    /// Whether the repository is enabled for backups.
    pub enabled: bool,
    /// Owning user ID, if any.
    pub owner_id: Option<i64>,
    /// Visibility scope.
    pub visibility: String,
    /// Optional cron expression for automatic sync.
    pub sync_schedule: Option<String>,
    /// Whether to send a Wake-on-LAN packet before a backup if the
    /// repository host isn't already reachable over SSH.
    pub wake_enabled: bool,
    /// MAC address to wake, required when `wake_enabled`.
    pub wake_mac_address: Option<String>,
    /// Broadcast address the magic packet is sent to (defaults to the
    /// global broadcast address when unset).
    pub wake_broadcast_address: Option<String>,
    /// How long to wait for the host to come online after waking it.
    pub wake_timeout_seconds: i32,
    /// Whether to shut the host down after the backup, but only if this run
    /// is what woke it.
    pub shutdown_after_backup: bool,
}

/// SSH connection details for a repository (passphrase omitted).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RepoConnectionRow {
    /// SSH user for the remote host.
    pub ssh_user: String,
    /// SSH hostname for the remote host.
    pub ssh_host: String,
    /// SSH port for the remote host.
    pub ssh_port: i32,
}

/// A row from the `ssh_tunnels` table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SshTunnel {
    /// Unique identifier.
    pub id: i64,
    /// Agent that owns this tunnel.
    pub agent_id: i64,
    /// SSH hostname for the tunnel destination.
    pub ssh_host: String,
    /// SSH user for the tunnel destination.
    pub ssh_user: String,
    /// SSH port for the tunnel destination.
    pub ssh_port: i32,
    /// Local port the tunnel listens on.
    pub tunnel_port: i32,
    /// Known host key of the destination.
    pub ssh_host_key: Option<String>,
    /// Whether the tunnel is enabled.
    pub enabled: bool,
    /// When the tunnel was created.
    pub created_at: DateTime<Utc>,
}

/// Parameters for creating a new SSH tunnel.
#[derive(Debug, Deserialize)]
pub struct NewSshTunnel {
    /// Agent that will own this tunnel.
    pub agent_id: i64,
    /// SSH hostname for the tunnel destination.
    pub ssh_host: String,
    /// SSH user for the tunnel destination.
    pub ssh_user: String,
    /// SSH port (defaults to 22).
    pub ssh_port: Option<i32>,
    /// Local port the tunnel will listen on.
    pub tunnel_port: i32,
    /// Whether the tunnel is enabled (defaults to true).
    pub enabled: Option<bool>,
    /// Known host key of the destination.
    pub ssh_host_key: Option<String>,
}

/// Parameters for updating an existing SSH tunnel (all fields optional).
#[derive(Debug, Deserialize)]
pub struct UpdateSshTunnel {
    /// New SSH hostname.
    pub ssh_host: Option<String>,
    /// New SSH user.
    pub ssh_user: Option<String>,
    /// New SSH port.
    pub ssh_port: Option<i32>,
    /// New local tunnel port.
    pub tunnel_port: Option<i32>,
    /// New enabled state.
    pub enabled: Option<bool>,
    /// New known host key.
    pub ssh_host_key: Option<String>,
}

/// Global exclude patterns applied to all schedules.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct GlobalExcludesConfig {
    /// Raw exclude pattern text.
    pub raw_text: String,
}

/// A row from the `schedules` table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct ScheduleRow {
    /// Unique identifier.
    pub id: i64,
    /// Target repository ID.
    pub repo_id: Option<i64>,
    /// Schedule display name.
    pub name: String,
    /// Schedule type (e.g. "cron", "interval").
    pub schedule_type: String,
    /// Cron expression for scheduling.
    pub cron_expression: String,
    /// Whether the schedule is enabled.
    pub enabled: bool,
    /// Whether canary backups are enabled.
    pub canary_enabled: bool,
    /// When the schedule last ran.
    pub last_run_at: Option<DateTime<Utc>>,
    /// When the schedule is next due.
    pub next_run_at: Option<DateTime<Utc>>,
    /// Raw exclude pattern text.
    pub exclude_patterns_raw: String,
    /// Raw file-change detection pattern text.
    pub file_change_patterns_raw: String,
    /// Whether to ignore global exclude patterns.
    pub ignore_global_excludes: bool,
    /// Number of hourly backups to keep.
    pub keep_hourly: i32,
    /// Number of daily backups to keep.
    pub keep_daily: i32,
    /// Number of weekly backups to keep.
    pub keep_weekly: i32,
    /// Number of monthly backups to keep.
    pub keep_monthly: i32,
    /// Number of yearly backups to keep.
    pub keep_yearly: i32,
    /// Whether automatic compaction is enabled.
    pub compact_enabled: bool,
    /// Rate limit in KB/s, if any.
    pub rate_limit_kbps: Option<i32>,
    /// Pre-backup commands.
    #[schema(value_type = Vec<String>)]
    pub pre_backup_commands: sqlx::types::Json<Vec<String>>,
    /// Post-backup commands.
    #[schema(value_type = Vec<String>)]
    pub post_backup_commands: sqlx::types::Json<Vec<String>>,
    /// Timeout in seconds applied to each pre/post-backup hook command.
    pub hook_timeout_seconds: i32,
    /// How many consecutive missed backups this schedule tolerates before it
    /// is marked failed and auto-disabled. Below this count, a miss only
    /// shows as a warning.
    pub missed_backup_threshold: i32,
    /// Execution mode (e.g. "sequential").
    pub execution_mode: String,
    /// On-failure behaviour (e.g. "continue", "abort").
    pub on_failure: String,
    /// Owning user ID, if any.
    pub owner_id: Option<i64>,
    /// Visibility scope.
    pub visibility: String,
    /// Hostnames of target agents, resolved at query time.
    #[serde(default)]
    #[sqlx(default)]
    pub target_hostnames: Vec<String>,
    /// How many consecutive attempts have failed to reach the schedule's target
    /// agent(s) since the last success (or reconnect). Reset to 0 on success, on
    /// reconnect from the causing agent, or on any direct edit of `enabled`.
    pub consecutive_failures: i32,
    /// Whether the scheduler itself disabled this schedule after
    /// `consecutive_failures` reached the threshold, specifically because its
    /// target agent stayed unreachable - as opposed to a local/data failure (e.g. a
    /// corrupted passphrase), a human, or quota enforcement. Only ever true while
    /// `enabled` is false; the schedule re-enables automatically once the causing
    /// agent reconnects.
    pub auto_disabled_agent_unreachable: bool,
}

/// A row from the `schedule_targets` join table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ScheduleTargetRow {
    /// Agent ID.
    pub agent_id: i64,
    /// Execution order among targets for the same schedule.
    pub execution_order: i32,
}

/// Number of schedules targeting a specific agent.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ScheduleCountByAgent {
    /// Agent ID.
    pub agent_id: i64,
    /// Number of distinct schedules targeting this agent.
    pub count: i64,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_schedule_counts_by_agent(
    pool: &PgPool,
) -> Result<Vec<ScheduleCountByAgent>, ApiError> {
    sqlx::query_as!(
        ScheduleCountByAgent,
        "SELECT agent_id, COUNT(DISTINCT schedule_id)::bigint AS \"count!\" FROM schedule_targets \
         GROUP BY agent_id",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Looks up an agent by hostname, optionally narrowed to a specific
/// `domain`.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: no agent matches
/// - [`ApiError::Conflict`]: `domain` was omitted and more than one agent
///   shares `hostname` across different domains -- the caller must specify
///   one
/// - [`ApiError::Database`]: the database query fails
pub async fn get_agent_by_hostname(
    pool: &PgPool,
    hostname: &str,
    domain: Option<&str>,
) -> Result<AgentRow, ApiError> {
    if let Some(domain) = domain {
        return sqlx::query_as!(
            AgentRow,
            "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
             agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
             default_backup_paths, default_exclude_patterns, default_pre_backup_commands AS \
             \"default_pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
             default_post_backup_commands AS \"default_post_backup_commands: \
             sqlx::types::Json<Vec<String>>\", default_file_change_patterns_raw, \
             agent_token_hash, is_hidden, last_ssh_user, domain, wake_enabled, wake_mac_address, \
             wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup, \
             start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, agent_service_name \
             FROM agents WHERE hostname = $1 AND domain = $2",
            hostname,
            domain,
        )
        .fetch_one(pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => {
                ApiError::NotFound(format!("agent '{hostname}' in domain '{domain}' not found"))
            }
            other => ApiError::Database(other),
        });
    }

    let matches = sqlx::query_as!(
        AgentRow,
        "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands AS \
         \"default_pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
         default_post_backup_commands AS \"default_post_backup_commands: \
         sqlx::types::Json<Vec<String>>\", default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user, domain, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup, \
         start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, agent_service_name \
         FROM agents WHERE hostname = $1 ORDER BY domain",
        hostname,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    match <[AgentRow; 1]>::try_from(matches) {
        Ok([agent]) => Ok(agent),
        Err(matches) if matches.is_empty() => {
            Err(ApiError::NotFound(format!("agent '{hostname}' not found")))
        }
        Err(matches) => {
            let domains: Vec<String> = matches
                .iter()
                .map(|a| a.domain.clone().unwrap_or_default())
                .collect();
            Err(ApiError::Conflict(format!(
                "hostname '{hostname}' is shared by agents in domains {domains:?}; specify a \
                 domain"
            )))
        }
    }
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_agent_by_id(pool: &PgPool, agent_id: i64) -> Result<AgentRow, ApiError> {
    sqlx::query_as!(
        AgentRow,
        "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands AS \
         \"default_pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
         default_post_backup_commands AS \"default_post_backup_commands: \
         sqlx::types::Json<Vec<String>>\", default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user, domain, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup, \
         start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, agent_service_name \
         FROM agents WHERE id = $1",
        agent_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent id '{agent_id}' not found")),
        other => ApiError::Database(other),
    })
}

/// Candidate agent identity for token verification during connection.
#[derive(sqlx::FromRow)]
pub struct AgentTokenCandidate {
    /// Agent ID.
    pub id: i64,
    /// Hash of the agent's authentication token.
    pub agent_token_hash: String,
}

/// Returns every agent registered under `hostname`. A connecting agent only
/// reports its OS hostname, never its domain, so when more than one agent
/// shares a hostname (agents in different domains), the caller must verify
/// the presented token against each candidate to find the right one.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: no agent is registered under `hostname`
/// - [`ApiError::Database`]: the database query fails
pub async fn get_agent_token_hashes(
    pool: &PgPool,
    hostname: &str,
) -> Result<Vec<AgentTokenCandidate>, ApiError> {
    let rows = sqlx::query_as!(
        AgentTokenCandidate,
        "SELECT id, agent_token_hash FROM agents WHERE hostname = $1",
        hostname
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    if rows.is_empty() {
        return Err(ApiError::NotFound(format!("agent '{hostname}' not found")));
    }

    Ok(rows)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_last_seen(pool: &PgPool, agent_id: i64) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE agents SET last_seen_at = NOW() WHERE id = $1",
        agent_id
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_last_seen_and_version(
    pool: &PgPool,
    agent_id: i64,
    agent_version: &str,
    agent_git_sha: Option<&str>,
    agent_build_time: Option<&str>,
    agent_commit_count: Option<i32>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE agents SET last_seen_at = NOW(), agent_version = $2, agent_git_sha = $3, \
         agent_build_time = $4, agent_commit_count = $5 WHERE id = $1",
        agent_id,
        agent_version,
        agent_git_sha,
        agent_build_time,
        agent_commit_count,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_last_ssh_user(
    pool: &PgPool,
    agent_id: i64,
    ssh_user: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE agents SET last_ssh_user = $2 WHERE id = $1",
        agent_id,
        ssh_user,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_last_seen_by_hostname(pool: &PgPool, hostname: &str) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE agents SET last_seen_at = NOW() WHERE hostname = $1",
        hostname
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_agents(pool: &PgPool, include_hidden: bool) -> Result<Vec<AgentRow>, ApiError> {
    if include_hidden {
        sqlx::query_as!(
            AgentRow,
            "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
             agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
             default_backup_paths, default_exclude_patterns, default_pre_backup_commands AS \
             \"default_pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
             default_post_backup_commands AS \"default_post_backup_commands: \
             sqlx::types::Json<Vec<String>>\", default_file_change_patterns_raw, \
             agent_token_hash, is_hidden, last_ssh_user, domain, wake_enabled, wake_mac_address, \
             wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup, \
             start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, agent_service_name \
             FROM agents ORDER BY hostname",
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    } else {
        sqlx::query_as!(
            AgentRow,
            "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
             agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
             default_backup_paths, default_exclude_patterns, default_pre_backup_commands AS \
             \"default_pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
             default_post_backup_commands AS \"default_post_backup_commands: \
             sqlx::types::Json<Vec<String>>\", default_file_change_patterns_raw, \
             agent_token_hash, is_hidden, last_ssh_user, domain, wake_enabled, wake_mac_address, \
             wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup, \
             start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, agent_service_name \
             FROM agents WHERE is_hidden = false ORDER BY hostname",
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    }
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn set_agent_hidden(
    pool: &PgPool,
    agent_id: i64,
    hidden: bool,
) -> Result<AgentRow, ApiError> {
    sqlx::query_as!(
        AgentRow,
        "UPDATE agents SET is_hidden = $2 WHERE id = $1 RETURNING id, hostname, display_name, \
         agent_version, agent_git_sha, agent_build_time, agent_commit_count, created_at, \
         last_seen_at, owner_id, visibility, default_backup_paths, default_exclude_patterns, \
         default_pre_backup_commands AS \"default_pre_backup_commands: \
         sqlx::types::Json<Vec<String>>\", default_post_backup_commands AS \
         \"default_post_backup_commands: sqlx::types::Json<Vec<String>>\", \
         default_file_change_patterns_raw, agent_token_hash, is_hidden, last_ssh_user, domain, \
         wake_enabled, wake_mac_address, wake_broadcast_address, wake_timeout_seconds, \
         shutdown_after_backup, start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, \
         agent_service_name",
        agent_id,
        hidden,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?
    .ok_or_else(|| ApiError::NotFound(format!("Agent id '{agent_id}' not found")))
}

/// Finds an agent by hostname, or creates a placeholder agent for archive imports.
///
/// Placeholder agents have a dummy token hash and cannot authenticate. They serve
/// only as a foreign key target for imported `backup_reports`.
///
/// If more than one agent already shares `hostname` (agents in different
/// domains), which one an archive belongs to can't be inferred from the
/// hostname alone, so a fresh placeholder is created instead of guessing; a
/// human resolves the ambiguity afterwards via hostname patterns or merge.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_or_create_agent_by_hostname(
    pool: &PgPool,
    hostname: &str,
) -> Result<AgentRow, ApiError> {
    let existing = sqlx::query_as!(
        AgentRow,
        "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands AS \
         \"default_pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
         default_post_backup_commands AS \"default_post_backup_commands: \
         sqlx::types::Json<Vec<String>>\", default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user, domain, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup, \
         start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, agent_service_name \
         FROM agents WHERE hostname = $1",
        hostname,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    if let Ok([agent]) = <[AgentRow; 1]>::try_from(existing) {
        return Ok(agent);
    }

    sqlx::query_as!(
        AgentRow,
        "INSERT INTO agents (hostname, display_name, agent_token_hash, owner_id) VALUES ($1, $2, \
         $3, NULL) RETURNING id, hostname, display_name, agent_version, agent_git_sha, \
         agent_build_time, agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands AS \
         \"default_pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
         default_post_backup_commands AS \"default_post_backup_commands: \
         sqlx::types::Json<Vec<String>>\", default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user, domain, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup, \
         start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, agent_service_name",
        hostname,
        Some(format!("{hostname} (imported)")),
        "imported:no-auth",
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_agent(
    pool: &PgPool,
    hostname: &str,
    display_name: Option<&str>,
    token_hash: &str,
    owner_id: Option<i64>,
    domain: Option<&str>,
) -> Result<AgentRow, ApiError> {
    sqlx::query_as!(
        AgentRow,
        "INSERT INTO agents (hostname, display_name, agent_token_hash, owner_id, domain) VALUES \
         ($1, $2, $3, $4, $5) RETURNING id, hostname, display_name, agent_version, agent_git_sha, \
         agent_build_time, agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands AS \
         \"default_pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
         default_post_backup_commands AS \"default_post_backup_commands: \
         sqlx::types::Json<Vec<String>>\", default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user, domain, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup, \
         start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, agent_service_name",
        hostname,
        display_name,
        token_hash,
        owner_id,
        domain,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// Default configuration values for an agent.
pub struct AgentDefaults<'a> {
    /// Optional display name.
    pub display_name: Option<&'a str>,
    /// Optional DNS domain, to disambiguate agents that share a hostname.
    pub domain: Option<&'a str>,
    /// Default backup paths.
    pub default_backup_paths: &'a [String],
    /// Default exclude patterns.
    pub default_exclude_patterns: &'a [String],
    /// Default pre-backup commands.
    pub default_pre_backup_commands: &'a [String],
    /// Default post-backup commands.
    pub default_post_backup_commands: &'a [String],
    /// Default file-change detection patterns (raw text).
    pub default_file_change_patterns_raw: &'a str,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_agent_with_paths(
    pool: &PgPool,
    hostname: &str,
    token_hash: &str,
    defaults: AgentDefaults<'_>,
) -> Result<AgentRow, ApiError> {
    sqlx::query_as!(
        AgentRow,
        "INSERT INTO agents (hostname, display_name, agent_token_hash, default_backup_paths, \
         default_exclude_patterns, default_pre_backup_commands, default_post_backup_commands, \
         default_file_change_patterns_raw, domain) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         RETURNING id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands AS \
         \"default_pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
         default_post_backup_commands AS \"default_post_backup_commands: \
         sqlx::types::Json<Vec<String>>\", default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user, domain, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup, \
         start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, agent_service_name",
        hostname,
        defaults.display_name,
        token_hash,
        defaults.default_backup_paths,
        defaults.default_exclude_patterns,
        sqlx::types::Json(defaults.default_pre_backup_commands) as _,
        sqlx::types::Json(defaults.default_post_backup_commands) as _,
        defaults.default_file_change_patterns_raw,
        defaults.domain,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn update_agent(
    pool: &PgPool,
    agent_id: i64,
    new_hostname: &str,
    defaults: AgentDefaults<'_>,
) -> Result<AgentRow, ApiError> {
    sqlx::query_as!(
        AgentRow,
        "UPDATE agents SET hostname = $2, display_name = $3, default_backup_paths = $4, \
         default_exclude_patterns = $5, default_pre_backup_commands = $6, \
         default_post_backup_commands = $7, default_file_change_patterns_raw = $8, domain = $9 \
         WHERE id = $1 RETURNING id, hostname, display_name, agent_version, agent_git_sha, \
         agent_build_time, agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands AS \
         \"default_pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
         default_post_backup_commands AS \"default_post_backup_commands: \
         sqlx::types::Json<Vec<String>>\", default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user, domain, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup, \
         start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, agent_service_name",
        agent_id,
        new_hostname,
        defaults.display_name,
        defaults.default_backup_paths,
        defaults.default_exclude_patterns,
        sqlx::types::Json(defaults.default_pre_backup_commands) as _,
        sqlx::types::Json(defaults.default_post_backup_commands) as _,
        defaults.default_file_change_patterns_raw,
        defaults.domain,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent id '{agent_id}' not found")),
        other => ApiError::Database(other),
    })
}

/// Power-management configuration for an agent's host: waking it
/// (Wake-on-LAN) and starting/stopping the agent process over SSH around a
/// backup. Kept separate from [`AgentDefaults`] -- unlike backup defaults,
/// these settings are never touched by the config import/export flow.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring AgentRow's own columns, not mutually-exclusive states; \
              splitting into enums or sub-structs would require restructuring the query_as! call \
              in update_agent_power for no correctness benefit"
)]
pub struct AgentPowerPatch<'a> {
    /// Whether to send a Wake-on-LAN packet before a backup if the agent
    /// isn't already reachable.
    pub wake_enabled: bool,
    /// MAC address to wake, required when `wake_enabled`.
    pub wake_mac_address: Option<&'a str>,
    /// Broadcast address the magic packet is sent to.
    pub wake_broadcast_address: Option<&'a str>,
    /// How long to wait for the host to come online after waking it.
    pub wake_timeout_seconds: i32,
    /// Whether to shut the host down after the backup, but only if this run
    /// is what woke it.
    pub shutdown_after_backup: bool,
    /// Whether to start the agent process over SSH before a backup if it
    /// isn't already connected once the host is up.
    pub start_agent_enabled: bool,
    /// Whether to stop the agent process after the backup, but only if this
    /// run is what started it.
    pub stop_agent_after_backup: bool,
    /// SSH hostname used to start/stop the agent process and to shut the
    /// host down, required when `start_agent_enabled`.
    pub ssh_host: Option<&'a str>,
    /// SSH port for `ssh_host`.
    pub ssh_port: i32,
    /// Name of the systemd unit managing the agent process.
    pub agent_service_name: &'a str,
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn update_agent_power(
    pool: &PgPool,
    agent_id: i64,
    power: AgentPowerPatch<'_>,
) -> Result<AgentRow, ApiError> {
    sqlx::query_as!(
        AgentRow,
        "UPDATE agents SET wake_enabled = $2, wake_mac_address = $3, wake_broadcast_address = $4, \
         wake_timeout_seconds = $5, shutdown_after_backup = $6, start_agent_enabled = $7, \
         stop_agent_after_backup = $8, ssh_host = $9, ssh_port = $10, agent_service_name = $11 \
         WHERE id = $1 RETURNING id, hostname, display_name, agent_version, agent_git_sha, \
         agent_build_time, agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands AS \
         \"default_pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
         default_post_backup_commands AS \"default_post_backup_commands: \
         sqlx::types::Json<Vec<String>>\", default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user, domain, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup, \
         start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, agent_service_name",
        agent_id,
        power.wake_enabled,
        power.wake_mac_address,
        power.wake_broadcast_address,
        power.wake_timeout_seconds,
        power.shutdown_after_backup,
        power.start_agent_enabled,
        power.stop_agent_after_backup,
        power.ssh_host,
        power.ssh_port,
        power.agent_service_name,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent id '{agent_id}' not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn regenerate_agent_token(
    pool: &PgPool,
    agent_id: i64,
    token_hash: &str,
) -> Result<AgentRow, ApiError> {
    sqlx::query_as!(
        AgentRow,
        "UPDATE agents SET agent_token_hash = $2 WHERE id = $1 RETURNING id, hostname, \
         display_name, agent_version, agent_git_sha, agent_build_time, agent_commit_count, \
         created_at, last_seen_at, owner_id, visibility, default_backup_paths, \
         default_exclude_patterns, default_pre_backup_commands AS \"default_pre_backup_commands: \
         sqlx::types::Json<Vec<String>>\", default_post_backup_commands AS \
         \"default_post_backup_commands: sqlx::types::Json<Vec<String>>\", \
         default_file_change_patterns_raw, agent_token_hash, is_hidden, last_ssh_user, domain, \
         wake_enabled, wake_mac_address, wake_broadcast_address, wake_timeout_seconds, \
         shutdown_after_backup, start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, \
         agent_service_name",
        agent_id,
        token_hash,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent id '{agent_id}' not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn mark_agent_reports_matched(pool: &PgPool, agent_id: i64) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE backup_reports SET matched = true WHERE agent_id = $1 AND matched = false",
        agent_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn delete_agent(pool: &PgPool, agent_id: i64) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    // See lock_schedules_targeting_agent's doc comment: without this, a schedule that
    // crosses the auto-disable threshold for this agent concurrently (e.g. an
    // in-flight scheduler tick) could race the bookkeeping-reset UPDATE below and end
    // up permanently stale once DELETE FROM agents runs.
    lock_schedules_targeting_agent(&mut tx, agent_id).await?;

    // Clears the auto-disable bookkeeping the same way
    // reset_schedule_failure_tracking_if_target_dropped does for retargeting - a
    // deleted agent can never appear in a later PUT's old-vs-new target diff, so that
    // function alone can't reach this case. Otherwise deleting the agent that caused a
    // schedule's auto-disable would leave consecutive_failures/auto_disabled_agent_unreachable
    // stale (the FK only nulls auto_disabled_by_agent_id).
    sqlx::query!(
        "UPDATE schedules SET auto_disabled_agent_unreachable = false, auto_disabled_by_agent_id \
         = NULL, consecutive_failures = 0, failure_streak_pure_connectivity = true WHERE \
         auto_disabled_by_agent_id = $1 AND auto_disabled_agent_unreachable = true",
        agent_id
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    let result = sqlx::query!("DELETE FROM agents WHERE id = $1", agent_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!(
            "agent id '{agent_id}' not found"
        )));
    }

    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_archives_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<(shared::types::RepoId, Vec<String>)>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        repo_id: i64,
        archive_name: Option<String>,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT repo_id, archive_name FROM backup_reports WHERE agent_id = $1 AND archive_name IS \
         NOT NULL",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    let mut map: std::collections::HashMap<i64, Vec<String>> = std::collections::HashMap::new();
    for row in rows {
        if let Some(name) = row.archive_name {
            map.entry(row.repo_id).or_default().push(name);
        }
    }

    Ok(map
        .into_iter()
        .map(|(repo_id, names)| (shared::types::RepoId(repo_id), names))
        .collect())
}

#[derive(sqlx::FromRow)]
struct AgentArchiveRow {
    repo_id: i64,
    archive_name: Option<String>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_archives_for_agent_with_patterns(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<(shared::types::RepoId, Vec<String>)>, ApiError> {
    let patterns = patterns::list_patterns_for_agent(pool, agent_id).await?;

    let mut agent_ids = vec![agent_id];

    if !patterns.is_empty() {
        #[derive(sqlx::FromRow)]
        struct IdHostname {
            id: i64,
            hostname: String,
        }

        let all_agents = sqlx::query_as!(
            IdHostname,
            "SELECT id, hostname FROM agents WHERE id != $1",
            agent_id
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)?;

        for a in &all_agents {
            let hostname_base = a
                .hostname
                .strip_suffix(" (imported)")
                .unwrap_or(&a.hostname);
            if patterns
                .iter()
                .any(|p| glob_match::glob_match(&p.pattern, hostname_base))
            {
                agent_ids.push(a.id);
            }
        }
    }

    let rows = sqlx::query_as!(
        AgentArchiveRow,
        "SELECT repo_id, archive_name FROM backup_reports WHERE agent_id = ANY($1::bigint[]) AND \
         archive_name IS NOT NULL",
        &agent_ids,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    let mut map: std::collections::HashMap<i64, Vec<String>> = std::collections::HashMap::new();
    for row in rows {
        if let Some(name) = row.archive_name {
            map.entry(row.repo_id).or_default().push(name);
        }
    }

    Ok(map
        .into_iter()
        .map(|(repo_id, names)| (shared::types::RepoId(repo_id), names))
        .collect())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_schedule_target_agents_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<Vec<ScheduleRunTarget>, ApiError> {
    sqlx::query_as!(
        ScheduleRunTarget,
        "SELECT DISTINCT a.id AS agent_id, a.hostname FROM agents a JOIN schedule_targets st ON \
         st.agent_id = a.id JOIN schedules s ON s.id = st.schedule_id WHERE s.repo_id = $1",
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Parameters for inserting a new repository.
pub struct InsertRepoParams<'a> {
    /// Repository display name.
    pub name: &'a str,
    /// Borg repository path on the remote host.
    pub repo_path: &'a str,
    /// SSH user for the remote host.
    pub ssh_user: &'a str,
    /// SSH hostname for the remote host.
    pub ssh_host: &'a str,
    /// SSH port for the remote host.
    pub ssh_port: i32,
    /// Encrypted passphrase bytes.
    pub passphrase_encrypted: &'a [u8],
    /// Compression algorithm.
    pub compression: &'a str,
    /// Encryption mode.
    pub encryption: &'a str,
    /// Owning user ID, if any.
    pub owner_id: Option<i64>,
    /// Cron expression for automatic repository sync.
    /// `None` = use DB default; `Some(Some(s))` = set value; `Some(None)` = disable.
    pub sync_schedule: Option<Option<&'a str>>,
}

/// Parameters for updating an existing repository.
pub struct UpdateRepoParams<'a> {
    /// Repository ID to update.
    pub repo_id: i64,
    /// New display name.
    pub name: &'a str,
    /// New borg repository path.
    pub repo_path: &'a str,
    /// New SSH user.
    pub ssh_user: &'a str,
    /// New SSH hostname.
    pub ssh_host: &'a str,
    /// New SSH port.
    pub ssh_port: i32,
    /// New compression algorithm.
    pub compression: &'a str,
    /// New encryption mode.
    pub encryption: &'a str,
    /// Whether the repository is enabled.
    pub enabled: bool,
    /// New sync schedule cron expression.
    /// `None` = keep existing value; `Some(None)` = disable; `Some(Some(s))` = set to `s`.
    pub sync_schedule: Option<Option<&'a str>>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_importing_repo_ids(pool: &PgPool) -> Result<Vec<i64>, ApiError> {
    let rows = sqlx::query_scalar!("SELECT repo_id FROM repo_import_state WHERE importing = true")
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(rows)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_repo_importing(
    pool: &PgPool,
    repo_id: i64,
    importing: bool,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_import_state (repo_id, importing) VALUES ($1, $2) ON CONFLICT (repo_id) \
         DO UPDATE SET importing = EXCLUDED.importing",
        repo_id,
        importing
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Guards `repo_import_state.importing` for the duration of a sync/import
/// operation, clearing it back to `false` on drop - including during a panic
/// unwind - so a panic partway through the sync work can't leave a repo stuck
/// showing "syncing" forever. Unlike [`crate::repo_op_tracker::RepoOpTracker`]'s
/// in-memory active-op tracking (which already has a panic-safe
/// `RepoOpGuard`), this DB-persisted flag had no such net: a scheduled repo
/// sync skips any repo already marked `importing`, so a stuck flag also
/// permanently blocks that repo's own periodic sync, not just the UI's
/// "syncing" indicator.
///
/// `Drop` can't await, so cleanup runs as a spawned task (mirroring
/// `RepoOpGuard`); call [`Self::clear_now`] on the normal-completion path so
/// the flag is cleared before you return, instead of racing the spawned task.
/// Calling `clear_now` disarms the `Drop` cleanup, so a concurrent operation
/// that legitimately re-sets `importing = true` for this `repo_id` right
/// after can't be clobbered by a stale deferred clear.
pub struct ImportingGuard {
    pool: PgPool,
    repo_id: i64,
    task_registry: shared::task_registry::TaskRegistry,
    cleared: bool,
}

impl ImportingGuard {
    /// Sets `importing = true` for `repo_id` and returns a guard that clears
    /// it back to `false` on drop.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError::Database`] if the database query fails.
    pub async fn acquire(
        pool: &PgPool,
        repo_id: i64,
        task_registry: shared::task_registry::TaskRegistry,
    ) -> Result<Self, ApiError> {
        set_repo_importing(pool, repo_id, true).await?;
        Ok(Self {
            pool: pool.clone(),
            repo_id,
            task_registry,
            cleared: false,
        })
    }

    /// Clears the importing flag immediately, awaiting the write instead of
    /// leaving it to the guard's deferred `Drop` cleanup.
    pub async fn clear_now(mut self) {
        match set_repo_importing(&self.pool, self.repo_id, false).await {
            Ok(()) => self.cleared = true,
            Err(e) => {
                tracing::error!(
                    repo_id = self.repo_id,
                    error = %e,
                    "failed to clear importing flag"
                );
            }
        }
    }
}

impl Drop for ImportingGuard {
    fn drop(&mut self) {
        if self.cleared {
            return;
        }
        let pool = self.pool.clone();
        let repo_id = self.repo_id;
        let handle = tokio::spawn(async move {
            if let Err(e) = set_repo_importing(&pool, repo_id, false).await {
                tracing::error!(
                    repo_id,
                    error = %e,
                    "failed to clear importing flag on guard drop"
                );
            }
        });
        self.task_registry.register(handle);
    }
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_repo_import_error(
    pool: &PgPool,
    repo_id: i64,
    error: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_import_state (repo_id, error) VALUES ($1, $2) ON CONFLICT (repo_id) DO \
         UPDATE SET error = EXCLUDED.error",
        repo_id,
        error
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_import_status_message(
    pool: &PgPool,
    repo_id: i64,
    msg: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_import_state (repo_id, status_message) VALUES ($1, $2) ON CONFLICT \
         (repo_id) DO UPDATE SET status_message = EXCLUDED.status_message",
        repo_id,
        msg
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_repo_import_progress(
    pool: &PgPool,
    repo_id: i64,
    progress: i64,
    total: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_import_state (repo_id, progress, total) VALUES ($1, $2, $3) ON CONFLICT \
         (repo_id) DO UPDATE SET progress = EXCLUDED.progress, total = EXCLUDED.total",
        repo_id,
        i32::try_from(progress).unwrap_or(i32::MAX),
        i32::try_from(total).unwrap_or(i32::MAX),
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_repo_last_synced(pool: &PgPool, repo_id: i64) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_stats (repo_id, last_synced_at) VALUES ($1, NOW()) ON CONFLICT \
         (repo_id) DO UPDATE SET last_synced_at = EXCLUDED.last_synced_at",
        repo_id
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Returns `true` if the agent is linked to the repo via at least one
/// schedule target (i.e., the agent is assigned a schedule whose target
/// repo matches `repo_id`).
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn check_agent_repo_access(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
) -> Result<bool, ApiError> {
    sqlx::query_scalar!(
        "SELECT EXISTS(
           SELECT 1 FROM schedule_targets st
           JOIN schedules s ON s.id = st.schedule_id
           WHERE st.agent_id = $1 AND s.repo_id = $2
         ) AS \"exists!\"",
        agent_id,
        repo_id,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// Authoritative repository statistics parsed from `borg info --json`
/// (`cache.stats`) and `borg list --json`. This is the single source of truth
/// for repo size and archive counts; never derive these from `backup_reports`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RepoInfoStats {
    /// Total original (uncompressed) size in bytes.
    pub original_size: i64,
    /// Total compressed size in bytes.
    pub compressed_size: i64,
    /// Total deduplicated size in bytes.
    pub deduplicated_size: i64,
    /// Total number of chunks.
    pub total_chunks: i64,
    /// Number of unique chunks.
    pub unique_chunks: i64,
    /// Number of archives in the repository.
    pub archive_count: i64,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_repo_info_stats(
    pool: &PgPool,
    repo_id: i64,
    stats: &RepoInfoStats,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_stats (repo_id, original_size, compressed_size, deduplicated_size, \
         total_chunks, unique_chunks, archive_count, updated_at) VALUES ($1, $2, $3, $4, $5, $6, \
         $7, NOW()) ON CONFLICT (repo_id) DO UPDATE SET original_size = EXCLUDED.original_size, \
         compressed_size = EXCLUDED.compressed_size, deduplicated_size = \
         EXCLUDED.deduplicated_size, total_chunks = EXCLUDED.total_chunks, unique_chunks = \
         EXCLUDED.unique_chunks, archive_count = EXCLUDED.archive_count, updated_at = \
         EXCLUDED.updated_at",
        repo_id,
        stats.original_size,
        stats.compressed_size,
        stats.deduplicated_size,
        stats.total_chunks,
        stats.unique_chunks,
        i32::try_from(stats.archive_count).unwrap_or(i32::MAX),
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn clear_relocation_pending(pool: &PgPool, repo_id: i64) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    sqlx::query!(
        "DELETE FROM repo_relocation_pending_hosts WHERE repo_id = $1",
        repo_id
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;
    sqlx::query!(
        "UPDATE repos SET relocation_pending = false WHERE id = $1",
        repo_id
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;
    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

/// Remove `hostname` from the pending-hosts set for this repo. Clears `relocation_pending`
/// on the repo itself once every registered host has confirmed the new location.
///
/// Only clears the flag when this host's entry was actually present (`rows_affected` > 0) AND
/// no other hosts remain pending. This prevents spurious clears when a host that was never
/// registered in the pending table completes a backup.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn clear_relocation_for_host(
    pool: &PgPool,
    repo_id: i64,
    hostname: &str,
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    let deleted = sqlx::query!(
        "DELETE FROM repo_relocation_pending_hosts WHERE repo_id = $1 AND hostname = $2",
        repo_id,
        hostname,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    if deleted.rows_affected() > 0 {
        let remaining: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*)::BIGINT AS \"COUNT!\" FROM repo_relocation_pending_hosts WHERE \
             repo_id = $1",
            repo_id
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

        if remaining == 0 {
            sqlx::query!(
                "UPDATE repos SET relocation_pending = false WHERE id = $1",
                repo_id
            )
            .execute(&mut *tx)
            .await
            .map_err(ApiError::Database)?;
        }
    }
    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_relocation_pending(pool: &PgPool, repo_id: i64) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    sqlx::query!(
        "UPDATE repos SET relocation_pending = true WHERE id = $1",
        repo_id
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;
    sqlx::query!(
        "INSERT INTO repo_relocation_pending_hosts (repo_id, hostname) SELECT $1, a.hostname FROM \
         agents a JOIN schedule_targets st ON st.agent_id = a.id JOIN schedules s ON s.id = \
         st.schedule_id WHERE s.repo_id = $1 ON CONFLICT DO NOTHING",
        repo_id,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;
    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_repo_encryption(
    pool: &PgPool,
    repo_id: i64,
    encryption: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE repos SET encryption = $2 WHERE id = $1",
        repo_id,
        encryption
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_repo(
    pool: &PgPool,
    params: &InsertRepoParams<'_>,
) -> Result<RepoRow, ApiError> {
    let Some(sync_schedule) = params.sync_schedule else {
        return sqlx::query_as!(
            RepoRow,
            "INSERT INTO repos (name, repo_path, ssh_user, ssh_host, ssh_port, \
             passphrase_encrypted, compression, encryption, owner_id) VALUES ($1, $2, $3, $4, $5, \
             $6, $7, $8, $9) RETURNING id, name, repo_path, ssh_user, ssh_host, ssh_port, \
             compression, encryption, enabled, owner_id, visibility, sync_schedule, wake_enabled, \
             wake_mac_address, wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup",
            params.name,
            params.repo_path,
            params.ssh_user,
            params.ssh_host,
            params.ssh_port,
            params.passphrase_encrypted,
            params.compression,
            params.encryption,
            params.owner_id,
        )
        .fetch_one(pool)
        .await
        .map_err(ApiError::Database);
    };

    sqlx::query_as!(
        RepoRow,
        "INSERT INTO repos (name, repo_path, ssh_user, ssh_host, ssh_port, passphrase_encrypted, \
         compression, encryption, owner_id, sync_schedule) VALUES ($1, $2, $3, $4, $5, $6, $7, \
         $8, $9, $10) RETURNING id, name, repo_path, ssh_user, ssh_host, ssh_port, compression, \
         encryption, enabled, owner_id, visibility, sync_schedule, wake_enabled, \
         wake_mac_address, wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup",
        params.name,
        params.repo_path,
        params.ssh_user,
        params.ssh_host,
        params.ssh_port,
        params.passphrase_encrypted,
        params.compression,
        params.encryption,
        params.owner_id,
        sync_schedule,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_repo_by_id(pool: &PgPool, repo_id: i64) -> Result<RepoRow, ApiError> {
    sqlx::query_as!(
        RepoRow,
        "SELECT id, name, repo_path, ssh_user, ssh_host, ssh_port, compression, encryption, \
         enabled, owner_id, visibility, sync_schedule, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup FROM repos WHERE id \
         = $1",
        repo_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_repo_connection(
    pool: &PgPool,
    repo_id: i64,
) -> Result<RepoConnectionRow, ApiError> {
    sqlx::query_as!(
        RepoConnectionRow,
        "SELECT ssh_user, ssh_host, ssh_port FROM repos WHERE id = $1",
        repo_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn update_repo(
    pool: &PgPool,
    params: &UpdateRepoParams<'_>,
) -> Result<RepoRow, ApiError> {
    let Some(sync_schedule) = params.sync_schedule else {
        return sqlx::query_as!(
            RepoRow,
            "UPDATE repos SET name = $2, repo_path = $3, ssh_user = $4, ssh_host = $5, ssh_port = \
             $6, compression = $7, encryption = $8, enabled = $9 WHERE id = $1 RETURNING id, \
             name, repo_path, ssh_user, ssh_host, ssh_port, compression, encryption, enabled, \
             owner_id, visibility, sync_schedule, wake_enabled, wake_mac_address, \
             wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup",
            params.repo_id,
            params.name,
            params.repo_path,
            params.ssh_user,
            params.ssh_host,
            params.ssh_port,
            params.compression,
            params.encryption,
            params.enabled,
        )
        .fetch_one(pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => {
                ApiError::NotFound(format!("repo {} not found", params.repo_id))
            }
            other => ApiError::Database(other),
        });
    };

    sqlx::query_as!(
        RepoRow,
        "UPDATE repos SET name = $2, repo_path = $3, ssh_user = $4, ssh_host = $5, ssh_port = $6, \
         compression = $7, encryption = $8, enabled = $9, sync_schedule = $10 WHERE id = $1 \
         RETURNING id, name, repo_path, ssh_user, ssh_host, ssh_port, compression, encryption, \
         enabled, owner_id, visibility, sync_schedule, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup",
        params.repo_id,
        params.name,
        params.repo_path,
        params.ssh_user,
        params.ssh_host,
        params.ssh_port,
        params.compression,
        params.encryption,
        params.enabled,
        sync_schedule,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => {
            ApiError::NotFound(format!("repo {} not found", params.repo_id))
        }
        other => ApiError::Database(other),
    })
}

/// Power-management configuration for the host a repository lives on:
/// waking it (Wake-on-LAN) and shutting it back down around a backup. A
/// repository host has no agent process, so unlike [`AgentPowerPatch`] there
/// is no start/stop-agent counterpart here.
pub struct RepoPowerPatch<'a> {
    /// Whether to send a Wake-on-LAN packet before a backup if the
    /// repository host isn't already reachable over SSH.
    pub wake_enabled: bool,
    /// MAC address to wake, required when `wake_enabled`.
    pub wake_mac_address: Option<&'a str>,
    /// Broadcast address the magic packet is sent to.
    pub wake_broadcast_address: Option<&'a str>,
    /// How long to wait for the host to come online after waking it.
    pub wake_timeout_seconds: i32,
    /// Whether to shut the host down after the backup, but only if this run
    /// is what woke it.
    pub shutdown_after_backup: bool,
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn update_repo_power(
    pool: &PgPool,
    repo_id: i64,
    power: RepoPowerPatch<'_>,
) -> Result<RepoRow, ApiError> {
    sqlx::query_as!(
        RepoRow,
        "UPDATE repos SET wake_enabled = $2, wake_mac_address = $3, wake_broadcast_address = $4, \
         wake_timeout_seconds = $5, shutdown_after_backup = $6 WHERE id = $1 RETURNING id, name, \
         repo_path, ssh_user, ssh_host, ssh_port, compression, encryption, enabled, owner_id, \
         visibility, sync_schedule, wake_enabled, wake_mac_address, wake_broadcast_address, \
         wake_timeout_seconds, shutdown_after_backup",
        repo_id,
        power.wake_enabled,
        power.wake_mac_address,
        power.wake_broadcast_address,
        power.wake_timeout_seconds,
        power.shutdown_after_backup,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
        other => ApiError::Database(other),
    })
}

/// Like [`update_repo`] but atomically sets `relocation_pending = true` and registers all
/// currently-scheduled agents as pending confirmation in the same transaction. Use this when
/// the repository location (host, port, or path) has changed so the scheduler never observes
/// the new path with the flag still `false`.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn update_repo_and_set_relocation_pending(
    pool: &PgPool,
    params: &UpdateRepoParams<'_>,
) -> Result<RepoRow, ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    let repo = if let Some(sync_schedule) = params.sync_schedule {
        sqlx::query_as!(
            RepoRow,
            "UPDATE repos SET name = $2, repo_path = $3, ssh_user = $4, ssh_host = $5, ssh_port = \
             $6, compression = $7, encryption = $8, enabled = $9, sync_schedule = $10, \
             relocation_pending = true WHERE id = $1 RETURNING id, name, repo_path, ssh_user, \
             ssh_host, ssh_port, compression, encryption, enabled, owner_id, visibility, \
             sync_schedule, wake_enabled, wake_mac_address, wake_broadcast_address, \
             wake_timeout_seconds, shutdown_after_backup",
            params.repo_id,
            params.name,
            params.repo_path,
            params.ssh_user,
            params.ssh_host,
            params.ssh_port,
            params.compression,
            params.encryption,
            params.enabled,
            sync_schedule,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => {
                ApiError::NotFound(format!("repo {} not found", params.repo_id))
            }
            other => ApiError::Database(other),
        })?
    } else {
        sqlx::query_as!(
            RepoRow,
            "UPDATE repos SET name = $2, repo_path = $3, ssh_user = $4, ssh_host = $5, ssh_port = \
             $6, compression = $7, encryption = $8, enabled = $9, relocation_pending = true WHERE \
             id = $1 RETURNING id, name, repo_path, ssh_user, ssh_host, ssh_port, compression, \
             encryption, enabled, owner_id, visibility, sync_schedule, wake_enabled, \
             wake_mac_address, wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup",
            params.repo_id,
            params.name,
            params.repo_path,
            params.ssh_user,
            params.ssh_host,
            params.ssh_port,
            params.compression,
            params.encryption,
            params.enabled,
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => {
                ApiError::NotFound(format!("repo {} not found", params.repo_id))
            }
            other => ApiError::Database(other),
        })?
    };

    sqlx::query!(
        "INSERT INTO repo_relocation_pending_hosts (repo_id, hostname) SELECT $1, a.hostname FROM \
         agents a JOIN schedule_targets st ON st.agent_id = a.id JOIN schedules s ON s.id = \
         st.schedule_id WHERE s.repo_id = $1 ON CONFLICT DO NOTHING",
        params.repo_id,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    tx.commit().await.map_err(ApiError::Database)?;
    Ok(repo)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn delete_repo(pool: &PgPool, repo_id: i64) -> Result<(), ApiError> {
    // Clears the auto-disable bookkeeping the same way set_schedule_enabled does for
    // any other direct `enabled` write - otherwise a schedule auto-disabled for an
    // unreachable agent keeps auto_disabled_by_agent_id pointing at that agent even
    // after its repo (and thus this schedule's only reason to run) is gone, so a
    // later reconnect from that agent would silently flip enabled back to true on an
    // orphaned schedule that nobody decided to re-enable.
    sqlx::query!(
        "UPDATE schedules SET enabled = false, auto_disabled_agent_unreachable = false, \
         auto_disabled_by_agent_id = NULL, consecutive_failures = 0, \
         failure_streak_pure_connectivity = true WHERE repo_id = $1",
        repo_id
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;

    let result = sqlx::query!("DELETE FROM repos WHERE id = $1", repo_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("repo {repo_id} not found")));
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_enabled_tunnels(pool: &PgPool) -> Result<Vec<SshTunnel>, ApiError> {
    sqlx::query_as!(
        SshTunnel,
        "SELECT id, agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, ssh_host_key, enabled, \
         created_at FROM ssh_tunnels WHERE enabled = true ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_all_tunnels(pool: &PgPool) -> Result<Vec<SshTunnel>, ApiError> {
    sqlx::query_as!(
        SshTunnel,
        "SELECT id, agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, ssh_host_key, enabled, \
         created_at FROM ssh_tunnels ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_tunnel_by_id(pool: &PgPool, id: i64) -> Result<SshTunnel, ApiError> {
    sqlx::query_as!(
        SshTunnel,
        "SELECT id, agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, ssh_host_key, enabled, \
         created_at FROM ssh_tunnels WHERE id = $1",
        id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("ssh tunnel {id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_tunnel_by_agent_id(pool: &PgPool, agent_id: i64) -> Result<SshTunnel, ApiError> {
    sqlx::query_as!(
        SshTunnel,
        "SELECT id, agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, ssh_host_key, enabled, \
         created_at FROM ssh_tunnels WHERE agent_id = $1",
        agent_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => {
            ApiError::NotFound(format!("ssh tunnel for agent {agent_id} not found"))
        }
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_tunnel(pool: &PgPool, params: &NewSshTunnel) -> Result<SshTunnel, ApiError> {
    sqlx::query_as!(
        SshTunnel,
        "INSERT INTO ssh_tunnels (agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, enabled, \
         ssh_host_key) VALUES ($1, $2, $3, COALESCE($4, 22), $5, COALESCE($6, true), $7) \
         RETURNING id, agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, ssh_host_key, \
         enabled, created_at",
        params.agent_id,
        params.ssh_host,
        params.ssh_user,
        params.ssh_port,
        params.tunnel_port,
        params.enabled,
        params.ssh_host_key,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn update_tunnel(
    pool: &PgPool,
    id: i64,
    params: &UpdateSshTunnel,
) -> Result<SshTunnel, ApiError> {
    sqlx::query_as!(
        SshTunnel,
        "UPDATE ssh_tunnels SET ssh_host = COALESCE($2, ssh_host), ssh_user = COALESCE($3, \
         ssh_user), ssh_port = COALESCE($4, ssh_port), tunnel_port = COALESCE($5, tunnel_port), \
         enabled = COALESCE($6, enabled), ssh_host_key = COALESCE($7, ssh_host_key) WHERE id = $1 \
         RETURNING id, agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, ssh_host_key, \
         enabled, created_at",
        id,
        params.ssh_host,
        params.ssh_user,
        params.ssh_port,
        params.tunnel_port,
        params.enabled,
        params.ssh_host_key,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("ssh tunnel {id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_tunnel_ssh_host_key(
    pool: &PgPool,
    id: i64,
    ssh_host_key: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE ssh_tunnels SET ssh_host_key = $2 WHERE id = $1",
        id,
        ssh_host_key,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn delete_tunnel(pool: &PgPool, id: i64) -> Result<(), ApiError> {
    let result = sqlx::query!("DELETE FROM ssh_tunnels WHERE id = $1", id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("ssh tunnel {id} not found")));
    }
    Ok(())
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn update_repo_passphrase(
    pool: &PgPool,
    repo_id: i64,
    passphrase_encrypted: &[u8],
) -> Result<(), ApiError> {
    let result = sqlx::query!(
        "UPDATE repos SET passphrase_encrypted = $2 WHERE id = $1",
        repo_id,
        passphrase_encrypted,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("repo {repo_id} not found")));
    }
    Ok(())
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_repo_passphrase(pool: &PgPool, repo_id: i64) -> Result<Vec<u8>, ApiError> {
    sqlx::query_scalar!(
        "SELECT passphrase_encrypted FROM repos WHERE id = $1",
        repo_id
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_repo_with_passphrase(
    pool: &PgPool,
    repo_id: i64,
) -> Result<RepoWithPassphraseRow, ApiError> {
    sqlx::query_as!(
        RepoWithPassphraseRow,
        "SELECT id, name, repo_path, ssh_user, ssh_host, ssh_port, ssh_host_key, \
         passphrase_encrypted, compression, encryption, enabled, relocation_pending, \
         sync_schedule FROM repos WHERE id = $1",
        repo_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_repo_ssh_host_key(
    pool: &PgPool,
    repo_id: i64,
    ssh_host_key: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE repos SET ssh_host_key = $2 WHERE id = $1",
        repo_id,
        ssh_host_key,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_repo_ssh_host_key(pool: &PgPool, name: &str) -> Result<Option<String>, ApiError> {
    let row = sqlx::query_scalar!("SELECT ssh_host_key FROM repos WHERE name = $1", name,)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(row.flatten())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_global_excludes_raw(pool: &PgPool) -> Result<String, ApiError> {
    let row: Option<String> =
        sqlx::query_scalar!("SELECT raw_text FROM excludes_global_config LIMIT 1")
            .fetch_optional(pool)
            .await
            .map_err(ApiError::Database)?;
    Ok(row.unwrap_or_default())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_global_excludes_raw(pool: &PgPool, raw_text: &str) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO excludes_global_config (raw_text) VALUES ($1) ON CONFLICT (id) DO UPDATE SET \
         raw_text = EXCLUDED.raw_text",
        raw_text,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_schedules(pool: &PgPool) -> Result<Vec<ScheduleRow>, ApiError> {
    let rows = sqlx::query_as!(
        ScheduleRow,
        "SELECT s.id, s.repo_id, s.name, s.schedule_type, s.cron_expression, s.enabled, \
         s.canary_enabled, s.last_run_at, s.next_run_at, s.exclude_patterns_raw, \
         s.file_change_patterns_raw, s.ignore_global_excludes, s.keep_hourly, s.keep_daily, \
         s.keep_weekly, s.keep_monthly, s.keep_yearly, s.compact_enabled, s.rate_limit_kbps, \
         s.pre_backup_commands AS \"pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
         s.post_backup_commands AS \"post_backup_commands: sqlx::types::Json<Vec<String>>\", \
         s.hook_timeout_seconds, s.missed_backup_threshold, s.execution_mode, s.on_failure, \
         s.owner_id, s.visibility, s.consecutive_failures, s.auto_disabled_agent_unreachable, \
         ARRAY(SELECT a.hostname FROM schedule_targets st JOIN agents a ON a.id = st.agent_id \
         WHERE st.schedule_id = s.id ORDER BY st.execution_order, a.hostname) AS \
         \"target_hostnames!\" FROM schedules s ORDER BY s.id",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows)
}

/// Parameters for creating or updating a schedule.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct ScheduleParams<'a> {
    /// Schedule display name.
    pub name: &'a str,
    /// Schedule type (e.g. "cron").
    pub schedule_type: &'a str,
    /// Cron expression.
    pub cron_expression: &'a str,
    /// Whether the schedule is enabled.
    pub enabled: bool,
    /// Whether canary backups are enabled.
    pub canary_enabled: bool,
    /// Raw exclude pattern text.
    pub exclude_patterns_raw: &'a str,
    /// Whether to ignore global excludes.
    pub ignore_global_excludes: bool,
    /// Hourly retention count.
    pub keep_hourly: i32,
    /// Daily retention count.
    pub keep_daily: i32,
    /// Weekly retention count.
    pub keep_weekly: i32,
    /// Monthly retention count.
    pub keep_monthly: i32,
    /// Yearly retention count.
    pub keep_yearly: i32,
    /// Whether automatic compaction is enabled.
    pub compact_enabled: bool,
    /// Rate limit in KB/s.
    pub rate_limit_kbps: Option<i32>,
    /// Pre-backup commands.
    pub pre_backup_commands: &'a [String],
    /// Post-backup commands.
    pub post_backup_commands: &'a [String],
    /// Timeout in seconds applied to each pre/post-backup hook command.
    pub hook_timeout_seconds: i32,
    /// How many consecutive missed backups this schedule tolerates before it
    /// is marked failed and auto-disabled.
    pub missed_backup_threshold: i32,
    /// On-failure behaviour.
    pub on_failure: &'a str,
    /// Raw file-change detection pattern text.
    pub file_change_patterns_raw: &'a str,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_schedule(
    pool: &PgPool,
    repo_id: i64,
    params: &ScheduleParams<'_>,
    owner_id: Option<i64>,
) -> Result<ScheduleRow, ApiError> {
    sqlx::query_as!(
        ScheduleRow,
        "INSERT INTO schedules (repo_id, name, schedule_type, cron_expression, enabled, \
         canary_enabled, exclude_patterns_raw, file_change_patterns_raw, ignore_global_excludes, \
         keep_hourly, keep_daily, keep_weekly, keep_monthly, keep_yearly, compact_enabled, \
         rate_limit_kbps, pre_backup_commands, post_backup_commands, execution_mode, on_failure, \
         owner_id, hook_timeout_seconds, missed_backup_threshold) VALUES ($1, $2, $3, $4, $5, $6, \
         $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, 'sequential', $19, $20, $21, \
         $22) RETURNING id, repo_id, name, schedule_type, cron_expression, enabled, \
         canary_enabled, last_run_at, next_run_at, exclude_patterns_raw, \
         file_change_patterns_raw, ignore_global_excludes, keep_hourly, keep_daily, keep_weekly, \
         keep_monthly, keep_yearly, compact_enabled, rate_limit_kbps, pre_backup_commands AS \
         \"pre_backup_commands: sqlx::types::Json<Vec<String>>\", post_backup_commands AS \
         \"post_backup_commands: sqlx::types::Json<Vec<String>>\", hook_timeout_seconds, \
         missed_backup_threshold, execution_mode, on_failure, owner_id, visibility, \
         consecutive_failures, auto_disabled_agent_unreachable, ARRAY[]::TEXT[] AS \
         \"target_hostnames!\"",
        repo_id,
        params.name,
        params.schedule_type,
        params.cron_expression,
        params.enabled,
        params.canary_enabled,
        params.exclude_patterns_raw,
        params.file_change_patterns_raw,
        params.ignore_global_excludes,
        params.keep_hourly,
        params.keep_daily,
        params.keep_weekly,
        params.keep_monthly,
        params.keep_yearly,
        params.compact_enabled,
        params.rate_limit_kbps,
        sqlx::types::Json(params.pre_backup_commands) as _,
        sqlx::types::Json(params.post_backup_commands) as _,
        params.on_failure,
        owner_id,
        params.hook_timeout_seconds,
        params.missed_backup_threshold,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn update_schedule(
    pool: &PgPool,
    id: i64,
    params: &ScheduleParams<'_>,
) -> Result<ScheduleRow, ApiError> {
    // Unlike set_schedule_enabled (only ever called to explicitly flip `enabled`),
    // this backs the general edit-schedule form, which resubmits `enabled` unchanged
    // on every save of any other field (rename, retention tweak, cron change, ...).
    // Clearing the auto-disable bookkeeping unconditionally here would let an
    // unrelated edit permanently strand an auto-disabled schedule: the reconnect
    // handler only matches on auto_disabled_by_agent_id, so once this cleared it,
    // reconnecting would never re-enable the schedule again. Only clear it when
    // `enabled` is actually transitioning - i.e. a human explicitly toggled it
    // through this same form - the same "direct write from outside the failure-
    // tracking path" trigger set_schedule_enabled uses.
    sqlx::query_as!(
        ScheduleRow,
        "UPDATE schedules SET name = $2, cron_expression = $3, enabled = $4, canary_enabled = $5, \
         exclude_patterns_raw = $6, file_change_patterns_raw = $7, ignore_global_excludes = $8, \
         keep_hourly = $9, keep_daily = $10, keep_weekly = $11, keep_monthly = $12, keep_yearly = \
         $13, compact_enabled = $14, rate_limit_kbps = $15, pre_backup_commands = $16, \
         post_backup_commands = $17, execution_mode = 'sequential', on_failure = $18, \
         hook_timeout_seconds = $19, missed_backup_threshold = $20, \
         auto_disabled_agent_unreachable = CASE WHEN enabled IS DISTINCT FROM $4 THEN false ELSE \
         auto_disabled_agent_unreachable END, auto_disabled_by_agent_id = CASE WHEN enabled IS \
         DISTINCT FROM $4 THEN NULL ELSE auto_disabled_by_agent_id END, consecutive_failures = \
         CASE WHEN enabled IS DISTINCT FROM $4 THEN 0 ELSE consecutive_failures END, \
         failure_streak_pure_connectivity = CASE WHEN enabled IS DISTINCT FROM $4 THEN true ELSE \
         failure_streak_pure_connectivity END WHERE id = $1 RETURNING id, repo_id, name, \
         schedule_type, cron_expression, enabled, canary_enabled, last_run_at, next_run_at, \
         exclude_patterns_raw, file_change_patterns_raw, ignore_global_excludes, keep_hourly, \
         keep_daily, keep_weekly, keep_monthly, keep_yearly, compact_enabled, rate_limit_kbps, \
         pre_backup_commands AS \"pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
         post_backup_commands AS \"post_backup_commands: sqlx::types::Json<Vec<String>>\", \
         hook_timeout_seconds, missed_backup_threshold, execution_mode, on_failure, owner_id, \
         visibility, consecutive_failures, auto_disabled_agent_unreachable, ARRAY[]::TEXT[] AS \
         \"target_hostnames!\"",
        id,
        params.name,
        params.cron_expression,
        params.enabled,
        params.canary_enabled,
        params.exclude_patterns_raw,
        params.file_change_patterns_raw,
        params.ignore_global_excludes,
        params.keep_hourly,
        params.keep_daily,
        params.keep_weekly,
        params.keep_monthly,
        params.keep_yearly,
        params.compact_enabled,
        params.rate_limit_kbps,
        sqlx::types::Json(params.pre_backup_commands) as _,
        sqlx::types::Json(params.post_backup_commands) as _,
        params.on_failure,
        params.hook_timeout_seconds,
        params.missed_backup_threshold,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("schedule {id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn update_schedule_repo(pool: &PgPool, id: i64, repo_id: i64) -> Result<(), ApiError> {
    let rows_affected = sqlx::query!(
        "UPDATE schedules SET repo_id = $2 WHERE id = $1",
        id,
        repo_id
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?
    .rows_affected();
    if rows_affected == 0 {
        return Err(ApiError::NotFound(format!("schedule {id} not found")));
    }
    Ok(())
}

/// A row from the `repos` table including the encrypted passphrase.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RepoWithPassphraseRow {
    /// Unique identifier.
    pub id: i64,
    /// Repository display name.
    pub name: String,
    /// Borg repository path on the remote host.
    pub repo_path: String,
    /// SSH user for the remote host.
    pub ssh_user: String,
    /// SSH hostname for the remote host.
    pub ssh_host: String,
    /// SSH port for the remote host.
    pub ssh_port: i32,
    /// Known host key of the remote host.
    pub ssh_host_key: Option<String>,
    /// Encrypted passphrase bytes.
    pub passphrase_encrypted: Vec<u8>,
    /// Compression algorithm.
    pub compression: String,
    /// Encryption mode.
    pub encryption: String,
    /// Whether the repository is enabled.
    pub enabled: bool,
    /// Whether a relocation is pending confirmation.
    pub relocation_pending: bool,
    /// Sync schedule cron expression, if any.
    pub sync_schedule: Option<String>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_all_repos(pool: &PgPool) -> Result<Vec<RepoRow>, ApiError> {
    sqlx::query_as!(
        RepoRow,
        "SELECT id, name, repo_path, ssh_user, ssh_host, ssh_port, compression, encryption, \
         enabled, owner_id, visibility, sync_schedule, wake_enabled, wake_mac_address, \
         wake_broadcast_address, wake_timeout_seconds, shutdown_after_backup FROM repos ORDER BY \
         name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Repo row with sync schedule metadata.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RepoRowWithSync {
    /// Unique identifier.
    pub id: i64,
    /// Repository display name.
    pub name: String,
    /// Borg repository path.
    pub repo_path: String,
    /// SSH user.
    pub ssh_user: String,
    /// SSH hostname.
    pub ssh_host: String,
    /// SSH port.
    pub ssh_port: i32,
    /// Compression algorithm.
    pub compression: String,
    /// Encryption mode.
    pub encryption: String,
    /// Whether the repository is enabled.
    pub enabled: bool,
    /// Owning user ID.
    pub owner_id: Option<i64>,
    /// Visibility scope.
    pub visibility: String,
    /// Sync schedule cron expression.
    pub sync_schedule: Option<String>,
    /// When the repo was last synced.
    pub last_synced_at: Option<DateTime<Utc>>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_repos_with_sync_schedule(
    pool: &PgPool,
) -> Result<Vec<RepoRowWithSync>, ApiError> {
    sqlx::query_as!(
        RepoRowWithSync,
        "SELECT r.id, r.name, r.repo_path, r.ssh_user, r.ssh_host, r.ssh_port, r.compression, \
         r.encryption, r.enabled, r.owner_id, r.visibility, r.sync_schedule, rs.last_synced_at \
         FROM repos r LEFT JOIN repo_stats rs ON rs.repo_id = r.id ORDER BY r.name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_repos_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<RepoWithPassphraseRow>, ApiError> {
    sqlx::query_as!(
        RepoWithPassphraseRow,
        "SELECT DISTINCT r.id, r.name, r.repo_path, r.ssh_user, r.ssh_host, r.ssh_port, \
         r.ssh_host_key, r.passphrase_encrypted, r.compression, r.encryption, r.enabled, \
         r.relocation_pending, r.sync_schedule FROM repos r JOIN schedules s ON s.repo_id = r.id \
         JOIN schedule_targets st ON st.schedule_id = s.id WHERE st.agent_id = $1 ORDER BY r.id",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_repos_for_agent_public(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<RepoRow>, ApiError> {
    sqlx::query_as!(
        RepoRow,
        "SELECT DISTINCT r.id, r.name, r.repo_path, r.ssh_user, r.ssh_host, r.ssh_port, \
         r.compression, r.encryption, r.enabled, r.owner_id, r.visibility, r.sync_schedule, \
         r.wake_enabled, r.wake_mac_address, r.wake_broadcast_address, r.wake_timeout_seconds, \
         r.shutdown_after_backup FROM repos r JOIN schedules s ON s.repo_id = r.id JOIN \
         schedule_targets st ON st.schedule_id = s.id WHERE st.agent_id = $1 ORDER BY r.id",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_backup_sources_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<Vec<String>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct PathRow {
        path: String,
    }

    let rows = sqlx::query_as!(
        PathRow,
        "SELECT path FROM backup_sources WHERE repo_id = $1 ORDER BY sort_order, id",
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows.into_iter().map(|r| r.path).collect())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_backup_sources_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<String>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct PathRow {
        path: String,
    }

    let rows = sqlx::query_as!(
        PathRow,
        "SELECT path FROM backup_sources WHERE schedule_id = $1 AND agent_id IS NULL ORDER BY \
         sort_order, id",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows.into_iter().map(|r| r.path).collect())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_backup_sources_for_schedule_agent(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
) -> Result<Vec<String>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct PathRow {
        path: String,
    }

    let rows = sqlx::query_as!(
        PathRow,
        "SELECT path FROM backup_sources WHERE schedule_id = $1 AND agent_id = $2 ORDER BY \
         sort_order, id",
        schedule_id,
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows.into_iter().map(|r| r.path).collect())
}

/// Per-agent backup sources for a schedule override.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PerAgentBackupSources {
    /// Agent ID.
    pub agent_id: i64,
    /// Backup source paths for this agent.
    pub paths: Vec<String>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_all_per_agent_backup_sources_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<PerAgentBackupSources>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        agent_id: i64,
        path: String,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT agent_id AS \"agent_id!\", path FROM backup_sources WHERE schedule_id = $1 AND \
         agent_id IS NOT NULL ORDER BY agent_id, sort_order, id",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    let mut map: std::collections::BTreeMap<i64, Vec<String>> = std::collections::BTreeMap::new();
    for row in rows {
        map.entry(row.agent_id).or_default().push(row.path);
    }

    Ok(map
        .into_iter()
        .map(|(agent_id, paths)| PerAgentBackupSources { agent_id, paths })
        .collect())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_backup_source_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
    path: &str,
    sort_order: i32,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO backup_sources (schedule_id, path, sort_order) VALUES ($1, $2, $3)",
        schedule_id,
        path,
        sort_order,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_backup_source_for_schedule_agent(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
    path: &str,
    sort_order: i32,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO backup_sources (schedule_id, agent_id, path, sort_order) VALUES ($1, $2, $3, \
         $4)",
        schedule_id,
        agent_id,
        path,
        sort_order,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_backup_sources_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "DELETE FROM backup_sources WHERE schedule_id = $1 AND agent_id IS NULL",
        schedule_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_per_agent_backup_sources_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "DELETE FROM backup_sources WHERE schedule_id = $1 AND agent_id IS NOT NULL",
        schedule_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Per-agent exclude patterns for a schedule override.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PerAgentExcludePatterns {
    /// Agent ID.
    pub agent_id: i64,
    /// Raw exclude pattern text for this agent.
    pub raw_text: String,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_all_per_agent_excludes_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<PerAgentExcludePatterns>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        agent_id: i64,
        raw_text: String,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT agent_id, raw_text FROM per_agent_excludes WHERE schedule_id = $1 ORDER BY \
         agent_id",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows
        .into_iter()
        .map(|r| PerAgentExcludePatterns {
            agent_id: r.agent_id,
            raw_text: r.raw_text,
        })
        .collect())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn upsert_per_agent_excludes_raw(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
    raw_text: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO per_agent_excludes (schedule_id, agent_id, raw_text) VALUES ($1, $2, $3) ON \
         CONFLICT (schedule_id, agent_id) DO UPDATE SET raw_text = EXCLUDED.raw_text",
        schedule_id,
        agent_id,
        raw_text,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_per_agent_excludes_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "DELETE FROM per_agent_excludes WHERE schedule_id = $1",
        schedule_id
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_per_agent_excludes_raw(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar!(
        "SELECT raw_text FROM per_agent_excludes WHERE schedule_id = $1 AND agent_id = $2",
        schedule_id,
        agent_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// Per-agent pre/post backup commands for a schedule override.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PerAgentCommands {
    /// Agent ID.
    pub agent_id: i64,
    /// Pre-backup commands for this agent.
    pub pre_backup_commands: Vec<String>,
    /// Post-backup commands for this agent.
    pub post_backup_commands: Vec<String>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_all_per_agent_commands_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<PerAgentCommands>, ApiError> {
    struct Row {
        agent_id: i64,
        pre_backup_commands: sqlx::types::Json<Vec<String>>,
        post_backup_commands: sqlx::types::Json<Vec<String>>,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT agent_id, pre_backup_commands AS \"pre_backup_commands: \
         sqlx::types::Json<Vec<String>>\", post_backup_commands AS \"post_backup_commands: \
         sqlx::types::Json<Vec<String>>\" FROM per_agent_commands WHERE schedule_id = $1 ORDER BY \
         agent_id",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows
        .into_iter()
        .map(|r| PerAgentCommands {
            agent_id: r.agent_id,
            pre_backup_commands: r.pre_backup_commands.0,
            post_backup_commands: r.post_backup_commands.0,
        })
        .collect())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_per_agent_commands(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
) -> Result<Option<PerAgentCommands>, ApiError> {
    struct Row {
        pre_backup_commands: sqlx::types::Json<Vec<String>>,
        post_backup_commands: sqlx::types::Json<Vec<String>>,
    }

    let row = sqlx::query_as!(
        Row,
        "SELECT pre_backup_commands AS \"pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
         post_backup_commands AS \"post_backup_commands: sqlx::types::Json<Vec<String>>\" FROM \
         per_agent_commands WHERE schedule_id = $1 AND agent_id = $2",
        schedule_id,
        agent_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(row.map(|r| PerAgentCommands {
        agent_id,
        pre_backup_commands: r.pre_backup_commands.0,
        post_backup_commands: r.post_backup_commands.0,
    }))
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn upsert_per_agent_commands(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
    pre_backup_commands: &[String],
    post_backup_commands: &[String],
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO per_agent_commands (schedule_id, agent_id, pre_backup_commands, \
         post_backup_commands) VALUES ($1, $2, $3, $4) ON CONFLICT (schedule_id, agent_id) DO \
         UPDATE SET pre_backup_commands = EXCLUDED.pre_backup_commands, post_backup_commands = \
         EXCLUDED.post_backup_commands",
        schedule_id,
        agent_id,
        sqlx::types::Json(pre_backup_commands) as _,
        sqlx::types::Json(post_backup_commands) as _,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_per_agent_commands_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "DELETE FROM per_agent_commands WHERE schedule_id = $1",
        schedule_id
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Per-agent file-change detection patterns for a schedule override.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PerAgentFileChangePatterns {
    /// Agent ID.
    pub agent_id: i64,
    /// Raw file-change pattern text for this agent.
    pub raw_text: String,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_all_per_agent_file_change_patterns_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<PerAgentFileChangePatterns>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        agent_id: i64,
        raw_text: String,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT agent_id, raw_text FROM per_agent_file_change_patterns WHERE schedule_id = $1 \
         ORDER BY agent_id",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows
        .into_iter()
        .map(|r| PerAgentFileChangePatterns {
            agent_id: r.agent_id,
            raw_text: r.raw_text,
        })
        .collect())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn upsert_per_agent_file_change_patterns_raw(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
    raw_text: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO per_agent_file_change_patterns (schedule_id, agent_id, raw_text) VALUES ($1, \
         $2, $3) ON CONFLICT (schedule_id, agent_id) DO UPDATE SET raw_text = EXCLUDED.raw_text",
        schedule_id,
        agent_id,
        raw_text,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_per_agent_file_change_patterns_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "DELETE FROM per_agent_file_change_patterns WHERE schedule_id = $1",
        schedule_id
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_per_agent_file_change_patterns_raw(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar!(
        "SELECT raw_text FROM per_agent_file_change_patterns WHERE schedule_id = $1 AND agent_id \
         = $2",
        schedule_id,
        agent_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_schedule_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<Option<ScheduleRow>, ApiError> {
    sqlx::query_as!(
        ScheduleRow,
        "SELECT id, repo_id, name, schedule_type, cron_expression, enabled, canary_enabled, \
         last_run_at, next_run_at, exclude_patterns_raw, file_change_patterns_raw, \
         ignore_global_excludes, keep_hourly, keep_daily, keep_weekly, keep_monthly, keep_yearly, \
         compact_enabled, rate_limit_kbps, pre_backup_commands AS \"pre_backup_commands: \
         sqlx::types::Json<Vec<String>>\", post_backup_commands AS \"post_backup_commands: \
         sqlx::types::Json<Vec<String>>\", hook_timeout_seconds, missed_backup_threshold, \
         execution_mode, on_failure, owner_id, visibility, consecutive_failures, \
         auto_disabled_agent_unreachable, ARRAY[]::TEXT[] AS \"target_hostnames!\" FROM schedules \
         WHERE repo_id = $1",
        repo_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// Finds the schedule (of the given type) that targets `hostname` and `repo_id`.
/// Used to attribute a completion reported by the agent (which only carries a
/// repo id, not a schedule id) back to the schedule that most likely triggered
/// it. If multiple schedules of the same type target the same host/repo pair,
/// an arbitrary one is returned.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_schedule_for_hostname_repo(
    pool: &PgPool,
    hostname: &str,
    repo_id: i64,
    schedule_type: ScheduleType,
) -> Result<Option<ScheduleRow>, ApiError> {
    sqlx::query_as!(
        ScheduleRow,
        "SELECT s.id, s.repo_id, s.name, s.schedule_type, s.cron_expression, s.enabled, \
         s.canary_enabled, s.last_run_at, s.next_run_at, s.exclude_patterns_raw, \
         s.file_change_patterns_raw, s.ignore_global_excludes, s.keep_hourly, s.keep_daily, \
         s.keep_weekly, s.keep_monthly, s.keep_yearly, s.compact_enabled, s.rate_limit_kbps, \
         s.pre_backup_commands AS \"pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
         s.post_backup_commands AS \"post_backup_commands: sqlx::types::Json<Vec<String>>\", \
         s.hook_timeout_seconds, s.missed_backup_threshold, s.execution_mode, s.on_failure, \
         s.owner_id, s.visibility, s.consecutive_failures, s.auto_disabled_agent_unreachable, \
         ARRAY[]::TEXT[] AS \"target_hostnames!\" FROM schedules s JOIN schedule_targets st ON \
         st.schedule_id = s.id JOIN agents m ON st.agent_id = m.id WHERE m.hostname = $1 AND \
         s.repo_id = $2 AND s.schedule_type = $3 LIMIT 1",
        hostname,
        repo_id,
        schedule_type.to_string(),
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_schedules_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<Vec<ScheduleRow>, ApiError> {
    sqlx::query_as!(
        ScheduleRow,
        "SELECT s.id, s.repo_id, s.name, s.schedule_type, s.cron_expression, s.enabled, \
         s.canary_enabled, s.last_run_at, s.next_run_at, s.exclude_patterns_raw, \
         s.file_change_patterns_raw, s.ignore_global_excludes, s.keep_hourly, s.keep_daily, \
         s.keep_weekly, s.keep_monthly, s.keep_yearly, s.compact_enabled, s.rate_limit_kbps, \
         s.pre_backup_commands AS \"pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
         s.post_backup_commands AS \"post_backup_commands: sqlx::types::Json<Vec<String>>\", \
         s.hook_timeout_seconds, s.missed_backup_threshold, s.execution_mode, s.on_failure, \
         s.owner_id, s.visibility, s.consecutive_failures, s.auto_disabled_agent_unreachable, \
         COALESCE(ARRAY(SELECT a.hostname FROM schedule_targets st JOIN agents a ON a.id = \
         st.agent_id WHERE st.schedule_id = s.id ORDER BY st.execution_order, a.hostname), \
         ARRAY[]::TEXT[]) AS \"target_hostnames!\" FROM schedules s WHERE s.repo_id = $1 ORDER BY \
         s.id",
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn delete_schedule(pool: &PgPool, id: i64) -> Result<(), ApiError> {
    let result = sqlx::query!("DELETE FROM schedules WHERE id = $1", id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("schedule {id} not found")));
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_schedules_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<ScheduleRow>, ApiError> {
    sqlx::query_as!(
        ScheduleRow,
        "SELECT s.id, s.repo_id, s.name, s.schedule_type, s.cron_expression, s.enabled, \
         s.canary_enabled, s.last_run_at, s.next_run_at, s.exclude_patterns_raw, \
         s.file_change_patterns_raw, s.ignore_global_excludes, s.keep_hourly, s.keep_daily, \
         s.keep_weekly, s.keep_monthly, s.keep_yearly, s.compact_enabled, s.rate_limit_kbps, \
         s.pre_backup_commands AS \"pre_backup_commands: sqlx::types::Json<Vec<String>>\", \
         s.post_backup_commands AS \"post_backup_commands: sqlx::types::Json<Vec<String>>\", \
         s.hook_timeout_seconds, s.missed_backup_threshold, s.execution_mode, s.on_failure, \
         s.owner_id, s.visibility, s.consecutive_failures, s.auto_disabled_agent_unreachable, \
         ARRAY[]::TEXT[] AS \"target_hostnames!\" FROM schedules s JOIN schedule_targets st ON \
         st.schedule_id = s.id WHERE st.agent_id = $1 ORDER by s.id",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// A schedule that is due to run, joined with its target agent.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DueScheduleRow {
    /// Schedule ID.
    pub schedule_id: i64,
    /// Schedule display name.
    pub schedule_name: String,
    /// Repository ID.
    pub repo_id: i64,
    /// Target agent ID.
    pub agent_id: i64,
    /// Target agent hostname.
    pub hostname: String,
    /// Schedule type.
    pub schedule_type: String,
    /// Cron expression.
    pub cron_expression: String,
    /// On-failure behaviour.
    pub on_failure: String,
    /// Execution order among targets.
    pub execution_order: i32,
    /// How many consecutive missed backups this schedule tolerates before it
    /// is marked failed and auto-disabled.
    pub missed_backup_threshold: i32,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_due_schedules(
    pool: &PgPool,
    now: DateTime<Utc>,
) -> Result<Vec<DueScheduleRow>, ApiError> {
    sqlx::query_as!(
        DueScheduleRow,
        "SELECT s.id AS schedule_id, s.name AS schedule_name, s.repo_id AS \"repo_id!\", \
         st.agent_id, a.hostname, s.schedule_type, s.cron_expression, s.on_failure, \
         st.execution_order, s.missed_backup_threshold FROM schedules s JOIN repos r ON r.id = \
         s.repo_id JOIN schedule_targets st ON st.schedule_id = s.id JOIN agents a ON a.id = \
         st.agent_id WHERE s.enabled = true AND r.enabled = true AND a.is_hidden = false AND \
         s.next_run_at IS NOT NULL AND s.next_run_at <= $1 ORDER BY s.id, st.execution_order",
        now,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn mark_schedule_triggered(
    pool: &PgPool,
    schedule_id: i64,
    now: DateTime<Utc>,
    next_run_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE schedules SET last_run_at = $2, next_run_at = $3 WHERE id = $1",
        schedule_id,
        now,
        next_run_at,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Resets a schedule's consecutive-failure count once a tick completes having
/// recorded no failure for any of its targets - the only place `consecutive_failures`
/// goes back to 0 (deliberately *not* folded into [`mark_schedule_triggered`], which
/// runs on each individual target's success: a multi-target schedule can have one
/// target succeed while another fails in the same tick, and that success must not
/// erase the other target's failure count - see the call site in `scheduler.rs`).
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn reset_schedule_consecutive_failures(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE schedules SET consecutive_failures = 0, failure_streak_pure_connectivity = true \
         WHERE id = $1",
        schedule_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Outcome of [`record_schedule_failure`]: the schedule's updated consecutive-failure
/// count, and whether this call was the one that crossed the auto-disable threshold.
pub struct ScheduleFailureOutcome {
    /// The schedule's consecutive-failure count after this call.
    pub consecutive_failures: i32,
    /// Whether this call disabled the schedule (crossed `max_consecutive_failures`).
    pub auto_disabled: bool,
    /// The schedule's `auto_disabled_agent_unreachable` value after this call - only
    /// ever true if the *whole* failure streak was pure connectivity, which can differ
    /// from this call's own `agent_unreachable` argument when an earlier local/data
    /// failure in the same streak already marked it impure. Callers reporting *why* a
    /// schedule was disabled (e.g. a log line or system event) must derive the reason
    /// from this field, not from their own `agent_unreachable` argument, or the
    /// message can claim "agent unreachable" for a call that actually left
    /// `auto_disabled_agent_unreachable` false in the database.
    pub auto_disabled_agent_unreachable: bool,
}

/// Records one failed attempt to reach a schedule's agent (config push or trigger
/// send). Advances `next_run_at` to the next scheduled occurrence - same as a
/// successful trigger - so the scheduler backs off to the normal cron cadence
/// instead of retrying every tick, and disables the schedule once
/// `consecutive_failures` reaches `max_consecutive_failures`, so an agent that stays
/// offline indefinitely doesn't generate failures forever.
///
/// `agent_unreachable` distinguishes *why* this attempt failed: only a connectivity
/// failure (the agent itself unreachable) marks `auto_disabled_agent_unreachable` and
/// records `agent_id` as `auto_disabled_by_agent_id`, so the reconnect handler (see
/// [`list_auto_disabled_schedule_ids_for_agent`]/[`reenable_specific_schedules`])
/// re-enables the schedule once every one of its targets reconnects. A local/data
/// failure (e.g. config assembly, a corrupted encrypted passphrase) still counts
/// toward `consecutive_failures` and still disables
/// the schedule at the threshold, but deliberately leaves that bookkeeping untouched:
/// the agent reconnecting over the websocket says nothing about whether the underlying
/// data problem was fixed, so it must not silently self-heal a disable that was never
/// about connectivity - a human has to fix it and re-enable the schedule.
///
/// `failure_streak_pure_connectivity` tracks this across the *whole* streak, not just
/// this one call: it's only ever true if every failure since the last reset was
/// `agent_unreachable`, so a streak with even one local/data failure in it can never
/// mark `auto_disabled_agent_unreachable` - even if the specific call that happens to
/// cross the threshold is itself a connectivity failure.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn record_schedule_failure(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
    next_run_at: DateTime<Utc>,
    max_consecutive_failures: i32,
    agent_unreachable: bool,
) -> Result<ScheduleFailureOutcome, ApiError> {
    let row = sqlx::query!(
        "UPDATE schedules SET consecutive_failures = consecutive_failures + 1, next_run_at = $2, \
         enabled = enabled AND consecutive_failures + 1 < $3, failure_streak_pure_connectivity = \
         failure_streak_pure_connectivity AND $5, auto_disabled_agent_unreachable = CASE WHEN \
         (failure_streak_pure_connectivity AND $5) AND consecutive_failures + 1 >= $3 THEN true \
         ELSE auto_disabled_agent_unreachable END, auto_disabled_by_agent_id = CASE WHEN \
         (failure_streak_pure_connectivity AND $5) AND consecutive_failures + 1 >= $3 THEN $4 \
         ELSE auto_disabled_by_agent_id END WHERE id = $1 RETURNING consecutive_failures, \
         enabled, auto_disabled_agent_unreachable",
        schedule_id,
        next_run_at,
        max_consecutive_failures,
        agent_id,
        agent_unreachable,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(ScheduleFailureOutcome {
        consecutive_failures: row.consecutive_failures,
        // Derived from the threshold check itself, not from `enabled`'s final value:
        // if a concurrent human/quota write disabled this schedule (and reset
        // consecutive_failures) between it being selected as due and this write
        // landing, `enabled` is already false for an unrelated reason and staying
        // false here doesn't mean *this* call crossed the threshold.
        auto_disabled: row.consecutive_failures >= max_consecutive_failures,
        auto_disabled_agent_unreachable: row.auto_disabled_agent_unreachable,
    })
}

/// Re-enables every schedule that the scheduler had auto-disabled after repeated
/// unreachable-agent failures *from this specific `agent_id`* (never a schedule a
/// human or quota enforcement disabled), resetting its failure count and making it due
/// again immediately - matches only on `auto_disabled_by_agent_id`, an atomic
/// single-step primitive kept as a lower-level building block and exercised directly
/// by tests.
///
/// Not used by the production reconnect path (see
/// `ws::handler::reenable_system_disabled_schedules_on_reconnect`) - a multi-target
/// schedule's `auto_disabled_by_agent_id` only ever names whichever target happened to
/// be first-recorded on the disabling tick, not every target that contributed to the
/// streak, so gating solely on that one credited agent's reconnect (as this function
/// does) can leave a schedule with an unreachable *other* target disabled forever. The
/// production path instead reconsiders on any target's reconnect and only actually
/// re-enables once every target is independently confirmed connected - see
/// [`list_auto_disabled_schedule_ids_for_agent`] and [`reenable_specific_schedules`].
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn reenable_system_disabled_schedules_for_agent(
    pool: &PgPool,
    agent_id: i64,
    now: DateTime<Utc>,
) -> Result<Vec<i64>, ApiError> {
    sqlx::query_scalar!(
        "UPDATE schedules SET enabled = true, auto_disabled_agent_unreachable = false, \
         auto_disabled_by_agent_id = NULL, consecutive_failures = 0, \
         failure_streak_pure_connectivity = true, next_run_at = $2 WHERE \
         auto_disabled_agent_unreachable = true AND auto_disabled_by_agent_id = $1 RETURNING id",
        agent_id,
        now,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Schedules currently auto-disabled where `agent_id` is one of the schedule's
/// targets - the candidate set for [`reenable_specific_schedules`]. Split out as a
/// read so a caller (the reconnect handler) can filter the candidates before
/// committing to the write.
///
/// Deliberately matches on *any* target of the schedule, not just the one recorded in
/// `auto_disabled_by_agent_id`: for a multi-target schedule, that column only ever
/// names whichever target happened to be first-recorded on the disabling tick, not
/// every target that contributed to the streak (`consecutive_failures` is
/// schedule-wide). Gating solely on that one credited agent's reconnect would mean a
/// schedule could stay disabled forever if *that* agent never reconnects again after
/// the disable (e.g. it already recovered and stayed connected while a different
/// target was the one still down) - even once every target is actually back.
/// Reconsidering on any target's reconnect is safe because
/// [`reenable_specific_schedules`]'s caller only ever includes a schedule here once it
/// has independently verified every one of its targets is currently connected.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_auto_disabled_schedule_ids_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<i64>, ApiError> {
    sqlx::query_scalar!(
        "SELECT s.id FROM schedules s JOIN schedule_targets st ON st.schedule_id = s.id WHERE \
         s.auto_disabled_agent_unreachable = true AND st.agent_id = $1",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Re-enables exactly the given `schedule_ids` (a caller-filtered subset of
/// [`list_auto_disabled_schedule_ids_for_agent`]'s result), the same way
/// [`reenable_system_disabled_schedules_for_agent`] does. Matches only on
/// `auto_disabled_agent_unreachable = true`, not on which agent is recorded in
/// `auto_disabled_by_agent_id` - the reconnecting agent that produced `schedule_ids`
/// may not be the one originally credited with the disable, see
/// [`list_auto_disabled_schedule_ids_for_agent`] - so a schedule whose state changed
/// between the two calls (e.g. a human disabled it for an unrelated reason in between)
/// is safely skipped rather than blindly overwritten.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn reenable_specific_schedules(
    pool: &PgPool,
    schedule_ids: &[i64],
    now: DateTime<Utc>,
) -> Result<Vec<i64>, ApiError> {
    sqlx::query_scalar!(
        "UPDATE schedules SET enabled = true, auto_disabled_agent_unreachable = false, \
         auto_disabled_by_agent_id = NULL, consecutive_failures = 0, \
         failure_streak_pure_connectivity = true, next_run_at = $2 WHERE id = ANY($1) AND \
         auto_disabled_agent_unreachable = true RETURNING id",
        schedule_ids,
        now,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_next_run_at(
    pool: &PgPool,
    schedule_id: i64,
    next_run_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE schedules SET next_run_at = $2 WHERE id = $1",
        schedule_id,
        next_run_at,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_schedule_enabled(
    pool: &PgPool,
    schedule_id: i64,
    enabled: bool,
) -> Result<(), ApiError> {
    // Clears the agent-unreachable failure-tracking columns whenever something other
    // than record_schedule_failure/reenable_system_disabled_schedules_for_agent writes
    // `enabled` directly - otherwise auto_disabled_agent_unreachable can outlive the
    // auto-disable it was set for (e.g. a human re-enables the schedule, then quota
    // enforcement disables it again for an unrelated reason: without this, the stale
    // `true` flag would make a later agent reconnect silently lift the quota block).
    sqlx::query!(
        "UPDATE schedules SET enabled = $2, auto_disabled_agent_unreachable = false, \
         auto_disabled_by_agent_id = NULL, consecutive_failures = 0, \
         failure_streak_pure_connectivity = true WHERE id = $1",
        schedule_id,
        enabled,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Resets a schedule's connectivity-failure bookkeeping (`consecutive_failures`,
/// `auto_disabled_agent_unreachable`, `auto_disabled_by_agent_id`) when retargeting
/// drops the specific agent a failure streak is attributable to - never merely
/// because *some* target changed, which would wrongly forgive a still-targeted,
/// still-broken agent just because an unrelated sibling target was added or
/// removed. Two cases, based on whether `auto_disabled_by_agent_id` has attribution
/// recorded yet (it's only ever set once the schedule fully auto-disables):
/// - **Already auto-disabled**: resets only if `auto_disabled_by_agent_id` itself is
///   no longer in the new target list. Without this, retargeting away from the
///   causing agent - the realistic way an admin fixes this - would leave
///   `auto_disabled_by_agent_id` pointing at an agent that isn't a target anymore, so
///   [`reenable_system_disabled_schedules_for_agent`] could never match it again and
///   the schedule would stay disabled forever despite the admin's fix.
/// - **Not yet disabled** (a partial streak, e.g. `consecutive_failures = 2` against
///   threshold 3): there's no per-agent attribution to check yet, so this only
///   resets if *every* previously-targeted agent is gone from the new list - the one
///   case where whichever agent(s) were actually accruing the streak are
///   unambiguously no longer targeted. If even one old target remains, the streak
///   might still belong to it, so it's left untouched.
///
/// Deliberately a no-op either way when the streak contains a local/config failure
/// (`failure_streak_pure_connectivity = false`): that state must only ever be
/// cleared by a human fixing the actual cause and re-enabling the schedule
/// themselves, never implicitly by a retarget.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn reset_schedule_failure_tracking_if_target_dropped(
    pool: &PgPool,
    schedule_id: i64,
    old_agent_ids: &[i64],
    new_agent_ids: &[i64],
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE schedules SET auto_disabled_agent_unreachable = false, auto_disabled_by_agent_id \
         = NULL, consecutive_failures = 0 WHERE id = $1 AND failure_streak_pure_connectivity = \
         true AND ((auto_disabled_by_agent_id IS NOT NULL AND NOT (auto_disabled_by_agent_id = \
         ANY($2))) OR (auto_disabled_by_agent_id IS NULL AND NOT ($3::bigint[] && $2::bigint[])))",
        schedule_id,
        new_agent_ids,
        old_agent_ids,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// IDs of every schedule belonging to a repo whose `ssh_host` matches, used to enforce a
/// `server_quotas` `block_backups` action across all repos sharing that host.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_schedule_ids_for_ssh_host(
    pool: &PgPool,
    ssh_host: &str,
) -> Result<Vec<i64>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i64,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT s.id FROM schedules s JOIN repos r ON r.id = s.repo_id WHERE r.ssh_host = $1",
        ssh_host,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows.into_iter().map(|r| r.id).collect())
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_schedule_by_id(pool: &PgPool, id: i64) -> Result<ScheduleRow, ApiError> {
    sqlx::query_as!(
        ScheduleRow,
        "SELECT id, repo_id, name, schedule_type, cron_expression, enabled, canary_enabled, \
         last_run_at, next_run_at, exclude_patterns_raw, file_change_patterns_raw, \
         ignore_global_excludes, keep_hourly, keep_daily, keep_weekly, keep_monthly, keep_yearly, \
         compact_enabled, rate_limit_kbps, pre_backup_commands AS \"pre_backup_commands: \
         sqlx::types::Json<Vec<String>>\", post_backup_commands AS \"post_backup_commands: \
         sqlx::types::Json<Vec<String>>\", hook_timeout_seconds, missed_backup_threshold, \
         execution_mode, on_failure, owner_id, visibility, consecutive_failures, \
         auto_disabled_agent_unreachable, ARRAY[]::TEXT[] AS \"target_hostnames!\" FROM schedules \
         WHERE id = $1",
        id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("schedule {id} not found")),
        other => ApiError::Database(other),
    })
}

/// Batched form of [`get_schedule_targets_for_run`] for callers that need target hostnames
/// for many schedules at once (e.g. projecting calendar events for every schedule in a
/// fleet) -- one round trip instead of one query per schedule.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_schedule_target_hostnames_by_schedule(
    pool: &PgPool,
    schedule_ids: &[i64],
) -> Result<std::collections::HashMap<i64, Vec<String>>, ApiError> {
    struct Row {
        schedule_id: i64,
        hostname: String,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT st.schedule_id, a.hostname FROM agents a JOIN schedule_targets st ON st.agent_id \
         = a.id WHERE st.schedule_id = ANY($1) AND a.is_hidden = false ORDER BY st.schedule_id, \
         st.execution_order",
        schedule_ids,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    let mut by_schedule: std::collections::HashMap<i64, Vec<String>> =
        std::collections::HashMap::new();
    for row in rows {
        by_schedule
            .entry(row.schedule_id)
            .or_default()
            .push(row.hostname);
    }
    Ok(by_schedule)
}

/// Batched form of [`get_schedule_target_hostnames_by_schedule`] returning agent IDs
/// instead of hostnames, for callers that need to key live-connection state (which is
/// tracked per agent ID, not per hostname, since a hostname can be shared by more than
/// one agent).
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_schedule_target_agent_ids_by_schedule(
    pool: &PgPool,
    schedule_ids: &[i64],
) -> Result<std::collections::HashMap<i64, Vec<i64>>, ApiError> {
    struct Row {
        schedule_id: i64,
        agent_id: i64,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT st.schedule_id, st.agent_id FROM schedule_targets st JOIN agents a ON a.id = \
         st.agent_id WHERE st.schedule_id = ANY($1) AND a.is_hidden = false ORDER BY \
         st.schedule_id, st.execution_order",
        schedule_ids,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    let mut by_schedule: std::collections::HashMap<i64, Vec<i64>> =
        std::collections::HashMap::new();
    for row in rows {
        by_schedule
            .entry(row.schedule_id)
            .or_default()
            .push(row.agent_id);
    }
    Ok(by_schedule)
}

/// A target agent for a schedule run.
#[derive(Debug, sqlx::FromRow)]
pub struct ScheduleRunTarget {
    /// Agent ID.
    pub agent_id: i64,
    /// Agent hostname.
    pub hostname: String,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_schedule_targets_for_run(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<ScheduleRunTarget>, ApiError> {
    sqlx::query_as!(
        ScheduleRunTarget,
        "SELECT a.id AS agent_id, a.hostname FROM agents a JOIN schedule_targets st ON \
         st.agent_id = a.id WHERE st.schedule_id = $1 AND a.is_hidden = false ORDER BY \
         st.execution_order",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_schedule_targets(
    pool: &PgPool,
    schedule_id: i64,
    targets: &[(i64, i32)],
) -> Result<(), ApiError> {
    for (agent_id, execution_order) in targets {
        sqlx::query!(
            "INSERT INTO schedule_targets (schedule_id, agent_id, execution_order) VALUES ($1, \
             $2, $3)",
            schedule_id,
            *agent_id,
            *execution_order,
        )
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_schedule_targets(pool: &PgPool, schedule_id: i64) -> Result<(), ApiError> {
    sqlx::query!(
        "DELETE FROM schedule_targets WHERE schedule_id = $1",
        schedule_id
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_schedule_targets(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<ScheduleTargetRow>, ApiError> {
    sqlx::query_as!(
        ScheduleTargetRow,
        "SELECT agent_id, execution_order FROM schedule_targets WHERE schedule_id = $1 ORDER BY \
         execution_order",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_repo_name(pool: &PgPool, repo_id: i64) -> Result<String, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        name: String,
    }

    let row = sqlx::query_as!(Row, "SELECT name FROM repos WHERE id = $1", repo_id)
        .fetch_one(pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
            other => ApiError::Database(other),
        })?;

    Ok(row.name)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_repo_ssh_host(pool: &PgPool, repo_id: i64) -> Result<String, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        ssh_host: String,
    }

    let row = sqlx::query_as!(Row, "SELECT ssh_host FROM repos WHERE id = $1", repo_id)
        .fetch_one(pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
            other => ApiError::Database(other),
        })?;

    Ok(row.ssh_host)
}

/// Resolves a schedule's display name, falling back to `default_name` (typically
/// the repo name) when the schedule has no custom name set, mirroring the
/// `COALESCE(NULLIF(s.name, ''), r.name)` convention used elsewhere.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_schedule_display_name(
    pool: &PgPool,
    schedule_id: i64,
    default_name: &str,
) -> Result<String, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        name: String,
    }

    let row = sqlx::query_as!(Row, "SELECT name FROM schedules WHERE id = $1", schedule_id)
        .fetch_one(pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => {
                ApiError::NotFound(format!("schedule {schedule_id} not found"))
            }
            other => ApiError::Database(other),
        })?;

    Ok(if row.name.trim().is_empty() {
        default_name.to_owned()
    } else {
        row.name
    })
}

/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn insert_canary_result(
    pool: &PgPool,
    schedule_id: i64,
    success: bool,
    canary_filename: &str,
    error_message: Option<&str>,
    archive_name: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO canary_results (schedule_id, success, canary_filename, error_message, \
         archive_name) VALUES ($1, $2, $3, $4, $5)",
        schedule_id,
        success,
        canary_filename,
        error_message,
        archive_name,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// A row from the `canary_results` table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct CanaryResultRow {
    /// Unique identifier.
    pub id: i64,
    /// Schedule ID this result belongs to.
    pub schedule_id: Option<i64>,
    /// When the canary was verified.
    pub verified_at: DateTime<Utc>,
    /// Whether the canary check succeeded.
    pub success: bool,
    /// Name of the canary file that was checked.
    pub canary_filename: Option<String>,
    /// Error message if the check failed.
    pub error_message: Option<String>,
    /// Archive name created by the canary run.
    pub archive_name: Option<String>,
}

/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn get_latest_canary_result(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Option<CanaryResultRow>, ApiError> {
    let row = sqlx::query_as!(
        CanaryResultRow,
        "SELECT id, schedule_id, verified_at, success, canary_filename, error_message, \
         archive_name FROM canary_results WHERE schedule_id = $1 ORDER BY verified_at DESC LIMIT 1",
        schedule_id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// # Errors
///
/// Returns an error if the underlying operation fails.
pub async fn list_canary_results(
    pool: &PgPool,
    schedule_id: i64,
    limit: i64,
) -> Result<Vec<CanaryResultRow>, ApiError> {
    let rows = sqlx::query_as!(
        CanaryResultRow,
        "SELECT id, schedule_id, verified_at, success, canary_filename, error_message, \
         archive_name FROM canary_results WHERE schedule_id = $1 ORDER BY verified_at DESC LIMIT \
         $2",
        schedule_id,
        limit,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// A row from the `backup_reports` table with joined repo/schedule names.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ReportRow {
    /// Unique identifier.
    pub id: i64,
    /// Agent that ran the backup.
    pub agent_id: i64,
    /// Repository that was backed up.
    pub repo_id: i64,
    /// Repository display name.
    pub repo_name: String,
    /// Schedule ID that triggered the backup, if any.
    pub schedule_id: Option<i64>,
    /// Schedule display name, if any.
    pub schedule_name: Option<String>,
    /// When the backup started.
    pub started_at: DateTime<Utc>,
    /// When the backup finished.
    pub finished_at: DateTime<Utc>,
    /// Backup status (e.g. "success", "failed", "warning").
    pub status: String,
    /// Total original size in bytes.
    pub original_size: i64,
    /// Total compressed size in bytes.
    pub compressed_size: i64,
    /// Total deduplicated size in bytes.
    pub deduplicated_size: i64,
    /// Number of files processed.
    pub files_processed: i64,
    /// Duration in seconds.
    pub duration_secs: i64,
    /// Error message, if any.
    pub error_message: Option<String>,
    /// Warning messages, if any.
    pub warnings: Vec<String>,
    /// Borg version used.
    pub borg_version: Option<String>,
    /// Borg archive name, if any.
    pub archive_name: Option<String>,
    /// Borg command that was executed.
    pub borg_command: Option<String>,
    /// Correlates to `backup_run_events.run_id` for this run's
    /// power-management timeline, when one was recorded.
    pub run_id: Option<String>,
}

/// Storage statistics grouped by agent and repo.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct StorageStatRow {
    /// Agent hostname.
    pub hostname: String,
    /// Repository display name.
    pub target_name: String,
    /// Total original size in bytes.
    pub total_original_size: i64,
    /// Total compressed size in bytes.
    pub total_compressed_size: i64,
    /// Total deduplicated size in bytes.
    pub total_deduplicated_size: i64,
    /// Number of backup reports.
    pub report_count: i64,
}

/// An activity feed entry representing a backup run.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ActivityRow {
    /// Report ID.
    pub id: i64,
    /// Agent hostname.
    pub hostname: String,
    /// Repository display name.
    pub target_name: String,
    /// When the backup started.
    pub started_at: DateTime<Utc>,
    /// When the backup finished.
    pub finished_at: DateTime<Utc>,
    /// Backup status.
    pub status: String,
    /// Duration in seconds.
    pub duration_secs: i64,
    /// Repository ID.
    pub repo_id: Option<i64>,
    /// Borg archive name, if any.
    pub archive_name: Option<String>,
    /// Error message, if any.
    pub error_message: Option<String>,
    /// Schedule ID, if any.
    #[serde(default)]
    pub schedule_id: Option<i64>,
    /// Schedule display name, if any.
    #[serde(default)]
    pub schedule_name: Option<String>,
    /// Run ID for tracking multi-step backups.
    #[serde(default)]
    pub run_id: Option<String>,
    /// Whether a human has acknowledged this run's warning/failure.
    #[serde(default)]
    pub acknowledged: bool,
}

/// Health summary for a schedule-agent-repo combination.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct HealthRow {
    /// Repository ID.
    pub repo_id: i64,
    /// Schedule ID.
    pub schedule_id: i64,
    /// Agent hostname.
    pub hostname: String,
    /// Repository display name.
    pub target_name: String,
    /// Status of the last backup run.
    pub last_status: Option<String>,
    /// When the last backup finished.
    pub last_backup_at: Option<DateTime<Utc>>,
    /// Error message from the last failure.
    pub last_error_message: Option<String>,
    /// Schedule cron expression.
    pub cron_expression: Option<String>,
    /// Whether the schedule is enabled.
    pub schedule_enabled: Option<bool>,
    /// How many consecutive scheduled runs this schedule has missed since its
    /// last success.
    pub consecutive_missed_backups: i32,
    /// The schedule's configured missed-backup threshold.
    pub missed_backup_threshold: i32,
}

/// Parameters for inserting or upserting a backup report.
#[derive(Clone)]
pub struct InsertReportParams {
    /// Agent that ran the backup.
    pub agent_id: i64,
    /// Repository that was backed up.
    pub repo_id: i64,
    /// Schedule ID, if any.
    pub schedule_id: Option<i64>,
    /// When the backup started.
    pub started_at: DateTime<Utc>,
    /// When the backup finished.
    pub finished_at: DateTime<Utc>,
    /// Backup status.
    pub status: BackupStatus,
    /// Total original size in bytes.
    pub original_size: i64,
    /// Total compressed size in bytes.
    pub compressed_size: i64,
    /// Total deduplicated size in bytes.
    pub deduplicated_size: i64,
    /// Repository-level unique compressed size.
    pub repo_unique_csize: i64,
    /// Number of files processed.
    pub files_processed: i64,
    /// Duration in seconds.
    pub duration_secs: i64,
    /// Error message, if any.
    pub error_message: Option<String>,
    /// Warning messages, if any.
    pub warnings: Vec<String>,
    /// Borg version used.
    pub borg_version: Option<String>,
    /// Whether the agent has been matched to a known host.
    pub matched: bool,
    /// Borg archive name, if any.
    pub archive_name: Option<String>,
    /// Borg command that was executed.
    pub borg_command: Option<String>,
    /// Run ID for tracking multi-step backups.
    pub run_id: Option<String>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_backup_pending(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
    schedule_id: Option<i64>,
    run_id: &str,
    triggered_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
         status, run_id) VALUES ($1, $2, $3, $4, $4, 'pending', $5) ON CONFLICT (repo_id, \
         agent_id, started_at) WHERE archive_name IS NULL DO NOTHING",
        agent_id,
        repo_id,
        schedule_id,
        triggered_at,
        run_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_backup_started(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
    schedule_id: Option<i64>,
    started_at: DateTime<Utc>,
    borg_command: Option<&str>,
    run_id: Option<&str>,
) -> Result<(), ApiError> {
    if let Some(rid) = run_id {
        sqlx::query!(
            "UPDATE backup_reports SET started_at = $1, status = 'started', borg_command = $2 \
             WHERE run_id = $3 AND agent_id = $4 AND status = 'pending'",
            started_at,
            borg_command,
            rid,
            agent_id,
        )
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    } else {
        sqlx::query!(
            "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
             status, borg_command) VALUES ($1, $2, $3, $4, $4, 'started', $5) ON CONFLICT \
             (repo_id, agent_id, started_at) WHERE archive_name IS NULL DO NOTHING",
            agent_id,
            repo_id,
            schedule_id,
            started_at,
            borg_command,
        )
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn cancel_backup_report(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE backup_reports SET status = 'cancelled', finished_at = NOW(), \
         cancellation_acknowledged = false WHERE agent_id = $1 AND repo_id = $2 AND status IN \
         ('pending', 'started')",
        agent_id,
        repo_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn cancel_all_active_backups(pool: &PgPool) -> Result<u64, ApiError> {
    let result = sqlx::query!(
        "UPDATE backup_reports SET status = 'cancelled', finished_at = NOW(), \
         cancellation_acknowledged = false WHERE status IN ('pending', 'started')",
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn acknowledge_cancellation(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE backup_reports SET cancellation_acknowledged = true WHERE agent_id = $1 AND \
         repo_id = $2 AND status = 'cancelled'",
        agent_id,
        repo_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn fail_other_started_backups(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
    current_run_id: Option<&str>,
    hostname: &str,
) -> Result<u64, ApiError> {
    let result = sqlx::query!(
        "UPDATE backup_reports SET status = 'failed', finished_at = NOW(), error_message = $1 \
         WHERE agent_id = $2 AND repo_id = $3 AND status IN ('pending', 'started') AND ($4::text \
         IS NULL OR run_id IS DISTINCT FROM $4)",
        format!("Agent '{hostname}' restarted; backup abandoned"),
        agent_id,
        repo_id,
        current_run_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// Marks every in-flight (`pending`/`started`) backup report for an agent as
/// abandoned, across all repos - called when the agent's connection is
/// replaced by a new one (see [`crate::ws::registry::AgentRegistry::register`]),
/// since a reconnect means the previous session, and anything it was in the
/// middle of, is gone for good regardless of which repo it targeted. Returns
/// the distinct repo IDs that had a row updated, so the caller can wake up
/// anything still waiting on those operations via the completion bus instead
/// of leaving it blocked on a session that will never report back.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn fail_started_backups_for_agent_reconnect(
    pool: &PgPool,
    agent_id: i64,
    hostname: &str,
) -> Result<Vec<i64>, ApiError> {
    let mut repo_ids: Vec<i64> = sqlx::query_scalar!(
        "UPDATE backup_reports SET status = 'failed', finished_at = NOW(), error_message = $1 \
         WHERE agent_id = $2 AND status IN ('pending', 'started') RETURNING repo_id",
        format!("Agent '{hostname}' reconnected; previous backup abandoned"),
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    repo_ids.sort_unstable();
    repo_ids.dedup();
    Ok(repo_ids)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_backup_report(
    pool: &PgPool,
    params: &InsertReportParams,
) -> Result<(), ApiError> {
    if let Some(ref run_id) = params.run_id {
        update_backup_report_by_run_id(pool, params, run_id).await
    } else if params.archive_name.is_some() {
        upsert_backup_report_with_archive_name(pool, params).await
    } else {
        upsert_backup_report_without_archive_name(pool, params).await
    }
}

async fn update_backup_report_by_run_id(
    pool: &PgPool,
    params: &InsertReportParams,
    run_id: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE backup_reports SET schedule_id = COALESCE($1, schedule_id), finished_at = $2, \
         status = $3, original_size = $4, compressed_size = $5, deduplicated_size = $6, \
         repo_unique_csize = $7, files_processed = $8, duration_secs = $9, error_message = $10, \
         warnings = $11, borg_version = $12, matched = $13, archive_name = $14, borg_command = \
         COALESCE($15, borg_command), started_at = $16 WHERE run_id = $17 AND agent_id = $18 AND \
         status IN ('pending', 'started')",
        params.schedule_id,
        params.finished_at,
        &params.status.to_string(),
        params.original_size,
        params.compressed_size,
        params.deduplicated_size,
        params.repo_unique_csize,
        params.files_processed,
        params.duration_secs,
        params.error_message.as_deref(),
        &params.warnings,
        params.borg_version.as_deref(),
        params.matched,
        params.archive_name.as_deref(),
        params.borg_command.as_deref(),
        params.started_at,
        run_id,
        params.agent_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

async fn upsert_backup_report_with_archive_name(
    pool: &PgPool,
    params: &InsertReportParams,
) -> Result<(), ApiError> {
    let status_str = params.status.to_string();
    sqlx::query!(
        "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
         status, original_size, compressed_size, deduplicated_size, repo_unique_csize, \
         files_processed, duration_secs, error_message, warnings, borg_version, matched, \
         archive_name, borg_command) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, \
         $13, $14, $15, $16, $17, $18) ON CONFLICT (repo_id, agent_id, started_at, archive_name) \
         WHERE archive_name IS NOT NULL DO UPDATE SET schedule_id = \
         COALESCE(EXCLUDED.schedule_id, backup_reports.schedule_id), finished_at = \
         EXCLUDED.finished_at, status = EXCLUDED.status, original_size = EXCLUDED.original_size, \
         compressed_size = EXCLUDED.compressed_size, deduplicated_size = \
         EXCLUDED.deduplicated_size, repo_unique_csize = EXCLUDED.repo_unique_csize, \
         files_processed = EXCLUDED.files_processed, duration_secs = EXCLUDED.duration_secs, \
         error_message = EXCLUDED.error_message, warnings = EXCLUDED.warnings, borg_version = \
         EXCLUDED.borg_version, matched = EXCLUDED.matched, archive_name = EXCLUDED.archive_name, \
         borg_command = COALESCE(EXCLUDED.borg_command, backup_reports.borg_command)",
        params.agent_id,
        params.repo_id,
        params.schedule_id,
        params.started_at,
        params.finished_at,
        &status_str,
        params.original_size,
        params.compressed_size,
        params.deduplicated_size,
        params.repo_unique_csize,
        params.files_processed,
        params.duration_secs,
        params.error_message.as_deref(),
        &params.warnings,
        params.borg_version.as_deref(),
        params.matched,
        params.archive_name.as_deref(),
        params.borg_command.as_deref(),
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

async fn upsert_backup_report_without_archive_name(
    pool: &PgPool,
    params: &InsertReportParams,
) -> Result<(), ApiError> {
    let status_str = params.status.to_string();
    sqlx::query!(
        "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
         status, original_size, compressed_size, deduplicated_size, repo_unique_csize, \
         files_processed, duration_secs, error_message, warnings, borg_version, matched, \
         archive_name, borg_command) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, \
         $13, $14, $15, $16, $17, $18) ON CONFLICT (repo_id, agent_id, started_at) WHERE \
         archive_name IS NULL DO UPDATE SET schedule_id = COALESCE(EXCLUDED.schedule_id, \
         backup_reports.schedule_id), finished_at = EXCLUDED.finished_at, status = \
         EXCLUDED.status, original_size = EXCLUDED.original_size, compressed_size = \
         EXCLUDED.compressed_size, deduplicated_size = EXCLUDED.deduplicated_size, \
         repo_unique_csize = EXCLUDED.repo_unique_csize, files_processed = \
         EXCLUDED.files_processed, duration_secs = EXCLUDED.duration_secs, error_message = \
         EXCLUDED.error_message, warnings = EXCLUDED.warnings, borg_version = \
         EXCLUDED.borg_version, matched = EXCLUDED.matched, archive_name = EXCLUDED.archive_name, \
         borg_command = COALESCE(EXCLUDED.borg_command, backup_reports.borg_command)",
        params.agent_id,
        params.repo_id,
        params.schedule_id,
        params.started_at,
        params.finished_at,
        &status_str,
        params.original_size,
        params.compressed_size,
        params.deduplicated_size,
        params.repo_unique_csize,
        params.files_processed,
        params.duration_secs,
        params.error_message.as_deref(),
        &params.warnings,
        params.borg_version.as_deref(),
        params.matched,
        params.archive_name.as_deref(),
        params.borg_command.as_deref(),
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn bulk_insert_backup_reports(
    pool: &PgPool,
    params: &[InsertReportParams],
) -> Result<u64, ApiError> {
    if params.is_empty() {
        return Ok(0);
    }

    let mut agent_ids = Vec::with_capacity(params.len());
    let mut repo_ids = Vec::with_capacity(params.len());
    let mut started_ats = Vec::with_capacity(params.len());
    let mut finished_ats = Vec::with_capacity(params.len());
    let mut statuses: Vec<String> = Vec::with_capacity(params.len());
    let mut original_sizes = Vec::with_capacity(params.len());
    let mut compressed_sizes = Vec::with_capacity(params.len());
    let mut deduplicated_sizes = Vec::with_capacity(params.len());
    let mut repo_unique_csizes = Vec::with_capacity(params.len());
    let mut files_processed_v = Vec::with_capacity(params.len());
    let mut duration_secs_v = Vec::with_capacity(params.len());
    let mut error_messages: Vec<Option<&str>> = Vec::with_capacity(params.len());
    let mut borg_versions: Vec<Option<&str>> = Vec::with_capacity(params.len());
    let mut matcheds = Vec::with_capacity(params.len());
    let mut archive_names: Vec<Option<&str>> = Vec::with_capacity(params.len());
    let mut borg_commands: Vec<Option<&str>> = Vec::with_capacity(params.len());

    for p in params {
        agent_ids.push(p.agent_id);
        repo_ids.push(p.repo_id);
        started_ats.push(p.started_at);
        finished_ats.push(p.finished_at);
        statuses.push(p.status.to_string());
        original_sizes.push(p.original_size);
        compressed_sizes.push(p.compressed_size);
        deduplicated_sizes.push(p.deduplicated_size);
        repo_unique_csizes.push(p.repo_unique_csize);
        files_processed_v.push(p.files_processed);
        duration_secs_v.push(p.duration_secs);
        error_messages.push(p.error_message.as_deref());
        borg_versions.push(p.borg_version.as_deref());
        matcheds.push(p.matched);
        archive_names.push(p.archive_name.as_deref());
        borg_commands.push(p.borg_command.as_deref());
    }

    let status_strs: Vec<&str> = statuses.iter().map(String::as_str).collect();

    let result = sqlx::query!(
        "INSERT INTO backup_reports (agent_id, repo_id, started_at, finished_at, status, \
         original_size, compressed_size, deduplicated_size, repo_unique_csize, files_processed, \
         duration_secs, error_message, warnings, borg_version, matched, archive_name, \
         borg_command) SELECT t.agent_id, t.repo_id, t.started_at, t.finished_at, t.status, \
         t.original_size, t.compressed_size, t.deduplicated_size, t.repo_unique_csize, \
         t.files_processed, t.duration_secs, t.error_message, ARRAY[]::text[], t.borg_version, \
         t.matched, t.archive_name, t.borg_command FROM UNNEST($1::bigint[], $2::bigint[], \
         $3::timestamptz[], $4::timestamptz[], $5::text[], $6::bigint[], $7::bigint[], \
         $8::bigint[], $9::bigint[], $10::bigint[], $11::bigint[], $12::text[], $13::text[], \
         $14::bool[], $15::text[], $16::text[]) AS t(agent_id, repo_id, started_at, finished_at, \
         status, original_size, compressed_size, deduplicated_size, repo_unique_csize, \
         files_processed, duration_secs, error_message, borg_version, matched, archive_name, \
         borg_command) ON CONFLICT (repo_id, agent_id, started_at, archive_name) WHERE \
         archive_name IS NOT NULL DO NOTHING",
        &agent_ids,
        &repo_ids,
        &started_ats,
        &finished_ats,
        &status_strs as &[&str],
        &original_sizes,
        &compressed_sizes,
        &deduplicated_sizes,
        &repo_unique_csizes,
        &files_processed_v,
        &duration_secs_v,
        &error_messages as &[Option<&str>],
        &borg_versions as &[Option<&str>],
        &matcheds,
        &archive_names as &[Option<&str>],
        &borg_commands as &[Option<&str>],
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(result.rows_affected())
}

/// Statistics for a single borg archive.
pub struct ArchiveStats {
    /// Original (uncompressed) size in bytes.
    pub original_size: i64,
    /// Compressed size in bytes.
    pub compressed_size: i64,
    /// Deduplicated size in bytes.
    pub deduplicated_size: i64,
    /// Number of files processed.
    pub files_processed: i64,
    /// Duration in seconds.
    pub duration_secs: i64,
    /// Repository-level unique compressed size.
    pub repo_unique_csize: i64,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_backup_report_stats(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
    stats: &ArchiveStats,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE backup_reports SET original_size = $3, compressed_size = $4, deduplicated_size = \
         $5, files_processed = $6, duration_secs = $7 WHERE repo_id = $1 AND archive_name = $2 \
         AND original_size = 0 AND compressed_size = 0 AND deduplicated_size = 0",
        repo_id,
        archive_name,
        stats.original_size,
        stats.compressed_size,
        stats.deduplicated_size,
        stats.files_processed,
        stats.duration_secs,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;

    if stats.repo_unique_csize > 0 {
        sqlx::query!(
            "UPDATE backup_reports SET repo_unique_csize = $3 WHERE repo_id = $1 AND archive_name \
             = $2 AND repo_unique_csize = 0",
            repo_id,
            archive_name,
            stats.repo_unique_csize,
        )
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    }

    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_reports_for_agent(
    pool: &PgPool,
    agent_id: i64,
    target: Option<&str>,
    limit: i64,
) -> Result<Vec<ReportRow>, ApiError> {
    if let Some(target_name) = target {
        sqlx::query_as!(
            ReportRow,
            "SELECT br.id, br.agent_id, br.repo_id, r.name AS repo_name, br.schedule_id, CASE \
             WHEN s.id IS NOT NULL THEN COALESCE(NULLIF(s.name, ''), r.name) END AS \
             schedule_name, br.started_at, br.finished_at, br.status, br.original_size, \
             br.compressed_size, br.deduplicated_size, br.files_processed, br.duration_secs, \
             br.error_message, br.warnings, br.borg_version, br.archive_name, br.borg_command, \
             br.run_id FROM backup_reports br JOIN repos r ON r.id = br.repo_id LEFT JOIN \
             schedules s ON s.id = br.schedule_id WHERE br.agent_id = $1 AND r.name = $2 ORDER by \
             br.started_at DESC LIMIT $3",
            agent_id,
            target_name,
            limit,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    } else {
        sqlx::query_as!(
            ReportRow,
            "SELECT br.id, br.agent_id, br.repo_id, r.name AS repo_name, br.schedule_id, CASE \
             WHEN s.id IS NOT NULL THEN COALESCE(NULLIF(s.name, ''), r.name) END AS \
             schedule_name, br.started_at, br.finished_at, br.status, br.original_size, \
             br.compressed_size, br.deduplicated_size, br.files_processed, br.duration_secs, \
             br.error_message, br.warnings, br.borg_version, br.archive_name, br.borg_command, \
             br.run_id FROM backup_reports br JOIN repos r ON r.id = br.repo_id LEFT JOIN \
             schedules s ON s.id = br.schedule_id WHERE br.agent_id = $1 ORDER BY br.started_at \
             DESC LIMIT $2",
            agent_id,
            limit,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    }
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_reports_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
    limit: i64,
) -> Result<Vec<ReportRow>, ApiError> {
    sqlx::query_as!(
        ReportRow,
        "SELECT br.id, br.agent_id, br.repo_id, r.name AS repo_name, br.schedule_id, CASE WHEN \
         s.id IS NOT NULL THEN COALESCE(NULLIF(s.name, ''), r.name) END AS schedule_name, \
         br.started_at, br.finished_at, br.status, br.original_size, br.compressed_size, \
         br.deduplicated_size, br.files_processed, br.duration_secs, br.error_message, \
         br.warnings, br.borg_version, br.archive_name, br.borg_command, br.run_id FROM \
         backup_reports br JOIN repos r ON r.id = br.repo_id LEFT JOIN schedules s ON s.id = \
         br.schedule_id WHERE br.schedule_id = $1 ORDER BY br.started_at DESC LIMIT $2",
        schedule_id,
        limit,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_storage_stats(pool: &PgPool) -> Result<Vec<StorageStatRow>, ApiError> {
    sqlx::query_as!(
        StorageStatRow,
        "SELECT a.hostname, r.name AS target_name, COALESCE(SUM(br.original_size), 0)::INT8 AS \
         \"total_original_size!\", COALESCE(SUM(br.compressed_size), 0)::INT8 AS \
         \"total_compressed_size!\", COALESCE(SUM(br.deduplicated_size), 0)::INT8 AS \
         \"total_deduplicated_size!\", COUNT(br.id)::INT8 AS \"report_count!\" FROM \
         backup_reports br JOIN agents a ON a.id = br.agent_id JOIN repos r ON r.id = br.repo_id \
         WHERE a.is_hidden = false GROUP BY a.hostname, r.name ORDER BY a.hostname, r.name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// The filters an activity-feed query narrows on. Both feed queries take the
/// same set, and passing them one by one outgrew a readable argument list.
#[derive(Debug, Clone, Copy, Default)]
pub struct ActivityFeedFilters<'a> {
    /// Only runs against this repository.
    pub repo_id: Option<i64>,
    /// Only runs from this agent hostname.
    pub hostname: Option<&'a str>,
    /// Only runs of this schedule.
    pub schedule_id: Option<i64>,
    /// Only runs belonging to this run ID.
    pub run_id: Option<&'a str>,
    /// Which acknowledgment state to return.
    pub acknowledged: AcknowledgedFilter,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_activity_feed(
    pool: &PgPool,
    limit: i64,
    filters: ActivityFeedFilters<'_>,
) -> Result<Vec<ActivityRow>, ApiError> {
    sqlx::query_as!(
        ActivityRow,
        "SELECT br.id, a.hostname, r.name AS target_name, br.started_at, br.finished_at, \
         br.status, br.duration_secs, br.repo_id, br.archive_name, br.error_message, \
         br.schedule_id, s.name AS \"schedule_name?\", br.run_id, br.acknowledged FROM \
         backup_reports br JOIN agents a ON a.id = br.agent_id JOIN repos r ON r.id = br.repo_id \
         LEFT JOIN schedules s ON s.id = br.schedule_id WHERE a.is_hidden = false AND \
         a.visibility <> 'hidden' AND COALESCE(a.display_name, '') NOT ILIKE '%(imported)%' AND \
         ($1::bigint IS NULL OR br.repo_id = $1) AND ($2::text IS NULL OR a.hostname = $2) AND \
         ($3::bigint IS NULL OR br.schedule_id = $3) AND ($4::text IS NULL OR br.run_id = $4) AND \
         ($5::bool IS NULL OR br.acknowledged = $5) ORDER BY br.started_at DESC LIMIT $6",
        filters.repo_id,
        filters.hostname,
        filters.schedule_id,
        filters.run_id,
        filters.acknowledged.as_sql_predicate(),
        limit,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_health_summary(pool: &PgPool) -> Result<Vec<HealthRow>, ApiError> {
    // Single LATERAL join per (schedule, agent) row instead of three separate correlated
    // subqueries that each re-sorted the same filtered backup_reports rows -- matches the
    // pattern already used by dashboard::targets() for the equivalent "latest report" lookup.
    sqlx::query_as!(
        HealthRow,
        "SELECT r.id AS repo_id, s.id AS schedule_id, a.hostname, r.name AS target_name, \
         latest.status AS \"last_status?\", latest.finished_at AS \"last_backup_at?\", \
         latest.error_message AS \"last_error_message?\", s.cron_expression, s.enabled AS \
         schedule_enabled, s.consecutive_failures AS consecutive_missed_backups, \
         s.missed_backup_threshold FROM schedules s JOIN schedule_targets st ON st.schedule_id = \
         s.id JOIN agents a ON a.id = st.agent_id JOIN repos r ON r.id = s.repo_id LEFT JOIN \
         LATERAL ( SELECT br.status, br.finished_at, br.error_message FROM backup_reports br \
         WHERE br.schedule_id = s.id AND br.agent_id = a.id ORDER BY br.started_at DESC LIMIT 1 ) \
         latest ON true WHERE a.is_hidden = false ORDER BY a.hostname, r.name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// A row from the `users` table (excluding the password hash).
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct UserRow {
    /// Unique identifier.
    pub id: i64,
    /// Username for login.
    pub username: String,
    /// Whether the user must change their password on next login.
    pub must_change_password: bool,
    /// When the user was created.
    pub created_at: DateTime<Utc>,
    /// When the user last logged in.
    pub last_login_at: Option<DateTime<Utc>>,
    /// When the account is locked until (if applicable).
    pub locked_until: Option<DateTime<Utc>>,
}

/// A row from the `sessions` table.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SessionRow {
    /// Session ID (token).
    pub id: String,
    /// User ID this session belongs to.
    pub user_id: i64,
    /// When the session was created.
    pub created_at: DateTime<Utc>,
    /// When the session expires.
    pub expires_at: DateTime<Utc>,
    /// Whether the "remember me" flag was set.
    pub remember_me: bool,
    /// When the session was last used.
    pub last_seen_at: DateTime<Utc>,
    /// Whether this session is pending TOTP verification (pre-login temp session).
    pub pending_totp: bool,
}

/// A session row returned for user-facing session listing.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SessionForUser {
    /// Hashed session ID.
    pub id: String,
    /// User ID the session belongs to.
    pub user_id: i64,
    /// When the session was created.
    pub created_at: DateTime<Utc>,
    /// When the session expires.
    pub expires_at: DateTime<Utc>,
    /// When the session was last used.
    pub last_seen_at: DateTime<Utc>,
    /// Whether the "remember me" flag was set.
    pub remember_me: bool,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_user(
    pool: &PgPool,
    username: &str,
    password_hash: &str,
) -> Result<UserRow, ApiError> {
    sqlx::query_as!(
        UserRow,
        "INSERT INTO users (username, password_hash) VALUES ($1, $2) RETURNING id, username, \
         must_change_password, created_at, last_login_at, locked_until",
        username,
        password_hash,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_user_by_username(pool: &PgPool, username: &str) -> Result<UserRow, ApiError> {
    sqlx::query_as!(
        UserRow,
        "SELECT id, username, must_change_password, created_at, last_login_at, locked_until FROM \
         users WHERE username = $1",
        username,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("user '{username}' not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_user_password_hash(
    pool: &PgPool,
    username: &str,
) -> Result<(UserRow, String), ApiError> {
    #[derive(sqlx::FromRow)]
    struct FullRow {
        id: i64,
        username: String,
        password_hash: String,
        must_change_password: bool,
        created_at: DateTime<Utc>,
        last_login_at: Option<DateTime<Utc>>,
        locked_until: Option<DateTime<Utc>>,
    }

    let row = sqlx::query_as!(
        FullRow,
        "SELECT id, username, password_hash, must_change_password, created_at, last_login_at, \
         locked_until FROM users WHERE username = $1",
        username,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("user '{username}' not found")),
        other => ApiError::Database(other),
    })?;

    let user = UserRow {
        id: row.id,
        username: row.username,
        must_change_password: row.must_change_password,
        created_at: row.created_at,
        last_login_at: row.last_login_at,
        locked_until: row.locked_until,
    };
    Ok((user, row.password_hash))
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_user_by_id(pool: &PgPool, user_id: i64) -> Result<UserRow, ApiError> {
    sqlx::query_as!(
        UserRow,
        "SELECT id, username, must_change_password, created_at, last_login_at, locked_until FROM \
         users WHERE id = $1",
        user_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("user {user_id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_users(pool: &PgPool) -> Result<Vec<UserRow>, ApiError> {
    sqlx::query_as!(
        UserRow,
        "SELECT id, username, must_change_password, created_at, last_login_at, locked_until FROM \
         users ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn delete_user(pool: &PgPool, user_id: i64) -> Result<(), ApiError> {
    let result = sqlx::query!("DELETE FROM users WHERE id = $1", user_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("user {user_id} not found")));
    }
    Ok(())
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn update_user_password(
    pool: &PgPool,
    user_id: i64,
    password_hash: &str,
) -> Result<(), ApiError> {
    let result = sqlx::query!(
        "UPDATE users SET password_hash = $2, must_change_password = false WHERE id = $1",
        user_id,
        password_hash,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("user {user_id} not found")));
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_last_login(pool: &PgPool, user_id: i64) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE users SET last_login_at = NOW() WHERE id = $1",
        user_id
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_user_totp_fields(
    pool: &PgPool,
    user_id: i64,
) -> Result<Option<UserTotpFields>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        secret_encrypted: Option<Vec<u8>>,
        enabled: bool,
        recovery_codes: Option<Vec<String>>,
        last_verified_step: Option<i64>,
    }

    let row = sqlx::query_as!(
        Row,
        "SELECT totp_secret_encrypted AS secret_encrypted, totp_enabled AS enabled, \
         totp_recovery_codes AS recovery_codes, totp_last_verified_step AS last_verified_step \
         FROM users WHERE id = $1",
        user_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(match row {
        Some(r) => {
            if r.secret_encrypted.is_some() {
                Some(UserTotpFields {
                    secret_encrypted: r.secret_encrypted,
                    enabled: r.enabled,
                    recovery_codes: r.recovery_codes.unwrap_or_default(),
                    last_verified_step: r.last_verified_step,
                })
            } else {
                None
            }
        }
        None => None,
    })
}

/// TOTP configuration fields for a user.
pub struct UserTotpFields {
    /// Encrypted TOTP secret (AES-256-GCM).
    pub secret_encrypted: Option<Vec<u8>>,
    /// Whether TOTP is enabled for this user.
    pub enabled: bool,
    /// Hashed recovery codes.
    pub recovery_codes: Vec<String>,
    /// The most recent TOTP time-step (`unix_time / step`) successfully
    /// consumed during login, used for replay protection.
    pub last_verified_step: Option<i64>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_user_totp_secret(
    pool: &PgPool,
    user_id: i64,
    encrypted_secret: &[u8],
    recovery_codes: &[String],
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE users SET totp_secret_encrypted = $2, totp_recovery_codes = $3 WHERE id = $1",
        user_id,
        encrypted_secret,
        recovery_codes,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn enable_user_totp(
    pool: &PgPool,
    user_id: i64,
    verified_step: i64,
) -> Result<(), ApiError> {
    // Record the step consumed by the enrollment code itself, so it can't be
    // replayed against the login endpoint for the rest of its validity window.
    sqlx::query!(
        "UPDATE users SET totp_enabled = true, totp_last_verified_step = $2 WHERE id = $1",
        user_id,
        verified_step,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn disable_user_totp(pool: &PgPool, user_id: i64) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE users SET totp_enabled = false, totp_secret_encrypted = NULL, totp_recovery_codes \
         = NULL, totp_last_verified_step = NULL WHERE id = $1",
        user_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn replace_totp_recovery_codes(
    pool: &PgPool,
    user_id: i64,
    recovery_codes: &[String],
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE users SET totp_recovery_codes = $2 WHERE id = $1",
        user_id,
        recovery_codes,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Atomically removes exactly one recovery code (matched by its stored
/// hash) from a user's recovery-code list, in a single statement rather
/// than a read-modify-write of the whole array, so two requests racing the
/// same code can't both observe it as still present before either write
/// commits. Returns `true` if the hash was found and removed, `false` if it
/// was already gone (e.g. consumed by a concurrent request).
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn try_consume_totp_recovery_code(
    pool: &PgPool,
    user_id: i64,
    code_hash: &str,
) -> Result<bool, ApiError> {
    let result = sqlx::query!(
        "UPDATE users SET totp_recovery_codes = array_remove(totp_recovery_codes, $2) WHERE id = \
         $1 AND $2 = ANY(totp_recovery_codes)",
        user_id,
        code_hash,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected() > 0)
}

/// Atomically checks and records the TOTP step consumed by a login, in a
/// single statement rather than a separate read-then-write, so two
/// concurrent requests racing the same code can't both pass the replay
/// check before either write commits. Returns `true` if `step` was newer
/// than whatever was previously recorded (and is now recorded), `false` if
/// it was a replay (at or before the recorded step) and the row was left
/// unchanged.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn try_consume_totp_step(
    pool: &PgPool,
    user_id: i64,
    step: i64,
) -> Result<bool, ApiError> {
    let result = sqlx::query!(
        "UPDATE users SET totp_last_verified_step = $2 WHERE id = $1 AND (totp_last_verified_step \
         IS NULL OR totp_last_verified_step < $2)",
        user_id,
        step,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected() > 0)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_session_last_seen(pool: &PgPool, session_id: &str) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE sessions SET last_seen_at = NOW() WHERE id = $1",
        session_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_sessions_for_user(
    pool: &PgPool,
    user_id: i64,
) -> Result<Vec<SessionForUser>, ApiError> {
    sqlx::query_as!(
        SessionForUser,
        "SELECT id, user_id, created_at, expires_at, last_seen_at, remember_me FROM sessions \
         WHERE user_id = $1 AND expires_at > NOW() AND pending_totp = false ORDER BY created_at \
         DESC",
        user_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_session_by_id(
    pool: &PgPool,
    session_id: &str,
    user_id: i64,
) -> Result<bool, ApiError> {
    let result = sqlx::query!(
        "DELETE FROM sessions WHERE id = $1 AND user_id = $2",
        session_id,
        user_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected() > 0)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_user_password_hash_by_id(pool: &PgPool, user_id: i64) -> Result<String, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        password_hash: String,
    }

    let row = sqlx::query_as!(
        Row,
        "SELECT password_hash FROM users WHERE id = $1",
        user_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("user {user_id} not found")),
        other => ApiError::Database(other),
    })?;
    Ok(row.password_hash)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_session(
    pool: &PgPool,
    session_id: &str,
    user_id: i64,
    expires_at: DateTime<Utc>,
    remember_me: bool,
    pending_totp: bool,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO sessions (id, user_id, expires_at, remember_me, last_seen_at, pending_totp) \
         VALUES ($1, $2, $3, $4, NOW(), $5)",
        session_id,
        user_id,
        expires_at,
        remember_me,
        pending_totp,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Unauthorized`]: the caller is not authenticated
/// - [`ApiError::Database`]: the database query fails
pub async fn get_session(pool: &PgPool, session_id: &str) -> Result<SessionRow, ApiError> {
    sqlx::query_as!(
        SessionRow,
        "SELECT id, user_id, created_at, expires_at, remember_me, last_seen_at, pending_totp FROM \
         sessions WHERE id = $1 AND expires_at > NOW()",
        session_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => {
            ApiError::Unauthorized("session expired or invalid".to_string())
        }
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn extend_session(
    pool: &PgPool,
    session_id: &str,
    new_expires_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE sessions SET expires_at = $1 WHERE id = $2",
        new_expires_at,
        session_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_session(pool: &PgPool, session_id: &str) -> Result<(), ApiError> {
    sqlx::query!("DELETE FROM sessions WHERE id = $1", session_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_expired_sessions(pool: &PgPool) -> Result<u64, ApiError> {
    let result = sqlx::query!("DELETE FROM sessions WHERE expires_at <= NOW()")
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn user_count(pool: &PgPool) -> Result<i64, ApiError> {
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: Option<i64>,
    }

    let row = sqlx::query_as!(CountRow, "SELECT COUNT(*) as count FROM users")
        .fetch_one(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(row.count.unwrap_or(0))
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn count_failed_login_attempts(
    pool: &PgPool,
    username: &str,
    ip: &str,
    window_minutes: i32,
) -> Result<i64, ApiError> {
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: Option<i64>,
    }

    let row = sqlx::query_as!(
        CountRow,
        "SELECT COUNT(*) as count FROM login_attempts WHERE username = $1 AND ip = $2 AND success \
         = false AND attempted_at > NOW() - ($3 || ' minutes')::INTERVAL",
        username,
        ip,
        window_minutes.to_string(),
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(row.count.unwrap_or(0))
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_login_attempt(
    pool: &PgPool,
    username: &str,
    ip: &str,
    success: bool,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO login_attempts (username, ip, success) VALUES ($1, $2, $3)",
        username,
        ip,
        success,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn count_failed_totp_attempts(
    pool: &PgPool,
    user_id: i64,
    window_minutes: i32,
) -> Result<i64, ApiError> {
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: Option<i64>,
    }

    let row = sqlx::query_as!(
        CountRow,
        "SELECT COUNT(*) as count FROM totp_attempts WHERE user_id = $1 AND success = false AND \
         attempted_at > NOW() - ($2 || ' minutes')::INTERVAL",
        user_id,
        window_minutes.to_string(),
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(row.count.unwrap_or(0))
}

/// Clear the lockout for an account and reset its lockout-escalation
/// counter. Called after a successful login so a future lockout starts
/// back at the shortest tier.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn clear_account_lockout<'e, E>(executor: E, username: &str) -> Result<(), ApiError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query!(
        "UPDATE users SET locked_until = NULL, lockout_escalation_level = 0 WHERE username = $1",
        username,
    )
    .execute(executor)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Records a fully-completed successful login: clears any account lockout
/// and resets the escalation counter, and inserts a `success = true`
/// `login_attempts` row. Wraps both writes in one transaction (matching
/// [`record_failed_login_and_check_lockout`]'s treatment of the failure
/// path) so a mid-write DB error can't wipe the lockout state without also
/// recording the successful login that justified clearing it.
///
/// Callers must only invoke this once authentication has *fully* completed
/// -- i.e. after any required TOTP step, not merely after the password
/// check -- since this both resets the password-lockout escalation tier and
/// records the attempt as successful in the audit trail.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn record_successful_login(
    pool: &PgPool,
    username: &str,
    ip: &str,
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    clear_account_lockout(&mut *tx, username).await?;

    sqlx::query!(
        "INSERT INTO login_attempts (username, ip, success) VALUES ($1, $2, true)",
        username,
        ip,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

/// Count failed login attempts since the last successful login for the given
/// username. If there has never been a successful login, counts all failures.
/// Generic over the executor so callers running inside a transaction (e.g.
/// [`record_failed_login_and_check_lockout`]) can reuse this instead of
/// re-embedding the same query against a `&mut Transaction`.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn count_failed_attempts_since_last_success<'e, E>(
    executor: E,
    username: &str,
) -> Result<i64, ApiError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: Option<i64>,
    }

    let row = sqlx::query_as!(
        CountRow,
        "SELECT COUNT(*) as count FROM login_attempts WHERE username = $1 AND success = false AND \
         attempted_at > COALESCE((SELECT MAX(attempted_at) FROM login_attempts WHERE username = \
         $1 AND success = true), '-infinity'::TIMESTAMPTZ)",
        username,
    )
    .fetch_one(executor)
    .await
    .map_err(ApiError::Database)?;

    Ok(row.count.unwrap_or(0))
}

/// Counts failed attempts that belong to the *current* lockout cycle --
/// i.e. failures recorded after the later of the last successful login or
/// `locked_until` (the account's most recent lock, whether still active or
/// already expired).
///
/// This is deliberately narrower than
/// [`count_failed_attempts_since_last_success`]: `login()`'s locked-account
/// branch calls [`record_failed_login_and_check_lockout`] on every attempt
/// against a locked account (for timing-uniformity reasons), so those
/// attempts get recorded as failures too. If the escalation gate counted
/// *all* failures since the last success, that count would never reset on
/// its own -- once an account first crosses `max_account_failures`, the
/// count is already inflated past the threshold forever, so the very next
/// failed attempt after any future lockout expires (not a fresh batch of
/// `max_account_failures`) would immediately re-trigger escalation. That
/// both lets an attacker keep an account locked at the maximum tier
/// indefinitely with roughly one low-frequency attempt per cycle, and lets
/// a locked-out legitimate user ratchet their own account up through
/// repeated retries. Excluding everything up to and including
/// `locked_until` means each lock's window (which is always at least
/// `LOCKOUT_DURATIONS[0]` long) guarantees every attempt from the prior
/// cycle -- including ones made while locked -- falls at or before the
/// cutoff, so a genuinely fresh `max_account_failures` is required to
/// escalate again after each lock expires.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
async fn count_failed_attempts_in_current_cycle<'e, E>(
    executor: E,
    username: &str,
    locked_until: Option<DateTime<Utc>>,
) -> Result<i64, ApiError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: Option<i64>,
    }

    let row = sqlx::query_as!(
        CountRow,
        "SELECT COUNT(*) as count FROM login_attempts WHERE username = $1 AND success = false AND \
         attempted_at > GREATEST(COALESCE((SELECT MAX(attempted_at) FROM login_attempts WHERE \
         username = $1 AND success = true), '-infinity'::TIMESTAMPTZ), COALESCE($2::TIMESTAMPTZ, \
         '-infinity'::TIMESTAMPTZ))",
        username,
        locked_until,
    )
    .fetch_one(executor)
    .await
    .map_err(ApiError::Database)?;

    Ok(row.count.unwrap_or(0))
}

/// Record a failed login attempt and check if the account should be locked.
///
/// Escalates the lockout duration by lockout *cycle*, not by raw failure
/// count: `users.lockout_escalation_level` only advances when this call
/// actually establishes a new lockout (the account isn't already locked),
/// and the threshold check itself only counts failures from the current
/// cycle (see [`count_failed_attempts_in_current_cycle`]) so a fresh
/// `max_account_failures` is required every cycle, not just once ever.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if a database query fails.
pub async fn record_failed_login_and_check_lockout(
    pool: &PgPool,
    username: &str,
    ip: &str,
    max_account_failures: i64,
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    sqlx::query!(
        "INSERT INTO login_attempts (username, ip, success) VALUES ($1, $2, false)",
        username,
        ip,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    // Lock the user row (a no-op if the username doesn't exist -- the
    // dummy-hash path) so concurrent failed attempts for the same account
    // serialize here: without this, two concurrent transactions could each
    // count the failures committed so far, both land just under
    // `max_account_failures`, and both skip escalation even though their
    // combined total already crossed the threshold. Also read back
    // `locked_until` while holding the lock, needed to scope the count
    // below to the current cycle.
    let locked_until_before: Option<DateTime<Utc>> = sqlx::query_scalar!(
        "SELECT locked_until FROM users WHERE username = $1 FOR UPDATE",
        username
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::Database)?
    .flatten();

    // Count failures in the current cycle only -- see
    // count_failed_attempts_in_current_cycle's doc comment for why this
    // must exclude attempts made before/during the most recent lock rather
    // than counting everything since the last successful login.
    let count =
        count_failed_attempts_in_current_cycle(&mut *tx, username, locked_until_before).await?;

    if count >= max_account_failures {
        // Advance the per-cycle escalation counter only if the account isn't
        // already locked. `login`'s locked-account branch calls this
        // function on every attempt (to keep DB work -- and therefore
        // response timing -- identical to the wrong-password branch), so
        // this guard is load-bearing: without it, continued brute-forcing
        // against an already-locked account would keep extending
        // `locked_until` and advancing the escalation tier further on every
        // single attempt, rather than only once per lockout cycle.
        let escalated: Option<i32> = sqlx::query_scalar!(
            "UPDATE users SET lockout_escalation_level = lockout_escalation_level + 1 WHERE \
             username = $1 AND (locked_until IS NULL OR locked_until <= NOW()) RETURNING \
             lockout_escalation_level",
            username,
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

        // `None` means either the user doesn't exist (the bcrypt dummy-hash
        // path) or the account is already locked -- nothing further to do.
        let Some(escalated) = escalated else {
            tx.commit().await.map_err(ApiError::Database)?;
            return Ok(());
        };

        // The counter was just incremented, so this lockout's tier is one
        // less than the new value -- the first lockout uses index 0, the
        // shortest duration.
        let escalation_level = i64::from(escalated).saturating_sub(1);
        let duration_minutes = LOCKOUT_DURATIONS
            .get(usize::try_from(escalation_level).unwrap_or(0))
            .copied()
            .unwrap_or(*LOCKOUT_DURATIONS.last().unwrap_or(&1));
        let locked_until = Utc::now()
            .checked_add_signed(chrono::Duration::try_minutes(duration_minutes).unwrap_or_default())
            .unwrap_or(Utc::now());

        sqlx::query!(
            "UPDATE users SET locked_until = $1 WHERE username = $2",
            locked_until,
            username,
        )
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

        tx.commit().await.map_err(ApiError::Database)?;

        // Spawned rather than awaited: this insert is pure audit logging,
        // not part of the login response's correctness, and awaiting it
        // here would make the threshold-crossing attempt measurably slower
        // than every attempt before it -- a narrow timing signal an
        // attacker could use to detect exactly which attempt locked the
        // account, in the same spirit as the timing-uniformity work
        // elsewhere in this function.
        let pool = pool.clone();
        let message = format!(
            "Account '{username}' locked until {locked_until} after {count} failed attempts"
        );
        tokio::spawn(async move {
            let _ =
                insert_system_event(&pool, SystemEventType::AccountLocked, None, &message).await;
        });

        return Ok(());
    }

    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_totp_attempt(
    pool: &PgPool,
    user_id: i64,
    ip: &str,
    success: bool,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO totp_attempts (user_id, ip, success) VALUES ($1, $2, $3)",
        user_id,
        ip,
        success,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// A row from the `api_tokens` table (excluding the token hash).
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ApiTokenRow {
    /// Unique identifier.
    pub id: i64,
    /// User ID that owns this token.
    pub user_id: i64,
    /// Human-readable token name.
    pub name: String,
    /// When the token was created.
    pub created_at: DateTime<Utc>,
    /// When the token was last used.
    pub last_used_at: Option<DateTime<Utc>>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_api_token(
    pool: &PgPool,
    user_id: i64,
    name: &str,
    token_hash: &str,
) -> Result<ApiTokenRow, ApiError> {
    sqlx::query_as!(
        ApiTokenRow,
        "INSERT INTO api_tokens (user_id, name, token_hash) VALUES ($1, $2, $3) RETURNING id, \
         user_id, name, created_at, last_used_at",
        user_id,
        name,
        token_hash,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_api_tokens_for_user(
    pool: &PgPool,
    user_id: i64,
) -> Result<Vec<ApiTokenRow>, ApiError> {
    sqlx::query_as!(
        ApiTokenRow,
        "SELECT id, user_id, name, created_at, last_used_at FROM api_tokens WHERE user_id = $1 \
         ORDER BY created_at DESC",
        user_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_all_api_tokens(pool: &PgPool) -> Result<Vec<ApiTokenRow>, ApiError> {
    sqlx::query_as!(
        ApiTokenRow,
        "SELECT id, user_id, name, created_at, last_used_at FROM api_tokens ORDER BY created_at \
         DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn delete_api_token(pool: &PgPool, token_id: i64) -> Result<(), ApiError> {
    let result = sqlx::query!("DELETE FROM api_tokens WHERE id = $1", token_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!(
            "api token {token_id} not found"
        )));
    }
    Ok(())
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_api_token_owner(pool: &PgPool, token_id: i64) -> Result<i64, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        user_id: i64,
    }

    let row = sqlx::query_as!(
        Row,
        "SELECT user_id FROM api_tokens WHERE id = $1",
        token_id
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("api token {token_id} not found")),
        other => ApiError::Database(other),
    })?;
    Ok(row.user_id)
}

/// Minimal row for API token lookup, containing only the user ID.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ApiTokenLookupRow {
    /// User ID that owns the token.
    pub user_id: i64,
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Unauthorized`]: the caller is not authenticated
/// - [`ApiError::Database`]: the database query fails
pub async fn get_user_by_token_hash(
    pool: &PgPool,
    token_hash: &str,
) -> Result<ApiTokenLookupRow, ApiError> {
    let row = sqlx::query_as!(
        ApiTokenLookupRow,
        "SELECT user_id FROM api_tokens WHERE token_hash = $1",
        token_hash,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::Unauthorized("invalid api token".to_string()),
        other => ApiError::Database(other),
    })?;
    Ok(row)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_api_token_last_used(pool: &PgPool, token_hash: &str) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE api_tokens SET last_used_at = NOW() WHERE token_hash = $1",
        token_hash,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// A row from the `repo_permissions` table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct RepoPermissionRow {
    /// User ID.
    pub user_id: i64,
    /// Repository ID.
    pub repo_id: i64,
    /// Whether the user can view the repo.
    pub can_view: bool,
    /// Whether the user can trigger backups.
    pub can_backup: bool,
    /// Whether the user can modify schedules.
    pub can_modify_schedules: bool,
    /// Whether the user can extract archives.
    pub can_extract: bool,
    /// Whether the user can delete archives.
    pub can_delete: bool,
}

/// Parameters for upserting a repo permission.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct UpsertRepoPermissionParams {
    /// User ID.
    pub user_id: i64,
    /// Repository ID.
    pub repo_id: i64,
    /// View permission.
    pub can_view: bool,
    /// Backup permission.
    pub can_backup: bool,
    /// Schedule modification permission.
    pub can_modify_schedules: bool,
    /// Extract permission.
    pub can_extract: bool,
    /// Delete permission.
    pub can_delete: bool,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn upsert_repo_permission(
    pool: &PgPool,
    params: &UpsertRepoPermissionParams,
) -> Result<RepoPermissionRow, ApiError> {
    sqlx::query_as!(
        RepoPermissionRow,
        "INSERT INTO repo_permissions (user_id, repo_id, can_view, can_backup, \
         can_modify_schedules, can_extract, can_delete) VALUES ($1, $2, $3, $4, $5, $6, $7) ON \
         CONFLICT (user_id, repo_id) DO UPDATE SET can_view = $3, can_backup = $4, \
         can_modify_schedules = $5, can_extract = $6, can_delete = $7 RETURNING user_id, repo_id, \
         can_view, can_backup, can_modify_schedules, can_extract, can_delete",
        params.user_id,
        params.repo_id,
        params.can_view,
        params.can_backup,
        params.can_modify_schedules,
        params.can_extract,
        params.can_delete,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_repo_permission(
    pool: &PgPool,
    user_id: i64,
    repo_id: i64,
) -> Result<Option<RepoPermissionRow>, ApiError> {
    sqlx::query_as!(
        RepoPermissionRow,
        "SELECT user_id, repo_id, can_view, can_backup, can_modify_schedules, can_extract, \
         can_delete FROM repo_permissions WHERE user_id = $1 AND repo_id = $2",
        user_id,
        repo_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_repo_permissions_for_user(
    pool: &PgPool,
    user_id: i64,
) -> Result<Vec<RepoPermissionRow>, ApiError> {
    sqlx::query_as!(
        RepoPermissionRow,
        "SELECT user_id, repo_id, can_view, can_backup, can_modify_schedules, can_extract, \
         can_delete FROM repo_permissions WHERE user_id = $1 ORDER BY repo_id",
        user_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_repo_permissions_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<Vec<RepoPermissionRow>, ApiError> {
    sqlx::query_as!(
        RepoPermissionRow,
        "SELECT user_id, repo_id, can_view, can_backup, can_modify_schedules, can_extract, \
         can_delete FROM repo_permissions WHERE repo_id = $1 ORDER BY user_id",
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// A row from the `system_events` table.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct SystemEventRow {
    /// Unique identifier.
    pub id: i64,
    /// When the event occurred.
    pub created_at: DateTime<Utc>,
    /// Event type.
    pub event_type: SystemEventType,
    /// How the event reads - drives the badge and whether it can be
    /// acknowledged.
    pub severity: SystemEventSeverity,
    /// Whether this event reports a problem a human can acknowledge.
    pub acknowledgeable: bool,
    /// Whether a human has acknowledged this event.
    pub acknowledged: bool,
    /// Hostname the event relates to, if any.
    pub hostname: Option<String>,
    /// Human-readable event message.
    pub message: String,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_system_event(
    pool: &PgPool,
    event_type: SystemEventType,
    hostname: Option<&str>,
    message: &str,
) -> Result<(), ApiError> {
    let event_type_str = event_type.to_string();
    sqlx::query!(
        "INSERT INTO system_events (event_type, hostname, message) VALUES ($1, $2, $3)",
        event_type_str,
        hostname,
        message,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_system_events(
    pool: &PgPool,
    limit: i64,
    acknowledged_filter: AcknowledgedFilter,
) -> Result<Vec<SystemEventRow>, ApiError> {
    let rows = sqlx::query!(
        "SELECT id, created_at, event_type, hostname, message, acknowledged FROM system_events \
         WHERE ($1::bool IS NULL OR acknowledged = $1) ORDER BY created_at DESC LIMIT $2",
        acknowledged_filter.as_sql_predicate(),
        limit,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows
        .into_iter()
        .filter_map(|r| match r.event_type.parse::<SystemEventType>() {
            Ok(event_type) => Some(SystemEventRow {
                id: r.id,
                created_at: r.created_at,
                event_type,
                severity: event_type.severity(),
                acknowledgeable: event_type.is_acknowledgeable(),
                acknowledged: r.acknowledged,
                hostname: r.hostname,
                message: r.message,
            }),
            Err(e) => {
                tracing::warn!(
                    row_id = r.id,
                    event_type = %r.event_type,
                    error = %e,
                    "skipping system event row with unrecognized event_type"
                );
                None
            }
        })
        .collect())
}

/// Looks up a system event's type so the caller can reject acknowledging one
/// that reports nothing to review.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if no event has this id,
/// [`ApiError::Unprocessable`] if the event's type is not acknowledgeable, and
/// [`ApiError::Database`] if the query fails.
pub async fn get_acknowledgeable_system_event_type(
    pool: &PgPool,
    id: i64,
) -> Result<SystemEventType, ApiError> {
    let raw = sqlx::query_scalar!("SELECT event_type FROM system_events WHERE id = $1", id)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::Database)?
        .ok_or_else(|| ApiError::NotFound(format!("system event id '{id}' not found")))?;

    let event_type = raw.parse::<SystemEventType>().map_err(|e| {
        ApiError::Unprocessable(format!("system event {id} has an unknown type: {e}"))
    })?;
    if !event_type.is_acknowledgeable() {
        return Err(ApiError::Unprocessable(format!(
            "system event {id} cannot be acknowledged: only warning or failed events can be \
             reviewed"
        )));
    }
    Ok(event_type)
}

/// Sets or clears the acknowledged flag on a system event. Like a backup
/// report's flag, this is shared across all users rather than scoped to
/// whoever clicked it.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if no event has this id, or
/// [`ApiError::Database`] if the query fails.
pub async fn set_system_event_acknowledged(
    pool: &PgPool,
    id: i64,
    acknowledged: bool,
) -> Result<(), ApiError> {
    let rows_affected = sqlx::query!(
        "UPDATE system_events SET acknowledged = $1 WHERE id = $2",
        acknowledged,
        id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?
    .rows_affected();
    if rows_affected == 0 {
        return Err(ApiError::NotFound(format!("system event {id} not found")));
    }
    Ok(())
}

/// The backup report statuses that carry something to acknowledge, as the
/// bind parameters the bulk acknowledgment queries pass to `status IN (..)`.
///
/// Shared so the acknowledge, the "which repositories are affected" lookup and
/// the count can never disagree about what counts as outstanding.
fn acknowledgeable_report_statuses() -> [String; 2] {
    [
        BackupStatus::Warning.to_string(),
        BackupStatus::Failed.to_string(),
    ]
}

#[cfg(test)]
mod acknowledgeable_status_tests {
    use super::{BackupStatus, acknowledgeable_report_statuses};

    /// The bulk queries bind these strings while the per-report check calls
    /// [`BackupStatus::is_acknowledgeable`]; this pins the two to the same set.
    #[test]
    fn bound_statuses_are_exactly_the_acknowledgeable_ones() {
        let bound = acknowledgeable_report_statuses();
        for status in [
            BackupStatus::Success,
            BackupStatus::Warning,
            BackupStatus::Failed,
        ] {
            assert_eq!(
                bound.contains(&status.to_string()),
                status.is_acknowledgeable(),
                "{status} disagrees between the bulk filter and is_acknowledgeable"
            );
        }
    }
}

/// The system event types that report a problem, as the bind parameter the
/// bulk acknowledgment queries pass to `event_type = ANY(..)`.
///
/// The system-event counterpart to [`acknowledgeable_report_statuses`], shared
/// for the same reason: a change to which [`SystemEventType`] variants are
/// acknowledgeable must reach every query at once rather than depending on
/// whoever makes it remembering there is more than one call site.
fn acknowledgeable_system_event_types() -> Vec<String> {
    SystemEventType::ALL
        .iter()
        .filter(|event_type| event_type.is_acknowledgeable())
        .map(ToString::to_string)
        .collect()
}

/// Acknowledges every unacknowledged warning/failed backup report belonging to
/// one of `repo_ids`, and reports how many rows that touched.
///
/// The caller narrows `repo_ids` to the repositories the user may act on, so
/// a bulk acknowledge can never reach further than the same user's per-report
/// acknowledge would.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn acknowledge_backup_reports_in_repos<'e, E>(
    executor: E,
    repo_ids: &[i64],
) -> Result<u64, ApiError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let [warning, failed] = acknowledgeable_report_statuses();
    Ok(sqlx::query!(
        "UPDATE backup_reports SET acknowledged = true WHERE acknowledged = false AND repo_id = \
         ANY($1) AND status IN ($2, $3)",
        repo_ids,
        warning,
        failed,
    )
    .execute(executor)
    .await
    .map_err(ApiError::Database)?
    .rows_affected())
}

/// The repositories that still hold at least one unacknowledged warning or
/// failed backup report, so the caller can permission-check just those.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn repos_with_unacknowledged_reports(pool: &PgPool) -> Result<Vec<i64>, ApiError> {
    let [warning, failed] = acknowledgeable_report_statuses();
    sqlx::query_scalar!(
        "SELECT DISTINCT repo_id FROM backup_reports WHERE acknowledged = false AND status IN \
         ($1, $2) ORDER BY repo_id",
        warning,
        failed,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Counts the unacknowledged warning/failed backup reports belonging to one of
/// `repo_ids`, so the UI can tell whether a bulk acknowledge would do anything
/// without first loading the feed.
///
/// Deliberately mirrors [`acknowledge_backup_reports_in_repos`]'s filter, so
/// "the button is shown" and "the button would acknowledge something" can
/// never disagree.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn count_unacknowledged_reports_in_repos(
    pool: &PgPool,
    repo_ids: &[i64],
) -> Result<i64, ApiError> {
    let [warning, failed] = acknowledgeable_report_statuses();
    Ok(sqlx::query_scalar!(
        "SELECT COUNT(*) FROM backup_reports WHERE acknowledged = false AND repo_id = ANY($1) AND \
         status IN ($2, $3)",
        repo_ids,
        warning,
        failed,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?
    .unwrap_or(0))
}

/// Counts the unacknowledged system events whose type reports a problem, the
/// counterpart to [`count_unacknowledged_reports_in_repos`] for the events an
/// admin can acknowledge.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn count_unacknowledged_system_events(pool: &PgPool) -> Result<i64, ApiError> {
    let acknowledgeable = acknowledgeable_system_event_types();
    Ok(sqlx::query_scalar!(
        "SELECT COUNT(*) FROM system_events WHERE acknowledged = false AND event_type = ANY($1)",
        &acknowledgeable,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?
    .unwrap_or(0))
}

/// Acknowledges every unacknowledged system event whose type reports a
/// problem, and reports how many rows that touched.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn acknowledge_all_system_events<'e, E>(executor: E) -> Result<u64, ApiError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let acknowledgeable = acknowledgeable_system_event_types();
    Ok(sqlx::query!(
        "UPDATE system_events SET acknowledged = true WHERE acknowledged = false AND event_type = \
         ANY($1)",
        &acknowledgeable,
    )
    .execute(executor)
    .await
    .map_err(ApiError::Database)?
    .rows_affected())
}

/// Acknowledges everything the caller may retire in one step - the warning and
/// failed backup reports in `repo_ids`, and, when `include_system_events`, the
/// problem-reporting system events - and reports the two counts.
///
/// One transaction, so a request the client sees fail leaves nothing
/// half-acknowledged: without it a failure on the second write would return
/// 500 while the first had already committed.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn acknowledge_all_outstanding(
    pool: &PgPool,
    repo_ids: &[i64],
    include_system_events: bool,
) -> Result<(u64, u64), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    let backup_reports = acknowledge_backup_reports_in_repos(&mut *tx, repo_ids).await?;
    let system_events = if include_system_events {
        acknowledge_all_system_events(&mut *tx).await?
    } else {
        0
    };
    tx.commit().await.map_err(ApiError::Database)?;
    Ok((backup_reports, system_events))
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_setting(pool: &PgPool, key: &str) -> Result<Option<String>, ApiError> {
    let row: Option<String> =
        sqlx::query_scalar!("SELECT value FROM system_settings WHERE key = $1", key)
            .fetch_optional(pool)
            .await
            .map_err(ApiError::Database)?;
    Ok(row)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_setting(pool: &PgPool, key: &str, value: &str) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO system_settings (key, value, updated_at) VALUES ($1, $2, NOW()) ON CONFLICT \
         (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
        key,
        value,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Size breakdown of a database relation (table + indexes + TOAST).
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct DatabaseRelationSizeRow {
    /// Table name.
    pub table_name: String,
    /// Bytes used by the main table.
    pub table_bytes: i64,
    /// Bytes used by indexes.
    pub index_bytes: i64,
    /// Bytes used by TOAST storage.
    pub toast_bytes: i64,
    /// Total bytes (table + indexes + TOAST).
    pub total_bytes: i64,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_database_storage(
    pool: &PgPool,
) -> Result<(i64, Vec<DatabaseRelationSizeRow>), ApiError> {
    let total_bytes: Option<i64> =
        sqlx::query_scalar!("SELECT pg_database_size(current_database())::BIGINT",)
            .fetch_one(pool)
            .await
            .map_err(ApiError::Database)?;

    let relations = sqlx::query_as!(
        DatabaseRelationSizeRow,
        "WITH sizes AS ( SELECT relname::TEXT AS table_name, pg_relation_size(relid)::BIGINT AS \
         table_bytes, pg_indexes_size(relid)::BIGINT AS index_bytes, \
         (pg_total_relation_size(relid) - pg_relation_size(relid) - \
         pg_indexes_size(relid))::BIGINT AS toast_bytes, pg_total_relation_size(relid)::BIGINT AS \
         total_bytes FROM pg_catalog.pg_statio_user_tables ) SELECT table_name AS \
         \"table_name!\", table_bytes AS \"table_bytes!\", index_bytes AS \"index_bytes!\", \
         toast_bytes AS \"toast_bytes!\", total_bytes AS \"total_bytes!\" FROM sizes ORDER BY \
         total_bytes DESC, table_name ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok((total_bytes.unwrap_or(0), relations))
}

/// # Errors
///
/// Returns [`ApiError::Internal`] if an internal error occurs.
pub async fn get_schedule_timezone(pool: &PgPool) -> Result<chrono_tz::Tz, ApiError> {
    let tz_str = get_setting(pool, "timezone").await?.unwrap_or_default();
    shared::schedule::parse_timezone(&tz_str)
        .map_err(|e| ApiError::Internal(format!("invalid timezone setting: {e}")))
}

/// Prunes old login-attempt history by age. `record_failed_login_and_check_lockout`
/// now also records attempts against nonexistent usernames (needed for the
/// constant-time dummy-hash login path), so this table grows without bound
/// otherwise. A 90-day cutoff is far longer than the longest lockout tier
/// (24h), so this cannot interfere with an in-progress lockout escalation in
/// practice.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_login_attempts_before(
    pool: &PgPool,
    before: DateTime<Utc>,
) -> Result<u64, ApiError> {
    let result = sqlx::query!("DELETE FROM login_attempts WHERE attempted_at < $1", before)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_system_events_before(
    pool: &PgPool,
    before: DateTime<Utc>,
) -> Result<u64, ApiError> {
    let result = sqlx::query!("DELETE FROM system_events WHERE created_at < $1", before)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// Prunes old notification delivery-attempt history by age. The table is
/// kept "for debugging and retry" (see `0002_notifications.sql`), not as a
/// permanent audit log, so it grows without bound otherwise.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_notification_deliveries_before(
    pool: &PgPool,
    before: DateTime<Utc>,
) -> Result<u64, ApiError> {
    let result = sqlx::query!(
        "DELETE FROM notification_deliveries WHERE attempted_at < $1",
        before
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// Prunes old backup-run history by age.
///
/// Reports that carry an `archive_name` represent an actual borg archive and
/// double as the archive list, so they must never be aged out here: imported
/// and synced archives keep their original (often very old) borg `start`
/// timestamp, and their lifecycle is governed by borg plus the sync stale
/// removal, not by the report-retention window. Only run history without an
/// archive (pending/started/failed/cancelled) is pruned.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_backup_reports_before(
    pool: &PgPool,
    before: DateTime<Utc>,
) -> Result<u64, ApiError> {
    let result = sqlx::query!(
        "DELETE FROM backup_reports WHERE started_at < $1 AND archive_name IS NULL",
        before,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_backup_reports_with_archive_before(
    pool: &PgPool,
    before: DateTime<Utc>,
) -> Result<u64, ApiError> {
    let result = sqlx::query!(
        "DELETE FROM backup_reports WHERE started_at < $1 AND archive_name IS NOT NULL",
        before,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// Deletes all failed backup-run history for an agent, on demand.
///
/// A failed run *usually* carries no `archive_name` (no archive was
/// produced), but `run_backup`'s create step can succeed and a later
/// prune/compact/post-backup-hook step can still fail the run overall - so
/// this guards on `archive_name IS NULL` too, matching
/// [`delete_backup_reports_before`]'s age-based equivalent, rather than
/// ever discarding the only report row linking to a retained archive.
/// Unlike that age-based retention, this acts immediately regardless of
/// `failed_report_retention_days`.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_failed_backup_reports_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<u64, ApiError> {
    let result = sqlx::query!(
        "DELETE FROM backup_reports WHERE agent_id = $1 AND status = 'failed' AND archive_name IS \
         NULL",
        agent_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// Deletes all failed backup-run history for a schedule, on demand. See
/// [`delete_failed_backup_reports_for_agent`] - same rationale, scoped to a
/// schedule instead of an agent.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_failed_backup_reports_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<u64, ApiError> {
    let result = sqlx::query!(
        "DELETE FROM backup_reports WHERE schedule_id = $1 AND status = 'failed' AND archive_name \
         IS NULL",
        schedule_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// Counts an agent's failed backup-run history, unbounded by the report-list
/// pagination `limit` a page's own display uses. A "clean up failed backups"
/// confirmation must state how many records [`delete_failed_backup_reports_for_agent`]
/// is actually about to remove, not how many happen to fall within the most
/// recently displayed page of reports - so this carries the same
/// `archive_name IS NULL` guard as that delete, or the count would overstate
/// what the delete actually removes.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn count_failed_backup_reports_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<i64, ApiError> {
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: Option<i64>,
    }

    let row = sqlx::query_as!(
        CountRow,
        "SELECT COUNT(*) as count FROM backup_reports WHERE agent_id = $1 AND status = 'failed' \
         AND archive_name IS NULL",
        agent_id,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(row.count.unwrap_or(0))
}

/// Counts a schedule's failed backup-run history. See
/// [`count_failed_backup_reports_for_agent`] - same rationale, scoped to a
/// schedule instead of an agent.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn count_failed_backup_reports_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<i64, ApiError> {
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: Option<i64>,
    }

    let row = sqlx::query_as!(
        CountRow,
        "SELECT COUNT(*) as count FROM backup_reports WHERE schedule_id = $1 AND status = \
         'failed' AND archive_name IS NULL",
        schedule_id,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(row.count.unwrap_or(0))
}

/// Looks up the repository a backup report belongs to, so the caller can run
/// a repo-scoped permission check before acknowledging it.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if no report has this id, or
/// [`ApiError::Database`] if the query fails.
pub async fn get_backup_report_repo_id(pool: &PgPool, id: i64) -> Result<i64, ApiError> {
    sqlx::query_scalar!("SELECT repo_id FROM backup_reports WHERE id = $1", id)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::Database)?
        .ok_or_else(|| ApiError::NotFound(format!("report id '{id}' not found")))
}

/// Looks up the repository a backup report belongs to, for the acknowledge
/// endpoint specifically - unlike [`get_backup_report_repo_id`], this also
/// rejects a report whose status isn't warning/failed, since a successful
/// run has nothing to review (matching the frontend's own `isAckable` gate,
/// which this backs up rather than duplicates trust in).
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if no report has this id,
/// [`ApiError::Unprocessable`] if the report's status isn't warning or
/// failed, or [`ApiError::Database`] if the query fails.
pub async fn get_ackable_backup_report_repo_id(pool: &PgPool, id: i64) -> Result<i64, ApiError> {
    let row = sqlx::query!(
        "SELECT repo_id, status FROM backup_reports WHERE id = $1",
        id
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?
    .ok_or_else(|| ApiError::NotFound(format!("report id '{id}' not found")))?;

    let ackable = row
        .status
        .parse::<BackupStatus>()
        .is_ok_and(BackupStatus::is_acknowledgeable);
    if !ackable {
        return Err(ApiError::Unprocessable(format!(
            "report {id} cannot be acknowledged: only warning or failed runs can be reviewed"
        )));
    }
    Ok(row.repo_id)
}

/// Sets or clears the acknowledged flag on a backup report. Acknowledging is
/// a shared, all-users action - like the report itself, it isn't scoped to
/// whoever clicked it.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_backup_report_acknowledged(
    pool: &PgPool,
    id: i64,
    acknowledged: bool,
) -> Result<(), ApiError> {
    let rows_affected = sqlx::query!(
        "UPDATE backup_reports SET acknowledged = $1 WHERE id = $2",
        acknowledged,
        id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?
    .rows_affected();
    if rows_affected == 0 {
        return Err(ApiError::NotFound(format!("backup report {id} not found")));
    }
    Ok(())
}

/// Get user preferences.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the query fails.
pub async fn get_user_preferences(
    pool: &PgPool,
    user_id: i64,
) -> Result<serde_json::Value, ApiError> {
    let row: Option<serde_json::Value> =
        sqlx::query_scalar!("SELECT preferences FROM users WHERE id = $1", user_id)
            .fetch_optional(pool)
            .await
            .map_err(ApiError::Database)?;
    Ok(row.unwrap_or(serde_json::Value::Null))
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_user_preferences(
    pool: &PgPool,
    user_id: i64,
    preferences: &serde_json::Value,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE users SET preferences = $1 WHERE id = $2",
        preferences,
        user_id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// Full repository row with aggregated stats (sizes, agent count, import state, last op).
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring this query's own columns via query_as!, not \
              mutually-exclusive states; splitting into enums or sub-structs would require \
              restructuring the query_as! call in list_repos_with_stats/get_repo_with_stats for \
              no correctness benefit -- the API layer (RepoWithStatsResponse) already nests the \
              power-related ones"
)]
pub struct RepoWithStatsRow {
    /// Unique identifier.
    pub id: i64,
    /// Repository display name.
    pub name: String,
    /// Borg repository path.
    pub repo_path: String,
    /// SSH user.
    pub ssh_user: String,
    /// SSH hostname.
    pub ssh_host: String,
    /// SSH port.
    pub ssh_port: i32,
    /// Known host key.
    pub ssh_host_key: Option<String>,
    /// Compression algorithm.
    pub compression: String,
    /// Encryption mode.
    pub encryption: String,
    /// Whether the repository is enabled.
    pub enabled: bool,
    /// Whether the repo is currently being imported.
    pub importing: bool,
    /// Import error message, if any.
    pub import_error: Option<String>,
    /// Import progress (items processed).
    pub import_progress: i32,
    /// Import total items.
    pub import_total: i32,
    /// Import status message.
    pub import_status_message: Option<String>,
    /// Owning user ID.
    pub owner_id: Option<i64>,
    /// Visibility scope.
    pub visibility: String,
    /// Sync schedule cron expression.
    pub sync_schedule: Option<String>,
    /// When the repo was last synced.
    pub last_synced_at: Option<DateTime<Utc>>,
    /// Number of archives.
    pub archive_count: i64,
    /// When the last successful backup finished.
    pub last_backup_at: Option<DateTime<Utc>>,
    /// Total original size in bytes.
    pub total_original_size: i64,
    /// Total compressed size in bytes.
    pub total_compressed_size: i64,
    /// Total deduplicated size in bytes.
    pub total_deduplicated_size: i64,
    /// Number of distinct agents that backed up to this repo.
    pub agent_count: i64,
    /// Number of unmatched agents (imported placeholders).
    pub unmatched_count: i64,
    /// Kind of the last operation performed on the repo.
    pub last_op_kind: Option<String>,
    /// Whether a relocation is pending confirmation.
    pub relocation_pending: bool,
    /// When the last operation was performed.
    pub last_op_at: Option<DateTime<Utc>>,
    /// Who performed the last operation.
    pub last_op_by: Option<String>,
    /// Own quota warn threshold in bytes, if a quota row exists.
    pub quota_warn_bytes: Option<i64>,
    /// Own quota critical threshold in bytes, if a quota row exists.
    pub quota_critical_bytes: Option<i64>,
    /// Own quota warn action, if a quota row exists.
    pub quota_warn_action: Option<String>,
    /// Own quota critical action, if a quota row exists.
    pub quota_critical_action: Option<String>,
    /// Whether a quota row exists for this repo, and if so, whether it's enabled.
    /// `NULL` means no quota is configured at all.
    pub quota_enabled: Option<bool>,
    /// Whether to send a Wake-on-LAN packet before a backup if the
    /// repository host isn't already reachable over SSH.
    pub wake_enabled: bool,
    /// MAC address to wake, required when `wake_enabled`.
    pub wake_mac_address: Option<String>,
    /// Broadcast address the magic packet is sent to (defaults to the
    /// global broadcast address when unset).
    pub wake_broadcast_address: Option<String>,
    /// How long to wait for the host to come online after waking it.
    pub wake_timeout_seconds: i32,
    /// Whether to shut the host down after the backup, but only if this run
    /// is what woke it.
    pub shutdown_after_backup: bool,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_repos_with_stats(pool: &PgPool) -> Result<Vec<RepoWithStatsRow>, ApiError> {
    sqlx::query_as!(
        RepoWithStatsRow,
        "SELECT r.id, r.name, r.repo_path, r.ssh_user, r.ssh_host, r.ssh_port, r.ssh_host_key, \
         r.compression, r.encryption, r.enabled, r.owner_id, r.visibility, r.sync_schedule, \
         r.wake_enabled, r.wake_mac_address, r.wake_broadcast_address, r.wake_timeout_seconds, \
         r.shutdown_after_backup, r.relocation_pending, COALESCE(rs.original_size, 0) AS \
         \"total_original_size!\", COALESCE(rs.compressed_size, 0) AS \"total_compressed_size!\", \
         COALESCE(rs.deduplicated_size, 0) AS \"total_deduplicated_size!\", \
         COALESCE(rs.archive_count::INT8, 0) AS \"archive_count!\", rs.last_synced_at AS \
         \"last_synced_at?\", COALESCE(ris.importing, false) AS \"importing!\", ris.error AS \
         \"import_error?\", COALESCE(ris.progress, 0) AS \"import_progress!\", \
         COALESCE(ris.total, 0) AS \"import_total!\", ris.status_message AS \
         \"import_status_message?\", rlo.kind AS \"last_op_kind?\", rlo.at AS \"last_op_at?\", \
         rlo.by_text AS \"last_op_by?\", agg.last_backup_at AS \"last_backup_at?\", \
         COALESCE(agg.agent_count, 0) AS \"agent_count!\", COALESCE(agg.unmatched_count, 0) AS \
         \"unmatched_count!\", q.warn_bytes AS \"quota_warn_bytes?\", q.critical_bytes AS \
         \"quota_critical_bytes?\", q.warn_action AS \"quota_warn_action?\", q.critical_action AS \
         \"quota_critical_action?\", q.enabled AS \"quota_enabled?\" FROM repos r LEFT JOIN \
         repo_stats rs ON rs.repo_id = r.id LEFT JOIN repo_import_state ris ON ris.repo_id = r.id \
         LEFT JOIN repo_last_op rlo ON rlo.repo_id = r.id LEFT JOIN repo_quotas q ON q.repo_id = \
         r.id LEFT JOIN LATERAL (SELECT MAX(CASE WHEN br.finished_at > '1970-01-01T00:00:00Z' \
         THEN br.finished_at END) AS last_backup_at, COUNT(DISTINCT br.agent_id) AS agent_count, \
         COUNT(DISTINCT br.agent_id) FILTER (WHERE br.matched = false) AS unmatched_count FROM \
         backup_reports br WHERE br.repo_id = r.id AND br.status = 'success') agg ON true ORDER \
         BY r.name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_repo_with_stats(
    pool: &PgPool,
    repo_id: i64,
) -> Result<RepoWithStatsRow, ApiError> {
    sqlx::query_as!(
        RepoWithStatsRow,
        "SELECT r.id, r.name, r.repo_path, r.ssh_user, r.ssh_host, r.ssh_port, r.ssh_host_key, \
         r.compression, r.encryption, r.enabled, r.owner_id, r.visibility, r.sync_schedule, \
         r.wake_enabled, r.wake_mac_address, r.wake_broadcast_address, r.wake_timeout_seconds, \
         r.shutdown_after_backup, r.relocation_pending, COALESCE(rs.original_size, 0) AS \
         \"total_original_size!\", COALESCE(rs.compressed_size, 0) AS \"total_compressed_size!\", \
         COALESCE(rs.deduplicated_size, 0) AS \"total_deduplicated_size!\", \
         COALESCE(rs.archive_count::INT8, 0) AS \"archive_count!\", rs.last_synced_at AS \
         \"last_synced_at?\", COALESCE(ris.importing, false) AS \"importing!\", ris.error AS \
         \"import_error?\", COALESCE(ris.progress, 0) AS \"import_progress!\", \
         COALESCE(ris.total, 0) AS \"import_total!\", ris.status_message AS \
         \"import_status_message?\", rlo.kind AS \"last_op_kind?\", rlo.at AS \"last_op_at?\", \
         rlo.by_text AS \"last_op_by?\", agg.last_backup_at AS \"last_backup_at?\", \
         COALESCE(agg.agent_count, 0) AS \"agent_count!\", COALESCE(agg.unmatched_count, 0) AS \
         \"unmatched_count!\", q.warn_bytes AS \"quota_warn_bytes?\", q.critical_bytes AS \
         \"quota_critical_bytes?\", q.warn_action AS \"quota_warn_action?\", q.critical_action AS \
         \"quota_critical_action?\", q.enabled AS \"quota_enabled?\" FROM repos r LEFT JOIN \
         repo_stats rs ON rs.repo_id = r.id LEFT JOIN repo_import_state ris ON ris.repo_id = r.id \
         LEFT JOIN repo_last_op rlo ON rlo.repo_id = r.id LEFT JOIN repo_quotas q ON q.repo_id = \
         r.id LEFT JOIN LATERAL (SELECT MAX(CASE WHEN br.finished_at > '1970-01-01T00:00:00Z' \
         THEN br.finished_at END) AS last_backup_at, COUNT(DISTINCT br.agent_id) AS agent_count, \
         COUNT(DISTINCT br.agent_id) FILTER (WHERE br.matched = false) AS unmatched_count FROM \
         backup_reports br WHERE br.repo_id = r.id AND br.status = 'success') agg ON true WHERE \
         r.id = $1",
        repo_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn update_repo_last_op(
    pool: &PgPool,
    repo_id: i64,
    kind: &str,
    at: chrono::DateTime<chrono::Utc>,
    by: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO repo_last_op (repo_id, kind, at, by_text) VALUES ($1, $2, $3, $4) ON \
         CONFLICT (repo_id) DO UPDATE SET kind = EXCLUDED.kind, at = EXCLUDED.at, by_text = \
         EXCLUDED.by_text",
        repo_id,
        kind,
        at,
        by,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

/// A row from the `tags` table.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct TagRow {
    /// Unique identifier.
    pub id: i64,
    /// Tag name.
    pub name: String,
    /// Tag color (hex string).
    pub color: String,
    /// Tag scope (e.g. "agent", "repo").
    pub scope: String,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_tags(pool: &PgPool, scope: &str) -> Result<Vec<TagRow>, ApiError> {
    sqlx::query_as!(
        TagRow,
        "SELECT id, name, color, scope FROM tags WHERE scope = $1 ORDER BY name",
        scope,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_tag(
    pool: &PgPool,
    name: &str,
    color: &str,
    scope: &str,
) -> Result<TagRow, ApiError> {
    sqlx::query_as!(
        TagRow,
        "INSERT INTO tags (name, color, scope) VALUES ($1, $2, $3) RETURNING id, name, color, \
         scope",
        name,
        color,
        scope,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn delete_tag(pool: &PgPool, id: i64) -> Result<(), ApiError> {
    let result = sqlx::query!("DELETE FROM tags WHERE id = $1", id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("tag {id} not found")));
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_repo_tags(pool: &PgPool, repo_id: i64, tag_ids: &[i64]) -> Result<(), ApiError> {
    sqlx::query!("DELETE FROM repo_tags WHERE repo_id = $1", repo_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    for tag_id in tag_ids {
        sqlx::query!(
            "INSERT INTO repo_tags (repo_id, tag_id) VALUES ($1, $2)",
            repo_id,
            tag_id
        )
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_agent_tags(pool: &PgPool, agent_id: i64, tag_ids: &[i64]) -> Result<(), ApiError> {
    sqlx::query!("DELETE FROM agent_tags WHERE agent_id = $1", agent_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    for tag_id in tag_ids {
        sqlx::query!(
            "INSERT INTO agent_tags (agent_id, tag_id) VALUES ($1, $2)",
            agent_id,
            tag_id
        )
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    }
    Ok(())
}

/// A tag associated with a repository (joined from `repo_tags` + `tags`).
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct RepoTagRow {
    /// Repository ID.
    pub repo_id: i64,
    /// Tag name.
    pub tag_name: String,
    /// Tag color.
    pub tag_color: String,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_all_repo_tags(pool: &PgPool) -> Result<Vec<RepoTagRow>, ApiError> {
    sqlx::query_as!(
        RepoTagRow,
        "SELECT rt.repo_id, t.name AS tag_name, t.color AS tag_color FROM repo_tags rt JOIN tags \
         t ON t.id = rt.tag_id ORDER BY rt.repo_id, t.name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_tags_for_repo(pool: &PgPool, repo_id: i64) -> Result<Vec<TagRow>, ApiError> {
    sqlx::query_as!(
        TagRow,
        "SELECT t.id, t.name, t.color, t.scope FROM tags t JOIN repo_tags rt ON rt.tag_id = t.id \
         WHERE rt.repo_id = $1 ORDER BY t.name",
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// A tag associated with an agent (joined from `agent_tags` + `tags`).
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct AgentTagRow {
    /// Agent ID.
    pub agent_id: i64,
    /// Tag name.
    pub tag_name: String,
    /// Tag color.
    pub tag_color: String,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_tags_for_agent(pool: &PgPool, agent_id: i64) -> Result<Vec<TagRow>, ApiError> {
    sqlx::query_as!(
        TagRow,
        "SELECT t.id, t.name, t.color, t.scope FROM tags t JOIN agent_tags at ON at.tag_id = t.id \
         WHERE at.agent_id = $1 ORDER BY t.name",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_all_agent_tags(pool: &PgPool) -> Result<Vec<AgentTagRow>, ApiError> {
    sqlx::query_as!(
        AgentTagRow,
        "SELECT at.agent_id, t.name AS tag_name, t.color AS tag_color FROM agent_tags at JOIN \
         tags t ON t.id = at.tag_id ORDER BY at.agent_id, t.name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Dashboard summary aggregated from all tables.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct DashboardSummaryRow {
    /// Total non-hidden agents.
    pub total_agents: i64,
    /// Total repositories.
    pub total_repos: i64,
    /// Number of enabled schedules.
    pub active_schedules: i64,
    /// Total number of schedules.
    pub total_schedules: i64,
    /// Total deduplicated storage across all repos.
    pub total_storage_bytes: i64,
    /// When the last successful backup finished.
    pub last_backup_at: Option<DateTime<Utc>>,
    /// When the next backup is scheduled.
    pub next_backup_at: Option<DateTime<Utc>>,
    /// Schedule ID of the last backup.
    pub last_backup_schedule_id: Option<i64>,
    /// Repo ID of the last backup.
    pub last_backup_repo_id: Option<i64>,
    /// Archive name of the last backup.
    pub last_backup_archive_name: Option<String>,
    /// Schedule ID of the next backup.
    pub next_backup_schedule_id: Option<i64>,
    /// Successful backups in the last 30 days.
    pub success_30d: i64,
    /// Failed backups in the last 30 days.
    pub failed_30d: i64,
    /// Total backups in the last 30 days.
    pub total_30d: i64,
    /// When the last failure occurred.
    pub last_failure_at: Option<DateTime<Utc>>,
    /// When the last warning occurred.
    pub last_warning_at: Option<DateTime<Utc>>,
    /// Schedule ID of the last failure.
    pub last_failure_schedule_id: Option<i64>,
    /// Schedule ID of the last warning.
    pub last_warning_schedule_id: Option<i64>,
    /// Error message from the last failure.
    pub last_failure_message: Option<String>,
    /// Warning message from the last warning.
    pub last_warning_message: Option<String>,
    /// Repo ID of the last failure.
    pub last_failure_repo_id: Option<i64>,
    /// Repo ID of the last warning.
    pub last_warning_repo_id: Option<i64>,
    /// Repo name of the last failure.
    pub last_failure_repo_name: Option<String>,
    /// Repo name of the last warning.
    pub last_warning_repo_name: Option<String>,
    /// Schedule name (cron expression) of the last failure.
    pub last_failure_schedule_name: Option<String>,
    /// Schedule name (cron expression) of the last warning.
    pub last_warning_schedule_name: Option<String>,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_dashboard_summary(pool: &PgPool) -> Result<DashboardSummaryRow, ApiError> {
    // The last_failure_*/last_warning_* CTEs skip acknowledged reports: once someone has
    // reviewed a warning or failure in the Activity Log, the dashboard's "Last failure" and
    // "Last warning" tiles fall back to the most recent one still awaiting review.
    //
    // Rewritten from ~18 independent correlated "last matching row" subqueries (each a
    // full sort of the filtered backup_reports rows, run on every dashboard load) into a
    // handful of CTEs that each scan/sort once. Every CTE's WHERE clause is copied verbatim
    // from the subquery(s) it replaces -- including the couple of intentional asymmetries
    // in the original (e.g. last_backup_at applies the epoch-sentinel guard but
    // last_backup_repo_id/last_backup_archive_name don't; the *_schedule_id/*_schedule_name
    // fields require a resolvable schedule_id while the sibling *_at/*_message/*_repo_*
    // fields don't) -- so the result set is identical, just computed more cheaply now that
    // `idx_backup_reports_status_finished_at` covers the ORDER BY.
    sqlx::query_as!(
        DashboardSummaryRow,
        "WITH last_success_at AS ( SELECT MAX(finished_at) AS finished_at FROM backup_reports \
         WHERE status = 'success' AND finished_at > '1970-01-01T00:00:00Z' ), last_success_row AS \
         ( SELECT br.repo_id, br.archive_name FROM backup_reports br WHERE br.status = 'success' \
         ORDER BY br.finished_at DESC LIMIT 1 ), last_backup_row AS ( SELECT br.schedule_id FROM \
         backup_reports br WHERE br.schedule_id IS NOT NULL ORDER BY br.finished_at DESC LIMIT 1 \
         ), next_backup_row AS ( SELECT s.id, s.next_run_at FROM schedules s JOIN repos r ON r.id \
         = s.repo_id WHERE s.enabled = true AND r.enabled = true AND s.next_run_at IS NOT NULL \
         AND s.next_run_at > NOW() ORDER BY s.next_run_at LIMIT 1 ), last_failure_general AS ( \
         SELECT br.finished_at, br.error_message, br.repo_id, r.name AS repo_name FROM \
         backup_reports br JOIN repos r ON r.id = br.repo_id WHERE br.status = 'failed' AND \
         br.acknowledged = false AND br.finished_at > '1970-01-01T00:00:00Z' ORDER BY \
         br.finished_at DESC LIMIT 1 ), last_failure_scheduled AS ( SELECT br.schedule_id, \
         s.cron_expression AS schedule_name FROM backup_reports br JOIN schedules s ON s.id = \
         br.schedule_id WHERE br.status = 'failed' AND br.acknowledged = false AND br.finished_at \
         > '1970-01-01T00:00:00Z' ORDER BY br.finished_at DESC LIMIT 1 ), last_warning_general AS \
         ( SELECT br.finished_at, br.warnings[1] AS warning_message, br.repo_id, r.name AS \
         repo_name FROM backup_reports br JOIN repos r ON r.id = br.repo_id WHERE br.status = \
         'warning' AND br.acknowledged = false AND br.finished_at > '1970-01-01T00:00:00Z' ORDER \
         BY br.finished_at DESC LIMIT 1 ), last_warning_scheduled AS ( SELECT br.schedule_id, \
         s.cron_expression AS schedule_name FROM backup_reports br JOIN schedules s ON s.id = \
         br.schedule_id WHERE br.status = 'warning' AND br.acknowledged = false AND \
         br.finished_at > '1970-01-01T00:00:00Z' ORDER BY br.finished_at DESC LIMIT 1 ) SELECT \
         (SELECT COUNT(*) FROM agents WHERE is_hidden = false) AS \"total_agents!\", (SELECT \
         COUNT(*) FROM repos) AS \"total_repos!\", (SELECT COUNT(*) FROM schedules WHERE enabled \
         = true) AS \"active_schedules!\", (SELECT COUNT(*) FROM schedules) AS \
         \"total_schedules!\", COALESCE((SELECT SUM(deduplicated_size) FROM repo_stats), 0)::INT8 \
         AS \"total_storage_bytes!\", last_success_at.finished_at AS last_backup_at, \
         next_backup_row.next_run_at AS next_backup_at, last_backup_row.schedule_id AS \
         last_backup_schedule_id, last_success_row.repo_id AS last_backup_repo_id, \
         last_success_row.archive_name AS last_backup_archive_name, next_backup_row.id AS \
         next_backup_schedule_id, (SELECT COUNT(*) FROM backup_reports WHERE status = 'success' \
         AND started_at > NOW() - INTERVAL '30 days') AS \"success_30d!\", (SELECT COUNT(*) FROM \
         backup_reports WHERE status != 'success' AND started_at > NOW() - INTERVAL '30 days') AS \
         \"failed_30d!\", (SELECT COUNT(*) FROM backup_reports WHERE started_at > NOW() - \
         INTERVAL '30 days') AS \"total_30d!\", last_failure_general.finished_at AS \
         last_failure_at, last_warning_general.finished_at AS last_warning_at, \
         last_failure_scheduled.schedule_id AS last_failure_schedule_id, \
         last_warning_scheduled.schedule_id AS last_warning_schedule_id, \
         last_failure_general.error_message AS last_failure_message, \
         last_warning_general.warning_message AS last_warning_message, \
         last_failure_general.repo_id AS last_failure_repo_id, last_warning_general.repo_id AS \
         last_warning_repo_id, last_failure_general.repo_name AS last_failure_repo_name, \
         last_warning_general.repo_name AS last_warning_repo_name, \
         last_failure_scheduled.schedule_name AS last_failure_schedule_name, \
         last_warning_scheduled.schedule_name AS last_warning_schedule_name FROM (SELECT 1) AS \
         one LEFT JOIN last_success_at ON true LEFT JOIN last_success_row ON true LEFT JOIN \
         last_backup_row ON true LEFT JOIN next_backup_row ON true LEFT JOIN last_failure_general \
         ON true LEFT JOIN last_failure_scheduled ON true LEFT JOIN last_warning_general ON true \
         LEFT JOIN last_warning_scheduled ON true",
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// Storage breakdown by repository.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StorageBreakdownRow {
    /// Repository name.
    pub name: String,
    /// Compressed size in bytes.
    pub compressed_size: i64,
    /// Deduplicated size in bytes.
    pub deduplicated_size: i64,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_storage_breakdown(pool: &PgPool) -> Result<Vec<StorageBreakdownRow>, ApiError> {
    sqlx::query_as!(
        StorageBreakdownRow,
        "SELECT r.name, COALESCE(rs.compressed_size, 0)::INT8 AS \"compressed_size!\", \
         COALESCE(rs.deduplicated_size, 0)::INT8 AS \"deduplicated_size!\" FROM repos r LEFT JOIN \
         repo_stats rs ON rs.repo_id = r.id ORDER BY rs.deduplicated_size DESC NULLS LAST",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_activity_feed_days(
    pool: &PgPool,
    days: i64,
    // Caps rows *per schedule*, not the result set overall - a plain global
    // LIMIT would let one frequently-running schedule's reports crowd out
    // every row belonging to a less-frequent one in the ranked window.
    per_schedule_limit: Option<i64>,
    filters: ActivityFeedFilters<'_>,
) -> Result<Vec<ActivityRow>, ApiError> {
    sqlx::query_as!(
        ActivityRow,
        "SELECT id, hostname, target_name, started_at, finished_at, status AS \"status!\", \
         duration_secs AS \"duration_secs!\", repo_id, archive_name, error_message, schedule_id, \
         schedule_name AS \"schedule_name?\", run_id, acknowledged AS \"acknowledged!\" FROM ( \
         SELECT br.id, a.hostname, r.name AS target_name, br.started_at, br.finished_at, \
         br.status, br.duration_secs, br.repo_id, br.archive_name, br.error_message, \
         br.schedule_id, s.name AS schedule_name, br.run_id, br.acknowledged, ROW_NUMBER() OVER \
         (PARTITION BY br.schedule_id ORDER BY br.started_at DESC) AS rn FROM backup_reports br \
         JOIN agents a ON a.id = br.agent_id JOIN repos r ON r.id = br.repo_id LEFT JOIN \
         schedules s ON s.id = br.schedule_id WHERE a.is_hidden = false AND a.visibility <> \
         'hidden' AND COALESCE(a.display_name, '') NOT ILIKE '%(imported)%' AND br.started_at > \
         NOW() - make_interval(days => $1::int) AND ($2::bigint IS NULL OR br.repo_id = $2) AND \
         ($3::text IS NULL OR a.hostname = $3) AND ($4::bigint IS NULL OR br.schedule_id = $4) \
         AND ($5::text IS NULL OR br.run_id = $5) AND ($6::bool IS NULL OR br.acknowledged = $6) \
         ) ranked WHERE $7::bigint IS NULL OR rn <= $7 ORDER BY started_at DESC",
        i32::try_from(days).unwrap_or(14),
        filters.repo_id,
        filters.hostname,
        filters.schedule_id,
        filters.run_id,
        filters.acknowledged.as_sql_predicate(),
        per_schedule_limit,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// A row from the `groups` table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct GroupRow {
    /// Unique identifier.
    pub id: i64,
    /// Group name.
    pub name: String,
    /// Optional group description.
    pub description: Option<String>,
    /// When the group was created.
    pub created_at: DateTime<Utc>,
}

/// A row from the `roles` table representing an RBAC role.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct RoleRow {
    /// Unique identifier.
    pub id: i64,
    /// Role name.
    pub name: String,
    /// Permission to create agents.
    pub can_create_agent: bool,
    /// Permission to delete any agent.
    pub can_delete_agent: bool,
    /// Permission to delete own agents.
    pub can_delete_own_agent: bool,
    /// Permission to create repos.
    pub can_create_repo: bool,
    /// Permission to delete any repo.
    pub can_delete_repo: bool,
    /// Permission to delete own repos.
    pub can_delete_own_repo: bool,
    /// Permission to create schedules.
    pub can_create_schedule: bool,
    /// Permission to delete any schedule.
    pub can_delete_schedule: bool,
    /// Permission to delete own schedules.
    pub can_delete_own_schedule: bool,
    /// Permission to manage tags.
    pub can_manage_tags: bool,
    /// Permission to view all repos.
    pub can_view_all_repos: bool,
    /// Permission to manage tunnels.
    pub can_manage_tunnels: bool,
    /// Permission to upgrade agents.
    pub can_upgrade_agent: bool,
    /// When the role was created.
    pub created_at: DateTime<Utc>,
}

/// A row from the `user_groups` join table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct UserGroupRow {
    /// User ID.
    pub user_id: i64,
    /// Group ID.
    pub group_id: i64,
}

/// A row from the `user_roles` join table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct UserRoleRow {
    /// User ID.
    pub user_id: i64,
    /// Role ID.
    pub role_id: i64,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_groups(pool: &PgPool) -> Result<Vec<GroupRow>, ApiError> {
    sqlx::query_as!(
        GroupRow,
        "SELECT id, name, description, created_at FROM groups ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_group(pool: &PgPool, id: i64) -> Result<Option<GroupRow>, ApiError> {
    sqlx::query_as!(
        GroupRow,
        "SELECT id, name, description, created_at FROM groups WHERE id = $1",
        id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_group(
    pool: &PgPool,
    name: &str,
    description: Option<&str>,
) -> Result<GroupRow, ApiError> {
    sqlx::query_as!(
        GroupRow,
        "INSERT INTO groups (name, description) VALUES ($1, $2) RETURNING id, name, description, \
         created_at",
        name,
        description,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn update_group(
    pool: &PgPool,
    id: i64,
    name: &str,
    description: Option<&str>,
) -> Result<GroupRow, ApiError> {
    sqlx::query_as!(
        GroupRow,
        "UPDATE groups SET name = $2, description = $3 WHERE id = $1 RETURNING id, name, \
         description, created_at",
        id,
        name,
        description,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("group {id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn delete_group(pool: &PgPool, id: i64) -> Result<(), ApiError> {
    let result = sqlx::query!("DELETE FROM groups WHERE id = $1", id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("group {id} not found")));
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_group_members(pool: &PgPool, group_id: i64) -> Result<Vec<i64>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        user_id: i64,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT user_id FROM user_groups WHERE group_id = $1 ORDER BY user_id",
        group_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows.into_iter().map(|r| r.user_id).collect())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_group_members(
    pool: &PgPool,
    group_id: i64,
    user_ids: &[i64],
) -> Result<(), ApiError> {
    sqlx::query!("DELETE FROM user_groups WHERE group_id = $1", group_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    for user_id in user_ids {
        sqlx::query!(
            "INSERT INTO user_groups (user_id, group_id) VALUES ($1, $2)",
            user_id,
            group_id
        )
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_user_groups(pool: &PgPool, user_id: i64) -> Result<Vec<GroupRow>, ApiError> {
    sqlx::query_as!(
        GroupRow,
        "SELECT g.id, g.name, g.description, g.created_at FROM groups g JOIN user_groups ug ON \
         ug.group_id = g.id WHERE ug.user_id = $1 ORDER BY g.name",
        user_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn user_shares_group_with(
    pool: &PgPool,
    user_id: i64,
    other_user_id: i64,
) -> Result<bool, ApiError> {
    #[derive(sqlx::FromRow)]
    struct ExistsRow {
        shared: Option<bool>,
    }

    let row = sqlx::query_as!(
        ExistsRow,
        "SELECT EXISTS(SELECT 1 FROM user_groups a JOIN user_groups b ON a.group_id = b.group_id \
         WHERE a.user_id = $1 AND b.user_id = $2) AS shared",
        user_id,
        other_user_id,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(row.shared.unwrap_or(false))
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_roles(pool: &PgPool) -> Result<Vec<RoleRow>, ApiError> {
    sqlx::query_as!(
        RoleRow,
        "SELECT id, name, can_create_agent, can_delete_agent, can_delete_own_agent, \
         can_create_repo, can_delete_repo, can_delete_own_repo, can_create_schedule, \
         can_delete_schedule, can_delete_own_schedule, can_manage_tags, can_view_all_repos, \
         can_manage_tunnels, can_upgrade_agent, created_at FROM roles ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_role(pool: &PgPool, id: i64) -> Result<Option<RoleRow>, ApiError> {
    sqlx::query_as!(
        RoleRow,
        "SELECT id, name, can_create_agent, can_delete_agent, can_delete_own_agent, \
         can_create_repo, can_delete_repo, can_delete_own_repo, can_create_schedule, \
         can_delete_schedule, can_delete_own_schedule, can_manage_tags, can_view_all_repos, \
         can_manage_tunnels, can_upgrade_agent, created_at FROM roles WHERE id = $1",
        id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// Parameters for inserting a new role.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring the API/DB contract, not mutually-exclusive states"
)]
pub struct InsertRoleParams<'a> {
    /// Role name.
    pub name: &'a str,
    /// Create agents permission.
    pub can_create_agent: bool,
    /// Delete any agent permission.
    pub can_delete_agent: bool,
    /// Delete own agents permission.
    pub can_delete_own_agent: bool,
    /// Create repos permission.
    pub can_create_repo: bool,
    /// Delete any repo permission.
    pub can_delete_repo: bool,
    /// Delete own repos permission.
    pub can_delete_own_repo: bool,
    /// Create schedules permission.
    pub can_create_schedule: bool,
    /// Delete any schedule permission.
    pub can_delete_schedule: bool,
    /// Delete own schedules permission.
    pub can_delete_own_schedule: bool,
    /// Manage tags permission.
    pub can_manage_tags: bool,
    /// View all repos permission.
    pub can_view_all_repos: bool,
    /// Manage tunnels permission.
    pub can_manage_tunnels: bool,
    /// Upgrade agents permission.
    pub can_upgrade_agent: bool,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_role(
    pool: &PgPool,
    params: &InsertRoleParams<'_>,
) -> Result<RoleRow, ApiError> {
    sqlx::query_as!(
        RoleRow,
        "INSERT INTO roles (name, can_create_agent, can_delete_agent, can_delete_own_agent, \
         can_create_repo, can_delete_repo, can_delete_own_repo, can_create_schedule, \
         can_delete_schedule, can_delete_own_schedule, can_manage_tags, can_view_all_repos, \
         can_manage_tunnels, can_upgrade_agent) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, \
         $11, $12, $13, $14) RETURNING id, name, can_create_agent, can_delete_agent, \
         can_delete_own_agent, can_create_repo, can_delete_repo, can_delete_own_repo, \
         can_create_schedule, can_delete_schedule, can_delete_own_schedule, can_manage_tags, \
         can_view_all_repos, can_manage_tunnels, can_upgrade_agent, created_at",
        params.name,
        params.can_create_agent,
        params.can_delete_agent,
        params.can_delete_own_agent,
        params.can_create_repo,
        params.can_delete_repo,
        params.can_delete_own_repo,
        params.can_create_schedule,
        params.can_delete_schedule,
        params.can_delete_own_schedule,
        params.can_manage_tags,
        params.can_view_all_repos,
        params.can_manage_tunnels,
        params.can_upgrade_agent,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn update_role(
    pool: &PgPool,
    id: i64,
    params: &InsertRoleParams<'_>,
) -> Result<RoleRow, ApiError> {
    sqlx::query_as!(
        RoleRow,
        "UPDATE roles SET name = $2, can_create_agent = $3, can_delete_agent = $4, \
         can_delete_own_agent = $5, can_create_repo = $6, can_delete_repo = $7, \
         can_delete_own_repo = $8, can_create_schedule = $9, can_delete_schedule = $10, \
         can_delete_own_schedule = $11, can_manage_tags = $12, can_view_all_repos = $13, \
         can_manage_tunnels = $14, can_upgrade_agent = $15 WHERE id = $1 RETURNING id, name, \
         can_create_agent, can_delete_agent, can_delete_own_agent, can_create_repo, \
         can_delete_repo, can_delete_own_repo, can_create_schedule, can_delete_schedule, \
         can_delete_own_schedule, can_manage_tags, can_view_all_repos, can_manage_tunnels, \
         can_upgrade_agent, created_at",
        id,
        params.name,
        params.can_create_agent,
        params.can_delete_agent,
        params.can_delete_own_agent,
        params.can_create_repo,
        params.can_delete_repo,
        params.can_delete_own_repo,
        params.can_create_schedule,
        params.can_delete_schedule,
        params.can_delete_own_schedule,
        params.can_manage_tags,
        params.can_view_all_repos,
        params.can_manage_tunnels,
        params.can_upgrade_agent,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("role {id} not found")),
        other => ApiError::Database(other),
    })
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the requested resource does not exist
pub async fn delete_role(pool: &PgPool, id: i64) -> Result<(), ApiError> {
    let result = sqlx::query!("DELETE FROM roles WHERE id = $1", id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("role {id} not found")));
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_user_roles(pool: &PgPool, user_id: i64) -> Result<Vec<RoleRow>, ApiError> {
    sqlx::query_as!(
        RoleRow,
        "SELECT r.id, r.name, r.can_create_agent, r.can_delete_agent, r.can_delete_own_agent, \
         r.can_create_repo, r.can_delete_repo, r.can_delete_own_repo, r.can_create_schedule, \
         r.can_delete_schedule, r.can_delete_own_schedule, r.can_manage_tags, \
         r.can_view_all_repos, r.can_manage_tunnels, r.can_upgrade_agent, r.created_at FROM roles \
         r JOIN user_roles ur ON ur.role_id = r.id WHERE ur.user_id = $1 ORDER BY r.name",
        user_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn set_user_roles(pool: &PgPool, user_id: i64, role_ids: &[i64]) -> Result<(), ApiError> {
    sqlx::query!("DELETE FROM user_roles WHERE user_id = $1", user_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    for role_id in role_ids {
        sqlx::query!(
            "INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)",
            user_id,
            role_id
        )
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    }
    Ok(())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_effective_permissions(pool: &PgPool, user_id: i64) -> Result<RoleRow, ApiError> {
    #[derive(sqlx::FromRow)]
    #[allow(
        clippy::struct_field_names,
        reason = "matches the can_* RBAC column/field naming used consistently across the codebase"
    )]
    struct AggRow {
        can_create_agent: Option<bool>,
        can_delete_agent: Option<bool>,
        can_delete_own_agent: Option<bool>,
        can_create_repo: Option<bool>,
        can_delete_repo: Option<bool>,
        can_delete_own_repo: Option<bool>,
        can_create_schedule: Option<bool>,
        can_delete_schedule: Option<bool>,
        can_delete_own_schedule: Option<bool>,
        can_manage_tags: Option<bool>,
        can_view_all_repos: Option<bool>,
        can_manage_tunnels: Option<bool>,
        can_upgrade_agent: Option<bool>,
    }

    let row = sqlx::query_as!(
        AggRow,
        "SELECT BOOL_OR(r.can_create_agent) AS can_create_agent, BOOL_OR(r.can_delete_agent) AS \
         can_delete_agent, BOOL_OR(r.can_delete_own_agent) AS can_delete_own_agent, \
         BOOL_OR(r.can_create_repo) AS can_create_repo, BOOL_OR(r.can_delete_repo) AS \
         can_delete_repo, BOOL_OR(r.can_delete_own_repo) AS can_delete_own_repo, \
         BOOL_OR(r.can_create_schedule) AS can_create_schedule, BOOL_OR(r.can_delete_schedule) AS \
         can_delete_schedule, BOOL_OR(r.can_delete_own_schedule) AS can_delete_own_schedule, \
         BOOL_OR(r.can_manage_tags) AS can_manage_tags, BOOL_OR(r.can_view_all_repos) AS \
         can_view_all_repos, BOOL_OR(r.can_manage_tunnels) AS can_manage_tunnels, \
         BOOL_OR(r.can_upgrade_agent) AS can_upgrade_agent FROM roles r JOIN user_roles ur ON \
         ur.role_id = r.id WHERE ur.user_id = $1",
        user_id,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(RoleRow {
        id: 0,
        name: String::from("effective"),
        can_create_agent: row.can_create_agent.unwrap_or(false),
        can_delete_agent: row.can_delete_agent.unwrap_or(false),
        can_delete_own_agent: row.can_delete_own_agent.unwrap_or(false),
        can_create_repo: row.can_create_repo.unwrap_or(false),
        can_delete_repo: row.can_delete_repo.unwrap_or(false),
        can_delete_own_repo: row.can_delete_own_repo.unwrap_or(false),
        can_create_schedule: row.can_create_schedule.unwrap_or(false),
        can_delete_schedule: row.can_delete_schedule.unwrap_or(false),
        can_delete_own_schedule: row.can_delete_own_schedule.unwrap_or(false),
        can_manage_tags: row.can_manage_tags.unwrap_or(false),
        can_view_all_repos: row.can_view_all_repos.unwrap_or(false),
        can_manage_tunnels: row.can_manage_tunnels.unwrap_or(false),
        can_upgrade_agent: row.can_upgrade_agent.unwrap_or(false),
        created_at: Utc::now(),
    })
}

/// A single day's aggregated backup trend data.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TrendRow {
    /// The date of the trend point.
    pub date: chrono::NaiveDate,
    /// Average original size.
    pub original_size: i64,
    /// Average compressed size.
    pub compressed_size: i64,
    /// Average deduplicated size.
    pub deduplicated_size: i64,
    /// Average file count.
    pub file_count: i64,
    /// Average duration in seconds.
    pub duration_seconds: i64,
    /// Number of backups on this date.
    pub backup_count: i64,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_backup_trends(
    pool: &PgPool,
    repo_id: Option<i64>,
    days: i64,
) -> Result<Vec<TrendRow>, ApiError> {
    let days = i32::try_from(days).unwrap_or(30);
    if let Some(rid) = repo_id {
        sqlx::query_as!(
            TrendRow,
            "SELECT started_at::date AS \"date!\", COALESCE(AVG(original_size), 0)::INT8 AS \
             \"original_size!\", COALESCE(AVG(compressed_size), 0)::INT8 AS \"compressed_size!\", \
             COALESCE(AVG(deduplicated_size), 0)::INT8 AS \"deduplicated_size!\", \
             COALESCE(AVG(files_processed), 0)::INT8 AS \"file_count!\", \
             COALESCE(AVG(duration_secs), 0)::INT8 AS \"duration_seconds!\", COUNT(*)::INT8 AS \
             \"backup_count!\" FROM backup_reports WHERE repo_id = $1 AND started_at > NOW() - \
             make_interval(days => $2) GROUP BY started_at::date ORDER BY 1",
            rid,
            days,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    } else {
        sqlx::query_as!(
            TrendRow,
            "SELECT started_at::date AS \"date!\", COALESCE(AVG(original_size), 0)::INT8 AS \
             \"original_size!\", COALESCE(AVG(compressed_size), 0)::INT8 AS \"compressed_size!\", \
             COALESCE(AVG(deduplicated_size), 0)::INT8 AS \"deduplicated_size!\", \
             COALESCE(AVG(files_processed), 0)::INT8 AS \"file_count!\", \
             COALESCE(AVG(duration_secs), 0)::INT8 AS \"duration_seconds!\", COUNT(*)::INT8 AS \
             \"backup_count!\" FROM backup_reports WHERE started_at > NOW() - make_interval(days \
             => $1) GROUP BY started_at::date ORDER BY 1",
            days,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    }
}

/// A calendar event representing a backup run.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct CalendarEventRow {
    /// Event date.
    pub date: chrono::NaiveDate,
    /// Event type (e.g. "backup").
    pub event_type: String,
    /// Backup status.
    pub status: String,
    /// Repository name.
    pub repo_name: String,
    /// Agent hostname.
    pub hostname: String,
    /// Event time string (HH:MM).
    pub time: String,
    /// Report ID, if any.
    pub report_id: Option<i64>,
    /// Repository ID, if any.
    pub repo_id: Option<i64>,
    /// Error message, if any.
    pub error_message: Option<String>,
    /// Archive name, if any.
    pub archive_name: Option<String>,
}

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::BadRequest`]: the request is invalid
/// - [`ApiError::Database`]: the database query fails
pub async fn get_calendar_events(
    pool: &PgPool,
    year: i32,
    month: u32,
    repo_id: Option<i64>,
    tz: chrono_tz::Tz,
) -> Result<Vec<CalendarEventRow>, ApiError> {
    let start = chrono::NaiveDate::from_ymd_opt(year, month, 1)
        .ok_or_else(|| ApiError::BadRequest("invalid month".to_string()))?;
    let end = if month == 12 {
        year.checked_add(1)
            .and_then(|y| chrono::NaiveDate::from_ymd_opt(y, 1, 1))
    } else {
        month
            .checked_add(1)
            .and_then(|m| chrono::NaiveDate::from_ymd_opt(year, m, 1))
    }
    .ok_or_else(|| ApiError::BadRequest("invalid month".to_string()))?;

    let tz_name = tz.name();

    if let Some(rid) = repo_id {
        sqlx::query_as!(
            CalendarEventRow,
            "SELECT (br.started_at AT TIME ZONE $4)::date AS \"date!\", 'backup' AS \
             \"event_type!\", CASE WHEN br.status = 'success' THEN 'success' ELSE 'failed' END AS \
             \"status!\", r.name AS \"repo_name!\", a.hostname AS \"hostname!\", \
             to_char(br.started_at AT TIME ZONE $4, 'HH24:MI') AS \"time!\", br.id AS \
             \"report_id?\", br.repo_id AS \"repo_id?\", br.error_message, br.archive_name FROM \
             backup_reports br JOIN repos r ON r.id = br.repo_id JOIN agents a ON a.id = \
             br.agent_id WHERE a.is_hidden = false AND (br.started_at AT TIME ZONE $4)::date >= \
             $1 AND (br.started_at AT TIME ZONE $4)::date < $2 AND br.repo_id = $3 ORDER BY \
             br.started_at",
            start,
            end,
            rid,
            tz_name,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    } else {
        sqlx::query_as!(
            CalendarEventRow,
            "SELECT (br.started_at AT TIME ZONE $3)::date AS \"date!\", 'backup' AS \
             \"event_type!\", CASE WHEN br.status = 'success' THEN 'success' ELSE 'failed' END AS \
             \"status!\", r.name AS \"repo_name!\", a.hostname AS \"hostname!\", \
             to_char(br.started_at AT TIME ZONE $3, 'HH24:MI') AS \"time!\", br.id AS \
             \"report_id?\", br.repo_id AS \"repo_id?\", br.error_message, br.archive_name FROM \
             backup_reports br JOIN repos r ON r.id = br.repo_id JOIN agents a ON a.id = \
             br.agent_id WHERE a.is_hidden = false AND (br.started_at AT TIME ZONE $3)::date >= \
             $1 AND (br.started_at AT TIME ZONE $3)::date < $2 ORDER BY br.started_at",
            start,
            end,
            tz_name,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    }
}

/// A single day's storage trend data (cumulative across all repos).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StorageTrendRow {
    /// The date of the trend point.
    pub date: chrono::NaiveDate,
    /// Cumulative original size up to this date.
    pub original_size: i64,
    /// Cumulative compressed size up to this date.
    pub compressed_size: i64,
    /// Latest deduplicated (``repo_unique_csize``) as of this date.
    pub deduplicated_size: Option<i64>,
}

/// A single day's storage trend data, per repository.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StorageTrendByRepoRow {
    /// The date of the trend point.
    pub date: chrono::NaiveDate,
    /// Repository ID.
    pub repo_id: i64,
    /// Repository name.
    pub repo_name: String,
    /// Cumulative original size up to this date.
    pub original_size: i64,
    /// Cumulative compressed size up to this date.
    pub compressed_size: i64,
    /// Latest deduplicated size as of this date.
    pub deduplicated_size: Option<i64>,
}

/// `original_size`/`compressed_size` are the cumulative sum, across every archive taken up to
/// that date, of that archive's (pre-deduplication) size; this mirrors how borg itself defines
/// a repository's total (non-deduplicated) size. `deduplicated_size` is the repository's actual
/// unique compressed size (`repo_unique_csize`) as of the most recent archive on or before that
/// date. Mixing a single archive's per-archive size with the repo-wide deduplicated size would
/// make the deduplicated line exceed the original/compressed lines, which is impossible.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_storage_trends(
    pool: &PgPool,
    repo_id: Option<i64>,
    days: i64,
) -> Result<Vec<StorageTrendRow>, ApiError> {
    let days = i32::try_from(days).unwrap_or(30);
    if let Some(rid) = repo_id {
        // Single-pass rewrite: aggregate each day's reports once (`daily`), then derive the
        // cumulative original/compressed totals and the forward-filled latest dedup snapshot
        // via window functions over just the (bounded) `days` series, instead of re-scanning
        // the entire report history with a correlated subquery per displayed day.
        sqlx::query_as!(
            StorageTrendRow,
            "WITH days AS ( SELECT generate_series( (CURRENT_DATE - make_interval(days => \
             $1))::date, CURRENT_DATE, '1 day'::interval )::date AS date ), daily AS ( SELECT \
             br.started_at::date AS date, SUM(br.original_size) AS day_original, \
             SUM(br.compressed_size) AS day_compressed, (ARRAY_AGG(br.repo_unique_csize ORDER BY \
             br.started_at DESC))[1] AS day_csize FROM backup_reports br WHERE br.repo_id = $2 \
             AND br.status = 'success' GROUP BY br.started_at::date ), joined AS ( SELECT d.date, \
             dl.day_original, dl.day_compressed, dl.day_csize, COUNT(dl.date) OVER (ORDER BY \
             d.date) AS fill_grp FROM days d LEFT JOIN daily dl ON dl.date = d.date ) SELECT date \
             AS \"date!\", COALESCE(SUM(day_original) OVER (ORDER BY date), 0)::INT8 AS \
             \"original_size!\", COALESCE(SUM(day_compressed) OVER (ORDER BY date), 0)::INT8 AS \
             \"compressed_size!\", NULLIF(MAX(day_csize) OVER (PARTITION BY fill_grp), 0)::INT8 \
             AS \"deduplicated_size?\" FROM joined ORDER BY date",
            days,
            rid,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    } else {
        // Same single-pass approach, but the daily rollup and cumulative window are computed
        // per repo first (`per_repo`) -- matching the original per-repo "latest known dedup
        // size" semantics -- then summed across repos per day. `days LEFT JOIN fleet_by_date`
        // (rather than driving from `days CROSS JOIN repos_list`) keeps the "always emit one
        // row per requested day" behaviour even when no repos have any reports yet.
        sqlx::query_as!(
            StorageTrendRow,
            "WITH days AS ( SELECT generate_series( (CURRENT_DATE - make_interval(days => \
             $1))::date, CURRENT_DATE, '1 day'::interval )::date AS date ), repos_list AS ( \
             SELECT DISTINCT br.repo_id FROM backup_reports br WHERE br.status = 'success' ), \
             daily AS ( SELECT br.repo_id, br.started_at::date AS date, SUM(br.original_size) AS \
             day_original, SUM(br.compressed_size) AS day_compressed, \
             (ARRAY_AGG(br.repo_unique_csize ORDER BY br.started_at DESC))[1] AS day_csize FROM \
             backup_reports br WHERE br.status = 'success' GROUP BY br.repo_id, \
             br.started_at::date ), joined AS ( SELECT rl.repo_id, d.date, dl.day_original, \
             dl.day_compressed, dl.day_csize, COUNT(dl.date) OVER (PARTITION BY rl.repo_id ORDER \
             BY d.date) AS fill_grp FROM repos_list rl CROSS JOIN days d LEFT JOIN daily dl ON \
             dl.repo_id = rl.repo_id AND dl.date = d.date ), per_repo AS ( SELECT repo_id, date, \
             COALESCE(SUM(day_original) OVER (PARTITION BY repo_id ORDER BY date), 0) AS \
             cum_original, COALESCE(SUM(day_compressed) OVER (PARTITION BY repo_id ORDER BY \
             date), 0) AS cum_compressed, MAX(day_csize) OVER (PARTITION BY repo_id, fill_grp) AS \
             cum_csize FROM joined ), fleet_by_date AS ( SELECT date, SUM(cum_original) AS \
             original_size, SUM(cum_compressed) AS compressed_size, SUM(COALESCE(cum_csize, 0)) \
             AS csize_sum FROM per_repo GROUP BY date ) SELECT d.date AS \"date!\", \
             COALESCE(f.original_size, 0)::INT8 AS \"original_size!\", \
             COALESCE(f.compressed_size, 0)::INT8 AS \"compressed_size!\", \
             NULLIF(COALESCE(f.csize_sum, 0), 0)::INT8 AS \"deduplicated_size?\" FROM days d LEFT \
             JOIN fleet_by_date f ON f.date = d.date ORDER BY d.date",
            days,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    }
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_archive_names_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<std::collections::HashSet<String>, ApiError> {
    let names: Vec<String> = sqlx::query_scalar!(
        "SELECT archive_name FROM backup_reports WHERE repo_id = $1 AND archive_name IS NOT NULL",
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?
    .into_iter()
    .flatten()
    .collect();
    Ok(names.into_iter().collect())
}

/// Archive names that need a `borg info` run.
///
/// Covers two cases:
/// - All sizes are still zero (archive was imported but never enriched).
/// - `repo_unique_csize` is zero even though other sizes are populated (archive was enriched
///   before `repo_unique_csize` was tracked).
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_archive_names_needing_stats(
    pool: &PgPool,
    repo_id: i64,
) -> Result<std::collections::HashSet<String>, ApiError> {
    let names: Vec<String> = sqlx::query_scalar!(
        "SELECT DISTINCT archive_name FROM backup_reports WHERE repo_id = $1 AND archive_name IS \
         NOT NULL AND ((original_size = 0 AND compressed_size = 0 AND deduplicated_size = 0) OR \
         repo_unique_csize = 0)",
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?
    .into_iter()
    .flatten()
    .collect();
    Ok(names.into_iter().collect())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_archive_reports_by_names(
    pool: &PgPool,
    repo_id: i64,
    names: &[String],
) -> Result<u64, ApiError> {
    if names.is_empty() {
        return Ok(0);
    }
    let result = sqlx::query!(
        "DELETE FROM backup_reports WHERE repo_id = $1 AND archive_name = ANY($2)",
        repo_id,
        names
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_archive_records_by_names(
    pool: &PgPool,
    repo_id: i64,
    names: &[String],
) -> Result<u64, ApiError> {
    if names.is_empty() {
        return Ok(0);
    }

    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    let result = sqlx::query!(
        "DELETE FROM backup_reports WHERE repo_id = $1 AND archive_name = ANY($2)",
        repo_id,
        names,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    // Collect candidate path IDs before the cascade delete removes archive_dirs.
    let candidate_ids: Vec<i64> = sqlx::query_scalar!(
        "SELECT DISTINCT dir_path_id AS \"dir_path_id!\" FROM archive_dirs WHERE archive_id IN \
         (SELECT id FROM archives WHERE repo_id = $1 AND name = ANY($2))",
        repo_id,
        names,
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    // Deleting from archives cascades to archive_dirs, archive_index_jobs, and archive_tags.
    sqlx::query!(
        "DELETE FROM archives WHERE repo_id = $1 AND name = ANY($2)",
        repo_id,
        names,
    )
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    // GC paths that are now orphaned, checking only the candidates from the deleted archives.
    if !candidate_ids.is_empty() {
        sqlx::query!(
            "DELETE FROM archive_paths WHERE repo_id = $1 AND id = ANY($2) AND NOT EXISTS (SELECT \
             1 FROM archive_dirs WHERE dir_path_id = archive_paths.id)",
            repo_id,
            &candidate_ids,
        )
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;
    }

    tx.commit().await.map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_all_repo_archive_data(pool: &PgPool, repo_id: i64) -> Result<u64, ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    // Collect candidate path IDs before the cascade delete removes archive_dirs.
    let candidate_ids: Vec<i64> = sqlx::query_scalar!(
        "SELECT DISTINCT dir_path_id AS \"dir_path_id!\" FROM archive_dirs WHERE archive_id IN \
         (SELECT id FROM archives WHERE repo_id = $1)",
        repo_id,
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    // Delete all backup_reports for the repo.
    let result = sqlx::query!("DELETE FROM backup_reports WHERE repo_id = $1", repo_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

    // Deleting from archives cascades to archive_dirs, archive_index_jobs, and archive_tags.
    sqlx::query!("DELETE FROM archives WHERE repo_id = $1", repo_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

    // GC paths that are now orphaned, checking only the candidates from the deleted archives.
    if !candidate_ids.is_empty() {
        sqlx::query!(
            "DELETE FROM archive_paths WHERE repo_id = $1 AND id = ANY($2) AND NOT EXISTS (SELECT \
             1 FROM archive_dirs WHERE dir_path_id = archive_paths.id)",
            repo_id,
            &candidate_ids,
        )
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;
    }

    tx.commit().await.map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_orphaned_placeholder_agents(pool: &PgPool) -> Result<u64, ApiError> {
    let result = sqlx::query!(
        "DELETE FROM agents WHERE agent_token_hash = 'imported:no-auth' AND NOT EXISTS (SELECT 1 \
         FROM backup_reports WHERE agent_id = agents.id)",
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

/// See [`get_storage_trends`] for why `original_size`/`compressed_size` are a cumulative sum
/// over all archives up to that date while `deduplicated_size` is the latest repo-wide
/// `repo_unique_csize` snapshot.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_storage_trends_by_repo(
    pool: &PgPool,
    days: i64,
) -> Result<Vec<StorageTrendByRepoRow>, ApiError> {
    let days_i32 = i32::try_from(days).unwrap_or(30);
    // Same single-pass rewrite as `get_storage_trends`: aggregate each repo's reports per day
    // once (`daily`), then derive per-(repo, day) cumulative totals and the forward-filled
    // latest dedup snapshot via window functions, instead of a correlated subquery per
    // (day, repo) pair that re-scans that repo's entire history each time.
    sqlx::query_as!(
        StorageTrendByRepoRow,
        "WITH days AS ( SELECT generate_series( (CURRENT_DATE - make_interval(days => $1))::date, \
         CURRENT_DATE, '1 day'::interval )::date AS date ), repos_list AS ( SELECT DISTINCT r.id \
         AS repo_id, r.name AS repo_name FROM repos r JOIN backup_reports br ON br.repo_id = r.id \
         ), daily AS ( SELECT br.repo_id, br.started_at::date AS date, SUM(br.original_size) AS \
         day_original, SUM(br.compressed_size) AS day_compressed, (ARRAY_AGG(br.repo_unique_csize \
         ORDER BY br.started_at DESC))[1] AS day_csize FROM backup_reports br WHERE br.status = \
         'success' GROUP BY br.repo_id, br.started_at::date ), joined AS ( SELECT rl.repo_id, \
         rl.repo_name, d.date, dl.day_original, dl.day_compressed, dl.day_csize, COUNT(dl.date) \
         OVER (PARTITION BY rl.repo_id ORDER BY d.date) AS fill_grp FROM repos_list rl CROSS JOIN \
         days d LEFT JOIN daily dl ON dl.repo_id = rl.repo_id AND dl.date = d.date ) SELECT date \
         AS \"date!\", repo_id AS \"repo_id!\", repo_name AS \"repo_name!\", \
         COALESCE(SUM(day_original) OVER (PARTITION BY repo_id ORDER BY date), 0)::INT8 AS \
         \"original_size!\", COALESCE(SUM(day_compressed) OVER (PARTITION BY repo_id ORDER BY \
         date), 0)::INT8 AS \"compressed_size!\", NULLIF(MAX(day_csize) OVER (PARTITION BY \
         repo_id, fill_grp), 0)::INT8 AS \"deduplicated_size?\" FROM joined ORDER BY date, \
         repo_name",
        days_i32,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_enabled_schedules_for_calendar(
    pool: &PgPool,
) -> Result<Vec<ScheduleRow>, ApiError> {
    let rows = sqlx::query_as!(
        ScheduleRow,
        "SELECT id, repo_id, name, schedule_type, cron_expression, enabled, canary_enabled, \
         last_run_at, next_run_at, exclude_patterns_raw, file_change_patterns_raw, \
         ignore_global_excludes, keep_hourly, keep_daily, keep_weekly, keep_monthly, keep_yearly, \
         compact_enabled, rate_limit_kbps, pre_backup_commands AS \"pre_backup_commands: \
         sqlx::types::Json<Vec<String>>\", post_backup_commands AS \"post_backup_commands: \
         sqlx::types::Json<Vec<String>>\", hook_timeout_seconds, missed_backup_threshold, \
         execution_mode, on_failure, owner_id, visibility, consecutive_failures, \
         auto_disabled_agent_unreachable, ARRAY[]::TEXT[] AS \"target_hostnames!\" FROM schedules \
         WHERE enabled = true",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows)
}
