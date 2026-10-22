// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The tag catalog and its assignment to repositories and agents. Archive
//! tags live in [`super::tags`].

use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::error::ApiError;

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
