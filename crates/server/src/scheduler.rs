// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use shared::{
    protocol::{ServerToAgent, ServerToUi},
    schedule::calculate_next_run,
    types::{OnFailure, RepoId, ScheduleType},
};
use sqlx::PgPool;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::{
    api::repos::sync_existing_archives,
    config_assembler, db,
    db::DueScheduleRow,
    tunnel::TunnelManager,
    ws::{completion_bus::CompletionBus, registry::AgentRegistry, ui_broadcast::UiBroadcast},
};

#[derive(Clone, Default)]
struct RepoLock {
    locks: Arc<tokio::sync::Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>>,
}

impl RepoLock {
    async fn acquire(&self, repo_id: i64) -> tokio::sync::OwnedMutexGuard<()> {
        let mutex = {
            let mut map = self.locks.lock().await;
            Arc::clone(
                map.entry(repo_id)
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
            )
        };
        mutex.lock_owned().await
    }
}

const TICK_INTERVAL: Duration = Duration::from_secs(30);
const RETENTION_INTERVAL: Duration = Duration::from_secs(3600);
const SYNC_CHECK_INTERVAL: Duration = Duration::from_secs(60);
const SYNC_WARN_DURATION: Duration = Duration::from_secs(300);
const DEFAULT_RETENTION_DAYS: i64 = 7;
const SEQUENTIAL_TIMEOUT: Duration = Duration::from_secs(4 * 3600);

