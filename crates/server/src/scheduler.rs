// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use chrono::Utc;
use shared::{
    protocol::ServerToAgent,
    schedule::calculate_next_run,
    types::{RepoId, ScheduleType},
};
use sqlx::PgPool;

use crate::{db, ws::registry::AgentRegistry};

const TICK_INTERVAL: Duration = Duration::from_secs(30);
const RETENTION_INTERVAL: Duration = Duration::from_secs(3600);
const DEFAULT_RETENTION_DAYS: i64 = 7;

pub async fn run(pool: PgPool, registry: AgentRegistry) {
    let schedule_pool = pool.clone();
    let retention_pool = pool;

    let schedule_task = async move {
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        loop {
            interval.tick().await;
            if let Err(e) = tick(&schedule_pool, &registry).await {
                tracing::error!(error = %e, "scheduler tick failed");
            }
        }
    };

    let retention_task = async move {
        let mut interval = tokio::time::interval(RETENTION_INTERVAL);
        loop {
            interval.tick().await;
            if let Err(e) = run_retention_cleanup(&retention_pool).await {
                tracing::error!(error = %e, "retention cleanup failed");
            }
        }
    };

    tokio::join!(schedule_task, retention_task);
}

async fn run_retention_cleanup(pool: &PgPool) -> Result<(), crate::error::ApiError> {
    let retention_days = db::get_setting(pool, "retention_days")
        .await?
        .and_then(|v| {
            v.parse::<i64>().inspect_err(|e| {
                tracing::warn!(value = %v, error = %e, "failed to parse retention_days setting");
            }).ok()
        })
        .unwrap_or(DEFAULT_RETENTION_DAYS);

    if retention_days <= 0 {
        return Ok(());
    }

    let cutoff = Utc::now() - chrono::Duration::days(retention_days);

    let events_deleted = db::delete_system_events_before(pool, cutoff).await?;
    let reports_deleted = db::delete_backup_reports_before(pool, cutoff).await?;

    if events_deleted > 0 || reports_deleted > 0 {
        tracing::info!(
            events_deleted,
            reports_deleted,
            retention_days,
            "retention cleanup completed"
        );
    }

    Ok(())
}

async fn tick(pool: &PgPool, registry: &AgentRegistry) -> Result<(), crate::error::ApiError> {
    let now = Utc::now();
    let due = db::list_due_schedules(pool, now).await?;

    if due.is_empty() {
        return Ok(());
    }

    let tz = db::get_schedule_timezone(pool).await?;

    for schedule in due {
        let repo_id = RepoId(schedule.repo_id);

        let Ok(schedule_type) = schedule.schedule_type.parse::<ScheduleType>() else {
            tracing::error!(
                schedule_id = schedule.schedule_id,
                schedule_type = %schedule.schedule_type,
                "invalid schedule type in database, skipping"
            );
            continue;
        };

        let msg = match schedule_type {
            ScheduleType::Check => ServerToAgent::RunCheckNow {
                repo_id,
                request_id: None,
            },
            ScheduleType::Verify => ServerToAgent::RunVerifyNow {
                repo_id,
                request_id: None,
            },
            ScheduleType::Backup => ServerToAgent::RunBackupNow {
                repo_id,
                request_id: None,
            },
        };

        let action = match schedule_type {
            ScheduleType::Check => "check",
            ScheduleType::Verify => "verify",
            ScheduleType::Backup => "backup",
        };

        match registry.send_to(&schedule.hostname, msg).await {
            Ok(()) => {
                tracing::info!(
                    hostname = %schedule.hostname,
                    repo_id = schedule.repo_id,
                    action,
                    "triggered schedule"
                );
            }
            Err(e) => {
                tracing::warn!(
                    hostname = %schedule.hostname,
                    repo_id = schedule.repo_id,
                    action,
                    error = %e,
                    "agent not connected, skipping trigger"
                );
                continue;
            }
        }

        let next = calculate_next_run(&schedule.cron_expression, now, tz)
            .map_err(|e| crate::error::ApiError::Internal(format!("cron error: {e}")))?;
        db::mark_schedule_triggered(pool, schedule.schedule_id, now, next).await?;
    }

    Ok(())
}
