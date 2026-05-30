// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct RepoQuota {
    pub repo_id: i64,
    pub warn_bytes: Option<i64>,
    pub critical_bytes: Option<i64>,
    pub enabled: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuotaStatus {
    Ok,
    Warning,
    Critical,
}

pub async fn upsert_quota(
    pool: &PgPool,
    repo_id: i64,
    warn_bytes: Option<i64>,
    critical_bytes: Option<i64>,
    enabled: bool,
) -> Result<RepoQuota, sqlx::Error> {
    sqlx::query_as::<_, RepoQuota>(
        r#"
        INSERT INTO repo_quotas (repo_id, warn_bytes, critical_bytes, enabled, updated_at)
        VALUES ($1, $2, $3, $4, NOW())
        ON CONFLICT (repo_id) DO UPDATE
        SET warn_bytes = EXCLUDED.warn_bytes,
            critical_bytes = EXCLUDED.critical_bytes,
            enabled = EXCLUDED.enabled,
            updated_at = NOW()
        RETURNING repo_id, warn_bytes, critical_bytes, enabled, updated_at
        "#,
    )
    .bind(repo_id)
    .bind(warn_bytes)
    .bind(critical_bytes)
    .bind(enabled)
    .fetch_one(pool)
    .await
}

impl RepoQuota {
    #[must_use]
    pub fn status(&self, deduplicated_size: i64) -> QuotaStatus {
        evaluate_quota(self, deduplicated_size)
    }
}

pub async fn get_quota(pool: &PgPool, repo_id: i64) -> Result<Option<RepoQuota>, sqlx::Error> {
    let quota = sqlx::query_as::<_, RepoQuota>(
        "SELECT repo_id, warn_bytes, critical_bytes, enabled, updated_at FROM repo_quotas WHERE \
         repo_id = $1",
    )
    .bind(repo_id)
    .fetch_optional(pool)
    .await?;

    Ok(quota)
}

pub fn evaluate_quota(quota: &RepoQuota, deduplicated_size: i64) -> QuotaStatus {
    if !quota.enabled {
        return QuotaStatus::Ok;
    }

    if quota
        .critical_bytes
        .is_some_and(|limit| deduplicated_size >= limit)
    {
        return QuotaStatus::Critical;
    }

    if quota
        .warn_bytes
        .is_some_and(|limit| deduplicated_size >= limit)
    {
        return QuotaStatus::Warning;
    }

    QuotaStatus::Ok
}
