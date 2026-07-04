// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared::types::QuotaAction;
use sqlx::PgPool;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuotaStatus {
    Ok,
    Warning,
    Critical,
}

/// Shared warn/critical threshold comparison used by both repo and server quotas.
pub fn evaluate_thresholds(
    enabled: bool,
    warn_bytes: Option<i64>,
    critical_bytes: Option<i64>,
    usage_bytes: i64,
) -> QuotaStatus {
    if !enabled {
        return QuotaStatus::Ok;
    }

    if critical_bytes.is_some_and(|limit| usage_bytes >= limit) {
        return QuotaStatus::Critical;
    }

    if warn_bytes.is_some_and(|limit| usage_bytes >= limit) {
        return QuotaStatus::Warning;
    }

    QuotaStatus::Ok
}

pub fn evaluate_quota(quota: &RepoQuota, deduplicated_size: i64) -> QuotaStatus {
    evaluate_thresholds(
        quota.enabled,
        quota.warn_bytes,
        quota.critical_bytes,
        deduplicated_size,
    )
}

impl RepoQuota {
    #[must_use]
    pub fn status(&self, deduplicated_size: i64) -> QuotaStatus {
        evaluate_quota(self, deduplicated_size)
    }

    /// Action configured for the given breach status, or `None` when the quota is not breached.
    #[must_use]
    pub fn action_for(&self, status: QuotaStatus) -> Option<QuotaAction> {
        match status {
            QuotaStatus::Ok => None,
            QuotaStatus::Warning => Some(self.warn_action.parse().unwrap_or_default()),
            QuotaStatus::Critical => Some(self.critical_action.parse().unwrap_or_default()),
        }
    }
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
    sqlx::query_as!(
        RepoQuota,
        r#"
        INSERT INTO repo_quotas
            (repo_id, warn_bytes, critical_bytes, warn_action, critical_action, enabled, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, NOW())
        ON CONFLICT (repo_id) DO UPDATE
        SET warn_bytes = EXCLUDED.warn_bytes,
            critical_bytes = EXCLUDED.critical_bytes,
            warn_action = EXCLUDED.warn_action,
            critical_action = EXCLUDED.critical_action,
            enabled = EXCLUDED.enabled,
            updated_at = NOW()
        RETURNING
            repo_id, warn_bytes, critical_bytes, warn_action, critical_action, enabled, updated_at
        "#,
        repo_id,
        warn_bytes,
        critical_bytes,
        warn_action.to_string(),
        critical_action.to_string(),
        enabled,
    )
    .fetch_one(pool)
    .await
}

pub async fn get_quota(pool: &PgPool, repo_id: i64) -> Result<Option<RepoQuota>, sqlx::Error> {
    let quota = sqlx::query_as!(
        RepoQuota,
        "SELECT repo_id, warn_bytes, critical_bytes, warn_action, critical_action, enabled, \
         updated_at FROM repo_quotas WHERE repo_id = $1",
        repo_id,
    )
    .fetch_optional(pool)
    .await?;

    Ok(quota)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_for_ok_is_none() {
        let quota = RepoQuota {
            repo_id: 1,
            warn_bytes: Some(100),
            critical_bytes: Some(200),
            warn_action: "block_backups".to_owned(),
            critical_action: "disable_schedule".to_owned(),
            enabled: true,
            updated_at: Utc::now(),
        };
        assert_eq!(quota.action_for(QuotaStatus::Ok), None);
    }

    #[test]
    fn action_for_warning_and_critical_parse_configured_action() {
        let quota = RepoQuota {
            repo_id: 1,
            warn_bytes: Some(100),
            critical_bytes: Some(200),
            warn_action: "block_backups".to_owned(),
            critical_action: "disable_schedule".to_owned(),
            enabled: true,
            updated_at: Utc::now(),
        };
        assert_eq!(
            quota.action_for(QuotaStatus::Warning),
            Some(QuotaAction::BlockBackups)
        );
        assert_eq!(
            quota.action_for(QuotaStatus::Critical),
            Some(QuotaAction::DisableSchedule)
        );
    }
}
