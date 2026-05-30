// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ArchiveTag {
    pub id: i64,
    pub repo_id: i64,
    pub archive_name: String,
    pub tag: String,
    pub created_by: Option<i64>,
    pub created_at: DateTime<Utc>,
}

pub async fn add_tag(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
    tag: &str,
    created_by: Option<i64>,
) -> Result<ArchiveTag, sqlx::Error> {
    sqlx::query_as::<_, ArchiveTag>(
        "INSERT INTO archive_tags (repo_id, archive_name, tag, created_by) VALUES ($1, $2, $3, \
         $4) RETURNING id, repo_id, archive_name, tag, created_by, created_at",
    )
    .bind(repo_id)
    .bind(archive_name)
    .bind(tag)
    .bind(created_by)
    .fetch_one(pool)
    .await
}

pub async fn remove_tag(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
    tag: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM archive_tags WHERE repo_id = $1 AND archive_name = $2 AND tag = $3",
    )
    .bind(repo_id)
    .bind(archive_name)
    .bind(tag)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn list_tags_for_archive(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
) -> Result<Vec<ArchiveTag>, sqlx::Error> {
    sqlx::query_as::<_, ArchiveTag>(
        "SELECT id, repo_id, archive_name, tag, created_by, created_at FROM archive_tags WHERE \
         repo_id = $1 AND archive_name = $2 ORDER BY tag",
    )
    .bind(repo_id)
    .bind(archive_name)
    .fetch_all(pool)
    .await
}

pub async fn list_archives_by_tag(
    pool: &PgPool,
    repo_id: i64,
    tag: &str,
) -> Result<Vec<String>, sqlx::Error> {
    #[derive(sqlx::FromRow)]
    struct Row {
        archive_name: String,
    }

    let rows = sqlx::query_as::<_, Row>(
        "SELECT DISTINCT archive_name FROM archive_tags WHERE repo_id = $1 AND tag = $2 ORDER BY \
         archive_name",
    )
    .bind(repo_id)
    .bind(tag)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|row| row.archive_name).collect())
}
