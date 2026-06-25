// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::error::ApiError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuotaAction {
    NotifyOnly,
    BlockBackups,
    DisableSchedule,
}

impl std::fmt::Display for QuotaAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotifyOnly => write!(f, "notify_only"),
            Self::BlockBackups => write!(f, "block_backups"),
            Self::DisableSchedule => write!(f, "disable_schedule"),
        }
    }
}

impl std::str::FromStr for QuotaAction {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "notify_only" => Ok(Self::NotifyOnly),
            "block_backups" => Ok(Self::BlockBackups),
            "disable_schedule" => Ok(Self::DisableSchedule),
            other => Err(format!("unknown quota action: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct RepoQuota {
    pub repo_id: i64,
    pub warn_bytes: Option<i64>,
    pub critical_bytes: Option<i64>,
    pub warn_action: String,
    pub critical_action: String,
    pub enabled: bool,
    pub updated_at: DateTime<Utc>,
}

impl RepoQuota {
    #[must_use]
    pub fn status(&self, deduplicated_size: i64) -> QuotaStatus {
        evaluate_quota(self, deduplicated_size)
    }

    #[must_use]
    pub fn warn_action_parsed(&self) -> QuotaAction {
        self.warn_action.parse().unwrap_or(QuotaAction::NotifyOnly)
    }

    #[must_use]
    pub fn critical_action_parsed(&self) -> QuotaAction {
        self.critical_action
            .parse()
            .unwrap_or(QuotaAction::NotifyOnly)
    }
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
    warn_action: QuotaAction,
    critical_action: QuotaAction,
    enabled: bool,
) -> Result<RepoQuota, sqlx::Error> {
    sqlx::query_as::<_, RepoQuota>(
        "INSERT INTO repo_quotas (repo_id, warn_bytes, critical_bytes, warn_action, \
         critical_action, enabled, updated_at) VALUES ($1, $2, $3, $4, $5, $6, NOW()) ON CONFLICT \
         (repo_id) DO UPDATE SET warn_bytes = EXCLUDED.warn_bytes, critical_bytes = \
         EXCLUDED.critical_bytes, warn_action = EXCLUDED.warn_action, critical_action = \
         EXCLUDED.critical_action, enabled = EXCLUDED.enabled, updated_at = NOW() RETURNING \
         repo_id, warn_bytes, critical_bytes, warn_action, critical_action, enabled, updated_at",
    )
    .bind(repo_id)
    .bind(warn_bytes)
    .bind(critical_bytes)
    .bind(warn_action.to_string())
    .bind(critical_action.to_string())
    .bind(enabled)
    .fetch_one(pool)
    .await
}

pub async fn get_quota(pool: &PgPool, repo_id: i64) -> Result<Option<RepoQuota>, sqlx::Error> {
    sqlx::query_as::<_, RepoQuota>(
        "SELECT repo_id, warn_bytes, critical_bytes, warn_action, critical_action, enabled, \
         updated_at FROM repo_quotas WHERE repo_id = $1",
    )
    .bind(repo_id)
    .fetch_optional(pool)
    .await
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

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ServerQuota {
    pub ssh_host: String,
    pub warn_bytes: Option<i64>,
    pub critical_bytes: Option<i64>,
    pub warn_action: String,
    pub critical_action: String,
    pub enabled: bool,
    pub updated_at: DateTime<Utc>,
}

impl ServerQuota {
    #[must_use]
    pub fn warn_action_parsed(&self) -> QuotaAction {
        self.warn_action.parse().unwrap_or(QuotaAction::NotifyOnly)
    }

    #[must_use]
    pub fn critical_action_parsed(&self) -> QuotaAction {
        self.critical_action
            .parse()
            .unwrap_or(QuotaAction::NotifyOnly)
    }
}

pub async fn upsert_server_quota(
    pool: &PgPool,
    ssh_host: &str,
    warn_bytes: Option<i64>,
    critical_bytes: Option<i64>,
    warn_action: QuotaAction,
    critical_action: QuotaAction,
    enabled: bool,
) -> Result<ServerQuota, sqlx::Error> {
    sqlx::query_as::<_, ServerQuota>(
        "INSERT INTO server_quotas (ssh_host, warn_bytes, critical_bytes, warn_action, \
         critical_action, enabled, updated_at) VALUES ($1, $2, $3, $4, $5, $6, NOW()) ON CONFLICT \
         (ssh_host) DO UPDATE SET warn_bytes = EXCLUDED.warn_bytes, critical_bytes = \
         EXCLUDED.critical_bytes, warn_action = EXCLUDED.warn_action, critical_action = \
         EXCLUDED.critical_action, enabled = EXCLUDED.enabled, updated_at = NOW() RETURNING \
         ssh_host, warn_bytes, critical_bytes, warn_action, critical_action, enabled, updated_at",
    )
    .bind(ssh_host)
    .bind(warn_bytes)
    .bind(critical_bytes)
    .bind(warn_action.to_string())
    .bind(critical_action.to_string())
    .bind(enabled)
    .fetch_one(pool)
    .await
}

pub async fn get_server_quota(
    pool: &PgPool,
    ssh_host: &str,
) -> Result<Option<ServerQuota>, sqlx::Error> {
    sqlx::query_as::<_, ServerQuota>(
        "SELECT ssh_host, warn_bytes, critical_bytes, warn_action, critical_action, enabled, \
         updated_at FROM server_quotas WHERE ssh_host = $1",
    )
    .bind(ssh_host)
    .fetch_optional(pool)
    .await
}

pub async fn list_server_quotas(pool: &PgPool) -> Result<Vec<ServerQuota>, sqlx::Error> {
    sqlx::query_as::<_, ServerQuota>(
        "SELECT ssh_host, warn_bytes, critical_bytes, warn_action, critical_action, enabled, \
         updated_at FROM server_quotas ORDER BY ssh_host",
    )
    .fetch_all(pool)
    .await
}

pub async fn delete_server_quota(pool: &PgPool, ssh_host: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM server_quotas WHERE ssh_host = $1")
        .bind(ssh_host)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn get_server_total_size(pool: &PgPool, ssh_host: &str) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(info_deduplicated_size), 0) FROM repos WHERE ssh_host = $1",
    )
    .bind(ssh_host)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

pub fn evaluate_server_quota(quota: &ServerQuota, total_size: i64) -> QuotaStatus {
    if !quota.enabled {
        return QuotaStatus::Ok;
    }

    if quota
        .critical_bytes
        .is_some_and(|limit| total_size >= limit)
    {
        return QuotaStatus::Critical;
    }

    if quota.warn_bytes.is_some_and(|limit| total_size >= limit) {
        return QuotaStatus::Warning;
    }

    QuotaStatus::Ok
}

pub async fn list_ssh_hosts(pool: &PgPool) -> Result<Vec<String>, ApiError> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT DISTINCT ssh_host FROM repos ORDER BY ssh_host")
            .fetch_all(pool)
            .await
            .map_err(ApiError::Database)?;
    Ok(rows.into_iter().map(|(h,)| h).collect())
}
