// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Applies a [`QuotaAction`] once a repo or server quota threshold is breached.
//! `NotifyOnly` is a no-op here since the caller already dispatches a
//! notification for every breach regardless of the configured action.

use shared::types::QuotaAction;

use crate::{AppState, config_assembler, db};

async fn disable_schedule_and_push(state: &AppState, schedule_id: i64) {
    if let Err(e) = db::set_schedule_enabled(&state.pool, schedule_id, false).await {
        tracing::error!(schedule_id, error = %e, "failed to disable schedule for quota action");
        return;
    }
    config_assembler::push_config_to_all_schedule_targets(state, schedule_id).await;
}

/// Shared `block_backups`/`disable_schedule` branching for both repo- and server-scoped quota
/// actions. `all_schedule_ids` is every schedule in the breached scope (a repo, or every repo on
/// a shared SSH host); `triggering_schedule_id` is the schedule that ran the backup which
/// crossed the threshold, when known.
///
/// `disable_schedule` falls back to disabling every schedule in scope when
/// `triggering_schedule_id` is `None` (a manual/ad-hoc "run now" backup has no schedule id):
/// otherwise the configured action would silently never fire for manually triggered backups.
async fn enforce_quota_action(
    state: &AppState,
    all_schedule_ids: &[i64],
    triggering_schedule_id: Option<i64>,
    action: QuotaAction,
) {
    match action {
        QuotaAction::NotifyOnly => {}
        QuotaAction::BlockBackups => {
            for &schedule_id in all_schedule_ids {
                disable_schedule_and_push(state, schedule_id).await;
            }
        }
        QuotaAction::DisableSchedule => match triggering_schedule_id {
            Some(schedule_id) => disable_schedule_and_push(state, schedule_id).await,
            None => {
                tracing::warn!(
                    "quota action disable_schedule has no triggering schedule (manual/ad-hoc \
                     backup); disabling every schedule in scope as a fallback"
                );
                for &schedule_id in all_schedule_ids {
                    disable_schedule_and_push(state, schedule_id).await;
                }
            }
        },
    }
}

/// Enforces a repo-level quota action: `block_backups` disables every schedule for the
/// repo, `disable_schedule` disables `triggering_schedule_id` (falling back to every schedule
/// for the repo when the breaching backup was manually triggered and has no schedule id).
pub async fn enforce_repo_quota_action(
    state: &AppState,
    repo_id: i64,
    triggering_schedule_id: Option<i64>,
    action: QuotaAction,
) {
    if matches!(action, QuotaAction::NotifyOnly) {
        return;
    }

    let all_schedule_ids = match db::list_schedules_for_repo(&state.pool, repo_id).await {
        Ok(schedules) => schedules.into_iter().map(|s| s.id).collect::<Vec<_>>(),
        Err(e) => {
            tracing::error!(repo_id, error = %e, "failed to list schedules for quota action");
            return;
        }
    };
    enforce_quota_action(state, &all_schedule_ids, triggering_schedule_id, action).await;
}

/// Enforces a server-level (shared SSH host) quota action: `block_backups` disables every
/// schedule for every repo on `ssh_host`, `disable_schedule` disables `triggering_schedule_id`
/// (falling back to every schedule on the host when the breaching backup was manually triggered
/// and has no schedule id).
pub async fn enforce_server_quota_action(
    state: &AppState,
    ssh_host: &str,
    triggering_schedule_id: Option<i64>,
    action: QuotaAction,
) {
    if matches!(action, QuotaAction::NotifyOnly) {
        return;
    }

    let all_schedule_ids = match db::list_schedule_ids_for_ssh_host(&state.pool, ssh_host).await {
        Ok(ids) => ids,
        Err(e) => {
            tracing::error!(
                ssh_host,
                error = %e,
                "failed to list schedules for server quota action"
            );
            return;
        }
    };
    enforce_quota_action(state, &all_schedule_ids, triggering_schedule_id, action).await;
}

#[cfg(test)]
mod tests {
    use shared::crypto::derive_key;
    use sqlx::PgPool;

