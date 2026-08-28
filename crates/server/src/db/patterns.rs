// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

use crate::error::ApiError;

/// A row from `agent_hostname_patterns`, associating a glob-style hostname
/// pattern with the agent whose configuration it should apply to.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct HostnamePatternRow {
    /// Primary key of the pattern row.
    pub id: i64,
    /// Foreign key of the agent this pattern is associated with.
    pub agent_id: i64,
    /// Glob-style hostname pattern (e.g. `web-server-*`) matched against
    /// unmatched/imported client hostnames.
    pub pattern: String,
    /// Timestamp at which the pattern was created.
    pub created_at: DateTime<Utc>,
}

/// Lists all hostname patterns registered for a given agent, ordered
/// alphabetically by pattern.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_patterns_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<HostnamePatternRow>, ApiError> {
    sqlx::query_as!(
        HostnamePatternRow,
        "SELECT id, agent_id, pattern, created_at FROM agent_hostname_patterns WHERE agent_id = \
         $1 ORDER BY pattern",
        agent_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Inserts a new hostname pattern for an agent and returns the created row.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn add_hostname_pattern(
    pool: &PgPool,
    agent_id: i64,
    pattern: &str,
) -> Result<HostnamePatternRow, ApiError> {
    sqlx::query_as!(
        HostnamePatternRow,
        "INSERT INTO agent_hostname_patterns (agent_id, pattern) VALUES ($1, $2) RETURNING id, \
         agent_id, pattern, created_at",
        agent_id,
        pattern,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// Deletes a hostname pattern by its primary key.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn delete_hostname_pattern(pool: &PgPool, pattern_id: i64) -> Result<(), ApiError> {
    sqlx::query!(
        "DELETE FROM agent_hostname_patterns WHERE id = $1",
        pattern_id
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent flags mirroring AgentRow's own columns via query_as!, not \
              mutually-exclusive states; splitting into enums or sub-structs would require \
              restructuring the query_as! call in find_agent_by_pattern for no correctness benefit"
)]
struct PatternAgentJoinRow {
    pub pattern: String,
    pub id: i64,
    pub hostname: String,
    pub display_name: Option<String>,
    pub agent_version: Option<String>,
    pub agent_git_sha: Option<String>,
    pub agent_build_time: Option<String>,
    pub agent_commit_count: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub owner_id: Option<i64>,
    pub visibility: String,
    pub default_backup_paths: Vec<String>,
    pub default_exclude_patterns: Vec<String>,
    pub default_pre_backup_commands: sqlx::types::Json<Vec<String>>,
    pub default_post_backup_commands: sqlx::types::Json<Vec<String>>,
    pub default_file_change_patterns_raw: String,
    pub agent_token_hash: String,
    pub is_hidden: bool,
    pub last_ssh_user: Option<String>,
    pub domain: Option<String>,
    pub wake_enabled: bool,
    pub wake_mac_address: Option<String>,
    pub wake_broadcast_address: Option<String>,
    pub wake_timeout_seconds: i32,
    pub shutdown_after_backup: bool,
    pub start_agent_enabled: bool,
    pub stop_agent_after_backup: bool,
    pub ssh_host: Option<String>,
    pub ssh_port: i32,
    pub agent_service_name: String,
}

/// Looks up an agent whose registered hostname pattern glob-matches the
/// given hostname, used to resolve unmatched/imported clients to a
/// configured agent.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn find_agent_by_pattern(
    pool: &PgPool,
    hostname: &str,
) -> Result<Option<super::AgentRow>, ApiError> {
    let rows = sqlx::query_as!(
        PatternAgentJoinRow,
        "SELECT p.pattern, a.id, a.hostname, a.display_name, a.agent_version, a.agent_git_sha, \
         a.agent_build_time, a.agent_commit_count, a.created_at, a.last_seen_at, a.owner_id, \
         a.visibility, a.default_backup_paths, a.default_exclude_patterns, \
         a.default_pre_backup_commands AS \"default_pre_backup_commands: \
         sqlx::types::Json<Vec<String>>\", a.default_post_backup_commands AS \
         \"default_post_backup_commands: sqlx::types::Json<Vec<String>>\", \
         a.default_file_change_patterns_raw, a.agent_token_hash, a.is_hidden, a.last_ssh_user, \
         a.domain, a.wake_enabled, a.wake_mac_address, a.wake_broadcast_address, \
         a.wake_timeout_seconds, a.shutdown_after_backup, a.start_agent_enabled, \
         a.stop_agent_after_backup, a.ssh_host, a.ssh_port, a.agent_service_name FROM \
         agent_hostname_patterns p JOIN agents a ON a.id = p.agent_id ORDER BY p.pattern",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    let matched = rows
        .iter()
        .find(|row| glob_match::glob_match(&row.pattern, hostname));

    Ok(matched.map(|row| super::AgentRow {
        id: row.id,
        hostname: row.hostname.clone(),
        display_name: row.display_name.clone(),
        agent_version: row.agent_version.clone(),
        agent_git_sha: row.agent_git_sha.clone(),
        agent_build_time: row.agent_build_time.clone(),
        agent_commit_count: row.agent_commit_count,
        created_at: row.created_at,
        last_seen_at: row.last_seen_at,
        owner_id: row.owner_id,
        visibility: row.visibility.clone(),
        default_backup_paths: row.default_backup_paths.clone(),
        default_exclude_patterns: row.default_exclude_patterns.clone(),
        default_pre_backup_commands: row.default_pre_backup_commands.clone(),
        default_post_backup_commands: row.default_post_backup_commands.clone(),
        default_file_change_patterns_raw: row.default_file_change_patterns_raw.clone(),
        agent_token_hash: row.agent_token_hash.clone(),
        is_hidden: row.is_hidden,
        last_ssh_user: row.last_ssh_user.clone(),
        domain: row.domain.clone(),
        wake_enabled: row.wake_enabled,
        wake_mac_address: row.wake_mac_address.clone(),
        wake_broadcast_address: row.wake_broadcast_address.clone(),
        wake_timeout_seconds: row.wake_timeout_seconds,
        shutdown_after_backup: row.shutdown_after_backup,
        start_agent_enabled: row.start_agent_enabled,
        stop_agent_after_backup: row.stop_agent_after_backup,
        ssh_host: row.ssh_host.clone(),
        ssh_port: row.ssh_port,
        agent_service_name: row.agent_service_name.clone(),
    }))
}
