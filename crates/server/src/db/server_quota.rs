// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use chrono::{DateTime, Utc};
use serde::Serialize;
use shared::types::QuotaAction;
use sqlx::PgPool;

use super::quota::{QuotaStatus, evaluate_thresholds};

/// Storage quota shared across every repo whose `ssh_host` matches, for the case where
/// multiple repositories reside on one server with a shared disk quota.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ServerQuota {
    /// SSH hostname shared by repos on the same server.
    pub ssh_host: String,
    /// Warn threshold in bytes.
    pub warn_bytes: Option<i64>,
    /// Critical threshold in bytes.
    pub critical_bytes: Option<i64>,
    /// Action to take when the warn threshold is breached.
    pub warn_action: String,
    /// Action to take when the critical threshold is breached.
    pub critical_action: String,
    /// Whether this quota is enforced.
    pub enabled: bool,
    /// When this quota was last updated.
    pub updated_at: DateTime<Utc>,
}

impl ServerQuota {
    /// Current quota status for the given total deduplicated size across the host's repos.
    #[must_use]
    pub fn status(&self, total_deduplicated_size: i64) -> QuotaStatus {
        evaluate_thresholds(
            self.enabled,
            self.warn_bytes,
            self.critical_bytes,
            total_deduplicated_size,
        )
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

/// A distinct `ssh_host` shared by one or more repos, joined with its (optional)
/// `server_quotas` configuration and current aggregated usage across those repos.
#[derive(Debug, Clone)]
pub struct ServerQuotaWithUsage {
    /// SSH hostname.
    pub ssh_host: String,
    /// Number of repos on this host.
    pub repo_count: i64,
    /// Combined deduplicated size across all repos on this host.
    pub total_deduplicated_size: i64,
    /// Server-level quota configuration, if any.
    pub quota: Option<ServerQuota>,
}

/// # Errors
///
/// Returns an error if the database query fails.
pub async fn upsert_server_quota(
    pool: &PgPool,
    ssh_host: &str,
    warn_bytes: Option<i64>,
    critical_bytes: Option<i64>,
    warn_action: QuotaAction,
    critical_action: QuotaAction,
    enabled: bool,
) -> Result<ServerQuota, sqlx::Error> {
    sqlx::query_as!(
        ServerQuota,
        r#"
        INSERT INTO server_quotas
            (ssh_host, warn_bytes, critical_bytes, warn_action, critical_action, enabled,
             updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, NOW())
        ON CONFLICT (ssh_host) DO UPDATE
        SET warn_bytes = EXCLUDED.warn_bytes,
            critical_bytes = EXCLUDED.critical_bytes,
            warn_action = EXCLUDED.warn_action,
            critical_action = EXCLUDED.critical_action,
            enabled = EXCLUDED.enabled,
            updated_at = NOW()
        RETURNING
            ssh_host, warn_bytes, critical_bytes, warn_action, critical_action, enabled, updated_at
        "#,
        ssh_host,
        warn_bytes,
        critical_bytes,
        warn_action.to_string(),
        critical_action.to_string(),
        enabled,
    )
    .fetch_one(pool)
    .await
}

/// # Errors
///
/// Returns an error if the database query fails.
pub async fn get_server_quota(
    pool: &PgPool,
    ssh_host: &str,
) -> Result<Option<ServerQuota>, sqlx::Error> {
    sqlx::query_as!(
        ServerQuota,
        "SELECT ssh_host, warn_bytes, critical_bytes, warn_action, critical_action, enabled, \
         updated_at FROM server_quotas WHERE ssh_host = $1",
        ssh_host,
    )
    .fetch_optional(pool)
    .await
}

/// Returns `true` if a quota row existed and was deleted.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn delete_server_quota(pool: &PgPool, ssh_host: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM server_quotas WHERE ssh_host = $1", ssh_host)
        .execute(pool)
        .await?;

    Ok(result.rows_affected() > 0)
}

/// Every distinct `ssh_host` that hosts at least one repo, together with the number of
/// repos on that host, their combined deduplicated size, and the quota configured for it
/// (if any).
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn list_server_quotas_with_usage(
    pool: &PgPool,
) -> Result<Vec<ServerQuotaWithUsage>, sqlx::Error> {
    #[derive(sqlx::FromRow)]
    struct Row {
        ssh_host: String,
        repo_count: i64,
        total_deduplicated_size: i64,
        warn_bytes: Option<i64>,
        critical_bytes: Option<i64>,
        warn_action: Option<String>,
        critical_action: Option<String>,
        enabled: Option<bool>,
        updated_at: Option<DateTime<Utc>>,
    }

    let rows = sqlx::query_as!(
        Row,
        r#"
        SELECT
            r.ssh_host AS "ssh_host!",
            COUNT(DISTINCT r.id) AS "repo_count!",
            COALESCE(SUM(rs.deduplicated_size)::bigint, 0) AS "total_deduplicated_size!",
            sq.warn_bytes AS "warn_bytes?",
            sq.critical_bytes AS "critical_bytes?",
            sq.warn_action AS "warn_action?",
            sq.critical_action AS "critical_action?",
            sq.enabled AS "enabled?",
            sq.updated_at AS "updated_at?"
        FROM repos r
        LEFT JOIN repo_stats rs ON rs.repo_id = r.id
        LEFT JOIN server_quotas sq ON sq.ssh_host = r.ssh_host
        GROUP BY
            r.ssh_host, sq.warn_bytes, sq.critical_bytes, sq.warn_action, sq.critical_action,
            sq.enabled, sq.updated_at
        ORDER BY r.ssh_host
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let ssh_host = row.ssh_host;
            let quota = match (
                row.warn_action,
                row.critical_action,
                row.enabled,
                row.updated_at,
            ) {
                (Some(warn_action), Some(critical_action), Some(enabled), Some(updated_at)) => {
                    Some(ServerQuota {
                        ssh_host: ssh_host.clone(),
                        warn_bytes: row.warn_bytes,
                        critical_bytes: row.critical_bytes,
                        warn_action,
                        critical_action,
                        enabled,
                        updated_at,
                    })
                }
                _ => None,
            };
            ServerQuotaWithUsage {
                ssh_host,
                repo_count: row.repo_count,
                total_deduplicated_size: row.total_deduplicated_size,
                quota,
            }
        })
        .collect())
}