pub async fn run(
    pool: PgPool,
    registry: AgentRegistry,
    encryption_key: [u8; 32],
    ui_broadcast: UiBroadcast,
    tunnel_manager: TunnelManager,
    completion_bus: CompletionBus,
) {
    let _receiver = completion_bus.subscribe();
    let schedule_pool = pool.clone();
    let retention_pool = pool.clone();
    let sync_pool = pool;

    let repo_lock = RepoLock::default();
    let schedule_task = async move {
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        loop {
            interval.tick().await;
            if let Err(e) = tick(
                &schedule_pool,
                &registry,
                &encryption_key,
                &tunnel_manager,
                &completion_bus,
                &repo_lock,
            )
            .await
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
        match sync_existing_archives(pool, encryption_key, repo.id, ui_broadcast, false).await {
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
    completion_bus: &CompletionBus,
    repo_lock: &RepoLock,
) -> Result<(), crate::error::ApiError> {
    let now = Utc::now();
    let due = db::list_due_schedules(pool, now).await?;

    if due.is_empty() {
        return Ok(());
    }

    let tz = db::get_schedule_timezone(pool).await?;

    // Group rows by schedule_id, preserving ORDER BY s.id, st.execution_order from the query.
    let mut schedule_groups: Vec<(i64, String, Vec<DueScheduleRow>)> = Vec::new();
    for row in due {
        match schedule_groups.last_mut() {
            Some((id, _, targets)) if *id == row.schedule_id => {
                targets.push(row);
            }
            _ => {
                let cron = row.cron_expression.clone();
                schedule_groups.push((row.schedule_id, cron, vec![row]));
            }
        }
    }

    for (schedule_id, cron, targets) in schedule_groups {
        let first = &targets[0];
        let on_failure = first.on_failure.parse::<OnFailure>().unwrap_or_default();

        let run_id = Uuid::new_v4().to_string();

        for target in &targets {
            if let Err(e) = db::insert_backup_pending(
                pool,
                target.client_id,
                target.repo_id,
                Some(schedule_id),
                &run_id,
                now,
            )
            .await
            {
                tracing::warn!(
                    schedule_id,
                    hostname = %target.hostname,
                    error = %e,
                    "failed to insert pending record"
                );
            }
        }

        let ctx = SequentialExecution {
            pool: pool.clone(),
            registry: registry.clone(),
            encryption_key: *encryption_key,
            tunnel_manager: tunnel_manager.clone(),
            completion_bus: completion_bus.clone(),
            repo_lock: repo_lock.clone(),
            schedule_id,
            cron,
            targets,
            on_failure,
            triggered_at: now,
            tz,
            run_id,
        };
        tokio::spawn(async move {
            run_sequential_schedule(ctx).await;
        });
    }

    Ok(())
}

fn build_trigger_msg(
    schedule_type: ScheduleType,
    repo_id: RepoId,
    schedule_id: i64,
    run_id: &str,
) -> ServerToAgent {
    match schedule_type {
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
            schedule_id: Some(schedule_id),
            request_id: None,
            run_id: Some(run_id.to_string()),
        },
    }
}

fn schedule_type_label(schedule_type: ScheduleType) -> &'static str {
    match schedule_type {
        ScheduleType::Check => "check",
        ScheduleType::Verify => "verify",
        ScheduleType::Backup => "backup",
    }
}

struct SequentialExecution {
    pool: PgPool,
    registry: AgentRegistry,
    encryption_key: [u8; 32],
    tunnel_manager: TunnelManager,
    completion_bus: CompletionBus,
    repo_lock: RepoLock,
    schedule_id: i64,
    cron: String,
    targets: Vec<DueScheduleRow>,
    on_failure: OnFailure,
    triggered_at: DateTime<Utc>,
    tz: chrono_tz::Tz,
    run_id: String,
}

async fn run_sequential_schedule(ctx: SequentialExecution) {
    let SequentialExecution {
        pool,
        registry,
        encryption_key,
        tunnel_manager,
        completion_bus,
        repo_lock,
        schedule_id,
        cron,
        targets,
        on_failure,
        triggered_at: now,
        tz,
        run_id,
    } = ctx;
    let mut marked_triggered = false;

    'targets: for target in &targets {
        let Ok(schedule_type) = target.schedule_type.parse::<ScheduleType>() else {
            tracing::error!(
                schedule_id,
                schedule_type = %target.schedule_type,
                "sequential: invalid schedule type in database, skipping target"
            );
            match on_failure {
                OnFailure::Stop => break 'targets,
                OnFailure::Continue => continue 'targets,
            }
        };

        // Subscribe before sending so we don't miss the completion event.
        let mut rx = completion_bus.subscribe();

        // Acquire the per-repo lock to prevent concurrent backups across schedules.
        let _repo_guard = repo_lock.acquire(target.repo_id).await;

        tunnel_manager
            .ensure_client_tunnel_connected(target.client_id)
            .await;

        match config_assembler::assemble_config(&pool, &encryption_key, &target.hostname).await {
            Ok(config) => {
                let config_msg = ServerToAgent::ConfigUpdate(config);
                if let Err(e) = registry.send_to(&target.hostname, config_msg).await {
                    tracing::warn!(
                        hostname = %target.hostname,
                        schedule_id,
                        error = %e,
                        "sequential: agent not connected for pre-run config push, skipping target"
                    );
                    match on_failure {
                        OnFailure::Stop => break 'targets,
                        OnFailure::Continue => continue 'targets,
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    hostname = %target.hostname,
                    schedule_id,
                    error = %e,
                    "sequential: failed to assemble config, skipping target"
                );
                match on_failure {
                    OnFailure::Stop => break 'targets,
                    OnFailure::Continue => continue 'targets,
                }
            }
        }

        let repo_id = RepoId(target.repo_id);
        let msg = build_trigger_msg(schedule_type, repo_id, schedule_id, &run_id);
        let action = schedule_type_label(schedule_type);

        match registry.send_to(&target.hostname, msg).await {
            Ok(()) => {
                tracing::info!(
                    hostname = %target.hostname,
                    repo_id = target.repo_id,
                    action,
                    schedule_id,
                    "sequential: triggered"
                );
                if !marked_triggered {
                    match calculate_next_run(&cron, now, tz) {
                        Ok(next) => {
                            if let Err(e) =
                                db::mark_schedule_triggered(&pool, schedule_id, now, next).await
                            {
                                tracing::error!(
                                    schedule_id,
                                    error = %e,
                                    "sequential: failed to mark schedule triggered"
                                );
                            } else {
                                marked_triggered = true;
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                schedule_id,
                                cron = %cron,
                                error = %e,
                                "sequential: invalid cron expression"
                            );
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    hostname = %target.hostname,
                    repo_id = target.repo_id,
                    action,
                    schedule_id,
                    error = %e,
                    "sequential: agent not connected, skipping target"
                );
                match on_failure {
                    OnFailure::Stop => break 'targets,
                    OnFailure::Continue => continue 'targets,
                }
            }
        }

        let hostname = target.hostname.clone();
        let repo_id_val = target.repo_id;

        let result = tokio::time::timeout(SEQUENTIAL_TIMEOUT, async {
            loop {
                match rx.recv().await {
                    Ok(o) if o.hostname == hostname && o.repo_id == repo_id_val => {
                        break o.success;
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break false,
                }
            }
        })
        .await;

        let success = match result {
            Ok(s) => s,
            Err(_) => {
                tracing::error!(
                    schedule_id,
                    hostname = %target.hostname,
                    repo_id = target.repo_id,
                    "sequential: timed out waiting for operation to complete"
                );
                false
            }
        };

        if !success {
            match on_failure {
                OnFailure::Stop => {
                    tracing::warn!(
                        schedule_id,
                        hostname = %target.hostname,
                        "sequential: stopping remaining targets due to failure"
                    );
                    break 'targets;
                }
                OnFailure::Continue => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{os::unix::fs::PermissionsExt, sync::OnceLock};

    use chrono::TimeZone;
    use tempfile::TempDir;
    use tokio::sync::{Mutex, mpsc};

    use super::*;
    use crate::{
        db::{self, InsertRepoParams, ScheduleParams},
        tunnel::TunnelManager,
        ws::{completion_bus::CompletionBus, registry::AgentRegistry, ui_broadcast::UiBroadcast},
    };

    static BORG_BINARY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    struct BorgBinaryGuard {
        previous: Option<String>,
    }

    impl Drop for BorgBinaryGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.clone() {
                // SAFETY: tests serialize BORG_BINARY changes with a process-local lock.
                unsafe { std::env::set_var("BORG_BINARY", previous) };
            } else {
                // SAFETY: tests serialize BORG_BINARY changes with a process-local lock.
                unsafe { std::env::remove_var("BORG_BINARY") };
            }
        }
    }

    async fn borg_binary_lock() -> tokio::sync::MutexGuard<'static, ()> {
        BORG_BINARY_LOCK.get_or_init(|| Mutex::new(())).lock().await
    }

    async fn install_fake_borg(
        list_json: &str,
        info_all_json: &str,
        info_repo_json: &str,
    ) -> (TempDir, BorgBinaryGuard) {
        let tempdir = tempfile::tempdir().unwrap();
        let script = format!(
            r#"#!/bin/sh
set -eu
case "$1" in
  list)
    case " $* " in
      *" --json-lines "*) exit 1 ;;
      *) cat <<'EOF'
{list_json}
EOF
        ;;
    esac
    ;;
  info)
    case " $* " in
      *" --glob-archives "*) cat <<'EOF'
{info_all_json}
EOF
        ;;
      *) cat <<'EOF'
{info_repo_json}
EOF
        ;;
    esac
    ;;
  *)
    exit 1
    ;;
