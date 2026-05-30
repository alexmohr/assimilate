// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use chrono::Utc;
use shared::{
    protocol::{ServerToAgent, ServerToUi},
    schedule::calculate_next_run,
    types::{RepoId, ScheduleType},
};
use sqlx::PgPool;

use crate::{
    api::repos::sync_existing_archives,
    db,
    tunnel::TunnelManager,
    ws::{registry::AgentRegistry, ui_broadcast::UiBroadcast},
};

const TICK_INTERVAL: Duration = Duration::from_secs(30);
const RETENTION_INTERVAL: Duration = Duration::from_secs(3600);
const SYNC_INTERVAL: Duration = Duration::from_secs(900);
const SYNC_WARN_DURATION: Duration = Duration::from_secs(300);
const DEFAULT_RETENTION_DAYS: i64 = 7;

pub async fn run(
    pool: PgPool,
    registry: AgentRegistry,
    encryption_key: [u8; 32],
    ui_broadcast: UiBroadcast,
    tunnel_manager: TunnelManager,
) {
    let schedule_pool = pool.clone();
    let retention_pool = pool.clone();
    let sync_pool = pool;

    let schedule_task = async move {
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        loop {
            interval.tick().await;
            if let Err(e) = tick(&schedule_pool, &registry, &tunnel_manager).await {
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

    let sync_task = async {
        let mut interval = tokio::time::interval(SYNC_INTERVAL);
        loop {
            interval.tick().await;
            run_repo_sync(&sync_pool, &encryption_key, &ui_broadcast).await;
        }
    };

    tokio::join!(schedule_task, retention_task, sync_task);
}

async fn run_repo_sync(pool: &PgPool, encryption_key: &[u8; 32], ui_broadcast: &UiBroadcast) {
    let repos = match db::list_all_repos(pool).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "failed to list repos for sync");
            return;
        }
    };

    for repo in repos {
        if !repo.enabled {
            continue;
        }

        let start = std::time::Instant::now();
        match sync_existing_archives(pool, encryption_key, repo.id).await {
            Ok(imported) => {
                let elapsed = start.elapsed();
                let duration_secs = elapsed.as_secs();

                if let Err(e) = db::update_repo_last_synced(pool, repo.id).await {
                    tracing::error!(repo_id = repo.id, error = %e, "failed to update last_synced_at");
                }

                if imported > 0 {
                    let msg = format!(
                        "periodic sync for '{}': imported {imported} new archives in \
                         {duration_secs}s",
                        repo.name
                    );
                    tracing::info!("{msg}");
                    if let Err(e) = db::insert_system_event(pool, "repo_sync", None, &msg).await {
                        tracing::error!(error = %e, "failed to log sync event");
                    }
                    ui_broadcast.send(ServerToUi::DataChanged);
                }

                if elapsed > SYNC_WARN_DURATION {
                    let msg = format!(
                        "periodic sync for '{}' took {duration_secs}s (exceeds {}s threshold)",
                        repo.name,
                        SYNC_WARN_DURATION.as_secs()
                    );
                    tracing::error!("{msg}");
                    if let Err(e) =
                        db::insert_system_event(pool, "repo_sync_slow", None, &msg).await
                    {
                        tracing::error!(error = %e, "failed to log slow sync event");
                    }
                }
            }
            Err(crate::error::ApiError::NotFound(ref reason)) => {
                tracing::warn!(
                    repo_id = repo.id,
                    repo_name = %repo.name,
                    reason = %reason,
                    "skipping sync for repo that no longer exists"
                );
            }
            Err(e) => {
                let elapsed = start.elapsed();
                let msg = format!(
                    "periodic sync failed for '{}' after {:.1}s: {e}",
                    repo.name,
                    elapsed.as_secs_f64()
                );
                tracing::error!("{msg}");
                if let Err(log_err) = db::insert_system_event(pool, "repo_sync", None, &msg).await {
                    tracing::error!(error = %log_err, "failed to log sync event");
                }
            }
        }
    }
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

async fn tick(
    pool: &PgPool,
    registry: &AgentRegistry,
    tunnel_manager: &TunnelManager,
) -> Result<(), crate::error::ApiError> {
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

        tunnel_manager
            .ensure_client_tunnel_connected(schedule.client_id)
            .await;

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