/// Total deduplicated size across every repo sharing `ssh_host`, from the authoritative
/// `repo_stats` snapshot (never derived from `backup_reports`).
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn total_deduplicated_size_for_ssh_host(
    pool: &PgPool,
    ssh_host: &str,
) -> Result<i64, sqlx::Error> {
    #[derive(sqlx::FromRow)]
    struct Row {
        total: i64,
    }

    let row = sqlx::query_as!(
        Row,
        r#"
        SELECT COALESCE(SUM(rs.deduplicated_size)::bigint, 0) AS "total!"
        FROM repos r
        LEFT JOIN repo_stats rs ON rs.repo_id = r.id
        WHERE r.ssh_host = $1
        "#,
        ssh_host,
    )
    .fetch_one(pool)
    .await?;

    Ok(row.total)
}

/// Total deduplicated size across every repo sharing `ssh_host` *other than* `exclude_repo_id`,
/// from the authoritative `repo_stats` snapshot. Used to combine a just-completed backup's own
/// (fresh) `report.deduplicated_size` with its sibling repos' (possibly stale, since
/// `repo_stats` is only refreshed by a sync/rescan) snapshot, so a quota breach on an otherwise
/// idle host is detected immediately rather than only after an unrelated rescan.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn total_deduplicated_size_for_ssh_host_excluding(
    pool: &PgPool,
    ssh_host: &str,
    exclude_repo_id: i64,
) -> Result<i64, sqlx::Error> {
    #[derive(sqlx::FromRow)]
    struct Row {
        total: i64,
    }

    let row = sqlx::query_as!(
        Row,
        r#"
        SELECT COALESCE(SUM(rs.deduplicated_size)::bigint, 0) AS "total!"
        FROM repos r
        LEFT JOIN repo_stats rs ON rs.repo_id = r.id
        WHERE r.ssh_host = $1 AND r.id != $2
        "#,
        ssh_host,
        exclude_repo_id,
    )
    .fetch_one(pool)
    .await?;

    Ok(row.total)
}

/// Number of repos sharing `ssh_host`.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn repo_count_for_ssh_host(pool: &PgPool, ssh_host: &str) -> Result<i64, sqlx::Error> {
    #[derive(sqlx::FromRow)]
    struct Row {
        count: i64,
    }

    let row = sqlx::query_as!(
        Row,
        r#"SELECT COUNT(*) AS "count!" FROM repos WHERE ssh_host = $1"#,
        ssh_host,
    )
    .fetch_one(pool)
    .await?;

    Ok(row.count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_for_ok_is_none() {
        let quota = ServerQuota {
            ssh_host: "backup.example.com".to_owned(),
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
        let quota = ServerQuota {
            ssh_host: "backup.example.com".to_owned(),
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

    #[test]
    fn disabled_quota_is_always_ok() {
        let quota = ServerQuota {
            ssh_host: "backup.example.com".to_owned(),
            warn_bytes: Some(100),
            critical_bytes: Some(200),
            warn_action: "block_backups".to_owned(),
            critical_action: "block_backups".to_owned(),
            enabled: false,
            updated_at: Utc::now(),
        };
        assert_eq!(quota.status(1_000_000), QuotaStatus::Ok);
    }
}