esac
"#
        );

        let borg_path = tempdir.path().join("borg");
        tokio::fs::write(&borg_path, script).await.unwrap();
        let mut permissions = tokio::fs::metadata(&borg_path).await.unwrap().permissions();
        permissions.set_mode(0o755);
        tokio::fs::set_permissions(&borg_path, permissions)
            .await
            .unwrap();

        let previous = std::env::var("BORG_BINARY").ok();
        // SAFETY: tests serialize BORG_BINARY changes with a process-local lock.
        unsafe { std::env::set_var("BORG_BINARY", &borg_path) };

        (tempdir, BorgBinaryGuard { previous })
    }

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

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn run_repo_sync_full_reimports_and_prunes_stale_archives(pool: sqlx::PgPool) {
        let _borg_lock = borg_binary_lock().await;
        let list_json = r#"{
  "archives": [
    {
      "name": "fresh-archive",
      "hostname": "scheduler-test-host",
      "start": "2026-06-05T10:00:00Z",
      "end": "2026-06-05T10:05:00Z",
      "duration": 300.0,
      "stats": {
        "original_size": 1000,
        "compressed_size": 500,
        "deduplicated_size": 250,
        "nfiles": 2
      }
    }
  ]
}"#;
        let info_repo_json = r#"{
  "cache": {
    "stats": {
      "total_size": 1000,
      "total_csize": 600,
      "unique_csize": 500,
      "total_chunks": 10,
      "unique_chunks": 8
    }
  }
}"#;

        let (_borg_dir, _borg_guard) =
            install_fake_borg(list_json, list_json, info_repo_json).await;
        let encryption_key = shared::crypto::derive_key(b"test-secret-key-for-scheduler");
        let passphrase_encrypted =
            shared::crypto::encrypt_passphrase("test-pass", &encryption_key).unwrap();
        let client = db::insert_client(
            &pool,
            "scheduler-test-host",
            Some("Scheduler Test Host"),
            "hash",
            None,
        )
        .await
        .unwrap();
        let repo = db::insert_repo(
            &pool,
            &InsertRepoParams {
                name: "scheduler-test-repo",
                repo_path: "/backup/test",
                ssh_user: "borg",
                ssh_host: "storage.local",
                ssh_port: 22,
                passphrase_encrypted: &passphrase_encrypted,
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
            },
        )
        .await
        .unwrap();

        let stale_started_at = Utc::now() - chrono::Duration::days(1);
        let stale_finished_at = stale_started_at + chrono::Duration::minutes(5);
        sqlx::query(
            "INSERT INTO backup_reports (client_id, repo_id, schedule_id, started_at, \
             finished_at, status, original_size, compressed_size, deduplicated_size, \
             repo_unique_csize, files_processed, duration_secs, error_message, warnings, \
             borg_version, matched, archive_name, borg_command) VALUES ($1, $2, NULL, $3, $4, \
             'success', 10, 5, 5, 5, 1, 300, NULL, '{}'::text[], NULL, true, $5, NULL)",
        )
        .bind(client.id)
        .bind(repo.id)
        .bind(stale_started_at)
        .bind(stale_finished_at)
        .bind("stale-archive")
        .execute(&pool)
        .await
        .unwrap();

        run_repo_sync(&pool, &encryption_key, &UiBroadcast::new()).await;

        let stale_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM backup_reports WHERE repo_id = $1 AND archive_name = $2",
        )
        .bind(repo.id)
        .bind("stale-archive")
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stale_count, 0);

        let fresh_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM backup_reports WHERE repo_id = $1 AND archive_name = $2",
        )
        .bind(repo.id)
        .bind("fresh-archive")
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(fresh_count, 1);
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
        db::update_repo_ssh_host_key(pool, repo.id, "ssh-ed25519 AAAATICKTEST")
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
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 0,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: "[]",
                post_backup_commands: "[]",
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
        let bus = CompletionBus::new();

        tick(&pool, &registry, &key, &tunnel, &bus, &RepoLock::default())
            .await
            .unwrap();

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
        let bus = CompletionBus::new();

        tick(&pool, &registry, &key, &tunnel, &bus, &RepoLock::default())
            .await
            .unwrap();

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
        let bus = CompletionBus::new();

        tick(&pool, &registry, &key, &tunnel, &bus, &RepoLock::default())
            .await
            .unwrap();

        let due = db::list_due_schedules(&pool, Utc::now()).await.unwrap();
        assert!(
            due.iter().any(|s| s.schedule_id == schedule_id),
            "schedule must remain due when the agent was not connected"
        );
    }
}
