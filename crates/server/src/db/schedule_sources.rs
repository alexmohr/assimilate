// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! What a schedule backs up per agent: sources, excludes, includes, hook
//! commands and file-change patterns.

use serde::Serialize;
use shared::hooks::HookCommand;
use sqlx::PgPool;

use super::HookCommands;
use crate::error::ApiError;

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

/// Per-agent include patterns for a schedule override.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PerAgentIncludePatterns {
    /// Agent ID.
    pub agent_id: i64,
    /// Raw include pattern text for this agent.
    pub raw_text: String,
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_all_per_agent_includes_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<Vec<PerAgentIncludePatterns>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        agent_id: i64,
        raw_text: String,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT agent_id, raw_text FROM per_agent_includes WHERE schedule_id = $1 ORDER BY \
         agent_id",
        schedule_id,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)?;

    Ok(rows
        .into_iter()
        .map(|r| PerAgentIncludePatterns {
            agent_id: r.agent_id,
            raw_text: r.raw_text,
        })
        .collect())
}

/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn upsert_per_agent_includes_raw(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
    raw_text: &str,
) -> Result<(), ApiError> {
    sqlx::query!(
        "INSERT INTO per_agent_includes (schedule_id, agent_id, raw_text) VALUES ($1, $2, $3) ON \
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
pub async fn delete_per_agent_includes_for_schedule(
    pool: &PgPool,
    schedule_id: i64,
) -> Result<(), ApiError> {
    sqlx::query!(
        "DELETE FROM per_agent_includes WHERE schedule_id = $1",
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
pub async fn get_per_agent_includes_raw(
    pool: &PgPool,
    schedule_id: i64,
    agent_id: i64,
) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar!(
        "SELECT raw_text FROM per_agent_includes WHERE schedule_id = $1 AND agent_id = $2",
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
    pub pre_backup_commands: Vec<HookCommand>,
    /// Post-backup commands for this agent.
    pub post_backup_commands: Vec<HookCommand>,
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
        pre_backup_commands: HookCommands,
        post_backup_commands: HookCommands,
    }

    let rows = sqlx::query_as!(
        Row,
        "SELECT agent_id, pre_backup_commands AS \"pre_backup_commands: HookCommands\", \
         post_backup_commands AS \"post_backup_commands: HookCommands\" FROM per_agent_commands \
         WHERE schedule_id = $1 ORDER BY agent_id",
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
        pre_backup_commands: HookCommands,
        post_backup_commands: HookCommands,
    }

    let row = sqlx::query_as!(
        Row,
        "SELECT pre_backup_commands AS \"pre_backup_commands: HookCommands\", \
         post_backup_commands AS \"post_backup_commands: HookCommands\" FROM per_agent_commands \
         WHERE schedule_id = $1 AND agent_id = $2",
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
    pre_backup_commands: &[HookCommand],
    post_backup_commands: &[HookCommand],
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
