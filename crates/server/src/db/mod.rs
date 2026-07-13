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
/// Server-level quota database queries.
pub mod server_quota;
/// Tag database queries.
pub mod tags;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared::types::{Compression, ScheduleType};
use sqlx::PgPool;

use crate::error::ApiError;

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
pub async fn resolve_agent_for_hostname(
    pool: &PgPool,
    hostname: &str,
) -> Result<ResolveResult, ApiError> {
    let exact = sqlx::query_as!(
        AgentRow,
        "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
         default_post_backup_commands, default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user FROM agents WHERE hostname = $1 AND agent_token_hash != \
         'imported:no-auth'",
        hostname,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;

    if let Some(agent) = exact {
        return Ok(ResolveResult::ExactMatch(agent));
    }

    if let Some(agent) = patterns::find_agent_by_pattern(pool, hostname).await? {
        return Ok(ResolveResult::PatternMatch(agent));
    }

    Ok(ResolveResult::Unmatched)
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
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
         default_post_backup_commands, default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user FROM agents WHERE id = $1",
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

    sqlx::query!("DELETE FROM agents WHERE id = $1", source_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

/// A row from the `agents` table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
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
    /// Visibility scope (e.g. "public", "private").
    pub visibility: String,
    /// Default backup paths for schedules targeting this agent.
    #[serde(default)]
    pub default_backup_paths: Vec<String>,
    /// Default exclude patterns for schedules targeting this agent.
    #[serde(default)]
    pub default_exclude_patterns: Vec<String>,
    /// Default pre-backup commands.
    pub default_pre_backup_commands: String,
    /// Default post-backup commands.
    pub default_post_backup_commands: String,
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
    /// Pre-backup commands (raw text).
    pub pre_backup_commands: String,
    /// Post-backup commands (raw text).
    pub post_backup_commands: String,
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

/// # Errors
///
/// Returns an error if:
/// - [`ApiError::NotFound`]: the requested resource does not exist
/// - [`ApiError::Database`]: the database query fails
pub async fn get_agent_by_hostname(pool: &PgPool, hostname: &str) -> Result<AgentRow, ApiError> {
    sqlx::query_as!(
        AgentRow,
        "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
         default_post_backup_commands, default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user FROM agents WHERE hostname = $1",
        hostname,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent '{hostname}' not found")),
        other => ApiError::Database(other),
    })
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
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
         default_post_backup_commands, default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user FROM agents WHERE id = $1",
        agent_id,
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
pub async fn get_agent_token_hash(
    pool: &PgPool,
    hostname: &str,
) -> Result<(i64, String), ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i64,
        agent_token_hash: String,
    }

    let row = sqlx::query_as!(
        Row,
        "SELECT id, agent_token_hash FROM agents WHERE hostname = $1",
        hostname
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent '{hostname}' not found")),
        other => ApiError::Database(other),
    })?;

    Ok((row.id, row.agent_token_hash))
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
             default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
             default_post_backup_commands, default_file_change_patterns_raw, agent_token_hash, \
             is_hidden, last_ssh_user FROM agents ORDER BY hostname",
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    } else {
        sqlx::query_as!(
            AgentRow,
            "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
             agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
             default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
             default_post_backup_commands, default_file_change_patterns_raw, agent_token_hash, \
             is_hidden, last_ssh_user FROM agents WHERE is_hidden = false ORDER BY hostname",
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
    hostname: &str,
    hidden: bool,
) -> Result<AgentRow, ApiError> {
    sqlx::query_as!(
        AgentRow,
        "UPDATE agents SET is_hidden = $2 WHERE hostname = $1 RETURNING id, hostname, \
         display_name, agent_version, agent_git_sha, agent_build_time, agent_commit_count, \
         created_at, last_seen_at, owner_id, visibility, default_backup_paths, \
         default_exclude_patterns, default_pre_backup_commands, default_post_backup_commands, \
         default_file_change_patterns_raw, agent_token_hash, is_hidden, last_ssh_user",
        hostname,
        hidden,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?
    .ok_or_else(|| ApiError::NotFound(format!("Agent {hostname} not found")))
}

/// Finds an agent by hostname, or creates a placeholder agent for archive imports.
///
/// Placeholder agents have a dummy token hash and cannot authenticate. They serve
/// only as a foreign key target for imported `backup_reports`.
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
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
         default_post_backup_commands, default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user FROM agents WHERE hostname = $1",
        hostname,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;

    if let Some(agent) = existing {
        return Ok(agent);
    }

    sqlx::query_as!(
        AgentRow,
        "INSERT INTO agents (hostname, display_name, agent_token_hash, owner_id) VALUES ($1, $2, \
         $3, NULL) RETURNING id, hostname, display_name, agent_version, agent_git_sha, \
         agent_build_time, agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
         default_post_backup_commands, default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user",
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
) -> Result<AgentRow, ApiError> {
    sqlx::query_as!(
        AgentRow,
        "INSERT INTO agents (hostname, display_name, agent_token_hash, owner_id) VALUES ($1, $2, \
         $3, $4) RETURNING id, hostname, display_name, agent_version, agent_git_sha, \
         agent_build_time, agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
         default_post_backup_commands, default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user",
        hostname,
        display_name,
        token_hash,
        owner_id,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// Default configuration values for an agent.
pub struct AgentDefaults<'a> {
    /// Optional display name.
    pub display_name: Option<&'a str>,
    /// Default backup paths.
    pub default_backup_paths: &'a [String],
    /// Default exclude patterns.
    pub default_exclude_patterns: &'a [String],
    /// Default pre-backup commands.
    pub default_pre_backup_commands: &'a str,
    /// Default post-backup commands.
    pub default_post_backup_commands: &'a str,
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
         default_file_change_patterns_raw) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id, \
         hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
         default_post_backup_commands, default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user",
        hostname,
        defaults.display_name,
        token_hash,
        defaults.default_backup_paths,
        defaults.default_exclude_patterns,
        defaults.default_pre_backup_commands,
        defaults.default_post_backup_commands,
        defaults.default_file_change_patterns_raw,
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
    hostname: &str,
    new_hostname: &str,
    defaults: AgentDefaults<'_>,
) -> Result<AgentRow, ApiError> {
    sqlx::query_as!(
        AgentRow,
        "UPDATE agents SET hostname = $2, display_name = $3, default_backup_paths = $4, \
         default_exclude_patterns = $5, default_pre_backup_commands = $6, \
         default_post_backup_commands = $7, default_file_change_patterns_raw = $8 WHERE hostname \
         = $1 RETURNING id, hostname, display_name, agent_version, agent_git_sha, \
         agent_build_time, agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
         default_post_backup_commands, default_file_change_patterns_raw, agent_token_hash, \
         is_hidden, last_ssh_user",
        hostname,
        new_hostname,
        defaults.display_name,
        defaults.default_backup_paths,
        defaults.default_exclude_patterns,
        defaults.default_pre_backup_commands,
        defaults.default_post_backup_commands,
        defaults.default_file_change_patterns_raw,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent '{hostname}' not found")),
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
    hostname: &str,
    token_hash: &str,
) -> Result<AgentRow, ApiError> {
    sqlx::query_as!(
        AgentRow,
        "UPDATE agents SET agent_token_hash = $2 WHERE hostname = $1 RETURNING id, hostname, \
         display_name, agent_version, agent_git_sha, agent_build_time, agent_commit_count, \
         created_at, last_seen_at, owner_id, visibility, default_backup_paths, \
         default_exclude_patterns, default_pre_backup_commands, default_post_backup_commands, \
         default_file_change_patterns_raw, agent_token_hash, is_hidden, last_ssh_user",
        hostname,
        token_hash,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent '{hostname}' not found")),
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
pub async fn delete_agent(pool: &PgPool, hostname: &str) -> Result<(), ApiError> {
    let result = sqlx::query!("DELETE FROM agents WHERE hostname = $1", hostname)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("agent '{hostname}' not found")));
    }
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
pub async fn get_schedule_target_hostnames_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<Vec<String>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        hostname: String,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT DISTINCT a.hostname FROM agents a JOIN schedule_targets st ON st.agent_id = a.id \
         JOIN schedules s ON s.id = st.schedule_id WHERE s.repo_id = $1",
        repo_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows.into_iter().map(|r| r.hostname).collect())
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
    pub sync_schedule: Option<&'a str>,
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
             compression, encryption, enabled, owner_id, visibility, sync_schedule",
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
         encryption, enabled, owner_id, visibility, sync_schedule",
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
    sqlx::query_as!(
        RepoRow,
        "UPDATE repos SET name = $2, repo_path = $3, ssh_user = $4, ssh_host = $5, ssh_port = $6, \
         compression = $7, encryption = $8, enabled = $9, sync_schedule = $10 WHERE id = $1 \
         RETURNING id, name, repo_path, ssh_user, ssh_host, ssh_port, compression, encryption, \
         enabled, owner_id, visibility, sync_schedule",
        params.repo_id,
        params.name,
        params.repo_path,
        params.ssh_user,
        params.ssh_host,
        params.ssh_port,
        params.compression,
        params.encryption,
        params.enabled,
        params.sync_schedule,
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

    let repo = sqlx::query_as!(
        RepoRow,
        "UPDATE repos SET name = $2, repo_path = $3, ssh_user = $4, ssh_host = $5, ssh_port = $6, \
         compression = $7, encryption = $8, enabled = $9, sync_schedule = $10, relocation_pending \
         = true WHERE id = $1 RETURNING id, name, repo_path, ssh_user, ssh_host, ssh_port, \
         compression, encryption, enabled, owner_id, visibility, sync_schedule",
        params.repo_id,
        params.name,
        params.repo_path,
        params.ssh_user,
        params.ssh_host,
        params.ssh_port,
        params.compression,
        params.encryption,
        params.enabled,
        params.sync_schedule,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => {
            ApiError::NotFound(format!("repo {} not found", params.repo_id))
        }
        other => ApiError::Database(other),
    })?;

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
    sqlx::query!(
        "UPDATE schedules SET enabled = false WHERE repo_id = $1",
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
         s.pre_backup_commands, s.post_backup_commands, s.execution_mode, s.on_failure, \
         s.owner_id, s.visibility, ARRAY(SELECT a.hostname FROM schedule_targets st JOIN agents a \
         ON a.id = st.agent_id WHERE st.schedule_id = s.id ORDER BY st.execution_order, \
         a.hostname) AS \"target_hostnames!\" FROM schedules s ORDER BY s.id",
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
    pub pre_backup_commands: &'a str,
    /// Post-backup commands.
    pub post_backup_commands: &'a str,
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
         owner_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, \
         $17, $18, 'sequential', $19, $20) RETURNING id, repo_id, name, schedule_type, \
         cron_expression, enabled, canary_enabled, last_run_at, next_run_at, \
         exclude_patterns_raw, file_change_patterns_raw, ignore_global_excludes, keep_hourly, \
         keep_daily, keep_weekly, keep_monthly, keep_yearly, compact_enabled, rate_limit_kbps, \
         pre_backup_commands, post_backup_commands, execution_mode, on_failure, owner_id, \
         visibility, ARRAY[]::TEXT[] AS \"target_hostnames!\"",
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
        params.pre_backup_commands,
        params.post_backup_commands,
        params.on_failure,
        owner_id,
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
    sqlx::query_as!(
        ScheduleRow,
        "UPDATE schedules SET name = $2, cron_expression = $3, enabled = $4, canary_enabled = $5, \
         exclude_patterns_raw = $6, file_change_patterns_raw = $7, ignore_global_excludes = $8, \
         keep_hourly = $9, keep_daily = $10, keep_weekly = $11, keep_monthly = $12, keep_yearly = \
         $13, compact_enabled = $14, rate_limit_kbps = $15, pre_backup_commands = $16, \
         post_backup_commands = $17, execution_mode = 'sequential', on_failure = $18 WHERE id = \
         $1 RETURNING id, repo_id, name, schedule_type, cron_expression, enabled, canary_enabled, \
         last_run_at, next_run_at, exclude_patterns_raw, file_change_patterns_raw, \
         ignore_global_excludes, keep_hourly, keep_daily, keep_weekly, keep_monthly, keep_yearly, \
         compact_enabled, rate_limit_kbps, pre_backup_commands, post_backup_commands, \
         execution_mode, on_failure, owner_id, visibility, ARRAY[]::TEXT[] AS \
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
        params.pre_backup_commands,
        params.post_backup_commands,
        params.on_failure,
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

/// Converts a [`Compression`] enum to its string representation.
#[must_use]
pub fn compression_to_str(c: &Compression) -> String {
    c.to_string()
}

/// # Errors
///
/// Returns [`ApiError::Internal`] if an internal error occurs.
pub fn compression_from_str(s: &str) -> Result<Compression, ApiError> {
    s.parse::<Compression>()
        .map_err(|e| ApiError::Internal(format!("invalid compression: {e}")))
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
         enabled, owner_id, visibility, sync_schedule FROM repos ORDER BY name",
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
         r.compression, r.encryption, r.enabled, r.owner_id, r.visibility, r.sync_schedule FROM \
         repos r JOIN schedules s ON s.repo_id = r.id JOIN schedule_targets st ON st.schedule_id \
         = s.id WHERE st.agent_id = $1 ORDER BY r.id",
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
    pub pre_backup_commands: String,
    /// Post-backup commands for this agent.
    pub post_backup_commands: String,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_all_per_agent_commands_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<PerAgentCommands>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        agent_id: i64,
        pre_backup_commands: String,
        post_backup_commands: String,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT agent_id, pre_backup_commands, post_backup_commands FROM per_agent_commands WHERE \
         schedule_id = $1 ORDER BY agent_id",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows
        .into_iter()
        .map(|r| PerAgentCommands {
            agent_id: r.agent_id,
            pre_backup_commands: r.pre_backup_commands,
            post_backup_commands: r.post_backup_commands,
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
    #[derive(sqlx::FromRow)]
    struct Row {
        pre_backup_commands: String,
        post_backup_commands: String,
    }

    let row = sqlx::query_as!(
        Row,
        "SELECT pre_backup_commands, post_backup_commands FROM per_agent_commands WHERE \
         schedule_id = $1 AND agent_id = $2",
        schedule_id,
        agent_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(row.map(|r| PerAgentCommands {
        agent_id,
        pre_backup_commands: r.pre_backup_commands,
        post_backup_commands: r.post_backup_commands,
    }))
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn upsert_per_agent_commands(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
    pre_backup_commands: &str,
    post_backup_commands: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO per_agent_commands (schedule_id, agent_id, pre_backup_commands, \
         post_backup_commands) VALUES ($1, $2, $3, $4) ON CONFLICT (schedule_id, agent_id) DO \
         UPDATE SET pre_backup_commands = EXCLUDED.pre_backup_commands, post_backup_commands = \
         EXCLUDED.post_backup_commands",
        schedule_id,
        agent_id,
        pre_backup_commands,
        post_backup_commands,
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
         compact_enabled, rate_limit_kbps, pre_backup_commands, post_backup_commands, \
         execution_mode, on_failure, owner_id, visibility, ARRAY[]::TEXT[] AS \
         \"target_hostnames!\" FROM schedules WHERE repo_id = $1",
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
         s.pre_backup_commands, s.post_backup_commands, s.execution_mode, s.on_failure, \
         s.owner_id, s.visibility, ARRAY[]::TEXT[] AS \"target_hostnames!\" FROM schedules s JOIN \
         schedule_targets st ON st.schedule_id = s.id JOIN agents m ON st.agent_id = m.id WHERE \
         m.hostname = $1 AND s.repo_id = $2 AND s.schedule_type = $3 LIMIT 1",
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
         s.pre_backup_commands, s.post_backup_commands, s.execution_mode, s.on_failure, \
         s.owner_id, s.visibility, COALESCE(ARRAY(SELECT a.hostname FROM schedule_targets st JOIN \
         agents a ON a.id = st.agent_id WHERE st.schedule_id = s.id ORDER BY st.execution_order, \
         a.hostname), ARRAY[]::TEXT[]) AS \"target_hostnames!\" FROM schedules s WHERE s.repo_id \
         = $1 ORDER BY s.id",
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
         s.pre_backup_commands, s.post_backup_commands, s.execution_mode, s.on_failure, \
         s.owner_id, s.visibility, ARRAY[]::TEXT[] AS \"target_hostnames!\" FROM schedules s JOIN \
         schedule_targets st ON st.schedule_id = s.id WHERE st.agent_id = $1 ORDER by s.id",
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
        "SELECT s.id AS schedule_id, s.repo_id AS \"repo_id!\", st.agent_id, a.hostname, \
         s.schedule_type, s.cron_expression, s.on_failure, st.execution_order FROM schedules s \
         JOIN repos r ON r.id = s.repo_id JOIN schedule_targets st ON st.schedule_id = s.id JOIN \
         agents a ON a.id = st.agent_id WHERE s.enabled = true AND r.enabled = true AND \
         a.is_hidden = false AND s.next_run_at IS NOT NULL AND s.next_run_at <= $1 ORDER BY s.id, \
         st.execution_order",
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
    sqlx::query!(
        "UPDATE schedules SET enabled = $2 WHERE id = $1",
        schedule_id,
        enabled,
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
         compact_enabled, rate_limit_kbps, pre_backup_commands, post_backup_commands, \
         execution_mode, on_failure, owner_id, visibility, ARRAY[]::TEXT[] AS \
         \"target_hostnames!\" FROM schedules WHERE id = $1",
        id,
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
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_schedule_target_hostnames(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<String>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        hostname: String,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT a.hostname FROM agents a JOIN schedule_targets st ON st.agent_id = a.id WHERE \
         st.schedule_id = $1 AND a.is_hidden = false ORDER BY st.execution_order",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows.into_iter().map(|r| r.hostname).collect())
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
    pub status: String,
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
        &params.status,
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
        &params.status,
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
        &params.status,
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
    let mut statuses: Vec<&str> = Vec::with_capacity(params.len());
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
        statuses.push(p.status.as_str());
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
        &statuses as &[&str],
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
             br.error_message, br.warnings, br.borg_version, br.archive_name, br.borg_command \
             FROM backup_reports br JOIN repos r ON r.id = br.repo_id LEFT JOIN schedules s ON \
             s.id = br.schedule_id WHERE br.agent_id = $1 AND r.name = $2 ORDER by br.started_at \
             DESC LIMIT $3",
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
             br.error_message, br.warnings, br.borg_version, br.archive_name, br.borg_command \
             FROM backup_reports br JOIN repos r ON r.id = br.repo_id LEFT JOIN schedules s ON \
             s.id = br.schedule_id WHERE br.agent_id = $1 ORDER BY br.started_at DESC LIMIT $2",
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
         br.warnings, br.borg_version, br.archive_name, br.borg_command FROM backup_reports br \
         JOIN repos r ON r.id = br.repo_id LEFT JOIN schedules s ON s.id = br.schedule_id WHERE \
         br.schedule_id = $1 ORDER BY br.started_at DESC LIMIT $2",
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

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn get_activity_feed(
    pool: &PgPool,
    limit: i64,
    repo_id: Option<i64>,
    hostname: Option<&str>,
    schedule_id: Option<i64>,
    run_id: Option<&str>,
) -> Result<Vec<ActivityRow>, ApiError> {
    sqlx::query_as!(
        ActivityRow,
        "SELECT br.id, a.hostname, r.name AS target_name, br.started_at, br.finished_at, \
         br.status, br.duration_secs, br.repo_id, br.archive_name, br.error_message, \
         br.schedule_id, s.name AS \"schedule_name?\", br.run_id FROM backup_reports br JOIN \
         agents a ON a.id = br.agent_id JOIN repos r ON r.id = br.repo_id LEFT JOIN schedules s \
         ON s.id = br.schedule_id WHERE a.is_hidden = false AND a.visibility <> 'hidden' AND \
         COALESCE(a.display_name, '') NOT ILIKE '%(imported)%' AND ($1::bigint IS NULL OR \
         br.repo_id = $1) AND ($2::text IS NULL OR a.hostname = $2) AND ($3::bigint IS NULL OR \
         br.schedule_id = $3) AND ($4::text IS NULL OR br.run_id = $4) ORDER BY br.started_at \
         DESC LIMIT $5",
        repo_id,
        hostname,
        schedule_id,
        run_id,
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
    sqlx::query_as!(
        HealthRow,
        "SELECT r.id AS repo_id, s.id AS schedule_id, a.hostname, r.name AS target_name, (SELECT \
         br.status FROM backup_reports br WHERE br.schedule_id = s.id AND br.agent_id = a.id \
         ORDER BY br.started_at DESC LIMIT 1) AS last_status, (SELECT br.finished_at FROM \
         backup_reports br WHERE br.schedule_id = s.id AND br.agent_id = a.id ORDER BY \
         br.started_at DESC LIMIT 1) AS last_backup_at, (SELECT br.error_message FROM \
         backup_reports br WHERE br.schedule_id = s.id AND br.agent_id = a.id ORDER BY \
         br.started_at DESC LIMIT 1) AS last_error_message, s.cron_expression, s.enabled AS \
         schedule_enabled FROM schedules s JOIN schedule_targets st ON st.schedule_id = s.id JOIN \
         agents a ON a.id = st.agent_id JOIN repos r ON r.id = s.repo_id WHERE a.is_hidden = \
         false ORDER BY a.hostname, r.name",
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
         must_change_password, created_at, last_login_at",
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
        "SELECT id, username, must_change_password, created_at, last_login_at FROM users WHERE \
         username = $1",
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
    }

    let row = sqlx::query_as!(
        FullRow,
        "SELECT id, username, password_hash, must_change_password, created_at, last_login_at FROM \
         users WHERE username = $1",
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
        "SELECT id, username, must_change_password, created_at, last_login_at FROM users WHERE id \
         = $1",
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
        "SELECT id, username, must_change_password, created_at, last_login_at FROM users ORDER BY \
         id",
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
pub async fn insert_session(
    pool: &PgPool,
    session_id: &str,
    user_id: i64,
    expires_at: DateTime<Utc>,
    remember_me: bool,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO sessions (id, user_id, expires_at, remember_me) VALUES ($1, $2, $3, $4)",
        session_id,
        user_id,
        expires_at,
        remember_me,
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
        "SELECT id, user_id, created_at, expires_at, remember_me FROM sessions WHERE id = $1 AND \
         expires_at > NOW()",
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
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct SystemEventRow {
    /// Unique identifier.
    pub id: i64,
    /// When the event occurred.
    pub created_at: DateTime<Utc>,
    /// Event type (e.g. ``agent_connected``, ``backup_failed``).
    pub event_type: String,
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
    event_type: &str,
    hostname: Option<&str>,
    message: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO system_events (event_type, hostname, message) VALUES ($1, $2, $3)",
        event_type,
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
pub async fn get_system_events(pool: &PgPool, limit: i64) -> Result<Vec<SystemEventRow>, ApiError> {
    sqlx::query_as!(
        SystemEventRow,
        "SELECT id, created_at, event_type, hostname, message FROM system_events ORDER BY \
         created_at DESC LIMIT $1",
        limit,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
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

/// Get user preferences.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
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
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_repos_with_stats(pool: &PgPool) -> Result<Vec<RepoWithStatsRow>, ApiError> {
    sqlx::query_as!(
        RepoWithStatsRow,
        "SELECT r.id, r.name, r.repo_path, r.ssh_user, r.ssh_host, r.ssh_port, r.ssh_host_key, \
         r.compression, r.encryption, r.enabled, r.owner_id, r.visibility, r.sync_schedule, \
         r.relocation_pending, COALESCE(rs.original_size, 0) AS \"total_original_size!\", \
         COALESCE(rs.compressed_size, 0) AS \"total_compressed_size!\", \
         COALESCE(rs.deduplicated_size, 0) AS \"total_deduplicated_size!\", \
         COALESCE(rs.archive_count::INT8, 0) AS \"archive_count!\", rs.last_synced_at, \
         COALESCE(ris.importing, false) AS \"importing!\", ris.error AS import_error, \
         COALESCE(ris.progress, 0) AS \"import_progress!\", COALESCE(ris.total, 0) AS \
         \"import_total!\", ris.status_message AS import_status_message, rlo.kind AS \
         last_op_kind, rlo.at AS last_op_at, rlo.by_text AS last_op_by, agg.last_backup_at, \
         COALESCE(agg.agent_count, 0) AS \"agent_count!\", COALESCE(agg.unmatched_count, 0) AS \
         \"unmatched_count!\" FROM repos r LEFT JOIN repo_stats rs ON rs.repo_id = r.id LEFT JOIN \
         repo_import_state ris ON ris.repo_id = r.id LEFT JOIN repo_last_op rlo ON rlo.repo_id = \
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
         r.relocation_pending, COALESCE(rs.original_size, 0) AS \"total_original_size!\", \
         COALESCE(rs.compressed_size, 0) AS \"total_compressed_size!\", \
         COALESCE(rs.deduplicated_size, 0) AS \"total_deduplicated_size!\", \
         COALESCE(rs.archive_count::INT8, 0) AS \"archive_count!\", rs.last_synced_at, \
         COALESCE(ris.importing, false) AS \"importing!\", ris.error AS import_error, \
         COALESCE(ris.progress, 0) AS \"import_progress!\", COALESCE(ris.total, 0) AS \
         \"import_total!\", ris.status_message AS import_status_message, rlo.kind AS \
         last_op_kind, rlo.at AS last_op_at, rlo.by_text AS last_op_by, agg.last_backup_at, \
         COALESCE(agg.agent_count, 0) AS \"agent_count!\", COALESCE(agg.unmatched_count, 0) AS \
         \"unmatched_count!\" FROM repos r LEFT JOIN repo_stats rs ON rs.repo_id = r.id LEFT JOIN \
         repo_import_state ris ON ris.repo_id = r.id LEFT JOIN repo_last_op rlo ON rlo.repo_id = \
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
    sqlx::query_as!(
        DashboardSummaryRow,
        "SELECT (SELECT COUNT(*) FROM agents WHERE is_hidden = false) AS \"total_agents!\", \
         (SELECT COUNT(*) FROM repos) AS \"total_repos!\", (SELECT COUNT(*) FROM schedules WHERE \
         enabled = true) AS \"active_schedules!\", (SELECT COUNT(*) FROM schedules) AS \
         \"total_schedules!\", COALESCE((SELECT SUM(deduplicated_size) FROM repo_stats), 0)::INT8 \
         AS \"total_storage_bytes!\", (SELECT MAX(finished_at) FROM backup_reports WHERE status = \
         'success' AND finished_at > '1970-01-01T00:00:00Z') AS last_backup_at, (SELECT \
         MIN(s.next_run_at) FROM schedules s JOIN repos r ON r.id = s.repo_id WHERE s.enabled = \
         true AND r.enabled = true AND s.next_run_at IS NOT NULL AND s.next_run_at > NOW()) AS \
         next_backup_at, (SELECT br.schedule_id FROM backup_reports br WHERE br.schedule_id IS \
         NOT NULL ORDER BY br.finished_at DESC LIMIT 1) AS last_backup_schedule_id, (SELECT \
         br.repo_id FROM backup_reports br WHERE br.status = 'success' ORDER BY br.finished_at \
         DESC LIMIT 1) AS last_backup_repo_id, (SELECT br.archive_name FROM backup_reports br \
         WHERE br.status = 'success' ORDER BY br.finished_at DESC LIMIT 1) AS \
         last_backup_archive_name, (SELECT s.id FROM schedules s JOIN repos r ON r.id = s.repo_id \
         WHERE s.enabled = true AND r.enabled = true AND s.next_run_at IS NOT NULL AND \
         s.next_run_at > NOW() ORDER BY s.next_run_at LIMIT 1) AS next_backup_schedule_id, \
         (SELECT COUNT(*) FROM backup_reports WHERE status = 'success' AND started_at > NOW() - \
         INTERVAL '30 days') AS \"success_30d!\", (SELECT COUNT(*) FROM backup_reports WHERE \
         status != 'success' AND started_at > NOW() - INTERVAL '30 days') AS \"failed_30d!\", \
         (SELECT COUNT(*) FROM backup_reports WHERE started_at > NOW() - INTERVAL '30 days') AS \
         \"total_30d!\", (SELECT MAX(finished_at) FROM backup_reports WHERE status = 'failed' AND \
         finished_at > '1970-01-01T00:00:00Z') AS last_failure_at, (SELECT MAX(finished_at) FROM \
         backup_reports WHERE status = 'warning' AND finished_at > '1970-01-01T00:00:00Z') AS \
         last_warning_at, (SELECT br.schedule_id FROM backup_reports br WHERE br.schedule_id IS \
         NOT NULL AND br.status = 'failed' AND br.finished_at > '1970-01-01T00:00:00Z' ORDER BY \
         br.finished_at DESC LIMIT 1) AS last_failure_schedule_id, (SELECT br.schedule_id FROM \
         backup_reports br WHERE br.schedule_id IS NOT NULL AND br.status = 'warning' AND \
         br.finished_at > '1970-01-01T00:00:00Z' ORDER BY br.finished_at DESC LIMIT 1) AS \
         last_warning_schedule_id, (SELECT br.error_message FROM backup_reports br WHERE \
         br.status = 'failed' AND br.finished_at > '1970-01-01T00:00:00Z' ORDER BY br.finished_at \
         DESC LIMIT 1) AS last_failure_message, (SELECT br.warnings[1] FROM backup_reports br \
         WHERE br.status = 'warning' AND br.finished_at > '1970-01-01T00:00:00Z' ORDER BY \
         br.finished_at DESC LIMIT 1) AS last_warning_message, (SELECT br.repo_id FROM \
         backup_reports br WHERE br.status = 'failed' AND br.finished_at > '1970-01-01T00:00:00Z' \
         ORDER BY br.finished_at DESC LIMIT 1) AS last_failure_repo_id, (SELECT br.repo_id FROM \
         backup_reports br WHERE br.status = 'warning' AND br.finished_at > \
         '1970-01-01T00:00:00Z' ORDER BY br.finished_at DESC LIMIT 1) AS last_warning_repo_id, \
         (SELECT r.name FROM backup_reports br JOIN repos r ON r.id = br.repo_id WHERE br.status \
         = 'failed' AND br.finished_at > '1970-01-01T00:00:00Z' ORDER BY br.finished_at DESC \
         LIMIT 1) AS last_failure_repo_name, (SELECT r.name FROM backup_reports br JOIN repos r \
         ON r.id = br.repo_id WHERE br.status = 'warning' AND br.finished_at > \
         '1970-01-01T00:00:00Z' ORDER BY br.finished_at DESC LIMIT 1) AS last_warning_repo_name, \
         (SELECT s.cron_expression FROM backup_reports br JOIN schedules s ON s.id = \
         br.schedule_id WHERE br.status = 'failed' AND br.finished_at > '1970-01-01T00:00:00Z' \
         ORDER BY br.finished_at DESC LIMIT 1) AS last_failure_schedule_name, (SELECT \
         s.cron_expression FROM backup_reports br JOIN schedules s ON s.id = br.schedule_id WHERE \
         br.status = 'warning' AND br.finished_at > '1970-01-01T00:00:00Z' ORDER BY \
         br.finished_at DESC LIMIT 1) AS last_warning_schedule_name",
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
    repo_id: Option<i64>,
    hostname: Option<&str>,
    schedule_id: Option<i64>,
    run_id: Option<&str>,
) -> Result<Vec<ActivityRow>, ApiError> {
    sqlx::query_as!(
        ActivityRow,
        "SELECT br.id, a.hostname, r.name AS target_name, br.started_at, br.finished_at, \
         br.status, br.duration_secs, br.repo_id, br.archive_name, br.error_message, \
         br.schedule_id, s.name AS \"schedule_name?\", br.run_id FROM backup_reports br JOIN \
         agents a ON a.id = br.agent_id JOIN repos r ON r.id = br.repo_id LEFT JOIN schedules s \
         ON s.id = br.schedule_id WHERE a.is_hidden = false AND a.visibility <> 'hidden' AND \
         COALESCE(a.display_name, '') NOT ILIKE '%(imported)%' AND br.started_at > NOW() - \
         make_interval(days => $1::int) AND ($2::bigint IS NULL OR br.repo_id = $2) AND ($3::text \
         IS NULL OR a.hostname = $3) AND ($4::bigint IS NULL OR br.schedule_id = $4) AND \
         ($5::text IS NULL OR br.run_id = $5) ORDER BY br.started_at DESC",
        i32::try_from(days).unwrap_or(14),
        repo_id,
        hostname,
        schedule_id,
        run_id,
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
         can_manage_tunnels, created_at FROM roles ORDER BY name",
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
         can_manage_tunnels, created_at FROM roles WHERE id = $1",
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
         can_manage_tunnels) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
         RETURNING id, name, can_create_agent, can_delete_agent, can_delete_own_agent, \
         can_create_repo, can_delete_repo, can_delete_own_repo, can_create_schedule, \
         can_delete_schedule, can_delete_own_schedule, can_manage_tags, can_view_all_repos, \
         can_manage_tunnels, created_at",
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
         can_manage_tunnels = $14 WHERE id = $1 RETURNING id, name, can_create_agent, \
         can_delete_agent, can_delete_own_agent, can_create_repo, can_delete_repo, \
         can_delete_own_repo, can_create_schedule, can_delete_schedule, can_delete_own_schedule, \
         can_manage_tags, can_view_all_repos, can_manage_tunnels, created_at",
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
         r.can_view_all_repos, r.can_manage_tunnels, r.created_at FROM roles r JOIN user_roles ur \
         ON ur.role_id = r.id WHERE ur.user_id = $1 ORDER BY r.name",
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
         can_view_all_repos, BOOL_OR(r.can_manage_tunnels) AS can_manage_tunnels FROM roles r \
         JOIN user_roles ur ON ur.role_id = r.id WHERE ur.user_id = $1",
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
        sqlx::query_as!(
            StorageTrendRow,
            "WITH days AS ( SELECT generate_series( (CURRENT_DATE - make_interval(days => \
             $1))::date, CURRENT_DATE, '1 day'::interval )::date AS date ) SELECT d.date AS \
             \"date!\", COALESCE(totals.original_size, 0)::INT8 AS \"original_size!\", \
             COALESCE(totals.compressed_size, 0)::INT8 AS \"compressed_size!\", \
             NULLIF(COALESCE(latest.repo_unique_csize, 0), 0)::INT8 AS \"deduplicated_size?\" \
             FROM days d LEFT JOIN LATERAL ( SELECT SUM(br.original_size) AS original_size, \
             SUM(br.compressed_size) AS compressed_size FROM backup_reports br WHERE br.repo_id = \
             $2 AND br.started_at::date <= d.date AND br.status = 'success' ) totals ON true LEFT \
             JOIN LATERAL ( SELECT br.repo_unique_csize FROM backup_reports br WHERE br.repo_id = \
             $2 AND br.started_at::date <= d.date AND br.status = 'success' ORDER BY \
             br.started_at DESC LIMIT 1 ) latest ON true ORDER BY d.date",
            days,
            rid,
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    } else {
        sqlx::query_as!(
            StorageTrendRow,
            "WITH days AS ( SELECT generate_series( (CURRENT_DATE - make_interval(days => \
             $1))::date, CURRENT_DATE, '1 day'::interval )::date AS date ) SELECT d.date AS \
             \"date!\", COALESCE(totals.original_size, 0)::INT8 AS \"original_size!\", \
             COALESCE(totals.compressed_size, 0)::INT8 AS \"compressed_size!\", \
             NULLIF(COALESCE(dedup.repo_unique_csize, 0), 0)::INT8 AS \"deduplicated_size?\" FROM \
             days d LEFT JOIN LATERAL ( SELECT SUM(br.original_size) AS original_size, \
             SUM(br.compressed_size) AS compressed_size FROM backup_reports br WHERE \
             br.started_at::date <= d.date AND br.status = 'success' ) totals ON true LEFT JOIN \
             LATERAL ( SELECT SUM(latest.repo_unique_csize) AS repo_unique_csize FROM ( SELECT \
             DISTINCT ON (br.repo_id) br.repo_unique_csize FROM backup_reports br WHERE \
             br.started_at::date <= d.date AND br.status = 'success' ORDER BY br.repo_id, \
             br.started_at DESC ) latest ) dedup ON true ORDER BY d.date",
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

    // Collect candidate path IDs before the cascade delete removes archive_files.
    let candidate_ids: Vec<i64> = sqlx::query_scalar!(
        "SELECT path_id AS \"path_id!\" FROM archive_files WHERE archive_id IN (SELECT id FROM \
         archives WHERE repo_id = $1 AND name = ANY($2)) UNION SELECT parent_path_id FROM \
         archive_files WHERE archive_id IN (SELECT id FROM archives WHERE repo_id = $1 AND name = \
         ANY($2))",
        repo_id,
        names,
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    // Deleting from archives cascades to archive_files, archive_index_jobs, and archive_tags.
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
             1 FROM archive_files WHERE path_id = archive_paths.id) AND NOT EXISTS (SELECT 1 FROM \
             archive_files WHERE parent_path_id = archive_paths.id)",
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

    // Collect candidate path IDs before the cascade delete removes archive_files.
    let candidate_ids: Vec<i64> = sqlx::query_scalar!(
        "SELECT path_id AS \"path_id!\" FROM archive_files WHERE archive_id IN (SELECT id FROM \
         archives WHERE repo_id = $1) UNION SELECT parent_path_id FROM archive_files WHERE \
         archive_id IN (SELECT id FROM archives WHERE repo_id = $1)",
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

    // Deleting from archives cascades to archive_files, archive_index_jobs, and archive_tags.
    sqlx::query!("DELETE FROM archives WHERE repo_id = $1", repo_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

    // GC paths that are now orphaned, checking only the candidates from the deleted archives.
    if !candidate_ids.is_empty() {
        sqlx::query!(
            "DELETE FROM archive_paths WHERE repo_id = $1 AND id = ANY($2) AND NOT EXISTS (SELECT \
             1 FROM archive_files WHERE path_id = archive_paths.id) AND NOT EXISTS (SELECT 1 FROM \
             archive_files WHERE parent_path_id = archive_paths.id)",
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
    sqlx::query_as!(
        StorageTrendByRepoRow,
        "WITH days AS ( SELECT generate_series( (CURRENT_DATE - make_interval(days => $1))::date, \
         CURRENT_DATE, '1 day'::interval )::date AS date ), repos_list AS ( SELECT DISTINCT r.id \
         AS repo_id, r.name AS repo_name FROM repos r JOIN backup_reports br ON br.repo_id = r.id \
         ) SELECT d.date AS \"date!\", rl.repo_id AS \"repo_id!\", rl.repo_name AS \
         \"repo_name!\", COALESCE(totals.original_size, 0)::INT8 AS \"original_size!\", \
         COALESCE(totals.compressed_size, 0)::INT8 AS \"compressed_size!\", \
         NULLIF(COALESCE(latest.repo_unique_csize, 0), 0)::INT8 AS \"deduplicated_size?\" FROM \
         days d CROSS JOIN repos_list rl LEFT JOIN LATERAL ( SELECT SUM(br.original_size) AS \
         original_size, SUM(br.compressed_size) AS compressed_size FROM backup_reports br WHERE \
         br.repo_id = rl.repo_id AND br.started_at::date <= d.date AND br.status = 'success' ) \
         totals ON true LEFT JOIN LATERAL ( SELECT br.repo_unique_csize FROM backup_reports br \
         WHERE br.repo_id = rl.repo_id AND br.started_at::date <= d.date AND br.status = \
         'success' ORDER BY br.started_at DESC LIMIT 1 ) latest ON true ORDER BY d.date, \
         rl.repo_name",
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
         compact_enabled, rate_limit_kbps, pre_backup_commands, post_backup_commands, \
         execution_mode, on_failure, owner_id, visibility, ARRAY[]::TEXT[] AS \
         \"target_hostnames!\" FROM schedules WHERE enabled = true",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(rows)
}
