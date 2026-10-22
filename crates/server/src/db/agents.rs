// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Agents: registration, lookup by hostname or pattern, defaults, tokens and
//! merging imported placeholders.

use chrono::{DateTime, Utc};
use serde::Serialize;
use shared::{hooks::HookCommand, types::Visibility};
use sqlx::PgPool;

use super::{HookCommands, lock_schedules_targeting_agent, patterns};
use crate::error::ApiError;

/// Sentinel `agent_token_hash` value for imported placeholder agents that have
/// no real authentication token.
pub const IMPORTED_TOKEN_HASH: &str = "imported:no-auth";

/// Maps an error from a query that reads or writes one agent's row: a missing
/// row is that agent not existing, anything else a database failure.
pub(crate) fn agent_row_error(agent_id: i64) -> impl FnOnce(sqlx::Error) -> ApiError {
    move |e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent id '{agent_id}' not found")),
        other => ApiError::Database(other),
    }
}

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
        "SELECT a.id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, a.created_at, last_seen_at, owner_id, visibility AS \"visibility: \
         Visibility\", default_backup_paths, default_exclude_patterns, \
         default_pre_backup_commands AS \"default_pre_backup_commands: HookCommands\", \
         default_post_backup_commands AS \"default_post_backup_commands: HookCommands\", \
         default_file_change_patterns_raw, agent_token_hash, is_hidden, last_ssh_user, domain, \
         wake_enabled, wake_mac_address, wake_broadcast_address, wake_timeout_seconds, \
         shutdown_after_backup, start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, \
         agent_service_name, a.machine_id FROM agents a JOIN machines m ON m.id = a.machine_id \
         WHERE hostname = $1 AND agent_token_hash != 'imported:no-auth'",
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
        "SELECT a.id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, a.created_at, last_seen_at, owner_id, visibility AS \"visibility: \
         Visibility\", default_backup_paths, default_exclude_patterns, \
         default_pre_backup_commands AS \"default_pre_backup_commands: HookCommands\", \
         default_post_backup_commands AS \"default_post_backup_commands: HookCommands\", \
         default_file_change_patterns_raw, agent_token_hash, is_hidden, last_ssh_user, domain, \
         wake_enabled, wake_mac_address, wake_broadcast_address, wake_timeout_seconds, \
         shutdown_after_backup, start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, \
         agent_service_name, a.machine_id FROM agents a JOIN machines m ON m.id = a.machine_id \
         WHERE a.id = $1",
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
    pub visibility: Visibility,
    /// Default backup paths for schedules targeting this agent.
    #[serde(default)]
    pub default_backup_paths: Vec<String>,
    /// Default exclude patterns for schedules targeting this agent.
    #[serde(default)]
    pub default_exclude_patterns: Vec<String>,
    /// Default pre-backup commands.
    #[schema(value_type = Vec<HookCommand>)]
    pub default_pre_backup_commands: HookCommands,
    /// Default post-backup commands.
    #[schema(value_type = Vec<HookCommand>)]
    pub default_post_backup_commands: HookCommands,
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
    /// The machine this agent runs on, which holds the wake and availability
    /// settings above.
    pub machine_id: i64,
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
            "SELECT a.id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
             agent_commit_count, a.created_at, last_seen_at, owner_id, visibility AS \
             \"visibility: Visibility\", default_backup_paths, default_exclude_patterns, \
             default_pre_backup_commands AS \"default_pre_backup_commands: HookCommands\", \
             default_post_backup_commands AS \"default_post_backup_commands: HookCommands\", \
             default_file_change_patterns_raw, agent_token_hash, is_hidden, last_ssh_user, \
             domain, wake_enabled, wake_mac_address, wake_broadcast_address, \
             wake_timeout_seconds, shutdown_after_backup, start_agent_enabled, \
             stop_agent_after_backup, ssh_host, ssh_port, agent_service_name, a.machine_id FROM \
             agents a JOIN machines m ON m.id = a.machine_id WHERE hostname = $1 AND domain = $2",
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
        "SELECT a.id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, a.created_at, last_seen_at, owner_id, visibility AS \"visibility: \
         Visibility\", default_backup_paths, default_exclude_patterns, \
         default_pre_backup_commands AS \"default_pre_backup_commands: HookCommands\", \
         default_post_backup_commands AS \"default_post_backup_commands: HookCommands\", \
         default_file_change_patterns_raw, agent_token_hash, is_hidden, last_ssh_user, domain, \
         wake_enabled, wake_mac_address, wake_broadcast_address, wake_timeout_seconds, \
         shutdown_after_backup, start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, \
         agent_service_name, a.machine_id FROM agents a JOIN machines m ON m.id = a.machine_id \
         WHERE hostname = $1 ORDER BY domain",
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
        "SELECT a.id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, a.created_at, last_seen_at, owner_id, visibility AS \"visibility: \
         Visibility\", default_backup_paths, default_exclude_patterns, \
         default_pre_backup_commands AS \"default_pre_backup_commands: HookCommands\", \
         default_post_backup_commands AS \"default_post_backup_commands: HookCommands\", \
         default_file_change_patterns_raw, agent_token_hash, is_hidden, last_ssh_user, domain, \
         wake_enabled, wake_mac_address, wake_broadcast_address, wake_timeout_seconds, \
         shutdown_after_backup, start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, \
         agent_service_name, a.machine_id FROM agents a JOIN machines m ON m.id = a.machine_id \
         WHERE a.id = $1",
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
            "SELECT a.id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
             agent_commit_count, a.created_at, last_seen_at, owner_id, visibility AS \
             \"visibility: Visibility\", default_backup_paths, default_exclude_patterns, \
             default_pre_backup_commands AS \"default_pre_backup_commands: HookCommands\", \
             default_post_backup_commands AS \"default_post_backup_commands: HookCommands\", \
             default_file_change_patterns_raw, agent_token_hash, is_hidden, last_ssh_user, \
             domain, wake_enabled, wake_mac_address, wake_broadcast_address, \
             wake_timeout_seconds, shutdown_after_backup, start_agent_enabled, \
             stop_agent_after_backup, ssh_host, ssh_port, agent_service_name, a.machine_id FROM \
             agents a JOIN machines m ON m.id = a.machine_id ORDER BY hostname",
        )
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    } else {
        sqlx::query_as!(
            AgentRow,
            "SELECT a.id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
             agent_commit_count, a.created_at, last_seen_at, owner_id, visibility AS \
             \"visibility: Visibility\", default_backup_paths, default_exclude_patterns, \
             default_pre_backup_commands AS \"default_pre_backup_commands: HookCommands\", \
             default_post_backup_commands AS \"default_post_backup_commands: HookCommands\", \
             default_file_change_patterns_raw, agent_token_hash, is_hidden, last_ssh_user, \
             domain, wake_enabled, wake_mac_address, wake_broadcast_address, \
             wake_timeout_seconds, shutdown_after_backup, start_agent_enabled, \
             stop_agent_after_backup, ssh_host, ssh_port, agent_service_name, a.machine_id FROM \
             agents a JOIN machines m ON m.id = a.machine_id WHERE is_hidden = false ORDER BY \
             hostname",
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
    let agent_id = sqlx::query_scalar!(
        "UPDATE agents SET is_hidden = $2 WHERE id = $1 RETURNING id",
        agent_id,
        hidden,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?
    .ok_or_else(|| ApiError::NotFound(format!("Agent id '{agent_id}' not found")))?;
    get_agent_by_id(pool, agent_id).await
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
        "SELECT a.id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, a.created_at, last_seen_at, owner_id, visibility AS \"visibility: \
         Visibility\", default_backup_paths, default_exclude_patterns, \
         default_pre_backup_commands AS \"default_pre_backup_commands: HookCommands\", \
         default_post_backup_commands AS \"default_post_backup_commands: HookCommands\", \
         default_file_change_patterns_raw, agent_token_hash, is_hidden, last_ssh_user, domain, \
         wake_enabled, wake_mac_address, wake_broadcast_address, wake_timeout_seconds, \
         shutdown_after_backup, start_agent_enabled, stop_agent_after_backup, ssh_host, ssh_port, \
         agent_service_name, a.machine_id FROM agents a JOIN machines m ON m.id = a.machine_id \
         WHERE hostname = $1",
        hostname,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    if let Ok([agent]) = <[AgentRow; 1]>::try_from(existing) {
        return Ok(agent);
    }

    let agent_id = sqlx::query_scalar!(
        "INSERT INTO agents (hostname, display_name, agent_token_hash, owner_id) VALUES ($1, $2, \
         $3, NULL) RETURNING id",
        hostname,
        Some(format!("{hostname} (imported)")),
        "imported:no-auth",
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;
    get_agent_by_id(pool, agent_id).await
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
    let agent_id = sqlx::query_scalar!(
        "INSERT INTO agents (hostname, display_name, agent_token_hash, owner_id, domain) VALUES \
         ($1, $2, $3, $4, $5) RETURNING id",
        hostname,
        display_name,
        token_hash,
        owner_id,
        domain,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;
    get_agent_by_id(pool, agent_id).await
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
    pub default_pre_backup_commands: &'a [HookCommand],
    /// Default post-backup commands.
    pub default_post_backup_commands: &'a [HookCommand],
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
    let agent_id = sqlx::query_scalar!(
        "INSERT INTO agents (hostname, display_name, agent_token_hash, default_backup_paths, \
         default_exclude_patterns, default_pre_backup_commands, default_post_backup_commands, \
         default_file_change_patterns_raw, domain) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
         RETURNING id",
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
    .map_err(ApiError::Database)?;
    get_agent_by_id(pool, agent_id).await
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
    let agent_id = sqlx::query_scalar!(
        "UPDATE agents SET hostname = $2, display_name = $3, default_backup_paths = $4, \
         default_exclude_patterns = $5, default_pre_backup_commands = $6, \
         default_post_backup_commands = $7, default_file_change_patterns_raw = $8, domain = $9 \
         WHERE id = $1 RETURNING id",
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
    })?;
    get_agent_by_id(pool, agent_id).await
}

/// How the agent process is started and stopped around a backup, and the SSH
/// destination that is also used to shut the agent's machine down. Waking and
/// shutting the machine down are its machine's settings - see
/// [`machines::MachinePowerPatch`]. Kept separate from [`AgentDefaults`] --
/// unlike backup defaults, these settings are never touched by the config
/// import/export flow.
///
/// [`machines::MachinePowerPatch`]: super::machines::MachinePowerPatch
pub struct AgentProcessPatch<'a> {
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
pub async fn update_agent_process(
    pool: &PgPool,
    agent_id: i64,
    process: AgentProcessPatch<'_>,
) -> Result<AgentRow, ApiError> {
    let agent_id = sqlx::query_scalar!(
        "UPDATE agents SET start_agent_enabled = $2, stop_agent_after_backup = $3, ssh_host = $4, \
         ssh_port = $5, agent_service_name = $6 WHERE id = $1 RETURNING id",
        agent_id,
        process.start_agent_enabled,
        process.stop_agent_after_backup,
        process.ssh_host,
        process.ssh_port,
        process.agent_service_name,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent id '{agent_id}' not found")),
        other => ApiError::Database(other),
    })?;
    get_agent_by_id(pool, agent_id).await
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
    let agent_id = sqlx::query_scalar!(
        "UPDATE agents SET agent_token_hash = $2 WHERE id = $1 RETURNING id",
        agent_id,
        token_hash,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent id '{agent_id}' not found")),
        other => ApiError::Database(other),
    })?;
    get_agent_by_id(pool, agent_id).await
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
