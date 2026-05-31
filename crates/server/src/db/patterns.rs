// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

use crate::error::ApiError;

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct HostnamePatternRow {
    pub id: i64,
    pub client_id: i64,
    pub pattern: String,
    pub created_at: DateTime<Utc>,
}

pub async fn list_patterns_for_client(
    pool: &PgPool,
    client_id: i64,
) -> Result<Vec<HostnamePatternRow>, ApiError> {
    sqlx::query_as::<_, HostnamePatternRow>(
        "SELECT id, client_id, pattern, created_at FROM client_hostname_patterns WHERE client_id \
         = $1 ORDER BY pattern",
    )
    .bind(client_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn add_hostname_pattern(
    pool: &PgPool,
    client_id: i64,
    pattern: &str,
) -> Result<HostnamePatternRow, ApiError> {
    sqlx::query_as::<_, HostnamePatternRow>(
        "INSERT INTO client_hostname_patterns (client_id, pattern) VALUES ($1, $2) RETURNING id, \
         client_id, pattern, created_at",
    )
    .bind(client_id)
    .bind(pattern)
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn delete_hostname_pattern(pool: &PgPool, pattern_id: i64) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM client_hostname_patterns WHERE id = $1")
        .bind(pattern_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
struct PatternClientJoinRow {
    pub pattern: String,
    pub id: i64,
    pub hostname: String,
    pub display_name: Option<String>,
    pub agent_version: Option<String>,
    pub agent_git_sha: Option<String>,
    pub agent_build_time: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub owner_id: Option<i64>,
    pub visibility: String,
    pub default_backup_paths: Vec<String>,
    pub default_exclude_patterns: Vec<String>,
}

pub async fn find_client_by_pattern(
    pool: &PgPool,
    hostname: &str,
) -> Result<Option<super::ClientRow>, ApiError> {
    let rows = sqlx::query_as::<_, PatternClientJoinRow>(
        "SELECT p.pattern, c.id, c.hostname, c.display_name, c.agent_version, c.agent_git_sha, \
         c.agent_build_time, c.created_at, c.last_seen_at, c.owner_id, c.visibility, \
         c.default_backup_paths, c.default_exclude_patterns FROM client_hostname_patterns p JOIN \
         clients c ON c.id = p.client_id ORDER BY p.pattern",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    let matched = rows
        .iter()
        .find(|row| glob_match::glob_match(&row.pattern, hostname));

    Ok(matched.map(|row| super::ClientRow {
        id: row.id,
        hostname: row.hostname.clone(),
        display_name: row.display_name.clone(),
        agent_version: row.agent_version.clone(),
        agent_git_sha: row.agent_git_sha.clone(),
        agent_build_time: row.agent_build_time.clone(),
        created_at: row.created_at,
        last_seen_at: row.last_seen_at,
        owner_id: row.owner_id,
        visibility: row.visibility.clone(),
        default_backup_paths: row.default_backup_paths.clone(),
        default_exclude_patterns: row.default_exclude_patterns.clone(),
    }))
}
