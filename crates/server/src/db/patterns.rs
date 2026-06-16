// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

use crate::error::ApiError;

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct HostnamePatternRow {
    pub id: i64,
    pub agent_id: i64,
    pub pattern: String,
    pub created_at: DateTime<Utc>,
}

pub async fn list_patterns_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<HostnamePatternRow>, ApiError> {
    sqlx::query_as::<_, HostnamePatternRow>(
        "SELECT id, agent_id, pattern, created_at FROM agent_hostname_patterns WHERE agent_id = \
         $1 ORDER BY pattern",
    )
    .bind(agent_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn add_hostname_pattern(
    pool: &PgPool,
    agent_id: i64,
    pattern: &str,
) -> Result<HostnamePatternRow, ApiError> {
    sqlx::query_as::<_, HostnamePatternRow>(
        "INSERT INTO agent_hostname_patterns (agent_id, pattern) VALUES ($1, $2) RETURNING id, \
         agent_id, pattern, created_at",
    )
    .bind(agent_id)
    .bind(pattern)
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn delete_hostname_pattern(pool: &PgPool, pattern_id: i64) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM agent_hostname_patterns WHERE id = $1")
        .bind(pattern_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
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
    pub agent_token_hash: String,
    pub is_hidden: bool,
}

pub async fn find_agent_by_pattern(
    pool: &PgPool,
    hostname: &str,
) -> Result<Option<super::AgentRow>, ApiError> {
    let rows = sqlx::query_as::<_, PatternAgentJoinRow>(
        "SELECT p.pattern, a.id, a.hostname, a.display_name, a.agent_version, a.agent_git_sha, \
         a.agent_build_time, a.agent_commit_count, a.created_at, a.last_seen_at, a.owner_id, \
         a.visibility, a.default_backup_paths, a.default_exclude_patterns, a.agent_token_hash, \
         a.is_hidden FROM agent_hostname_patterns p JOIN agents a ON a.id = p.agent_id ORDER BY \
         p.pattern",
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
        agent_token_hash: row.agent_token_hash.clone(),
        is_hidden: row.is_hidden,
    }))
}
