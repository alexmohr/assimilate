// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use chrono::{DateTime, Utc};
use shared::{
    protocol::{ServerToAgent, ServerToUi},
    schedule::calculate_next_run,
    types::{RepoId, ScheduleType},
};
use sqlx::PgPool;

use crate::{
    api::repos::sync_new_archives,
    config_assembler, db,
    tunnel::TunnelManager,
    ws::{registry::AgentRegistry, ui_broadcast::UiBroadcast},
};

const TICK_INTERVAL: Duration = Duration::from_secs(30);
const RETENTION_INTERVAL: Duration = Duration::from_secs(3600);
const SYNC_CHECK_INTERVAL: Duration = Duration::from_secs(60);
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
            if let Err(e) = tick(&schedule_pool, &registry, &encryption_key, &tunnel_manager).await
            {
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
        let mut interval = tokio::time::interval(SYNC_CHECK_INTERVAL);
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

    let tz = db::get_schedule_timezone(pool)
        .await
        .unwrap_or(chrono_tz::Tz::UTC);
    let now = Utc::now();

    let importing_ids: std::collections::HashSet<i64> =
        match db::list_importing_repo_ids(pool).await {
            Ok(ids) => ids.into_iter().collect(),
            Err(e) => {
                tracing::error!(error = %e, "failed to list importing repos for sync guard");
                return;
            }
        };

    for repo in repos {
        if !repo.enabled {
            continue;
        }

        if importing_ids.contains(&repo.id) {
            tracing::debug!(
                repo_id = repo.id,
                "skipping scheduled sync: import in progress"
            );
            continue;
        }

        let Some(ref cron_expr) = repo.sync_schedule else {
            continue;
        };

        let from = repo.last_synced_at.unwrap_or(DateTime::UNIX_EPOCH);
        let next_run = match calculate_next_run(cron_expr, from, tz) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(
                    repo_id = repo.id,
                    cron = %cron_expr,
                    error = %e,
                    "invalid sync_schedule cron, skipping"
                );
                continue;
            }
        };

        if next_run > now {
            continue;
        }

        if let Err(e) = db::set_repo_importing(pool, repo.id, true).await {
            tracing::error!(repo_id = repo.id, error = %e, "failed to set importing flag for scheduled sync");
            continue;
        }

        let start = std::time::Instant::now();
        match sync_new_archives(pool, encryption_key, repo.id, ui_broadcast).await {
            Ok((added, removed)) => {
                let elapsed = start.elapsed();
                let duration_secs = elapsed.as_secs();

                if let Err(e) = db::update_repo_last_synced(pool, repo.id).await {
                    tracing::error!(repo_id = repo.id, error = %e, "failed to update last_synced_at");
                }

                if let Err(e) = db::set_repo_importing(pool, repo.id, false).await {
                    tracing::error!(repo_id = repo.id, error = %e, "failed to clear importing flag after sync");
                }
                if let Err(e) = db::set_repo_import_error(pool, repo.id, None).await {
                    tracing::error!(repo_id = repo.id, error = %e, "failed to clear import_error after sync");
                }

                if added > 0 || removed > 0 {
                    let msg = format!(
                        "periodic sync for '{}': added {added}, removed {removed} archives in \
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
                if let Err(e) = db::set_repo_importing(pool, repo.id, false).await {
                    tracing::error!(repo_id = repo.id, error = %e, "failed to clear importing flag after NotFound");
                }
            }
            Err(e) => {
                let elapsed = start.elapsed();
                let msg = format!(
                    "periodic sync failed for '{}' after {:.1}s: {e}",
                    repo.name,
                    elapsed.as_secs_f64()
                );
                tracing::error!("{msg}");
                if let Err(log_err) =
                    db::insert_system_event(pool, "repo_sync_failed", None, &msg).await
                {
                    tracing::error!(error = %log_err, "failed to log sync event");
                }
                if let Err(e2) = db::set_repo_importing(pool, repo.id, false).await {
                    tracing::error!(repo_id = repo.id, error = %e2, "failed to clear import flag");
                }
                if let Err(e2) =
                    db::set_repo_import_error(pool, repo.id, Some(&format!("{e}"))).await
                {
                    tracing::error!(repo_id = repo.id, error = %e2, "failed to set import_error");
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
    encryption_key: &[u8; 32],
    tunnel_manager: &TunnelManager,
) -> Result<(), crate::error::ApiError> {
    let now = Utc::now();
    let due = db::list_due_schedules(pool, now).await?;

    if due.is_empty() {
        return Ok(());
    }

    let tz = db::get_schedule_timezone(pool).await?;

    let mut triggered_schedules = std::collections::HashSet::new();

    for schedule in &due {
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
                schedule_id: Some(schedule.schedule_id),
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

        // Push the current config before triggering so the agent always runs
        // with the latest global excludes, not a potentially stale cached version.
        match config_assembler::assemble_config(pool, encryption_key, &schedule.hostname).await {
            Ok(config) => {
                let config_msg = ServerToAgent::ConfigUpdate(config);
                if let Err(e) = registry.send_to(&schedule.hostname, config_msg).await {
                    tracing::warn!(
                        hostname = %schedule.hostname,
                        error = %e,
                        "agent not connected for pre-run config push, skipping trigger"
                    );
                    continue;
                }
            }
            Err(e) => {
                tracing::warn!(
                    hostname = %schedule.hostname,
                    error = %e,
                    "failed to assemble config before scheduled run, skipping trigger"
                );
                continue;
            }
        }

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

        triggered_schedules.insert(schedule.schedule_id);
    }

    for schedule_id in &triggered_schedules {
        let cron = due
            .iter()
            .find(|s| s.schedule_id == *schedule_id)
            .map(|s| s.cron_expression.as_str())
            .unwrap_or_default();
        let next = calculate_next_run(cron, now, tz)
            .map_err(|e| crate::error::ApiError::Internal(format!("cron error: {e}")))?;
        db::mark_schedule_triggered(pool, *schedule_id, now, next).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use tokio::sync::mpsc;

    use super::*;
    use crate::{
        db::{self, InsertRepoParams, ScheduleParams},
        tunnel::TunnelManager,
        ws::{registry::AgentRegistry, ui_broadcast::UiBroadcast},
    };

    #[test]
    fn sync_due_when_next_run_in_past() {
        let cron_expr = "0 0,12 * * *";
        let last_synced = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 13, 0, 0).unwrap();
        let tz = chrono_tz::Tz::UTC;

        let next = calculate_next_run(cron_expr, last_synced, tz).unwrap();
        assert!(next <= now);
    }

    #[test]
    fn sync_not_due_when_next_run_in_future() {
        let cron_expr = "0 0,12 * * *";
        let last_synced = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 11, 0, 0).unwrap();
        let tz = chrono_tz::Tz::UTC;

        let next = calculate_next_run(cron_expr, last_synced, tz).unwrap();
        assert!(next > now);
    }

    #[test]
    fn sync_due_when_never_synced() {
        let cron_expr = "0 0,12 * * *";
        let last_synced = DateTime::UNIX_EPOCH;
        let now = Utc::now();
        let tz = chrono_tz::Tz::UTC;

        let next = calculate_next_run(cron_expr, last_synced, tz).unwrap();
        assert!(next <= now);
    }

    // tick() integration tests
    // Run with:
    //   DATABASE_URL=postgres://borg:borg_secret@localhost:5432/borg \
    //     cargo test -p server --test-threads=1

    const TICK_TEST_HOSTNAME: &str = "tick-test-agent";
    const TICK_TEST_KEY_MATERIAL: &[u8] = b"tick-test-scheduler-secret-key";

    fn tick_test_key() -> [u8; 32] {
        shared::crypto::derive_key(TICK_TEST_KEY_MATERIAL)
    }

    fn dummy_tunnel(pool: sqlx::PgPool) -> TunnelManager {
        TunnelManager::new(pool, UiBroadcast::new(), "127.0.0.1:0".parse().unwrap())
    }

    async fn setup_due_schedule(pool: &sqlx::PgPool, key: &[u8; 32]) -> (i64, i64) {
        let passphrase_enc = shared::crypto::encrypt_passphrase("test-pass", key).unwrap();
        let client = db::insert_client(pool, TICK_TEST_HOSTNAME, None, "hash", None)
            .await
            .unwrap();
        let repo = db::insert_repo(
            pool,
            &InsertRepoParams {
                name: "tick-repo",
                repo_path: "/backup/tick",
                ssh_user: "borg",
                ssh_host: "host.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_enc,
                compression: "lz4",
                encryption: "none",
                owner_id: None,
            },
        )
        .await
        .unwrap();
        let schedule = db::insert_schedule(
            pool,
            repo.id,
            &ScheduleParams {
                name: "tick-sched",
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                exclude_patterns_raw: "",
                ignore_global_excludes: false,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 0,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: "[]",
                post_backup_commands: "[]",
                execution_mode: "parallel",
                on_failure: "stop",
            },
            None,
        )
        .await
        .unwrap();
        db::insert_schedule_targets(pool, schedule.id, &[(client.id, 0)])
            .await
            .unwrap();
        let past = Utc::now() - chrono::Duration::hours(1);
        db::set_next_run_at(pool, schedule.id, past).await.unwrap();
        (repo.id, schedule.id)
    }

    async fn register_fake_agent(
        registry: &AgentRegistry,
    ) -> mpsc::Receiver<shared::protocol::ServerToAgent> {
        let (tx, rx) = mpsc::channel(32);
        registry
            .register(TICK_TEST_HOSTNAME.to_owned(), tx, false, None)
            .await;
        rx
    }

    /// tick() must send ConfigUpdate *before* the run trigger so the agent
    /// always executes with the current config.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn tick_sends_config_update_before_run_trigger(pool: sqlx::PgPool) {
        let key = tick_test_key();
        let (repo_id, _) = setup_due_schedule(&pool, &key).await;

        let registry = AgentRegistry::new();
        let mut rx = register_fake_agent(&registry).await;
        let tunnel = dummy_tunnel(pool.clone());

        tick(&pool, &registry, &key, &tunnel).await.unwrap();

        let first = rx
            .try_recv()
            .expect("expected ConfigUpdate as first message");
        assert!(
            matches!(first, shared::protocol::ServerToAgent::ConfigUpdate(_)),
            "first message must be ConfigUpdate, got: {first:?}"
        );

        let second = rx
            .try_recv()
            .expect("expected RunBackupNow as second message");
        match second {
            shared::protocol::ServerToAgent::RunBackupNow { repo_id: rid, .. } => {
                assert_eq!(rid.0, repo_id, "RunBackupNow repo_id mismatch");
            }
            other => panic!("expected RunBackupNow, got: {other:?}"),
        }
    }

    /// ConfigUpdate sent before each trigger must reflect the *current* global
    /// excludes, not those that were in place when the schedule was created.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn tick_config_carries_updated_global_excludes(pool: sqlx::PgPool) {
        let key = tick_test_key();
        setup_due_schedule(&pool, &key).await;

        // Set global excludes raw text; tick must deliver the current value.
        db::set_global_excludes_raw(&pool, "*.tmp").await.unwrap();

        let registry = AgentRegistry::new();
        let mut rx = register_fake_agent(&registry).await;
        let tunnel = dummy_tunnel(pool.clone());

        tick(&pool, &registry, &key, &tunnel).await.unwrap();

        let msg = rx.try_recv().expect("expected ConfigUpdate");
        match msg {
            shared::protocol::ServerToAgent::ConfigUpdate(config) => {
                let all_excludes: Vec<_> = config
                    .repos
                    .iter()
                    .flat_map(|r| r.schedules.iter())
                    .flat_map(|s| s.exclude_patterns.iter().cloned())
                    .collect();
                assert!(
                    all_excludes.iter().any(|p| p == "*.tmp"),
                    "exclude '*.tmp' missing; got: {all_excludes:?}"
                );
                assert!(
                    !all_excludes.iter().any(|p| p == "*.log"),
                    "stale exclude '*.log' present; got: {all_excludes:?}"
                );
            }
            other => panic!("expected ConfigUpdate, got: {other:?}"),
        }
    }

    /// When the target agent is not connected, tick() must not error and must
    /// leave the schedule in due state (not mark it as triggered).
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn tick_skips_trigger_gracefully_when_agent_disconnected(pool: sqlx::PgPool) {
        let key = tick_test_key();
        let (_, schedule_id) = setup_due_schedule(&pool, &key).await;

        let registry = AgentRegistry::new(); // no agent registered
        let tunnel = dummy_tunnel(pool.clone());

        tick(&pool, &registry, &key, &tunnel).await.unwrap();

        let due = db::list_due_schedules(&pool, Utc::now()).await.unwrap();
        assert!(
            due.iter().any(|s| s.schedule_id == schedule_id),
            "schedule must remain due when the agent was not connected"
        );
    }
}