    use super::*;
    use crate::db::{InsertRepoParams, ScheduleParams};

    fn build_test_state(pool: PgPool) -> AppState {
        let ui_broadcast = crate::ws::ui_broadcast::UiBroadcast::new();
        let tunnel_manager = crate::tunnel::TunnelManager::new(
            pool.clone(),
            ui_broadcast.clone(),
            "127.0.0.1:0".parse().expect("valid socket address"),
        );

        AppState {
            pool: pool.clone(),
            encryption_key: derive_key(b"quota-enforcement-test-secret-key").unwrap(),
            registry: crate::ws::registry::AgentRegistry::new(),
            ui_broadcast,
            tunnel_manager,
            log_buffer: crate::log_buffer::LogBuffer::default(),
            notification_service: crate::notifications::NotificationService::new(
                pool,
                reqwest::Client::new(),
            ),
            completion_bus: crate::ws::completion_bus::CompletionBus::new(),
            repo_op_tracker: crate::repo_op_tracker::RepoOpTracker::default(),
            repo_lock: crate::RepoLock::default(),
            import_tasks: crate::ImportTaskRegistry::default(),
            pending_dryruns: std::sync::Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            pending_restores: std::sync::Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            pending_migrations: std::sync::Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            pending_deletes: std::sync::Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            shutdown_token: tokio_util::sync::CancellationToken::new(),
            client_ip_resolver: crate::client_ip::ClientIpResolver::new(),
        }
    }

    async fn insert_test_repo(pool: &PgPool, name: &str, ssh_host: &str) -> i64 {
        db::insert_repo(
            pool,
            &InsertRepoParams {
                name,
                repo_path: "/backups/test",
                ssh_user: "backup",
                ssh_host,
                ssh_port: 22,
                passphrase_encrypted: b"encrypted",
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
            },
        )
        .await
        .unwrap()
        .id
    }

    async fn insert_test_schedule(pool: &PgPool, repo_id: i64, agent_id: i64, name: &str) -> i64 {
        let schedule = db::insert_schedule(
            pool,
            repo_id,
            &ScheduleParams {
                name,
                schedule_type: "backup",
                cron_expression: "0 3 * * *",
                enabled: true,
                canary_enabled: false,
                exclude_patterns_raw: "",
                file_change_patterns_raw: "",
                ignore_global_excludes: false,
                keep_hourly: 24,
                keep_daily: 7,
                keep_weekly: 4,
                keep_monthly: 6,
                keep_yearly: 1,
                compact_enabled: true,
                rate_limit_kbps: None,
                pre_backup_commands: "",
                post_backup_commands: "",
                on_failure: "stop",
            },
            None,
        )
        .await
        .unwrap();
        db::insert_schedule_targets(pool, schedule.id, &[(agent_id, 0)])
            .await
            .unwrap();
        schedule.id
    }

