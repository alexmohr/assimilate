// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ArchiveTag {
    pub id: i64,
    pub repo_id: Option<i64>,
    pub archive_name: Option<String>,
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
    // Ensure the archive row exists (created on first index or tag operation).
    sqlx::query!(
        "INSERT INTO archives (repo_id, name) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        repo_id,
        archive_name,
    )
    .execute(pool)
    .await?;

    sqlx::query_as!(
        ArchiveTag,
        "INSERT INTO archive_tags (archive_id, tag, created_by)
         SELECT a.id, $3, $4 FROM archives a WHERE a.repo_id = $1 AND a.name = $2
         RETURNING id,
                   (SELECT repo_id FROM archives WHERE id = archive_tags.archive_id) AS repo_id,
                   (SELECT name   FROM archives WHERE id = archive_tags.archive_id) AS \
         archive_name,
                   tag, created_by, created_at",
        repo_id,
        archive_name,
        tag,
        created_by,
    )
    .fetch_one(pool)
    .await
}

pub async fn remove_tag(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
    tag: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM archive_tags t USING archives a WHERE a.id = t.archive_id AND a.repo_id = $1 \
         AND a.name = $2 AND t.tag = $3",
        repo_id,
        archive_name,
        tag,
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn list_tags_for_archive(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
) -> Result<Vec<ArchiveTag>, sqlx::Error> {
    sqlx::query_as!(
        ArchiveTag,
        "SELECT t.id, a.repo_id, a.name AS archive_name, t.tag, t.created_by, t.created_at FROM \
         archive_tags t JOIN archives a ON a.id = t.archive_id WHERE a.repo_id = $1 AND a.name = \
         $2 ORDER BY t.tag",
        repo_id,
        archive_name,
    )
    .fetch_all(pool)
    .await
}

pub async fn list_archives_by_tag(
    pool: &PgPool,
    repo_id: i64,
    tag: &str,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT DISTINCT a.name FROM archive_tags t JOIN archives a ON a.id = t.archive_id WHERE \
         a.repo_id = $1 AND t.tag = $2 ORDER BY a.name",
        repo_id,
        tag,
    )
    .fetch_all(pool)
    .await
}
