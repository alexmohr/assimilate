// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

pub mod audit;
pub mod dashboard;
pub mod patterns;
pub mod quota;
pub mod tags;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared::types::{Compression, ScheduleType};
use sqlx::PgPool;

use crate::error::ApiError;

/// Sentinel `agent_token_hash` value for imported placeholder agents that have
/// no real authentication token.
pub const IMPORTED_TOKEN_HASH: &str = "imported:no-auth";

#[derive(Debug, Clone, Serialize)]
pub enum ResolveResult {
    ExactMatch(AgentRow),
    PatternMatch(AgentRow),
    Unmatched,
}

pub async fn resolve_agent_for_hostname(
    pool: &PgPool,
    hostname: &str,
) -> Result<ResolveResult, ApiError> {
    let exact = sqlx::query_as::<_, AgentRow>(
        "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
         default_post_backup_commands, agent_token_hash, is_hidden FROM agents WHERE hostname = \
         $1 AND agent_token_hash != 'imported:no-auth'",
    )
    .bind(hostname)
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

pub async fn merge_agent(pool: &PgPool, source_id: i64, target_id: i64) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    let source = sqlx::query_as::<_, AgentRow>(
        "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
         default_post_backup_commands, agent_token_hash, is_hidden FROM agents WHERE id = $1",
    )
    .bind(source_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    let Some(source) = source else {
        return Err(ApiError::NotFound(format!(
            "source agent {source_id} not found"
        )));
    };

    let has_imported_token =
        sqlx::query_scalar::<_, String>("SELECT agent_token_hash FROM agents WHERE id = $1")
            .bind(source.id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::Database)?;

    if has_imported_token != IMPORTED_TOKEN_HASH {
        return Err(ApiError::BadRequest(
            "source agent does not have imported:no-auth token".to_string(),
        ));
    }

    sqlx::query("UPDATE backup_reports SET agent_id = $1, matched = true WHERE agent_id = $2")
        .bind(target_id)
        .bind(source_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

    sqlx::query("UPDATE schedule_targets SET agent_id = $1 WHERE agent_id = $2")
        .bind(target_id)
        .bind(source_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

    sqlx::query(
        "INSERT INTO agent_tags (agent_id, tag_id) SELECT $1, tag_id FROM agent_tags WHERE \
         agent_id = $2 ON CONFLICT DO NOTHING",
    )
    .bind(target_id)
    .bind(source_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    sqlx::query("DELETE FROM agent_tags WHERE agent_id = $1")
        .bind(source_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

    sqlx::query("DELETE FROM agents WHERE id = $1")
        .bind(source_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct AgentRow {
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
    #[serde(default)]
    pub default_backup_paths: Vec<String>,
    #[serde(default)]
    pub default_exclude_patterns: Vec<String>,
    pub default_pre_backup_commands: String,
    pub default_post_backup_commands: String,
    #[serde(skip)]
    pub agent_token_hash: String,
    #[serde(default)]
    pub is_hidden: bool,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct RepoRow {
    pub id: i64,
    pub name: String,
    pub repo_path: String,
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: i32,
    pub compression: String,
    pub encryption: String,
    pub enabled: bool,
    pub owner_id: Option<i64>,
    pub visibility: String,
    pub sync_schedule: Option<String>,
    pub last_synced_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RepoConnectionRow {
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: i32,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SshTunnel {
    pub id: i64,
    pub agent_id: i64,
    pub ssh_host: String,
    pub ssh_user: String,
    pub ssh_port: i32,
    pub tunnel_port: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct NewSshTunnel {
    pub agent_id: i64,
    pub ssh_host: String,
    pub ssh_user: String,
    pub ssh_port: Option<i32>,
    pub tunnel_port: i32,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSshTunnel {
    pub ssh_host: Option<String>,
    pub ssh_user: Option<String>,
    pub ssh_port: Option<i32>,
    pub tunnel_port: Option<i32>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct GlobalExcludesConfig {
    pub raw_text: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ScheduleRow {
    pub id: i64,
    pub repo_id: Option<i64>,
    pub name: String,
    pub schedule_type: String,
    pub cron_expression: String,
    pub enabled: bool,
    pub canary_enabled: bool,
    pub last_run_at: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub exclude_patterns_raw: String,
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
    pub execution_mode: String,
    pub on_failure: String,
    pub owner_id: Option<i64>,
    pub visibility: String,
    #[serde(default)]
    #[sqlx(default)]
    pub target_hostnames: Vec<String>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ScheduleTargetRow {
    pub agent_id: i64,
    pub execution_order: i32,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ScheduleCountByAgent {
    pub agent_id: i64,
    pub count: i64,
}

pub async fn get_schedule_counts_by_agent(
    pool: &PgPool,
) -> Result<Vec<ScheduleCountByAgent>, ApiError> {
    sqlx::query_as::<_, ScheduleCountByAgent>(
        "SELECT agent_id, COUNT(DISTINCT schedule_id)::bigint AS count FROM schedule_targets \
         GROUP BY agent_id",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn get_agent_by_hostname(pool: &PgPool, hostname: &str) -> Result<AgentRow, ApiError> {
    sqlx::query_as::<_, AgentRow>(
        "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
         default_post_backup_commands, agent_token_hash, is_hidden FROM agents WHERE hostname = $1",
    )
    .bind(hostname)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent '{hostname}' not found")),
        other => ApiError::Database(other),
    })
}

pub async fn get_agent_by_id(pool: &PgPool, agent_id: i64) -> Result<AgentRow, ApiError> {
    sqlx::query_as::<_, AgentRow>(
        "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
         default_post_backup_commands, agent_token_hash, is_hidden FROM agents WHERE id = $1",
    )
    .bind(agent_id)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent id '{agent_id}' not found")),
        other => ApiError::Database(other),
    })
}

pub async fn get_agent_token_hash(
    pool: &PgPool,
    hostname: &str,
) -> Result<(i64, String), ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i64,
        agent_token_hash: String,
    }

    let row =
        sqlx::query_as::<_, Row>("SELECT id, agent_token_hash FROM agents WHERE hostname = $1")
            .bind(hostname)
            .fetch_one(pool)
            .await
            .map_err(|e| match e {
                sqlx::Error::RowNotFound => {
                    ApiError::NotFound(format!("agent '{hostname}' not found"))
                }
                other => ApiError::Database(other),
            })?;

    Ok((row.id, row.agent_token_hash))
}

pub async fn update_last_seen(pool: &PgPool, agent_id: i64) -> Result<(), ApiError> {
    sqlx::query("UPDATE agents SET last_seen_at = NOW() WHERE id = $1")
        .bind(agent_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn update_last_seen_and_version(
    pool: &PgPool,
    agent_id: i64,
    agent_version: &str,
    agent_git_sha: Option<&str>,
    agent_build_time: Option<&str>,
    agent_commit_count: Option<i32>,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE agents SET last_seen_at = NOW(), agent_version = $2, agent_git_sha = $3, \
         agent_build_time = $4, agent_commit_count = $5 WHERE id = $1",
    )
    .bind(agent_id)
    .bind(agent_version)
    .bind(agent_git_sha)
    .bind(agent_build_time)
    .bind(agent_commit_count)
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn update_last_seen_by_hostname(pool: &PgPool, hostname: &str) -> Result<(), ApiError> {
    sqlx::query("UPDATE agents SET last_seen_at = NOW() WHERE hostname = $1")
        .bind(hostname)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn list_agents(pool: &PgPool, include_hidden: bool) -> Result<Vec<AgentRow>, ApiError> {
    let sql = if include_hidden {
        "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, created_at, last_seen_at, owner_id, visibility, default_backup_paths, \
         default_exclude_patterns, default_pre_backup_commands, default_post_backup_commands, \
         agent_token_hash, is_hidden FROM agents ORDER BY hostname"
    } else {
        "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, created_at, last_seen_at, owner_id, visibility, default_backup_paths, \
         default_exclude_patterns, default_pre_backup_commands, default_post_backup_commands, \
         agent_token_hash, is_hidden FROM agents WHERE is_hidden = false ORDER BY hostname"
    };
    sqlx::query_as::<_, AgentRow>(sql)
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
}

pub async fn set_agent_hidden(
    pool: &PgPool,
    hostname: &str,
    hidden: bool,
) -> Result<AgentRow, ApiError> {
    sqlx::query_as::<_, AgentRow>(
        "UPDATE agents SET is_hidden = $2 WHERE hostname = $1 RETURNING id, hostname, \
         display_name, agent_version, agent_git_sha, agent_build_time, agent_commit_count, \
         created_at, last_seen_at, owner_id, visibility, default_backup_paths, \
         default_exclude_patterns, default_pre_backup_commands, default_post_backup_commands, \
         agent_token_hash, is_hidden",
    )
    .bind(hostname)
    .bind(hidden)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?
    .ok_or_else(|| ApiError::NotFound(format!("Agent {hostname} not found")))
}

/// Finds an agent by hostname, or creates a placeholder agent for archive imports.
///
/// Placeholder agents have a dummy token hash and cannot authenticate. They serve
/// only as a foreign key target for imported `backup_reports`.
pub async fn get_or_create_agent_by_hostname(
    pool: &PgPool,
    hostname: &str,
) -> Result<AgentRow, ApiError> {
    let existing = sqlx::query_as::<_, AgentRow>(
        "SELECT id, hostname, display_name, agent_version, agent_git_sha, agent_build_time, \
         agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
         default_post_backup_commands, agent_token_hash, is_hidden FROM agents WHERE hostname = $1",
    )
    .bind(hostname)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;

    if let Some(agent) = existing {
        return Ok(agent);
    }

    sqlx::query_as::<_, AgentRow>(
        "INSERT INTO agents (hostname, display_name, agent_token_hash, owner_id) VALUES ($1, $2, \
         $3, NULL) RETURNING id, hostname, display_name, agent_version, agent_git_sha, \
         agent_build_time, agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
         default_post_backup_commands, agent_token_hash, is_hidden",
    )
    .bind(hostname)
    .bind(Some(format!("{hostname} (imported)")))
    .bind("imported:no-auth")
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn insert_agent(
    pool: &PgPool,
    hostname: &str,
    display_name: Option<&str>,
    token_hash: &str,
    owner_id: Option<i64>,
) -> Result<AgentRow, ApiError> {
    sqlx::query_as::<_, AgentRow>(
        "INSERT INTO agents (hostname, display_name, agent_token_hash, owner_id) VALUES ($1, $2, \
         $3, $4) RETURNING id, hostname, display_name, agent_version, agent_git_sha, \
         agent_build_time, agent_commit_count, created_at, last_seen_at, owner_id, visibility, \
         default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
         default_post_backup_commands, agent_token_hash, is_hidden",
    )
    .bind(hostname)
    .bind(display_name)
    .bind(token_hash)
    .bind(owner_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

pub struct AgentDefaults<'a> {
    pub display_name: Option<&'a str>,
    pub default_backup_paths: &'a [String],
    pub default_exclude_patterns: &'a [String],
    pub default_pre_backup_commands: &'a str,
    pub default_post_backup_commands: &'a str,
}

pub async fn insert_agent_with_paths(
    pool: &PgPool,
    hostname: &str,
    token_hash: &str,
    defaults: AgentDefaults<'_>,
) -> Result<AgentRow, ApiError> {
    sqlx::query_as::<_, AgentRow>(
        "INSERT INTO agents (hostname, display_name, agent_token_hash, default_backup_paths, \
         default_exclude_patterns, default_pre_backup_commands, default_post_backup_commands) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id, hostname, display_name, agent_version, \
         agent_git_sha, agent_build_time, agent_commit_count, created_at, last_seen_at, owner_id, \
         visibility, default_backup_paths, default_exclude_patterns, default_pre_backup_commands, \
         default_post_backup_commands, agent_token_hash, is_hidden",
    )
    .bind(hostname)
    .bind(defaults.display_name)
    .bind(token_hash)
    .bind(defaults.default_backup_paths)
    .bind(defaults.default_exclude_patterns)
    .bind(defaults.default_pre_backup_commands)
    .bind(defaults.default_post_backup_commands)
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn update_agent(
    pool: &PgPool,
    hostname: &str,
    new_hostname: &str,
    defaults: AgentDefaults<'_>,
) -> Result<AgentRow, ApiError> {
    sqlx::query_as::<_, AgentRow>(
        "UPDATE agents SET hostname = $2, display_name = $3, default_backup_paths = $4, \
         default_exclude_patterns = $5, default_pre_backup_commands = $6, \
         default_post_backup_commands = $7 WHERE hostname = $1 RETURNING id, hostname, \
         display_name, agent_version, agent_git_sha, agent_build_time, agent_commit_count, \
         created_at, last_seen_at, owner_id, visibility, default_backup_paths, \
         default_exclude_patterns, default_pre_backup_commands, default_post_backup_commands, \
         agent_token_hash, is_hidden",
    )
    .bind(hostname)
    .bind(new_hostname)
    .bind(defaults.display_name)
    .bind(defaults.default_backup_paths)
    .bind(defaults.default_exclude_patterns)
    .bind(defaults.default_pre_backup_commands)
    .bind(defaults.default_post_backup_commands)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent '{hostname}' not found")),
        other => ApiError::Database(other),
    })
}

pub async fn regenerate_agent_token(
    pool: &PgPool,
    hostname: &str,
    token_hash: &str,
) -> Result<AgentRow, ApiError> {
    sqlx::query_as::<_, AgentRow>(
        "UPDATE agents SET agent_token_hash = $2 WHERE hostname = $1 RETURNING id, hostname, \
         display_name, agent_version, agent_git_sha, agent_build_time, agent_commit_count, \
         created_at, last_seen_at, owner_id, visibility, default_backup_paths, \
         default_exclude_patterns, default_pre_backup_commands, default_post_backup_commands, \
         agent_token_hash, is_hidden",
    )
    .bind(hostname)
    .bind(token_hash)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("agent '{hostname}' not found")),
        other => ApiError::Database(other),
    })
}

pub async fn mark_agent_reports_matched(pool: &PgPool, agent_id: i64) -> Result<(), ApiError> {
    sqlx::query("UPDATE backup_reports SET matched = true WHERE agent_id = $1 AND matched = false")
        .bind(agent_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn delete_agent(pool: &PgPool, hostname: &str) -> Result<(), ApiError> {
    let result = sqlx::query("DELETE FROM agents WHERE hostname = $1")
        .bind(hostname)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("agent '{hostname}' not found")));
    }
    Ok(())
}

pub async fn get_archives_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<(shared::types::RepoId, Vec<String>)>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        repo_id: i64,
        archive_name: Option<String>,
    }

    let rows = sqlx::query_as::<_, Row>(
        "SELECT repo_id, archive_name FROM backup_reports WHERE agent_id = $1 AND archive_name IS \
         NOT NULL",
    )
    .bind(agent_id)
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

        let all_agents =
            sqlx::query_as::<_, IdHostname>("SELECT id, hostname FROM agents WHERE id != $1")
                .bind(agent_id)
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

    #[derive(sqlx::FromRow)]
    struct Row {
        repo_id: i64,
        archive_name: Option<String>,
    }

    let placeholders: String = agent_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("${}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");

    let query_str = format!(
        "SELECT repo_id, archive_name FROM backup_reports WHERE agent_id IN ({placeholders}) AND \
         archive_name IS NOT NULL"
    );

    let mut query = sqlx::query_as::<_, Row>(&query_str);
    for id in &agent_ids {
        query = query.bind(id);
    }

    let rows = query.fetch_all(pool).await.map_err(ApiError::Database)?;

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

pub async fn get_schedule_target_hostnames_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<Vec<String>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        hostname: String,
    }

    let rows = sqlx::query_as::<_, Row>(
        "SELECT DISTINCT a.hostname FROM agents a JOIN schedule_targets st ON st.agent_id = a.id \
         JOIN schedules s ON s.id = st.schedule_id WHERE s.repo_id = $1",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows.into_iter().map(|r| r.hostname).collect())
}

pub struct InsertRepoParams<'a> {
    pub name: &'a str,
    pub repo_path: &'a str,
    pub ssh_user: &'a str,
    pub ssh_host: &'a str,
    pub ssh_port: i32,
    pub passphrase_encrypted: &'a [u8],
    pub compression: &'a str,
    pub encryption: &'a str,
    pub owner_id: Option<i64>,
}

pub struct UpdateRepoParams<'a> {
    pub repo_id: i64,
    pub name: &'a str,
    pub repo_path: &'a str,
    pub ssh_user: &'a str,
    pub ssh_host: &'a str,
    pub ssh_port: i32,
    pub compression: &'a str,
    pub encryption: &'a str,
    pub enabled: bool,
    pub sync_schedule: Option<&'a str>,
}

pub async fn list_importing_repo_ids(pool: &PgPool) -> Result<Vec<i64>, ApiError> {
    let rows = sqlx::query_scalar::<_, i64>("SELECT id FROM repos WHERE importing = true")
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(rows)
}

pub async fn set_repo_importing(
    pool: &PgPool,
    repo_id: i64,
    importing: bool,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE repos SET importing = $2 WHERE id = $1")
        .bind(repo_id)
        .bind(importing)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn set_repo_import_error(
    pool: &PgPool,
    repo_id: i64,
    error: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE repos SET import_error = $2 WHERE id = $1")
        .bind(repo_id)
        .bind(error)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn set_import_status_message(
    pool: &PgPool,
    repo_id: i64,
    msg: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE repos SET import_status_message = $2 WHERE id = $1")
        .bind(repo_id)
        .bind(msg)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn update_repo_import_progress(
    pool: &PgPool,
    repo_id: i64,
    progress: i64,
    total: i64,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE repos SET import_progress = $2, import_total = $3 WHERE id = $1")
        .bind(repo_id)
        .bind(i32::try_from(progress).unwrap_or(i32::MAX))
        .bind(i32::try_from(total).unwrap_or(i32::MAX))
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn update_repo_last_synced(pool: &PgPool, repo_id: i64) -> Result<(), ApiError> {
    sqlx::query("UPDATE repos SET last_synced_at = NOW() WHERE id = $1")
        .bind(repo_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

/// Authoritative repository statistics parsed from `borg info --json`
/// (`cache.stats`) and `borg list --json`. This is the single source of truth
/// for repo size and archive counts; never derive these from `backup_reports`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RepoInfoStats {
    pub original_size: i64,
    pub compressed_size: i64,
    pub deduplicated_size: i64,
    pub total_chunks: i64,
    pub unique_chunks: i64,
    pub archive_count: i64,
}

pub async fn update_repo_info_stats(
    pool: &PgPool,
    repo_id: i64,
    stats: &RepoInfoStats,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE repos SET info_original_size = $2, info_compressed_size = $3, \
         info_deduplicated_size = $4, info_total_chunks = $5, info_unique_chunks = $6, \
         info_archive_count = $7, info_updated_at = NOW() WHERE id = $1",
    )
    .bind(repo_id)
    .bind(stats.original_size)
    .bind(stats.compressed_size)
    .bind(stats.deduplicated_size)
    .bind(stats.total_chunks)
    .bind(stats.unique_chunks)
    .bind(i32::try_from(stats.archive_count).unwrap_or(i32::MAX))
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn clear_relocation_pending(pool: &PgPool, repo_id: i64) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    sqlx::query("DELETE FROM repo_relocation_pending_hosts WHERE repo_id = $1")
        .bind(repo_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;
    sqlx::query("UPDATE repos SET relocation_pending = false WHERE id = $1")
        .bind(repo_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;
    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

/// Remove `hostname` from the pending-hosts set for this repo. Clears `relocation_pending`
/// on the repo itself once every registered host has confirmed the new location.
///
/// Only clears the flag when this host's entry was actually present (rows_affected > 0) AND
/// no other hosts remain pending. This prevents spurious clears when a host that was never
/// registered in the pending table completes a backup.
pub async fn clear_relocation_for_host(
    pool: &PgPool,
    repo_id: i64,
    hostname: &str,
) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    let deleted = sqlx::query(
        "DELETE FROM repo_relocation_pending_hosts WHERE repo_id = $1 AND hostname = $2",
    )
    .bind(repo_id)
    .bind(hostname)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    if deleted.rows_affected() > 0 {
        let remaining: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM repo_relocation_pending_hosts WHERE repo_id = $1")
                .bind(repo_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(ApiError::Database)?;

        if remaining.0 == 0 {
            sqlx::query("UPDATE repos SET relocation_pending = false WHERE id = $1")
                .bind(repo_id)
                .execute(&mut *tx)
                .await
                .map_err(ApiError::Database)?;
        }
    }
    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

pub async fn set_relocation_pending(pool: &PgPool, repo_id: i64) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;
    sqlx::query("UPDATE repos SET relocation_pending = true WHERE id = $1")
        .bind(repo_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;
    sqlx::query(
        "INSERT INTO repo_relocation_pending_hosts (repo_id, hostname) SELECT $1, a.hostname FROM \
         agents a JOIN schedule_targets st ON st.agent_id = a.id JOIN schedules s ON s.id = \
         st.schedule_id WHERE s.repo_id = $1 ON CONFLICT DO NOTHING",
    )
    .bind(repo_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;
    tx.commit().await.map_err(ApiError::Database)?;
    Ok(())
}

pub async fn update_repo_encryption(
    pool: &PgPool,
    repo_id: i64,
    encryption: &str,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE repos SET encryption = $2 WHERE id = $1")
        .bind(repo_id)
        .bind(encryption)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn insert_repo(
    pool: &PgPool,
    params: &InsertRepoParams<'_>,
) -> Result<RepoRow, ApiError> {
    sqlx::query_as::<_, RepoRow>(
        "INSERT INTO repos (name, repo_path, ssh_user, ssh_host, ssh_port, passphrase_encrypted, \
         compression, encryption, owner_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING \
         id, name, repo_path, ssh_user, ssh_host, ssh_port, compression, encryption, enabled, \
         owner_id, visibility, sync_schedule, last_synced_at",
    )
    .bind(params.name)
    .bind(params.repo_path)
    .bind(params.ssh_user)
    .bind(params.ssh_host)
    .bind(params.ssh_port)
    .bind(params.passphrase_encrypted)
    .bind(params.compression)
    .bind(params.encryption)
    .bind(params.owner_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn get_repo_connection(
    pool: &PgPool,
    repo_id: i64,
) -> Result<RepoConnectionRow, ApiError> {
    sqlx::query_as::<_, RepoConnectionRow>(
        "SELECT ssh_user, ssh_host, ssh_port FROM repos WHERE id = $1",
    )
    .bind(repo_id)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
        other => ApiError::Database(other),
    })
}

pub async fn update_repo(
    pool: &PgPool,
    params: &UpdateRepoParams<'_>,
) -> Result<RepoRow, ApiError> {
    sqlx::query_as::<_, RepoRow>(
        "UPDATE repos SET name = $2, repo_path = $3, ssh_user = $4, ssh_host = $5, ssh_port = $6, \
         compression = $7, encryption = $8, enabled = $9, sync_schedule = $10 WHERE id = $1 \
         RETURNING id, name, repo_path, ssh_user, ssh_host, ssh_port, compression, encryption, \
         enabled, owner_id, visibility, sync_schedule, last_synced_at",
    )
    .bind(params.repo_id)
    .bind(params.name)
    .bind(params.repo_path)
    .bind(params.ssh_user)
    .bind(params.ssh_host)
    .bind(params.ssh_port)
    .bind(params.compression)
    .bind(params.encryption)
    .bind(params.enabled)
    .bind(params.sync_schedule)
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
pub async fn update_repo_and_set_relocation_pending(
    pool: &PgPool,
    params: &UpdateRepoParams<'_>,
) -> Result<RepoRow, ApiError> {
    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    let repo = sqlx::query_as::<_, RepoRow>(
        "UPDATE repos SET name = $2, repo_path = $3, ssh_user = $4, ssh_host = $5, ssh_port = $6, \
         compression = $7, encryption = $8, enabled = $9, sync_schedule = $10, relocation_pending \
         = true WHERE id = $1 RETURNING id, name, repo_path, ssh_user, ssh_host, ssh_port, \
         compression, encryption, enabled, owner_id, visibility, sync_schedule, last_synced_at",
    )
    .bind(params.repo_id)
    .bind(params.name)
    .bind(params.repo_path)
    .bind(params.ssh_user)
    .bind(params.ssh_host)
    .bind(params.ssh_port)
    .bind(params.compression)
    .bind(params.encryption)
    .bind(params.enabled)
    .bind(params.sync_schedule)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => {
            ApiError::NotFound(format!("repo {} not found", params.repo_id))
        }
        other => ApiError::Database(other),
    })?;

    sqlx::query(
        "INSERT INTO repo_relocation_pending_hosts (repo_id, hostname) SELECT $1, a.hostname FROM \
         agents a JOIN schedule_targets st ON st.agent_id = a.id JOIN schedules s ON s.id = \
         st.schedule_id WHERE s.repo_id = $1 ON CONFLICT DO NOTHING",
    )
    .bind(params.repo_id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    tx.commit().await.map_err(ApiError::Database)?;
    Ok(repo)
}

pub async fn delete_repo(pool: &PgPool, repo_id: i64) -> Result<(), ApiError> {
    sqlx::query("UPDATE schedules SET enabled = false WHERE repo_id = $1")
        .bind(repo_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    let result = sqlx::query("DELETE FROM repos WHERE id = $1")
        .bind(repo_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("repo {repo_id} not found")));
    }
    Ok(())
}

pub async fn list_enabled_tunnels(pool: &PgPool) -> Result<Vec<SshTunnel>, ApiError> {
    sqlx::query_as::<_, SshTunnel>(
        "SELECT id, agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, enabled, created_at FROM \
         ssh_tunnels WHERE enabled = true ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn list_all_tunnels(pool: &PgPool) -> Result<Vec<SshTunnel>, ApiError> {
    sqlx::query_as::<_, SshTunnel>(
        "SELECT id, agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, enabled, created_at FROM \
         ssh_tunnels ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn get_tunnel_by_id(pool: &PgPool, id: i64) -> Result<SshTunnel, ApiError> {
    sqlx::query_as::<_, SshTunnel>(
        "SELECT id, agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, enabled, created_at FROM \
         ssh_tunnels WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("ssh tunnel {id} not found")),
        other => ApiError::Database(other),
    })
}

pub async fn get_tunnel_by_agent_id(pool: &PgPool, agent_id: i64) -> Result<SshTunnel, ApiError> {
    sqlx::query_as::<_, SshTunnel>(
        "SELECT id, agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, enabled, created_at FROM \
         ssh_tunnels WHERE agent_id = $1",
    )
    .bind(agent_id)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => {
            ApiError::NotFound(format!("ssh tunnel for agent {agent_id} not found"))
        }
        other => ApiError::Database(other),
    })
}

pub async fn insert_tunnel(pool: &PgPool, params: &NewSshTunnel) -> Result<SshTunnel, ApiError> {
    sqlx::query_as::<_, SshTunnel>(
        "INSERT INTO ssh_tunnels (agent_id, ssh_host, ssh_user, ssh_port, tunnel_port, enabled) \
         VALUES ($1, $2, $3, COALESCE($4, 22), $5, COALESCE($6, true)) RETURNING id, agent_id, \
         ssh_host, ssh_user, ssh_port, tunnel_port, enabled, created_at",
    )
    .bind(params.agent_id)
    .bind(&params.ssh_host)
    .bind(&params.ssh_user)
    .bind(params.ssh_port)
    .bind(params.tunnel_port)
    .bind(params.enabled)
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn update_tunnel(
    pool: &PgPool,
    id: i64,
    params: &UpdateSshTunnel,
) -> Result<SshTunnel, ApiError> {
    sqlx::query_as::<_, SshTunnel>(
        "UPDATE ssh_tunnels SET ssh_host = COALESCE($2, ssh_host), ssh_user = COALESCE($3, \
         ssh_user), ssh_port = COALESCE($4, ssh_port), tunnel_port = COALESCE($5, tunnel_port), \
         enabled = COALESCE($6, enabled) WHERE id = $1 RETURNING id, agent_id, ssh_host, \
         ssh_user, ssh_port, tunnel_port, enabled, created_at",
    )
    .bind(id)
    .bind(&params.ssh_host)
    .bind(&params.ssh_user)
    .bind(params.ssh_port)
    .bind(params.tunnel_port)
    .bind(params.enabled)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("ssh tunnel {id} not found")),
        other => ApiError::Database(other),
    })
}

pub async fn delete_tunnel(pool: &PgPool, id: i64) -> Result<(), ApiError> {
    let result = sqlx::query("DELETE FROM ssh_tunnels WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("ssh tunnel {id} not found")));
    }
    Ok(())
}

pub async fn update_repo_passphrase(
    pool: &PgPool,
    repo_id: i64,
    passphrase_encrypted: &[u8],
) -> Result<(), ApiError> {
    let result = sqlx::query("UPDATE repos SET passphrase_encrypted = $2 WHERE id = $1")
        .bind(repo_id)
        .bind(passphrase_encrypted)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("repo {repo_id} not found")));
    }
    Ok(())
}

pub async fn get_repo_passphrase(pool: &PgPool, repo_id: i64) -> Result<Vec<u8>, ApiError> {
    let row: (Vec<u8>,) = sqlx::query_as("SELECT passphrase_encrypted FROM repos WHERE id = $1")
        .bind(repo_id)
        .fetch_one(pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
            other => ApiError::Database(other),
        })?;
    Ok(row.0)
}

pub async fn get_repo_with_passphrase(
    pool: &PgPool,
    repo_id: i64,
) -> Result<RepoWithPassphraseRow, ApiError> {
    sqlx::query_as::<_, RepoWithPassphraseRow>(
        "SELECT id, name, repo_path, ssh_user, ssh_host, ssh_port, ssh_host_key, \
         passphrase_encrypted, compression, encryption, enabled, relocation_pending, \
         sync_schedule FROM repos WHERE id = $1",
    )
    .bind(repo_id)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
        other => ApiError::Database(other),
    })
}

pub async fn update_repo_ssh_host_key(
    pool: &PgPool,
    repo_id: i64,
    ssh_host_key: &str,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE repos SET ssh_host_key = $2 WHERE id = $1")
        .bind(repo_id)
        .bind(ssh_host_key)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn get_global_excludes_raw(pool: &PgPool) -> Result<String, ApiError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT raw_text FROM excludes_global_config LIMIT 1")
            .fetch_optional(pool)
            .await
            .map_err(ApiError::Database)?;
    Ok(row.map(|(t,)| t).unwrap_or_default())
}

pub async fn set_global_excludes_raw(pool: &PgPool, raw_text: &str) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO excludes_global_config (raw_text) VALUES ($1) ON CONFLICT (id) DO UPDATE SET \
         raw_text = EXCLUDED.raw_text",
    )
    .bind(raw_text)
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn list_schedules(pool: &PgPool) -> Result<Vec<ScheduleRow>, ApiError> {
    sqlx::query_as::<_, ScheduleRow>(
        "SELECT s.id, s.repo_id, s.name, s.schedule_type, s.cron_expression, s.enabled, \
         s.canary_enabled, s.last_run_at, s.next_run_at, s.exclude_patterns_raw, \
         s.ignore_global_excludes, s.keep_hourly, s.keep_daily, s.keep_weekly, s.keep_monthly, \
         s.keep_yearly, s.compact_enabled, s.rate_limit_kbps, s.pre_backup_commands, \
         s.post_backup_commands, s.execution_mode, s.on_failure, s.owner_id, s.visibility, \
         ARRAY(SELECT a.hostname FROM schedule_targets st JOIN agents a ON a.id = st.agent_id \
         WHERE st.schedule_id = s.id ORDER BY st.execution_order, a.hostname) AS target_hostnames \
         FROM schedules s ORDER BY s.id",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub struct ScheduleParams<'a> {
    pub name: &'a str,
    pub schedule_type: &'a str,
    pub cron_expression: &'a str,
    pub enabled: bool,
    pub canary_enabled: bool,
    pub exclude_patterns_raw: &'a str,
    pub ignore_global_excludes: bool,
    pub keep_hourly: i32,
    pub keep_daily: i32,
    pub keep_weekly: i32,
    pub keep_monthly: i32,
    pub keep_yearly: i32,
    pub compact_enabled: bool,
    pub rate_limit_kbps: Option<i32>,
    pub pre_backup_commands: &'a str,
    pub post_backup_commands: &'a str,
    pub on_failure: &'a str,
}

pub async fn insert_schedule(
    pool: &PgPool,
    repo_id: i64,
    params: &ScheduleParams<'_>,
    owner_id: Option<i64>,
) -> Result<ScheduleRow, ApiError> {
    sqlx::query_as::<_, ScheduleRow>(
        "INSERT INTO schedules (repo_id, name, schedule_type, cron_expression, enabled, \
         canary_enabled, exclude_patterns_raw, ignore_global_excludes, keep_hourly, keep_daily, \
         keep_weekly, keep_monthly, keep_yearly, compact_enabled, rate_limit_kbps, \
         pre_backup_commands, post_backup_commands, execution_mode, on_failure, owner_id) VALUES \
         ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, \
         'sequential', $18, $19) RETURNING id, repo_id, name, schedule_type, cron_expression, \
         enabled, canary_enabled, last_run_at, next_run_at, exclude_patterns_raw, \
         ignore_global_excludes, keep_hourly, keep_daily, keep_weekly, keep_monthly, keep_yearly, \
         compact_enabled, rate_limit_kbps, pre_backup_commands, post_backup_commands, \
         execution_mode, on_failure, owner_id, visibility",
    )
    .bind(repo_id)
    .bind(params.name)
    .bind(params.schedule_type)
    .bind(params.cron_expression)
    .bind(params.enabled)
    .bind(params.canary_enabled)
    .bind(params.exclude_patterns_raw)
    .bind(params.ignore_global_excludes)
    .bind(params.keep_hourly)
    .bind(params.keep_daily)
    .bind(params.keep_weekly)
    .bind(params.keep_monthly)
    .bind(params.keep_yearly)
    .bind(params.compact_enabled)
    .bind(params.rate_limit_kbps)
    .bind(params.pre_backup_commands)
    .bind(params.post_backup_commands)
    .bind(params.on_failure)
    .bind(owner_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn update_schedule(
    pool: &PgPool,
    id: i64,
    params: &ScheduleParams<'_>,
) -> Result<ScheduleRow, ApiError> {
    sqlx::query_as::<_, ScheduleRow>(
        "UPDATE schedules SET name = $2, cron_expression = $3, enabled = $4, canary_enabled = $5, \
         exclude_patterns_raw = $6, ignore_global_excludes = $7, keep_hourly = $8, keep_daily = \
         $9, keep_weekly = $10, keep_monthly = $11, keep_yearly = $12, compact_enabled = $13, \
         rate_limit_kbps = $14, pre_backup_commands = $15, post_backup_commands = $16, \
         execution_mode = 'sequential', on_failure = $17 WHERE id = $1 RETURNING id, repo_id, \
         name, schedule_type, cron_expression, enabled, canary_enabled, last_run_at, next_run_at, \
         exclude_patterns_raw, ignore_global_excludes, keep_hourly, keep_daily, keep_weekly, \
         keep_monthly, keep_yearly, compact_enabled, rate_limit_kbps, pre_backup_commands, \
         post_backup_commands, execution_mode, on_failure, owner_id, visibility",
    )
    .bind(id)
    .bind(params.name)
    .bind(params.cron_expression)
    .bind(params.enabled)
    .bind(params.canary_enabled)
    .bind(params.exclude_patterns_raw)
    .bind(params.ignore_global_excludes)
    .bind(params.keep_hourly)
    .bind(params.keep_daily)
    .bind(params.keep_weekly)
    .bind(params.keep_monthly)
    .bind(params.keep_yearly)
    .bind(params.compact_enabled)
    .bind(params.rate_limit_kbps)
    .bind(params.pre_backup_commands)
    .bind(params.post_backup_commands)
    .bind(params.on_failure)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("schedule {id} not found")),
        other => ApiError::Database(other),
    })
}

pub async fn update_schedule_repo(pool: &PgPool, id: i64, repo_id: i64) -> Result<(), ApiError> {
    let rows_affected = sqlx::query("UPDATE schedules SET repo_id = $2 WHERE id = $1")
        .bind(id)
        .bind(repo_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?
        .rows_affected();
    if rows_affected == 0 {
        return Err(ApiError::NotFound(format!("schedule {id} not found")));
    }
    Ok(())
}

pub fn compression_to_str(c: &Compression) -> String {
    c.to_string()
}

pub fn compression_from_str(s: &str) -> Result<Compression, ApiError> {
    if s == "none" {
        return Ok(Compression::None);
    }
    if s == "lz4" {
        return Ok(Compression::Lz4);
    }
    if let Some(level_str) = s.strip_prefix("zstd,") {
        let level = level_str
            .parse::<i32>()
            .map_err(|_| ApiError::Internal(format!("invalid zstd level: {level_str}")))?;
        return Ok(Compression::Zstd { level });
    }
    if let Some(level_str) = s.strip_prefix("zlib,") {
        let level = level_str
            .parse::<i32>()
            .map_err(|_| ApiError::Internal(format!("invalid zlib level: {level_str}")))?;
        return Ok(Compression::Zlib { level });
    }
    Err(ApiError::Internal(format!("unknown compression: {s}")))
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RepoWithPassphraseRow {
    pub id: i64,
    pub name: String,
    pub repo_path: String,
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: i32,
    pub ssh_host_key: Option<String>,
    pub passphrase_encrypted: Vec<u8>,
    pub compression: String,
    pub encryption: String,
    pub enabled: bool,
    pub relocation_pending: bool,
    pub sync_schedule: Option<String>,
}

pub async fn list_all_repos(pool: &PgPool) -> Result<Vec<RepoRow>, ApiError> {
    sqlx::query_as::<_, RepoRow>(
        "SELECT id, name, repo_path, ssh_user, ssh_host, ssh_port, compression, encryption, \
         enabled, owner_id, visibility, sync_schedule, last_synced_at FROM repos ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn list_repos_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<RepoWithPassphraseRow>, ApiError> {
    sqlx::query_as::<_, RepoWithPassphraseRow>(
        "SELECT DISTINCT r.id, r.name, r.repo_path, r.ssh_user, r.ssh_host, r.ssh_port, \
         r.ssh_host_key, r.passphrase_encrypted, r.compression, r.encryption, r.enabled, \
         r.relocation_pending, r.sync_schedule FROM repos r JOIN schedules s ON s.repo_id = r.id \
         JOIN schedule_targets st ON st.schedule_id = s.id WHERE st.agent_id = $1 ORDER BY r.id",
    )
    .bind(agent_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn list_repos_for_agent_public(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<RepoRow>, ApiError> {
    sqlx::query_as::<_, RepoRow>(
        "SELECT DISTINCT r.id, r.name, r.repo_path, r.ssh_user, r.ssh_host, r.ssh_port, \
         r.compression, r.encryption, r.enabled, r.owner_id, r.visibility, r.sync_schedule, \
         r.last_synced_at FROM repos r JOIN schedules s ON s.repo_id = r.id JOIN schedule_targets \
         st ON st.schedule_id = s.id WHERE st.agent_id = $1 ORDER BY r.id",
    )
    .bind(agent_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn list_backup_sources_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<Vec<String>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct PathRow {
        path: String,
    }

    let rows = sqlx::query_as::<_, PathRow>(
        "SELECT path FROM backup_sources WHERE repo_id = $1 ORDER BY sort_order, id",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows.into_iter().map(|r| r.path).collect())
}

pub async fn list_backup_sources_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<String>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct PathRow {
        path: String,
    }

    let rows = sqlx::query_as::<_, PathRow>(
        "SELECT path FROM backup_sources WHERE schedule_id = $1 AND agent_id IS NULL ORDER BY \
         sort_order, id",
    )
    .bind(schedule_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows.into_iter().map(|r| r.path).collect())
}

pub async fn list_backup_sources_for_schedule_agent(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
) -> Result<Vec<String>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct PathRow {
        path: String,
    }

    let rows = sqlx::query_as::<_, PathRow>(
        "SELECT path FROM backup_sources WHERE schedule_id = $1 AND agent_id = $2 ORDER BY \
         sort_order, id",
    )
    .bind(schedule_id)
    .bind(agent_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows.into_iter().map(|r| r.path).collect())
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PerAgentBackupSources {
    pub agent_id: i64,
    pub paths: Vec<String>,
}

pub async fn list_all_per_agent_backup_sources_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<PerAgentBackupSources>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        agent_id: i64,
        path: String,
    }

    let rows = sqlx::query_as::<_, Row>(
        "SELECT agent_id, path FROM backup_sources WHERE schedule_id = $1 AND agent_id IS NOT \
         NULL ORDER BY agent_id, sort_order, id",
    )
    .bind(schedule_id)
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

pub async fn insert_backup_source_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
    path: &str,
    sort_order: i32,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO backup_sources (schedule_id, path, sort_order) VALUES ($1, $2, $3)")
        .bind(schedule_id)
        .bind(path)
        .bind(sort_order)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn insert_backup_source_for_schedule_agent(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
    path: &str,
    sort_order: i32,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO backup_sources (schedule_id, agent_id, path, sort_order) VALUES ($1, $2, $3, \
         $4)",
    )
    .bind(schedule_id)
    .bind(agent_id)
    .bind(path)
    .bind(sort_order)
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn delete_backup_sources_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM backup_sources WHERE schedule_id = $1 AND agent_id IS NULL")
        .bind(schedule_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn delete_per_agent_backup_sources_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM backup_sources WHERE schedule_id = $1 AND agent_id IS NOT NULL")
        .bind(schedule_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PerAgentExcludePatterns {
    pub agent_id: i64,
    pub raw_text: String,
}

pub async fn list_all_per_agent_excludes_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<PerAgentExcludePatterns>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        agent_id: i64,
        raw_text: String,
    }

    let rows = sqlx::query_as::<_, Row>(
        "SELECT agent_id, raw_text FROM per_agent_excludes WHERE schedule_id = $1 ORDER BY \
         agent_id",
    )
    .bind(schedule_id)
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

pub async fn upsert_per_agent_excludes_raw(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
    raw_text: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO per_agent_excludes (schedule_id, agent_id, raw_text) VALUES ($1, $2, $3) ON \
         CONFLICT (schedule_id, agent_id) DO UPDATE SET raw_text = EXCLUDED.raw_text",
    )
    .bind(schedule_id)
    .bind(agent_id)
    .bind(raw_text)
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn delete_per_agent_excludes_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM per_agent_excludes WHERE schedule_id = $1")
        .bind(schedule_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn get_per_agent_excludes_raw(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar::<_, String>(
        "SELECT raw_text FROM per_agent_excludes WHERE schedule_id = $1 AND agent_id = $2",
    )
    .bind(schedule_id)
    .bind(agent_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PerAgentCommands {
    pub agent_id: i64,
    pub pre_backup_commands: String,
    pub post_backup_commands: String,
}

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

    let rows = sqlx::query_as::<_, Row>(
        "SELECT agent_id, pre_backup_commands, post_backup_commands FROM per_agent_commands WHERE \
         schedule_id = $1 ORDER BY agent_id",
    )
    .bind(schedule_id)
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

    let row = sqlx::query_as::<_, Row>(
        "SELECT pre_backup_commands, post_backup_commands FROM per_agent_commands WHERE \
         schedule_id = $1 AND agent_id = $2",
    )
    .bind(schedule_id)
    .bind(agent_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(row.map(|r| PerAgentCommands {
        agent_id,
        pre_backup_commands: r.pre_backup_commands,
        post_backup_commands: r.post_backup_commands,
    }))
}

pub async fn upsert_per_agent_commands(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
    pre_backup_commands: &str,
    post_backup_commands: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO per_agent_commands (schedule_id, agent_id, pre_backup_commands, \
         post_backup_commands) VALUES ($1, $2, $3, $4) ON CONFLICT (schedule_id, agent_id) DO \
         UPDATE SET pre_backup_commands = EXCLUDED.pre_backup_commands, post_backup_commands = \
         EXCLUDED.post_backup_commands",
    )
    .bind(schedule_id)
    .bind(agent_id)
    .bind(pre_backup_commands)
    .bind(post_backup_commands)
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn delete_per_agent_commands_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM per_agent_commands WHERE schedule_id = $1")
        .bind(schedule_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn get_schedule_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<Option<ScheduleRow>, ApiError> {
    sqlx::query_as::<_, ScheduleRow>(
        "SELECT id, repo_id, name, schedule_type, cron_expression, enabled, canary_enabled, \
         last_run_at, next_run_at, exclude_patterns_raw, ignore_global_excludes, keep_hourly, \
         keep_daily, keep_weekly, keep_monthly, keep_yearly, compact_enabled, rate_limit_kbps, \
         pre_backup_commands, post_backup_commands, execution_mode, on_failure, owner_id, \
         visibility FROM schedules WHERE repo_id = $1",
    )
    .bind(repo_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

/// Finds the schedule (of the given type) that targets `hostname` and `repo_id`.
/// Used to attribute a completion reported by the agent (which only carries a
/// repo id, not a schedule id) back to the schedule that most likely triggered
/// it. If multiple schedules of the same type target the same host/repo pair,
/// an arbitrary one is returned.
pub async fn get_schedule_for_hostname_repo(
    pool: &PgPool,
    hostname: &str,
    repo_id: i64,
    schedule_type: ScheduleType,
) -> Result<Option<ScheduleRow>, ApiError> {
    sqlx::query_as::<_, ScheduleRow>(
        "SELECT s.id, s.repo_id, s.name, s.schedule_type, s.cron_expression, s.enabled, \
         s.canary_enabled, s.last_run_at, s.next_run_at, s.exclude_patterns_raw, \
         s.ignore_global_excludes, s.keep_hourly, s.keep_daily, s.keep_weekly, s.keep_monthly, \
         s.keep_yearly, s.compact_enabled, s.rate_limit_kbps, s.pre_backup_commands, \
         s.post_backup_commands, s.execution_mode, s.on_failure, s.owner_id, s.visibility FROM \
         schedules s JOIN schedule_targets st ON st.schedule_id = s.id JOIN agents m ON \
         st.agent_id = m.id WHERE m.hostname = $1 AND s.repo_id = $2 AND s.schedule_type = $3 \
         LIMIT 1",
    )
    .bind(hostname)
    .bind(repo_id)
    .bind(schedule_type.to_string())
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn list_schedules_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<Vec<ScheduleRow>, ApiError> {
    sqlx::query_as::<_, ScheduleRow>(
        "SELECT s.id, s.repo_id, s.name, s.schedule_type, s.cron_expression, s.enabled, \
         s.canary_enabled, s.last_run_at, s.next_run_at, s.exclude_patterns_raw, \
         s.ignore_global_excludes, s.keep_hourly, s.keep_daily, s.keep_weekly, s.keep_monthly, \
         s.keep_yearly, s.compact_enabled, s.rate_limit_kbps, s.pre_backup_commands, \
         s.post_backup_commands, s.execution_mode, s.on_failure, s.owner_id, s.visibility, \
         ARRAY(SELECT a.hostname FROM schedule_targets st JOIN agents a ON a.id = st.agent_id \
         WHERE st.schedule_id = s.id ORDER BY st.execution_order, a.hostname) AS target_hostnames \
         FROM schedules s WHERE s.repo_id = $1 ORDER BY s.id",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn delete_schedule(pool: &PgPool, id: i64) -> Result<(), ApiError> {
    let result = sqlx::query("DELETE FROM schedules WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("schedule {id} not found")));
    }
    Ok(())
}

pub async fn list_schedules_for_agent(
    pool: &PgPool,
    agent_id: i64,
) -> Result<Vec<ScheduleRow>, ApiError> {
    sqlx::query_as::<_, ScheduleRow>(
        "SELECT s.id, s.repo_id, s.name, s.schedule_type, s.cron_expression, s.enabled, \
         s.canary_enabled, s.last_run_at, s.next_run_at, s.exclude_patterns_raw, \
         s.ignore_global_excludes, s.keep_hourly, s.keep_daily, s.keep_weekly, s.keep_monthly, \
         s.keep_yearly, s.compact_enabled, s.rate_limit_kbps, s.pre_backup_commands, \
         s.post_backup_commands, s.execution_mode, s.on_failure, s.owner_id, s.visibility FROM \
         schedules s JOIN schedule_targets st ON st.schedule_id = s.id WHERE st.agent_id = $1 \
         ORDER by s.id",
    )
    .bind(agent_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DueScheduleRow {
    pub schedule_id: i64,
    pub repo_id: i64,
    pub agent_id: i64,
    pub hostname: String,
    pub schedule_type: String,
    pub cron_expression: String,
    pub on_failure: String,
    pub execution_order: i32,
}

pub async fn list_due_schedules(
    pool: &PgPool,
    now: DateTime<Utc>,
) -> Result<Vec<DueScheduleRow>, ApiError> {
    sqlx::query_as::<_, DueScheduleRow>(
        "SELECT s.id AS schedule_id, s.repo_id, st.agent_id, a.hostname, s.schedule_type, \
         s.cron_expression, s.on_failure, st.execution_order FROM schedules s JOIN repos r ON \
         r.id = s.repo_id JOIN schedule_targets st ON st.schedule_id = s.id JOIN agents a ON a.id \
         = st.agent_id WHERE s.enabled = true AND r.enabled = true AND a.is_hidden = false AND \
         s.next_run_at IS NOT NULL AND s.next_run_at <= $1 ORDER BY s.id, st.execution_order",
    )
    .bind(now)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn mark_schedule_triggered(
    pool: &PgPool,
    schedule_id: i64,
    now: DateTime<Utc>,
    next_run_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE schedules SET last_run_at = $2, next_run_at = $3 WHERE id = $1")
        .bind(schedule_id)
        .bind(now)
        .bind(next_run_at)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn set_next_run_at(
    pool: &PgPool,
    schedule_id: i64,
    next_run_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE schedules SET next_run_at = $2 WHERE id = $1")
        .bind(schedule_id)
        .bind(next_run_at)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn get_schedule_by_id(pool: &PgPool, id: i64) -> Result<ScheduleRow, ApiError> {
    sqlx::query_as::<_, ScheduleRow>(
        "SELECT id, repo_id, name, schedule_type, cron_expression, enabled, canary_enabled, \
         last_run_at, next_run_at, exclude_patterns_raw, ignore_global_excludes, keep_hourly, \
         keep_daily, keep_weekly, keep_monthly, keep_yearly, compact_enabled, rate_limit_kbps, \
         pre_backup_commands, post_backup_commands, execution_mode, on_failure, owner_id, \
         visibility FROM schedules WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("schedule {id} not found")),
        other => ApiError::Database(other),
    })
}

pub async fn get_schedule_target_hostnames(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<String>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        hostname: String,
    }

    let rows = sqlx::query_as::<_, Row>(
        "SELECT a.hostname FROM agents a JOIN schedule_targets st ON st.agent_id = a.id WHERE \
         st.schedule_id = $1 AND a.is_hidden = false ORDER BY st.execution_order",
    )
    .bind(schedule_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows.into_iter().map(|r| r.hostname).collect())
}

#[derive(Debug, sqlx::FromRow)]
pub struct ScheduleRunTarget {
    pub agent_id: i64,
    pub hostname: String,
}

pub async fn get_schedule_targets_for_run(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<ScheduleRunTarget>, ApiError> {
    sqlx::query_as::<_, ScheduleRunTarget>(
        "SELECT a.id AS agent_id, a.hostname FROM agents a JOIN schedule_targets st ON \
         st.agent_id = a.id WHERE st.schedule_id = $1 AND a.is_hidden = false ORDER BY \
         st.execution_order",
    )
    .bind(schedule_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn insert_schedule_targets(
    pool: &PgPool,
    schedule_id: i64,
    targets: &[(i64, i32)],
) -> Result<(), ApiError> {
    for (agent_id, execution_order) in targets {
        sqlx::query(
            "INSERT INTO schedule_targets (schedule_id, agent_id, execution_order) VALUES ($1, \
             $2, $3)",
        )
        .bind(schedule_id)
        .bind(*agent_id)
        .bind(*execution_order)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    }
    Ok(())
}

pub async fn delete_schedule_targets(pool: &PgPool, schedule_id: i64) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM schedule_targets WHERE schedule_id = $1")
        .bind(schedule_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn list_schedule_targets(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<ScheduleTargetRow>, ApiError> {
    sqlx::query_as::<_, ScheduleTargetRow>(
        "SELECT agent_id, execution_order FROM schedule_targets WHERE schedule_id = $1 ORDER BY \
         execution_order",
    )
    .bind(schedule_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn get_repo_name(pool: &PgPool, repo_id: i64) -> Result<String, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        name: String,
    }

    let row = sqlx::query_as::<_, Row>("SELECT name FROM repos WHERE id = $1")
        .bind(repo_id)
        .fetch_one(pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
            other => ApiError::Database(other),
        })?;

    Ok(row.name)
}

/// Resolves a schedule's display name, falling back to `default_name` (typically
/// the repo name) when the schedule has no custom name set, mirroring the
/// `COALESCE(NULLIF(s.name, ''), r.name)` convention used elsewhere.
pub async fn get_schedule_display_name(
    pool: &PgPool,
    schedule_id: i64,
    default_name: &str,
) -> Result<String, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        name: String,
    }

    let row = sqlx::query_as::<_, Row>("SELECT name FROM schedules WHERE id = $1")
        .bind(schedule_id)
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

pub async fn insert_canary_result(
    pool: &PgPool,
    schedule_id: i64,
    success: bool,
    canary_filename: &str,
    error_message: Option<&str>,
    archive_name: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO canary_results (schedule_id, success, canary_filename, error_message, \
         archive_name) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(schedule_id)
    .bind(success)
    .bind(canary_filename)
    .bind(error_message)
    .bind(archive_name)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct CanaryResultRow {
    pub id: i64,
    pub schedule_id: Option<i64>,
    pub verified_at: DateTime<Utc>,
    pub success: bool,
    pub canary_filename: Option<String>,
    pub error_message: Option<String>,
    pub archive_name: Option<String>,
}

pub async fn get_latest_canary_result(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Option<CanaryResultRow>, ApiError> {
    let row = sqlx::query_as::<_, CanaryResultRow>(
        "SELECT id, schedule_id, verified_at, success, canary_filename, error_message, \
         archive_name FROM canary_results WHERE schedule_id = $1 ORDER BY verified_at DESC LIMIT 1",
    )
    .bind(schedule_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn list_canary_results(
    pool: &PgPool,
    schedule_id: i64,
    limit: i64,
) -> Result<Vec<CanaryResultRow>, ApiError> {
    let rows = sqlx::query_as::<_, CanaryResultRow>(
        "SELECT id, schedule_id, verified_at, success, canary_filename, error_message, \
         archive_name FROM canary_results WHERE schedule_id = $1 ORDER BY verified_at DESC LIMIT \
         $2",
    )
    .bind(schedule_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ReportRow {
    pub id: i64,
    pub agent_id: i64,
    pub repo_id: i64,
    pub repo_name: String,
    pub schedule_id: Option<i64>,
    pub schedule_name: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub status: String,
    pub original_size: i64,
    pub compressed_size: i64,
    pub deduplicated_size: i64,
    pub files_processed: i64,
    pub duration_secs: i64,
    pub error_message: Option<String>,
    pub warnings: Vec<String>,
    pub borg_version: Option<String>,
    pub archive_name: Option<String>,
    pub borg_command: Option<String>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct StorageStatRow {
    pub hostname: String,
    pub target_name: String,
    pub total_original_size: i64,
    pub total_compressed_size: i64,
    pub total_deduplicated_size: i64,
    pub report_count: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ActivityRow {
    pub id: i64,
    pub hostname: String,
    pub target_name: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub status: String,
    pub duration_secs: i64,
    pub repo_id: Option<i64>,
    pub archive_name: Option<String>,
    pub error_message: Option<String>,
    #[serde(default)]
    pub schedule_id: Option<i64>,
    #[serde(default)]
    pub schedule_name: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct HealthRow {
    pub repo_id: i64,
    pub schedule_id: i64,
    pub hostname: String,
    pub target_name: String,
    pub last_status: Option<String>,
    pub last_backup_at: Option<DateTime<Utc>>,
    pub last_error_message: Option<String>,
    pub cron_expression: Option<String>,
    pub schedule_enabled: Option<bool>,
}

#[derive(Clone)]
pub struct InsertReportParams {
    pub agent_id: i64,
    pub repo_id: i64,
    pub schedule_id: Option<i64>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub status: String,
    pub original_size: i64,
    pub compressed_size: i64,
    pub deduplicated_size: i64,
    pub repo_unique_csize: i64,
    pub files_processed: i64,
    pub duration_secs: i64,
    pub error_message: Option<String>,
    pub warnings: Vec<String>,
    pub borg_version: Option<String>,
    pub matched: bool,
    pub archive_name: Option<String>,
    pub borg_command: Option<String>,
    pub run_id: Option<String>,
}

pub async fn insert_backup_pending(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
    schedule_id: Option<i64>,
    run_id: &str,
    triggered_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
         status, run_id) VALUES ($1, $2, $3, $4, $4, 'pending', $5) ON CONFLICT (repo_id, \
         agent_id, started_at) WHERE archive_name IS NULL DO NOTHING",
    )
    .bind(agent_id)
    .bind(repo_id)
    .bind(schedule_id)
    .bind(triggered_at)
    .bind(run_id)
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

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
        sqlx::query(
            "UPDATE backup_reports SET started_at = $1, status = 'started', borg_command = $2 \
             WHERE run_id = $3 AND agent_id = $4 AND status = 'pending'",
        )
        .bind(started_at)
        .bind(borg_command)
        .bind(rid)
        .bind(agent_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    } else {
        sqlx::query(
            "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
             status, borg_command) VALUES ($1, $2, $3, $4, $4, 'started', $5) ON CONFLICT \
             (repo_id, agent_id, started_at) WHERE archive_name IS NULL DO NOTHING",
        )
        .bind(agent_id)
        .bind(repo_id)
        .bind(schedule_id)
        .bind(started_at)
        .bind(borg_command)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    }
    Ok(())
}

pub async fn cancel_backup_report(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE backup_reports SET status = 'cancelled', finished_at = NOW() WHERE agent_id = $1 \
         AND repo_id = $2 AND status = 'started'",
    )
    .bind(agent_id)
    .bind(repo_id)
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn cancel_all_active_backups(pool: &PgPool) -> Result<u64, ApiError> {
    let result = sqlx::query(
        "UPDATE backup_reports SET status = 'cancelled', finished_at = NOW() WHERE status IN \
         ('pending', 'started')",
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

pub async fn insert_backup_report(
    pool: &PgPool,
    params: &InsertReportParams,
) -> Result<(), ApiError> {
    if let Some(ref run_id) = params.run_id {
        sqlx::query(
            "UPDATE backup_reports SET schedule_id = COALESCE($1, schedule_id), finished_at = $2, \
             status = $3, original_size = $4, compressed_size = $5, deduplicated_size = $6, \
             repo_unique_csize = $7, files_processed = $8, duration_secs = $9, error_message = \
             $10, warnings = $11, borg_version = $12, matched = $13, archive_name = $14, \
             borg_command = $15, started_at = $16 WHERE run_id = $17 AND agent_id = $18 AND \
             status IN ('pending', 'started')",
        )
        .bind(params.schedule_id)
        .bind(params.finished_at)
        .bind(&params.status)
        .bind(params.original_size)
        .bind(params.compressed_size)
        .bind(params.deduplicated_size)
        .bind(params.repo_unique_csize)
        .bind(params.files_processed)
        .bind(params.duration_secs)
        .bind(&params.error_message)
        .bind(&params.warnings)
        .bind(&params.borg_version)
        .bind(params.matched)
        .bind(&params.archive_name)
        .bind(&params.borg_command)
        .bind(params.started_at)
        .bind(run_id)
        .bind(params.agent_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    } else {
        // Reports carrying an archive name are deduplicated with the name included
        // so distinct same-second archives never collide; reports without one
        // (e.g. failures) fall back to the bare per-run triple.
        let conflict_target = if params.archive_name.is_some() {
            "(repo_id, agent_id, started_at, archive_name) WHERE archive_name IS NOT NULL"
        } else {
            "(repo_id, agent_id, started_at) WHERE archive_name IS NULL"
        };
        let sql = format!(
            "INSERT INTO backup_reports (agent_id, repo_id, schedule_id, started_at, finished_at, \
             status, original_size, compressed_size, deduplicated_size, repo_unique_csize, \
             files_processed, duration_secs, error_message, warnings, borg_version, matched, \
             archive_name, borg_command) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, \
             $12, $13, $14, $15, $16, $17, $18) ON CONFLICT {conflict_target} DO UPDATE SET \
             schedule_id = COALESCE(EXCLUDED.schedule_id, backup_reports.schedule_id), \
             finished_at = EXCLUDED.finished_at, status = EXCLUDED.status, original_size = \
             EXCLUDED.original_size, compressed_size = EXCLUDED.compressed_size, \
             deduplicated_size = EXCLUDED.deduplicated_size, repo_unique_csize = \
             EXCLUDED.repo_unique_csize, files_processed = EXCLUDED.files_processed, \
             duration_secs = EXCLUDED.duration_secs, error_message = EXCLUDED.error_message, \
             warnings = EXCLUDED.warnings, borg_version = EXCLUDED.borg_version, matched = \
             EXCLUDED.matched, archive_name = EXCLUDED.archive_name, borg_command = \
             EXCLUDED.borg_command"
        );
        sqlx::query(&sql)
            .bind(params.agent_id)
            .bind(params.repo_id)
            .bind(params.schedule_id)
            .bind(params.started_at)
            .bind(params.finished_at)
            .bind(&params.status)
            .bind(params.original_size)
            .bind(params.compressed_size)
            .bind(params.deduplicated_size)
            .bind(params.repo_unique_csize)
            .bind(params.files_processed)
            .bind(params.duration_secs)
            .bind(&params.error_message)
            .bind(&params.warnings)
            .bind(&params.borg_version)
            .bind(params.matched)
            .bind(&params.archive_name)
            .bind(&params.borg_command)
            .execute(pool)
            .await
            .map_err(ApiError::Database)?;
    }
    Ok(())
}

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

    let result = sqlx::query(
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
    )
    .bind(&agent_ids)
    .bind(&repo_ids)
    .bind(&started_ats)
    .bind(&finished_ats)
    .bind(&statuses)
    .bind(&original_sizes)
    .bind(&compressed_sizes)
    .bind(&deduplicated_sizes)
    .bind(&repo_unique_csizes)
    .bind(&files_processed_v)
    .bind(&duration_secs_v)
    .bind(&error_messages)
    .bind(&borg_versions)
    .bind(&matcheds)
    .bind(&archive_names)
    .bind(&borg_commands)
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(result.rows_affected())
}

pub struct ArchiveStats {
    pub original_size: i64,
    pub compressed_size: i64,
    pub deduplicated_size: i64,
    pub files_processed: i64,
    pub duration_secs: i64,
    pub repo_unique_csize: i64,
}

pub async fn update_backup_report_stats(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
    stats: &ArchiveStats,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE backup_reports SET original_size = $3, compressed_size = $4, deduplicated_size = \
         $5, files_processed = $6, duration_secs = $7 WHERE repo_id = $1 AND archive_name = $2 \
         AND original_size = 0 AND compressed_size = 0 AND deduplicated_size = 0",
    )
    .bind(repo_id)
    .bind(archive_name)
    .bind(stats.original_size)
    .bind(stats.compressed_size)
    .bind(stats.deduplicated_size)
    .bind(stats.files_processed)
    .bind(stats.duration_secs)
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;

    if stats.repo_unique_csize > 0 {
        sqlx::query(
            "UPDATE backup_reports SET repo_unique_csize = $3 WHERE repo_id = $1 AND archive_name \
             = $2 AND repo_unique_csize = 0",
        )
        .bind(repo_id)
        .bind(archive_name)
        .bind(stats.repo_unique_csize)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    }

    Ok(())
}

pub async fn list_reports_for_agent(
    pool: &PgPool,
    agent_id: i64,
    target: Option<&str>,
    limit: i64,
) -> Result<Vec<ReportRow>, ApiError> {
    if let Some(target_name) = target {
        sqlx::query_as::<_, ReportRow>(
            "SELECT br.id, br.agent_id, br.repo_id, r.name AS repo_name, br.schedule_id, CASE \
             WHEN s.id IS NOT NULL THEN COALESCE(NULLIF(s.name, ''), r.name) END AS \
             schedule_name, br.started_at, br.finished_at, br.status, br.original_size, \
             br.compressed_size, br.deduplicated_size, br.files_processed, br.duration_secs, \
             br.error_message, br.warnings, br.borg_version, br.archive_name, br.borg_command \
             FROM backup_reports br JOIN repos r ON r.id = br.repo_id LEFT JOIN schedules s ON \
             s.id = br.schedule_id WHERE br.agent_id = $1 AND r.name = $2 ORDER by br.started_at \
             DESC LIMIT $3",
        )
        .bind(agent_id)
        .bind(target_name)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    } else {
        sqlx::query_as::<_, ReportRow>(
            "SELECT br.id, br.agent_id, br.repo_id, r.name AS repo_name, br.schedule_id, CASE \
             WHEN s.id IS NOT NULL THEN COALESCE(NULLIF(s.name, ''), r.name) END AS \
             schedule_name, br.started_at, br.finished_at, br.status, br.original_size, \
             br.compressed_size, br.deduplicated_size, br.files_processed, br.duration_secs, \
             br.error_message, br.warnings, br.borg_version, br.archive_name, br.borg_command \
             FROM backup_reports br JOIN repos r ON r.id = br.repo_id LEFT JOIN schedules s ON \
             s.id = br.schedule_id WHERE br.agent_id = $1 ORDER BY br.started_at DESC LIMIT $2",
        )
        .bind(agent_id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    }
}

pub async fn list_reports_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
    limit: i64,
) -> Result<Vec<ReportRow>, ApiError> {
    sqlx::query_as::<_, ReportRow>(
        "SELECT br.id, br.agent_id, br.repo_id, r.name AS repo_name, br.schedule_id, CASE WHEN \
         s.id IS NOT NULL THEN COALESCE(NULLIF(s.name, ''), r.name) END AS schedule_name, \
         br.started_at, br.finished_at, br.status, br.original_size, br.compressed_size, \
         br.deduplicated_size, br.files_processed, br.duration_secs, br.error_message, \
         br.warnings, br.borg_version, br.archive_name, br.borg_command FROM backup_reports br \
         JOIN repos r ON r.id = br.repo_id LEFT JOIN schedules s ON s.id = br.schedule_id WHERE \
         br.schedule_id = $1 ORDER BY br.started_at DESC LIMIT $2",
    )
    .bind(schedule_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn get_storage_stats(pool: &PgPool) -> Result<Vec<StorageStatRow>, ApiError> {
    sqlx::query_as::<_, StorageStatRow>(
        "SELECT a.hostname, r.name AS target_name, COALESCE(SUM(br.original_size), 0)::INT8 AS \
         total_original_size, COALESCE(SUM(br.compressed_size), 0)::INT8 AS \
         total_compressed_size, COALESCE(SUM(br.deduplicated_size), 0)::INT8 AS \
         total_deduplicated_size, COUNT(br.id) AS report_count FROM backup_reports br JOIN agents \
         a ON a.id = br.agent_id JOIN repos r ON r.id = br.repo_id WHERE a.is_hidden = false \
         GROUP BY a.hostname, r.name ORDER BY a.hostname, r.name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn get_activity_feed(
    pool: &PgPool,
    limit: i64,
    repo_id: Option<i64>,
    hostname: Option<&str>,
    schedule_id: Option<i64>,
    run_id: Option<&str>,
) -> Result<Vec<ActivityRow>, ApiError> {
    let mut sql = String::from(
        "SELECT br.id, a.hostname, r.name AS target_name, br.started_at, br.finished_at, \
         br.status, br.duration_secs, br.repo_id, br.archive_name, br.error_message, \
         br.schedule_id, s.name AS schedule_name, br.run_id FROM backup_reports br JOIN agents a \
         ON a.id = br.agent_id JOIN repos r ON r.id = br.repo_id LEFT JOIN schedules s ON s.id = \
         br.schedule_id WHERE a.is_hidden = false AND a.visibility <> 'hidden' AND \
         COALESCE(a.display_name, '') NOT ILIKE '%(imported)%'",
    );
    let mut param_idx = 1u32;
    if repo_id.is_some() {
        sql.push_str(&format!(" AND br.repo_id = ${param_idx}"));
        param_idx += 1;
    }
    if hostname.is_some() {
        sql.push_str(&format!(" AND a.hostname = ${param_idx}"));
        param_idx += 1;
    }
    if schedule_id.is_some() {
        sql.push_str(&format!(" AND br.schedule_id = ${param_idx}"));
        param_idx += 1;
    }
    if run_id.is_some() {
        sql.push_str(&format!(" AND br.run_id = ${param_idx}"));
        param_idx += 1;
    }
    sql.push_str(&format!(" ORDER BY br.started_at DESC LIMIT ${param_idx}"));

    let mut query = sqlx::query_as::<_, ActivityRow>(&sql);
    if let Some(rid) = repo_id {
        query = query.bind(rid);
    }
    if let Some(host) = hostname {
        query = query.bind(host.to_owned());
    }
    if let Some(sid) = schedule_id {
        query = query.bind(sid);
    }
    if let Some(rid) = run_id {
        query = query.bind(rid.to_owned());
    }
    query = query.bind(limit);
    query.fetch_all(pool).await.map_err(ApiError::Database)
}

pub async fn get_health_summary(pool: &PgPool) -> Result<Vec<HealthRow>, ApiError> {
    sqlx::query_as::<_, HealthRow>(
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

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub must_change_password: bool,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SessionRow {
    pub id: String,
    pub user_id: i64,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

pub async fn insert_user(
    pool: &PgPool,
    username: &str,
    password_hash: &str,
    role: &str,
) -> Result<UserRow, ApiError> {
    sqlx::query_as::<_, UserRow>(
        "INSERT INTO users (username, password_hash, role) VALUES ($1, $2, $3) RETURNING id, \
         username, role, must_change_password, created_at, last_login_at",
    )
    .bind(username)
    .bind(password_hash)
    .bind(role)
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn get_user_by_username(pool: &PgPool, username: &str) -> Result<UserRow, ApiError> {
    sqlx::query_as::<_, UserRow>(
        "SELECT id, username, role, must_change_password, created_at, last_login_at FROM users \
         WHERE username = $1",
    )
    .bind(username)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("user '{username}' not found")),
        other => ApiError::Database(other),
    })
}

pub async fn get_user_password_hash(
    pool: &PgPool,
    username: &str,
) -> Result<(UserRow, String), ApiError> {
    #[derive(sqlx::FromRow)]
    struct FullRow {
        id: i64,
        username: String,
        password_hash: String,
        role: String,
        must_change_password: bool,
        created_at: DateTime<Utc>,
        last_login_at: Option<DateTime<Utc>>,
    }

    let row = sqlx::query_as::<_, FullRow>(
        "SELECT id, username, password_hash, role, must_change_password, created_at, \
         last_login_at FROM users WHERE username = $1",
    )
    .bind(username)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("user '{username}' not found")),
        other => ApiError::Database(other),
    })?;

    let user = UserRow {
        id: row.id,
        username: row.username,
        role: row.role,
        must_change_password: row.must_change_password,
        created_at: row.created_at,
        last_login_at: row.last_login_at,
    };
    Ok((user, row.password_hash))
}

pub async fn get_user_by_id(pool: &PgPool, user_id: i64) -> Result<UserRow, ApiError> {
    sqlx::query_as::<_, UserRow>(
        "SELECT id, username, role, must_change_password, created_at, last_login_at FROM users \
         WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("user {user_id} not found")),
        other => ApiError::Database(other),
    })
}

pub async fn list_users(pool: &PgPool) -> Result<Vec<UserRow>, ApiError> {
    sqlx::query_as::<_, UserRow>(
        "SELECT id, username, role, must_change_password, created_at, last_login_at FROM users \
         ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn update_user_role(
    pool: &PgPool,
    user_id: i64,
    role: &str,
) -> Result<UserRow, ApiError> {
    sqlx::query_as::<_, UserRow>(
        "UPDATE users SET role = $2 WHERE id = $1 RETURNING id, username, role, \
         must_change_password, created_at, last_login_at",
    )
    .bind(user_id)
    .bind(role)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("user {user_id} not found")),
        other => ApiError::Database(other),
    })
}

pub async fn delete_user(pool: &PgPool, user_id: i64) -> Result<(), ApiError> {
    let result = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("user {user_id} not found")));
    }
    Ok(())
}

pub async fn update_user_password(
    pool: &PgPool,
    user_id: i64,
    password_hash: &str,
) -> Result<(), ApiError> {
    let result = sqlx::query(
        "UPDATE users SET password_hash = $2, must_change_password = false WHERE id = $1",
    )
    .bind(user_id)
    .bind(password_hash)
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("user {user_id} not found")));
    }
    Ok(())
}

pub async fn update_last_login(pool: &PgPool, user_id: i64) -> Result<(), ApiError> {
    sqlx::query("UPDATE users SET last_login_at = NOW() WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn insert_session(
    pool: &PgPool,
    session_id: &str,
    user_id: i64,
    expires_at: DateTime<Utc>,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO sessions (id, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(session_id)
        .bind(user_id)
        .bind(expires_at)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn get_session(pool: &PgPool, session_id: &str) -> Result<SessionRow, ApiError> {
    sqlx::query_as::<_, SessionRow>(
        "SELECT id, user_id, created_at, expires_at FROM sessions WHERE id = $1 AND expires_at > \
         NOW()",
    )
    .bind(session_id)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => {
            ApiError::Unauthorized("session expired or invalid".to_string())
        }
        other => ApiError::Database(other),
    })
}

pub async fn delete_session(pool: &PgPool, session_id: &str) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM sessions WHERE id = $1")
        .bind(session_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn delete_expired_sessions(pool: &PgPool) -> Result<u64, ApiError> {
    let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= NOW()")
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

pub async fn user_count(pool: &PgPool) -> Result<i64, ApiError> {
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: i64,
    }

    let row = sqlx::query_as::<_, CountRow>("SELECT COUNT(*) as count FROM users")
        .fetch_one(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(row.count)
}

pub async fn count_failed_login_attempts(
    pool: &PgPool,
    username: &str,
    ip: &str,
    window_minutes: i32,
) -> Result<i64, ApiError> {
    #[derive(sqlx::FromRow)]
    struct CountRow {
        count: i64,
    }

    let row = sqlx::query_as::<_, CountRow>(
        "SELECT COUNT(*) as count FROM login_attempts WHERE username = $1 AND ip = $2 AND success \
         = false AND attempted_at > NOW() - ($3 || ' minutes')::INTERVAL",
    )
    .bind(username)
    .bind(ip)
    .bind(window_minutes.to_string())
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(row.count)
}

pub async fn insert_login_attempt(
    pool: &PgPool,
    username: &str,
    ip: &str,
    success: bool,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO login_attempts (username, ip, success) VALUES ($1, $2, $3)")
        .bind(username)
        .bind(ip)
        .bind(success)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ApiTokenRow {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

pub async fn insert_api_token(
    pool: &PgPool,
    user_id: i64,
    name: &str,
    token_hash: &str,
) -> Result<ApiTokenRow, ApiError> {
    sqlx::query_as::<_, ApiTokenRow>(
        "INSERT INTO api_tokens (user_id, name, token_hash) VALUES ($1, $2, $3) RETURNING id, \
         user_id, name, created_at, last_used_at",
    )
    .bind(user_id)
    .bind(name)
    .bind(token_hash)
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn list_api_tokens_for_user(
    pool: &PgPool,
    user_id: i64,
) -> Result<Vec<ApiTokenRow>, ApiError> {
    sqlx::query_as::<_, ApiTokenRow>(
        "SELECT id, user_id, name, created_at, last_used_at FROM api_tokens WHERE user_id = $1 \
         ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn list_all_api_tokens(pool: &PgPool) -> Result<Vec<ApiTokenRow>, ApiError> {
    sqlx::query_as::<_, ApiTokenRow>(
        "SELECT id, user_id, name, created_at, last_used_at FROM api_tokens ORDER BY created_at \
         DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn delete_api_token(pool: &PgPool, token_id: i64) -> Result<(), ApiError> {
    let result = sqlx::query("DELETE FROM api_tokens WHERE id = $1")
        .bind(token_id)
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

pub async fn get_api_token_owner(pool: &PgPool, token_id: i64) -> Result<i64, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        user_id: i64,
    }

    let row = sqlx::query_as::<_, Row>("SELECT user_id FROM api_tokens WHERE id = $1")
        .bind(token_id)
        .fetch_one(pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => {
                ApiError::NotFound(format!("api token {token_id} not found"))
            }
            other => ApiError::Database(other),
        })?;
    Ok(row.user_id)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ApiTokenLookupRow {
    pub user_id: i64,
}

pub async fn get_user_by_token_hash(
    pool: &PgPool,
    token_hash: &str,
) -> Result<ApiTokenLookupRow, ApiError> {
    let row = sqlx::query_as::<_, ApiTokenLookupRow>(
        "SELECT user_id FROM api_tokens WHERE token_hash = $1",
    )
    .bind(token_hash)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::Unauthorized("invalid api token".to_string()),
        other => ApiError::Database(other),
    })?;
    Ok(row)
}

pub async fn update_api_token_last_used(pool: &PgPool, token_hash: &str) -> Result<(), ApiError> {
    sqlx::query("UPDATE api_tokens SET last_used_at = NOW() WHERE token_hash = $1")
        .bind(token_hash)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct RepoPermissionRow {
    pub user_id: i64,
    pub repo_id: i64,
    pub can_view: bool,
    pub can_backup: bool,
    pub can_modify_schedules: bool,
    pub can_extract: bool,
    pub can_delete: bool,
}

pub struct UpsertRepoPermissionParams {
    pub user_id: i64,
    pub repo_id: i64,
    pub can_view: bool,
    pub can_backup: bool,
    pub can_modify_schedules: bool,
    pub can_extract: bool,
    pub can_delete: bool,
}

pub async fn upsert_repo_permission(
    pool: &PgPool,
    params: &UpsertRepoPermissionParams,
) -> Result<RepoPermissionRow, ApiError> {
    sqlx::query_as::<_, RepoPermissionRow>(
        "INSERT INTO repo_permissions (user_id, repo_id, can_view, can_backup, \
         can_modify_schedules, can_extract, can_delete) VALUES ($1, $2, $3, $4, $5, $6, $7) ON \
         CONFLICT (user_id, repo_id) DO UPDATE SET can_view = $3, can_backup = $4, \
         can_modify_schedules = $5, can_extract = $6, can_delete = $7 RETURNING user_id, repo_id, \
         can_view, can_backup, can_modify_schedules, can_extract, can_delete",
    )
    .bind(params.user_id)
    .bind(params.repo_id)
    .bind(params.can_view)
    .bind(params.can_backup)
    .bind(params.can_modify_schedules)
    .bind(params.can_extract)
    .bind(params.can_delete)
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn get_repo_permission(
    pool: &PgPool,
    user_id: i64,
    repo_id: i64,
) -> Result<Option<RepoPermissionRow>, ApiError> {
    sqlx::query_as::<_, RepoPermissionRow>(
        "SELECT user_id, repo_id, can_view, can_backup, can_modify_schedules, can_extract, \
         can_delete FROM repo_permissions WHERE user_id = $1 AND repo_id = $2",
    )
    .bind(user_id)
    .bind(repo_id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn list_repo_permissions_for_user(
    pool: &PgPool,
    user_id: i64,
) -> Result<Vec<RepoPermissionRow>, ApiError> {
    sqlx::query_as::<_, RepoPermissionRow>(
        "SELECT user_id, repo_id, can_view, can_backup, can_modify_schedules, can_extract, \
         can_delete FROM repo_permissions WHERE user_id = $1 ORDER BY repo_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn list_repo_permissions_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<Vec<RepoPermissionRow>, ApiError> {
    sqlx::query_as::<_, RepoPermissionRow>(
        "SELECT user_id, repo_id, can_view, can_backup, can_modify_schedules, can_extract, \
         can_delete FROM repo_permissions WHERE repo_id = $1 ORDER BY user_id",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct SystemEventRow {
    pub id: i64,
    pub created_at: DateTime<Utc>,
    pub event_type: String,
    pub hostname: Option<String>,
    pub message: String,
}

pub async fn insert_system_event(
    pool: &PgPool,
    event_type: &str,
    hostname: Option<&str>,
    message: &str,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO system_events (event_type, hostname, message) VALUES ($1, $2, $3)")
        .bind(event_type)
        .bind(hostname)
        .bind(message)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

pub async fn get_system_events(pool: &PgPool, limit: i64) -> Result<Vec<SystemEventRow>, ApiError> {
    sqlx::query_as::<_, SystemEventRow>(
        "SELECT id, created_at, event_type, hostname, message FROM system_events ORDER BY \
         created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn get_setting(pool: &PgPool, key: &str) -> Result<Option<String>, ApiError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM system_settings WHERE key = $1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(row.map(|r| r.0))
}

pub async fn set_setting(pool: &PgPool, key: &str, value: &str) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO system_settings (key, value, updated_at) VALUES ($1, $2, NOW()) ON CONFLICT \
         (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct DatabaseRelationSizeRow {
    pub table_name: String,
    pub table_bytes: i64,
    pub index_bytes: i64,
    pub toast_bytes: i64,
    pub total_bytes: i64,
}

pub async fn get_database_storage(
    pool: &PgPool,
) -> Result<(i64, Vec<DatabaseRelationSizeRow>), ApiError> {
    let total_bytes =
        sqlx::query_scalar::<_, i64>("SELECT pg_database_size(current_database())::BIGINT")
            .fetch_one(pool)
            .await
            .map_err(ApiError::Database)?;

    let relations = sqlx::query_as::<_, DatabaseRelationSizeRow>(
        "SELECT relname::TEXT AS table_name, pg_relation_size(relid)::BIGINT AS table_bytes, \
         pg_indexes_size(relid)::BIGINT AS index_bytes, (pg_total_relation_size(relid) - \
         pg_relation_size(relid) - pg_indexes_size(relid))::BIGINT AS toast_bytes, \
         pg_total_relation_size(relid)::BIGINT AS total_bytes FROM \
         pg_catalog.pg_statio_user_tables ORDER BY total_bytes DESC, table_name ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok((total_bytes, relations))
}

pub async fn get_schedule_timezone(pool: &PgPool) -> Result<chrono_tz::Tz, ApiError> {
    let tz_str = get_setting(pool, "timezone").await?.unwrap_or_default();
    shared::schedule::parse_timezone(&tz_str)
        .map_err(|e| ApiError::Internal(format!("invalid timezone setting: {e}")))
}

pub async fn delete_system_events_before(
    pool: &PgPool,
    before: DateTime<Utc>,
) -> Result<u64, ApiError> {
    let result = sqlx::query("DELETE FROM system_events WHERE created_at < $1")
        .bind(before)
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
pub async fn delete_backup_reports_before(
    pool: &PgPool,
    before: DateTime<Utc>,
) -> Result<u64, ApiError> {
    let result =
        sqlx::query("DELETE FROM backup_reports WHERE started_at < $1 AND archive_name IS NULL")
            .bind(before)
            .execute(pool)
            .await
            .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

pub async fn get_user_preferences(
    pool: &PgPool,
    user_id: i64,
) -> Result<serde_json::Value, ApiError> {
    let row: (serde_json::Value,) = sqlx::query_as("SELECT preferences FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(row.0)
}

pub async fn set_user_preferences(
    pool: &PgPool,
    user_id: i64,
    preferences: &serde_json::Value,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE users SET preferences = $1 WHERE id = $2")
        .bind(preferences)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct RepoWithStatsRow {
    pub id: i64,
    pub name: String,
    pub repo_path: String,
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: i32,
    pub ssh_host_key: Option<String>,
    pub compression: String,
    pub encryption: String,
    pub enabled: bool,
    pub importing: bool,
    pub import_error: Option<String>,
    pub import_progress: i32,
    pub import_total: i32,
    pub import_status_message: Option<String>,
    pub owner_id: Option<i64>,
    pub visibility: String,
    pub sync_schedule: Option<String>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub archive_count: i64,
    pub last_backup_at: Option<DateTime<Utc>>,
    pub total_original_size: i64,
    pub total_compressed_size: i64,
    pub total_deduplicated_size: i64,
    pub agent_count: i64,
    pub unmatched_count: i64,
    pub last_op_kind: Option<String>,
    pub last_op_at: Option<DateTime<Utc>>,
    pub last_op_by: Option<String>,
}

pub async fn list_repos_with_stats(pool: &PgPool) -> Result<Vec<RepoWithStatsRow>, ApiError> {
    sqlx::query_as::<_, RepoWithStatsRow>(
        "SELECT r.id, r.name, r.repo_path, r.ssh_user, r.ssh_host, r.ssh_port, r.ssh_host_key, \
         r.compression, r.encryption, r.enabled, r.importing, r.import_error, r.import_progress, \
         r.import_total, r.import_status_message, r.owner_id, r.visibility, r.sync_schedule, \
         r.last_synced_at, r.info_archive_count::INT8 AS archive_count, agg.last_backup_at, \
         r.info_original_size AS total_original_size, r.info_compressed_size AS \
         total_compressed_size, r.info_deduplicated_size AS total_deduplicated_size, \
         COALESCE(agg.agent_count, 0) AS agent_count, COALESCE(agg.unmatched_count, 0) AS \
         unmatched_count, r.last_op_kind, r.last_op_at, r.last_op_by FROM repos r LEFT JOIN \
         LATERAL (SELECT MAX(CASE WHEN br.finished_at > '1970-01-01T00:00:00Z' THEN \
         br.finished_at END) AS last_backup_at, COUNT(DISTINCT br.agent_id) AS agent_count, \
         COUNT(DISTINCT br.agent_id) FILTER (WHERE br.matched = false) AS unmatched_count FROM \
         backup_reports br WHERE br.repo_id = r.id AND br.status = 'success') agg ON true ORDER \
         BY r.name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn get_repo_with_stats(
    pool: &PgPool,
    repo_id: i64,
) -> Result<RepoWithStatsRow, ApiError> {
    sqlx::query_as::<_, RepoWithStatsRow>(
        "SELECT r.id, r.name, r.repo_path, r.ssh_user, r.ssh_host, r.ssh_port, r.ssh_host_key, \
         r.compression, r.encryption, r.enabled, r.importing, r.import_error, r.import_progress, \
         r.import_total, r.import_status_message, r.owner_id, r.visibility, r.sync_schedule, \
         r.last_synced_at, r.info_archive_count::INT8 AS archive_count, agg.last_backup_at, \
         r.info_original_size AS total_original_size, r.info_compressed_size AS \
         total_compressed_size, r.info_deduplicated_size AS total_deduplicated_size, \
         COALESCE(agg.agent_count, 0) AS agent_count, COALESCE(agg.unmatched_count, 0) AS \
         unmatched_count, r.last_op_kind, r.last_op_at, r.last_op_by FROM repos r LEFT JOIN \
         LATERAL (SELECT MAX(CASE WHEN br.finished_at > '1970-01-01T00:00:00Z' THEN \
         br.finished_at END) AS last_backup_at, COUNT(DISTINCT br.agent_id) AS agent_count, \
         COUNT(DISTINCT br.agent_id) FILTER (WHERE br.matched = false) AS unmatched_count FROM \
         backup_reports br WHERE br.repo_id = r.id AND br.status = 'success') agg ON true WHERE \
         r.id = $1",
    )
    .bind(repo_id)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("repo {repo_id} not found")),
        other => ApiError::Database(other),
    })
}

pub async fn update_repo_last_op(
    pool: &PgPool,
    repo_id: i64,
    kind: &str,
    at: chrono::DateTime<chrono::Utc>,
    by: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE repos SET last_op_kind = $1, last_op_at = $2, last_op_by = $3 WHERE id = $4",
    )
    .bind(kind)
    .bind(at)
    .bind(by)
    .bind(repo_id)
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct TagRow {
    pub id: i64,
    pub name: String,
    pub color: String,
    pub scope: String,
}

pub async fn list_tags(pool: &PgPool, scope: &str) -> Result<Vec<TagRow>, ApiError> {
    sqlx::query_as::<_, TagRow>(
        "SELECT id, name, color, scope FROM tags WHERE scope = $1 ORDER BY name",
    )
    .bind(scope)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn insert_tag(
    pool: &PgPool,
    name: &str,
    color: &str,
    scope: &str,
) -> Result<TagRow, ApiError> {
    sqlx::query_as::<_, TagRow>(
        "INSERT INTO tags (name, color, scope) VALUES ($1, $2, $3) RETURNING id, name, color, \
         scope",
    )
    .bind(name)
    .bind(color)
    .bind(scope)
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn delete_tag(pool: &PgPool, id: i64) -> Result<(), ApiError> {
    let result = sqlx::query("DELETE FROM tags WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("tag {id} not found")));
    }
    Ok(())
}

pub async fn set_repo_tags(pool: &PgPool, repo_id: i64, tag_ids: &[i64]) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM repo_tags WHERE repo_id = $1")
        .bind(repo_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    for tag_id in tag_ids {
        sqlx::query("INSERT INTO repo_tags (repo_id, tag_id) VALUES ($1, $2)")
            .bind(repo_id)
            .bind(tag_id)
            .execute(pool)
            .await
            .map_err(ApiError::Database)?;
    }
    Ok(())
}

pub async fn set_agent_tags(pool: &PgPool, agent_id: i64, tag_ids: &[i64]) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM agent_tags WHERE agent_id = $1")
        .bind(agent_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    for tag_id in tag_ids {
        sqlx::query("INSERT INTO agent_tags (agent_id, tag_id) VALUES ($1, $2)")
            .bind(agent_id)
            .bind(tag_id)
            .execute(pool)
            .await
            .map_err(ApiError::Database)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct RepoTagRow {
    pub repo_id: i64,
    pub tag_name: String,
    pub tag_color: String,
}

pub async fn list_all_repo_tags(pool: &PgPool) -> Result<Vec<RepoTagRow>, ApiError> {
    sqlx::query_as::<_, RepoTagRow>(
        "SELECT rt.repo_id, t.name AS tag_name, t.color AS tag_color FROM repo_tags rt JOIN tags \
         t ON t.id = rt.tag_id ORDER BY rt.repo_id, t.name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn list_tags_for_repo(pool: &PgPool, repo_id: i64) -> Result<Vec<TagRow>, ApiError> {
    sqlx::query_as::<_, TagRow>(
        "SELECT t.id, t.name, t.color, t.scope FROM tags t JOIN repo_tags rt ON rt.tag_id = t.id \
         WHERE rt.repo_id = $1 ORDER BY t.name",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct AgentTagRow {
    pub agent_id: i64,
    pub tag_name: String,
    pub tag_color: String,
}

pub async fn list_tags_for_agent(pool: &PgPool, agent_id: i64) -> Result<Vec<TagRow>, ApiError> {
    sqlx::query_as::<_, TagRow>(
        "SELECT t.id, t.name, t.color, t.scope FROM tags t JOIN agent_tags at ON at.tag_id = t.id \
         WHERE at.agent_id = $1 ORDER BY t.name",
    )
    .bind(agent_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn list_all_agent_tags(pool: &PgPool) -> Result<Vec<AgentTagRow>, ApiError> {
    sqlx::query_as::<_, AgentTagRow>(
        "SELECT at.agent_id, t.name AS tag_name, t.color AS tag_color FROM agent_tags at JOIN \
         tags t ON t.id = at.tag_id ORDER BY at.agent_id, t.name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct DashboardSummaryRow {
    pub total_agents: i64,
    pub total_repos: i64,
    pub active_schedules: i64,
    pub total_schedules: i64,
    pub total_storage_bytes: i64,
    pub last_backup_at: Option<DateTime<Utc>>,
    pub next_backup_at: Option<DateTime<Utc>>,
    pub last_backup_schedule_id: Option<i64>,
    pub last_backup_repo_id: Option<i64>,
    pub last_backup_archive_name: Option<String>,
    pub next_backup_schedule_id: Option<i64>,
    pub success_30d: i64,
    pub failed_30d: i64,
    pub total_30d: i64,
    pub last_failure_at: Option<DateTime<Utc>>,
    pub last_warning_at: Option<DateTime<Utc>>,
    pub last_failure_schedule_id: Option<i64>,
    pub last_warning_schedule_id: Option<i64>,
    pub last_failure_message: Option<String>,
    pub last_warning_message: Option<String>,
    pub last_failure_repo_id: Option<i64>,
    pub last_warning_repo_id: Option<i64>,
    pub last_failure_repo_name: Option<String>,
    pub last_warning_repo_name: Option<String>,
    pub last_failure_schedule_name: Option<String>,
    pub last_warning_schedule_name: Option<String>,
}

pub async fn get_dashboard_summary(pool: &PgPool) -> Result<DashboardSummaryRow, ApiError> {
    sqlx::query_as::<_, DashboardSummaryRow>(
        "SELECT (SELECT COUNT(*) FROM agents WHERE is_hidden = false) AS total_agents, (SELECT \
         COUNT(*) FROM repos) AS total_repos, (SELECT COUNT(*) FROM schedules WHERE enabled = \
         true) AS active_schedules, (SELECT COUNT(*) FROM schedules) AS total_schedules, \
         COALESCE((SELECT SUM(info_deduplicated_size) FROM repos), 0)::INT8 AS \
         total_storage_bytes, (SELECT MAX(finished_at) FROM backup_reports WHERE status = \
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
         INTERVAL '30 days') AS success_30d, (SELECT COUNT(*) FROM backup_reports WHERE status != \
         'success' AND started_at > NOW() - INTERVAL '30 days') AS failed_30d, (SELECT COUNT(*) \
         FROM backup_reports WHERE started_at > NOW() - INTERVAL '30 days') AS total_30d, (SELECT \
         MAX(finished_at) FROM backup_reports WHERE status = 'failed' AND finished_at > \
         '1970-01-01T00:00:00Z') AS last_failure_at, (SELECT MAX(finished_at) FROM backup_reports \
         WHERE status = 'warning' AND finished_at > '1970-01-01T00:00:00Z') AS last_warning_at, \
         (SELECT br.schedule_id FROM backup_reports br WHERE br.schedule_id IS NOT NULL AND \
         br.status = 'failed' AND br.finished_at > '1970-01-01T00:00:00Z' ORDER BY br.finished_at \
         DESC LIMIT 1) AS last_failure_schedule_id, (SELECT br.schedule_id FROM backup_reports br \
         WHERE br.schedule_id IS NOT NULL AND br.status = 'warning' AND br.finished_at > \
         '1970-01-01T00:00:00Z' ORDER BY br.finished_at DESC LIMIT 1) AS \
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

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StorageBreakdownRow {
    pub name: String,
    pub compressed_size: i64,
    pub deduplicated_size: i64,
}

pub async fn get_storage_breakdown(pool: &PgPool) -> Result<Vec<StorageBreakdownRow>, ApiError> {
    sqlx::query_as::<_, StorageBreakdownRow>(
        "SELECT r.name, r.info_compressed_size AS compressed_size, r.info_deduplicated_size AS \
         deduplicated_size FROM repos r ORDER BY r.info_deduplicated_size DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn get_activity_feed_days(
    pool: &PgPool,
    days: i64,
    repo_id: Option<i64>,
    hostname: Option<&str>,
    schedule_id: Option<i64>,
    run_id: Option<&str>,
) -> Result<Vec<ActivityRow>, ApiError> {
    let mut sql = String::from(
        "SELECT br.id, a.hostname, r.name AS target_name, br.started_at, br.finished_at, \
         br.status, br.duration_secs, br.repo_id, br.archive_name, br.error_message, \
         br.schedule_id, s.name AS schedule_name, br.run_id FROM backup_reports br JOIN agents a \
         ON a.id = br.agent_id JOIN repos r ON r.id = br.repo_id LEFT JOIN schedules s ON s.id = \
         br.schedule_id WHERE a.is_hidden = false AND a.visibility <> 'hidden' AND \
         COALESCE(a.display_name, '') NOT ILIKE '%(imported)%' AND br.started_at > NOW() - \
         make_interval(days => $1::int)",
    );
    let mut param_idx = 2u32;
    if repo_id.is_some() {
        sql.push_str(&format!(" AND br.repo_id = ${param_idx}"));
        param_idx += 1;
    }
    if hostname.is_some() {
        sql.push_str(&format!(" AND a.hostname = ${param_idx}"));
        param_idx += 1;
    }
    if schedule_id.is_some() {
        sql.push_str(&format!(" AND br.schedule_id = ${param_idx}"));
        param_idx += 1;
    }
    if run_id.is_some() {
        sql.push_str(&format!(" AND br.run_id = ${param_idx}"));
    }
    sql.push_str(" ORDER BY br.started_at DESC");

    let mut query = sqlx::query_as::<_, ActivityRow>(&sql);
    query = query.bind(i32::try_from(days).unwrap_or(14));
    if let Some(rid) = repo_id {
        query = query.bind(rid);
    }
    if let Some(host) = hostname {
        query = query.bind(host.to_owned());
    }
    if let Some(sid) = schedule_id {
        query = query.bind(sid);
    }
    if let Some(rid) = run_id {
        query = query.bind(rid.to_owned());
    }
    query.fetch_all(pool).await.map_err(ApiError::Database)
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct GroupRow {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct RoleRow {
    pub id: i64,
    pub name: String,
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
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct UserGroupRow {
    pub user_id: i64,
    pub group_id: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct UserRoleRow {
    pub user_id: i64,
    pub role_id: i64,
}

pub async fn list_groups(pool: &PgPool) -> Result<Vec<GroupRow>, ApiError> {
    sqlx::query_as::<_, GroupRow>(
        "SELECT id, name, description, created_at FROM groups ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn get_group(pool: &PgPool, id: i64) -> Result<Option<GroupRow>, ApiError> {
    sqlx::query_as::<_, GroupRow>(
        "SELECT id, name, description, created_at FROM groups WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn insert_group(
    pool: &PgPool,
    name: &str,
    description: Option<&str>,
) -> Result<GroupRow, ApiError> {
    sqlx::query_as::<_, GroupRow>(
        "INSERT INTO groups (name, description) VALUES ($1, $2) RETURNING id, name, description, \
         created_at",
    )
    .bind(name)
    .bind(description)
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn update_group(
    pool: &PgPool,
    id: i64,
    name: &str,
    description: Option<&str>,
) -> Result<GroupRow, ApiError> {
    sqlx::query_as::<_, GroupRow>(
        "UPDATE groups SET name = $2, description = $3 WHERE id = $1 RETURNING id, name, \
         description, created_at",
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("group {id} not found")),
        other => ApiError::Database(other),
    })
}

pub async fn delete_group(pool: &PgPool, id: i64) -> Result<(), ApiError> {
    let result = sqlx::query("DELETE FROM groups WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("group {id} not found")));
    }
    Ok(())
}

pub async fn list_group_members(pool: &PgPool, group_id: i64) -> Result<Vec<i64>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        user_id: i64,
    }

    let rows = sqlx::query_as::<_, Row>(
        "SELECT user_id FROM user_groups WHERE group_id = $1 ORDER BY user_id",
    )
    .bind(group_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows.into_iter().map(|r| r.user_id).collect())
}

pub async fn set_group_members(
    pool: &PgPool,
    group_id: i64,
    user_ids: &[i64],
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM user_groups WHERE group_id = $1")
        .bind(group_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    for user_id in user_ids {
        sqlx::query("INSERT INTO user_groups (user_id, group_id) VALUES ($1, $2)")
            .bind(user_id)
            .bind(group_id)
            .execute(pool)
            .await
            .map_err(ApiError::Database)?;
    }
    Ok(())
}

pub async fn list_user_groups(pool: &PgPool, user_id: i64) -> Result<Vec<GroupRow>, ApiError> {
    sqlx::query_as::<_, GroupRow>(
        "SELECT g.id, g.name, g.description, g.created_at FROM groups g JOIN user_groups ug ON \
         ug.group_id = g.id WHERE ug.user_id = $1 ORDER BY g.name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn user_shares_group_with(
    pool: &PgPool,
    user_id: i64,
    other_user_id: i64,
) -> Result<bool, ApiError> {
    #[derive(sqlx::FromRow)]
    struct ExistsRow {
        shared: bool,
    }

    let row = sqlx::query_as::<_, ExistsRow>(
        "SELECT EXISTS(SELECT 1 FROM user_groups a JOIN user_groups b ON a.group_id = b.group_id \
         WHERE a.user_id = $1 AND b.user_id = $2) AS shared",
    )
    .bind(user_id)
    .bind(other_user_id)
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(row.shared)
}

pub async fn list_roles(pool: &PgPool) -> Result<Vec<RoleRow>, ApiError> {
    sqlx::query_as::<_, RoleRow>(
        "SELECT id, name, can_create_agent, can_delete_agent, can_delete_own_agent, \
         can_create_repo, can_delete_repo, can_delete_own_repo, can_create_schedule, \
         can_delete_schedule, can_delete_own_schedule, can_manage_tags, can_view_all_repos, \
         can_manage_tunnels, created_at FROM roles ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn get_role(pool: &PgPool, id: i64) -> Result<Option<RoleRow>, ApiError> {
    sqlx::query_as::<_, RoleRow>(
        "SELECT id, name, can_create_agent, can_delete_agent, can_delete_own_agent, \
         can_create_repo, can_delete_repo, can_delete_own_repo, can_create_schedule, \
         can_delete_schedule, can_delete_own_schedule, can_manage_tags, can_view_all_repos, \
         can_manage_tunnels, created_at FROM roles WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)
}

pub struct InsertRoleParams<'a> {
    pub name: &'a str,
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

pub async fn insert_role(
    pool: &PgPool,
    params: &InsertRoleParams<'_>,
) -> Result<RoleRow, ApiError> {
    sqlx::query_as::<_, RoleRow>(
        "INSERT INTO roles (name, can_create_agent, can_delete_agent, can_delete_own_agent, \
         can_create_repo, can_delete_repo, can_delete_own_repo, can_create_schedule, \
         can_delete_schedule, can_delete_own_schedule, can_manage_tags, can_view_all_repos, \
         can_manage_tunnels) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
         RETURNING id, name, can_create_agent, can_delete_agent, can_delete_own_agent, \
         can_create_repo, can_delete_repo, can_delete_own_repo, can_create_schedule, \
         can_delete_schedule, can_delete_own_schedule, can_manage_tags, can_view_all_repos, \
         can_manage_tunnels, created_at",
    )
    .bind(params.name)
    .bind(params.can_create_agent)
    .bind(params.can_delete_agent)
    .bind(params.can_delete_own_agent)
    .bind(params.can_create_repo)
    .bind(params.can_delete_repo)
    .bind(params.can_delete_own_repo)
    .bind(params.can_create_schedule)
    .bind(params.can_delete_schedule)
    .bind(params.can_delete_own_schedule)
    .bind(params.can_manage_tags)
    .bind(params.can_view_all_repos)
    .bind(params.can_manage_tunnels)
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn update_role(
    pool: &PgPool,
    id: i64,
    params: &InsertRoleParams<'_>,
) -> Result<RoleRow, ApiError> {
    sqlx::query_as::<_, RoleRow>(
        "UPDATE roles SET name = $2, can_create_agent = $3, can_delete_agent = $4, \
         can_delete_own_agent = $5, can_create_repo = $6, can_delete_repo = $7, \
         can_delete_own_repo = $8, can_create_schedule = $9, can_delete_schedule = $10, \
         can_delete_own_schedule = $11, can_manage_tags = $12, can_view_all_repos = $13, \
         can_manage_tunnels = $14 WHERE id = $1 RETURNING id, name, can_create_agent, \
         can_delete_agent, can_delete_own_agent, can_create_repo, can_delete_repo, \
         can_delete_own_repo, can_create_schedule, can_delete_schedule, can_delete_own_schedule, \
         can_manage_tags, can_view_all_repos, can_manage_tunnels, created_at",
    )
    .bind(id)
    .bind(params.name)
    .bind(params.can_create_agent)
    .bind(params.can_delete_agent)
    .bind(params.can_delete_own_agent)
    .bind(params.can_create_repo)
    .bind(params.can_delete_repo)
    .bind(params.can_delete_own_repo)
    .bind(params.can_create_schedule)
    .bind(params.can_delete_schedule)
    .bind(params.can_delete_own_schedule)
    .bind(params.can_manage_tags)
    .bind(params.can_view_all_repos)
    .bind(params.can_manage_tunnels)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ApiError::NotFound(format!("role {id} not found")),
        other => ApiError::Database(other),
    })
}

pub async fn delete_role(pool: &PgPool, id: i64) -> Result<(), ApiError> {
    let result = sqlx::query("DELETE FROM roles WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound(format!("role {id} not found")));
    }
    Ok(())
}

pub async fn list_user_roles(pool: &PgPool, user_id: i64) -> Result<Vec<RoleRow>, ApiError> {
    sqlx::query_as::<_, RoleRow>(
        "SELECT r.id, r.name, r.can_create_agent, r.can_delete_agent, r.can_delete_own_agent, \
         r.can_create_repo, r.can_delete_repo, r.can_delete_own_repo, r.can_create_schedule, \
         r.can_delete_schedule, r.can_delete_own_schedule, r.can_manage_tags, \
         r.can_view_all_repos, r.can_manage_tunnels, r.created_at FROM roles r JOIN user_roles ur \
         ON ur.role_id = r.id WHERE ur.user_id = $1 ORDER BY r.name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn set_user_roles(pool: &PgPool, user_id: i64, role_ids: &[i64]) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM user_roles WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(ApiError::Database)?;

    for role_id in role_ids {
        sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)")
            .bind(user_id)
            .bind(role_id)
            .execute(pool)
            .await
            .map_err(ApiError::Database)?;
    }
    Ok(())
}

pub async fn get_effective_permissions(pool: &PgPool, user_id: i64) -> Result<RoleRow, ApiError> {
    #[derive(sqlx::FromRow)]
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

    let row = sqlx::query_as::<_, AggRow>(
        "SELECT BOOL_OR(r.can_create_agent) AS can_create_agent, BOOL_OR(r.can_delete_agent) AS \
         can_delete_agent, BOOL_OR(r.can_delete_own_agent) AS can_delete_own_agent, \
         BOOL_OR(r.can_create_repo) AS can_create_repo, BOOL_OR(r.can_delete_repo) AS \
         can_delete_repo, BOOL_OR(r.can_delete_own_repo) AS can_delete_own_repo, \
         BOOL_OR(r.can_create_schedule) AS can_create_schedule, BOOL_OR(r.can_delete_schedule) AS \
         can_delete_schedule, BOOL_OR(r.can_delete_own_schedule) AS can_delete_own_schedule, \
         BOOL_OR(r.can_manage_tags) AS can_manage_tags, BOOL_OR(r.can_view_all_repos) AS \
         can_view_all_repos, BOOL_OR(r.can_manage_tunnels) AS can_manage_tunnels FROM roles r \
         JOIN user_roles ur ON ur.role_id = r.id WHERE ur.user_id = $1",
    )
    .bind(user_id)
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

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TrendRow {
    pub date: chrono::NaiveDate,
    pub original_size: i64,
    pub compressed_size: i64,
    pub deduplicated_size: i64,
    pub file_count: i64,
    pub duration_seconds: i64,
    pub backup_count: i64,
}

pub async fn get_backup_trends(
    pool: &PgPool,
    repo_id: Option<i64>,
    days: i64,
) -> Result<Vec<TrendRow>, ApiError> {
    if let Some(rid) = repo_id {
        sqlx::query_as::<_, TrendRow>(
            "SELECT started_at::date AS date, COALESCE(AVG(original_size), 0)::INT8 AS \
             original_size, COALESCE(AVG(compressed_size), 0)::INT8 AS compressed_size, \
             COALESCE(AVG(deduplicated_size), 0)::INT8 AS deduplicated_size, \
             COALESCE(AVG(files_processed), 0)::INT8 AS file_count, COALESCE(AVG(duration_secs), \
             0)::INT8 AS duration_seconds, COUNT(*)::INT8 AS backup_count FROM backup_reports \
             WHERE repo_id = $1 AND started_at > NOW() - make_interval(days => $2) GROUP BY \
             started_at::date ORDER BY date",
        )
        .bind(rid)
        .bind(i32::try_from(days).unwrap_or(30))
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    } else {
        sqlx::query_as::<_, TrendRow>(
            "SELECT started_at::date AS date, COALESCE(AVG(original_size), 0)::INT8 AS \
             original_size, COALESCE(AVG(compressed_size), 0)::INT8 AS compressed_size, \
             COALESCE(AVG(deduplicated_size), 0)::INT8 AS deduplicated_size, \
             COALESCE(AVG(files_processed), 0)::INT8 AS file_count, COALESCE(AVG(duration_secs), \
             0)::INT8 AS duration_seconds, COUNT(*)::INT8 AS backup_count FROM backup_reports \
             WHERE started_at > NOW() - make_interval(days => $1) GROUP BY started_at::date ORDER \
             BY date",
        )
        .bind(i32::try_from(days).unwrap_or(30))
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct CalendarEventRow {
    pub date: chrono::NaiveDate,
    pub event_type: String,
    pub status: String,
    pub repo_name: String,
    pub hostname: String,
    pub time: String,
    pub report_id: Option<i64>,
    pub repo_id: Option<i64>,
    pub error_message: Option<String>,
    pub archive_name: Option<String>,
}

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
        chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .ok_or_else(|| ApiError::BadRequest("invalid month".to_string()))?;

    let tz_name = tz.name();

    if let Some(rid) = repo_id {
        sqlx::query_as::<_, CalendarEventRow>(
            "SELECT (br.started_at AT TIME ZONE $4)::date AS date, 'backup' AS event_type, CASE \
             WHEN br.status = 'success' THEN 'success' ELSE 'failed' END AS status, r.name AS \
             repo_name, a.hostname, to_char(br.started_at AT TIME ZONE $4, 'HH24:MI') AS time, \
             br.id AS report_id, br.repo_id, br.error_message, br.archive_name FROM \
             backup_reports br JOIN repos r ON r.id = br.repo_id JOIN agents a ON a.id = \
             br.agent_id WHERE a.is_hidden = false AND (br.started_at AT TIME ZONE $4)::date >= \
             $1 AND (br.started_at AT TIME ZONE $4)::date < $2 AND br.repo_id = $3 ORDER BY \
             br.started_at",
        )
        .bind(start)
        .bind(end)
        .bind(rid)
        .bind(tz_name)
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    } else {
        sqlx::query_as::<_, CalendarEventRow>(
            "SELECT (br.started_at AT TIME ZONE $3)::date AS date, 'backup' AS event_type, CASE \
             WHEN br.status = 'success' THEN 'success' ELSE 'failed' END AS status, r.name AS \
             repo_name, a.hostname, to_char(br.started_at AT TIME ZONE $3, 'HH24:MI') AS time, \
             br.id AS report_id, br.repo_id, br.error_message, br.archive_name FROM \
             backup_reports br JOIN repos r ON r.id = br.repo_id JOIN agents a ON a.id = \
             br.agent_id WHERE a.is_hidden = false AND (br.started_at AT TIME ZONE $3)::date >= \
             $1 AND (br.started_at AT TIME ZONE $3)::date < $2 ORDER BY br.started_at",
        )
        .bind(start)
        .bind(end)
        .bind(tz_name)
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StorageTrendRow {
    pub date: chrono::NaiveDate,
    pub original_size: i64,
    pub compressed_size: i64,
    pub deduplicated_size: Option<i64>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct StorageTrendByRepoRow {
    pub date: chrono::NaiveDate,
    pub repo_id: i64,
    pub repo_name: String,
    pub original_size: i64,
    pub compressed_size: i64,
    pub deduplicated_size: Option<i64>,
}

pub async fn get_storage_trends(
    pool: &PgPool,
    repo_id: Option<i64>,
    days: i64,
) -> Result<Vec<StorageTrendRow>, ApiError> {
    // For each day in the range, take the last backup report per repo up to that day
    // and sum their sizes. This gives the total repo footprint per day.
    let days_i32 = i32::try_from(days).unwrap_or(30);
    if let Some(rid) = repo_id {
        sqlx::query_as::<_, StorageTrendRow>(
            "WITH days AS ( SELECT generate_series( (CURRENT_DATE - make_interval(days => \
             $1))::date, CURRENT_DATE, '1 day'::interval )::date AS date ) SELECT d.date, \
             COALESCE(latest.original_size, 0)::INT8 AS original_size, \
             COALESCE(latest.compressed_size, 0)::INT8 AS compressed_size, \
             NULLIF(COALESCE(latest.repo_unique_csize, 0), 0)::INT8 AS deduplicated_size FROM \
             days d LEFT JOIN LATERAL ( SELECT br.original_size, br.compressed_size, \
             br.repo_unique_csize FROM backup_reports br WHERE br.repo_id = $2 AND \
             br.started_at::date <= d.date AND br.status = 'success' ORDER BY br.started_at DESC \
             LIMIT 1 ) latest ON true ORDER BY d.date",
        )
        .bind(days_i32)
        .bind(rid)
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    } else {
        sqlx::query_as::<_, StorageTrendRow>(
            "WITH days AS ( SELECT generate_series( (CURRENT_DATE - make_interval(days => \
             $1))::date, CURRENT_DATE, '1 day'::interval )::date AS date ) SELECT d.date, \
             COALESCE(SUM(latest.original_size), 0)::INT8 AS original_size, \
             COALESCE(SUM(latest.compressed_size), 0)::INT8 AS compressed_size, \
             NULLIF(COALESCE(SUM(latest.repo_unique_csize), 0), 0)::INT8 AS deduplicated_size \
             FROM days d LEFT JOIN LATERAL ( SELECT DISTINCT ON (br.repo_id) br.original_size, \
             br.compressed_size, br.repo_unique_csize FROM backup_reports br WHERE \
             br.started_at::date <= d.date AND br.status = 'success' ORDER BY br.repo_id, \
             br.started_at DESC ) latest ON true GROUP BY d.date ORDER BY d.date",
        )
        .bind(days_i32)
        .fetch_all(pool)
        .await
        .map_err(ApiError::Database)
    }
}

pub async fn list_archive_names_for_repo(
    pool: &PgPool,
    repo_id: i64,
) -> Result<std::collections::HashSet<String>, ApiError> {
    let names = sqlx::query_scalar::<_, String>(
        "SELECT archive_name FROM backup_reports WHERE repo_id = $1 AND archive_name IS NOT NULL",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(names.into_iter().collect())
}

/// Archive names that need a `borg info` run.
///
/// Covers two cases:
/// - All sizes are still zero (archive was imported but never enriched).
/// - `repo_unique_csize` is zero even though other sizes are populated (archive was enriched
///   before `repo_unique_csize` was tracked).
pub async fn list_archive_names_needing_stats(
    pool: &PgPool,
    repo_id: i64,
) -> Result<std::collections::HashSet<String>, ApiError> {
    let names = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT archive_name FROM backup_reports WHERE repo_id = $1 AND archive_name IS \
         NOT NULL AND ((original_size = 0 AND compressed_size = 0 AND deduplicated_size = 0) OR \
         repo_unique_csize = 0)",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(names.into_iter().collect())
}

pub async fn delete_archive_reports_by_names(
    pool: &PgPool,
    repo_id: i64,
    names: &[String],
) -> Result<u64, ApiError> {
    if names.is_empty() {
        return Ok(0);
    }
    let result =
        sqlx::query("DELETE FROM backup_reports WHERE repo_id = $1 AND archive_name = ANY($2)")
            .bind(repo_id)
            .bind(names)
            .execute(pool)
            .await
            .map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

pub async fn delete_archive_records_by_names(
    pool: &PgPool,
    repo_id: i64,
    names: &[String],
) -> Result<u64, ApiError> {
    if names.is_empty() {
        return Ok(0);
    }

    let mut tx = pool.begin().await.map_err(ApiError::Database)?;

    let result =
        sqlx::query("DELETE FROM backup_reports WHERE repo_id = $1 AND archive_name = ANY($2)")
            .bind(repo_id)
            .bind(names)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::Database)?;

    // Collect candidate path IDs before the cascade delete removes archive_files.
    let candidate_ids: Vec<i64> = sqlx::query_scalar::<_, i64>(
        "SELECT path_id FROM archive_files WHERE archive_id IN (SELECT id FROM archives WHERE \
         repo_id = $1 AND name = ANY($2)) UNION SELECT parent_path_id FROM archive_files WHERE \
         archive_id IN (SELECT id FROM archives WHERE repo_id = $1 AND name = ANY($2))",
    )
    .bind(repo_id)
    .bind(names)
    .fetch_all(&mut *tx)
    .await
    .map_err(ApiError::Database)?;

    // Deleting from archives cascades to archive_files, archive_index_jobs, and archive_tags.
    sqlx::query("DELETE FROM archives WHERE repo_id = $1 AND name = ANY($2)")
        .bind(repo_id)
        .bind(names)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;

    // GC paths that are now orphaned, checking only the candidates from the deleted archives.
    if !candidate_ids.is_empty() {
        sqlx::query(
            "DELETE FROM archive_paths WHERE repo_id = $1 AND id = ANY($2) AND NOT EXISTS (SELECT \
             1 FROM archive_files WHERE path_id = archive_paths.id) AND NOT EXISTS (SELECT 1 FROM \
             archive_files WHERE parent_path_id = archive_paths.id)",
        )
        .bind(repo_id)
        .bind(&candidate_ids)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::Database)?;
    }

    tx.commit().await.map_err(ApiError::Database)?;
    Ok(result.rows_affected())
}

pub async fn get_storage_trends_by_repo(
    pool: &PgPool,
    days: i64,
) -> Result<Vec<StorageTrendByRepoRow>, ApiError> {
    let days_i32 = i32::try_from(days).unwrap_or(30);
    sqlx::query_as::<_, StorageTrendByRepoRow>(
        "WITH days AS ( SELECT generate_series( (CURRENT_DATE - make_interval(days => $1))::date, \
         CURRENT_DATE, '1 day'::interval )::date AS date ), repos_list AS ( SELECT DISTINCT r.id \
         AS repo_id, r.name AS repo_name FROM repos r JOIN backup_reports br ON br.repo_id = r.id \
         ) SELECT d.date, rl.repo_id, rl.repo_name, COALESCE(latest.original_size, 0)::INT8 AS \
         original_size, COALESCE(latest.compressed_size, 0)::INT8 AS compressed_size, \
         NULLIF(COALESCE(latest.repo_unique_csize, 0), 0)::INT8 AS deduplicated_size FROM days d \
         CROSS JOIN repos_list rl LEFT JOIN LATERAL ( SELECT br.original_size, \
         br.compressed_size, br.repo_unique_csize FROM backup_reports br WHERE br.repo_id = \
         rl.repo_id AND br.started_at::date <= d.date AND br.status = 'success' ORDER BY \
         br.started_at DESC LIMIT 1 ) latest ON true ORDER BY d.date, rl.repo_name",
    )
    .bind(days_i32)
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

pub async fn get_enabled_schedules_for_calendar(
    pool: &PgPool,
) -> Result<Vec<ScheduleRow>, ApiError> {
    sqlx::query_as::<_, ScheduleRow>(
        "SELECT id, repo_id, name, schedule_type, cron_expression, enabled, canary_enabled, \
         last_run_at, next_run_at, exclude_patterns_raw, ignore_global_excludes, keep_hourly, \
         keep_daily, keep_weekly, keep_monthly, keep_yearly, compact_enabled, rate_limit_kbps, \
         pre_backup_commands, post_backup_commands, execution_mode, on_failure, owner_id, \
         visibility FROM schedules WHERE enabled = true",
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}