    async fn schedule_enabled(pool: &PgPool, schedule_id: i64) -> bool {
        db::get_schedule_by_id(pool, schedule_id)
            .await
            .unwrap()
            .enabled
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn repo_notify_only_leaves_schedules_enabled(pool: PgPool) {
        let state = build_test_state(pool.clone());
        let agent = db::insert_agent(&pool, "host-a", None, "hash", None)
            .await
            .unwrap();
        let repo_id = insert_test_repo(&pool, "repo-a", "storage.local").await;
        let schedule_id = insert_test_schedule(&pool, repo_id, agent.id, "sched-a").await;

        enforce_repo_quota_action(&state, repo_id, Some(schedule_id), QuotaAction::NotifyOnly)
            .await;

        assert!(schedule_enabled(&pool, schedule_id).await);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn repo_block_backups_disables_every_schedule_for_repo(pool: PgPool) {
        let state = build_test_state(pool.clone());
        let agent = db::insert_agent(&pool, "host-a", None, "hash", None)
            .await
            .unwrap();
        let repo_id = insert_test_repo(&pool, "repo-a", "storage.local").await;
        let schedule_a = insert_test_schedule(&pool, repo_id, agent.id, "sched-a").await;
        let schedule_b = insert_test_schedule(&pool, repo_id, agent.id, "sched-b").await;

        enforce_repo_quota_action(&state, repo_id, Some(schedule_a), QuotaAction::BlockBackups)
            .await;

        assert!(!schedule_enabled(&pool, schedule_a).await);
        assert!(!schedule_enabled(&pool, schedule_b).await);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn repo_disable_schedule_only_disables_triggering_schedule(pool: PgPool) {
        let state = build_test_state(pool.clone());
        let agent = db::insert_agent(&pool, "host-a", None, "hash", None)
            .await
            .unwrap();
        let repo_id = insert_test_repo(&pool, "repo-a", "storage.local").await;
        let schedule_a = insert_test_schedule(&pool, repo_id, agent.id, "sched-a").await;
        let schedule_b = insert_test_schedule(&pool, repo_id, agent.id, "sched-b").await;

        enforce_repo_quota_action(
            &state,
            repo_id,
            Some(schedule_a),
            QuotaAction::DisableSchedule,
        )
        .await;

        assert!(!schedule_enabled(&pool, schedule_a).await);
        assert!(schedule_enabled(&pool, schedule_b).await);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn repo_disable_schedule_without_trigger_falls_back_to_every_schedule(pool: PgPool) {
        let state = build_test_state(pool.clone());
        let agent = db::insert_agent(&pool, "host-a", None, "hash", None)
            .await
            .unwrap();
        let repo_id = insert_test_repo(&pool, "repo-a", "storage.local").await;
        let schedule_a = insert_test_schedule(&pool, repo_id, agent.id, "sched-a").await;
        let schedule_b = insert_test_schedule(&pool, repo_id, agent.id, "sched-b").await;

        // `triggering_schedule_id: None` mirrors a manual "run now" backup, which has no
        // schedule id.
        enforce_repo_quota_action(&state, repo_id, None, QuotaAction::DisableSchedule).await;

        assert!(!schedule_enabled(&pool, schedule_a).await);
        assert!(!schedule_enabled(&pool, schedule_b).await);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn server_block_backups_disables_schedules_across_every_repo_on_host(pool: PgPool) {
        let state = build_test_state(pool.clone());
        let agent = db::insert_agent(&pool, "host-a", None, "hash", None)
            .await
            .unwrap();
        let repo_a = insert_test_repo(&pool, "repo-a", "shared.local").await;
        let repo_b = insert_test_repo(&pool, "repo-b", "shared.local").await;
        let other_repo = insert_test_repo(&pool, "repo-c", "other.local").await;
        let schedule_a = insert_test_schedule(&pool, repo_a, agent.id, "sched-a").await;
        let schedule_b = insert_test_schedule(&pool, repo_b, agent.id, "sched-b").await;
        let other_schedule = insert_test_schedule(&pool, other_repo, agent.id, "sched-c").await;

        enforce_server_quota_action(
            &state,
            "shared.local",
            Some(schedule_a),
            QuotaAction::BlockBackups,
        )
        .await;

        assert!(!schedule_enabled(&pool, schedule_a).await);
        assert!(!schedule_enabled(&pool, schedule_b).await);
        assert!(schedule_enabled(&pool, other_schedule).await);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn server_disable_schedule_without_trigger_falls_back_to_every_schedule_on_host(
        pool: PgPool,
    ) {
        let state = build_test_state(pool.clone());
        let agent = db::insert_agent(&pool, "host-a", None, "hash", None)
            .await
            .unwrap();
        let repo_a = insert_test_repo(&pool, "repo-a", "shared.local").await;
        let repo_b = insert_test_repo(&pool, "repo-b", "shared.local").await;
        let schedule_a = insert_test_schedule(&pool, repo_a, agent.id, "sched-a").await;
        let schedule_b = insert_test_schedule(&pool, repo_b, agent.id, "sched-b").await;

        enforce_server_quota_action(&state, "shared.local", None, QuotaAction::DisableSchedule)
            .await;

        assert!(!schedule_enabled(&pool, schedule_a).await);
        assert!(!schedule_enabled(&pool, schedule_b).await);
    }
}
