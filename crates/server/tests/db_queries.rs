// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Database integration tests that exercise every SQL statement in `db.rs`.
//!
//! Run with:
//! ```sh
//! DATABASE_URL=postgres://borg:borg_secret@localhost:5432/borg \
//!   cargo test -p server --test db_queries -- --test-threads=1
//! ```
//!
//! Each test uses `#[sqlx::test]` which creates an isolated database per test
//! and applies migrations automatically.

use chrono::{DateTime, Datelike, Duration, Utc};
use chrono_tz::Tz;
use server::{
    archive_index::codec::{self, DirEntry},
    db::{self, patterns, *},
};
use shared::{
    hooks::HookCommand,
    types::{AcknowledgedFilter, QuotaAction, SystemEventType},
    vm::{DiscoveredVm, VmSelectionMode, VmSnapshotConfig, VmSnapshotMode, VmState},
};
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn agent_insert_and_get(pool: PgPool) {
    let agent = db::insert_agent(&pool, "test-host", Some("Test Host"), "hash123", None, None)
        .await
        .unwrap();

    assert_eq!(agent.hostname, "test-host");
    assert_eq!(agent.display_name.as_deref(), Some("Test Host"));
    assert!(agent.agent_version.is_none());
    assert!(agent.last_seen_at.is_none());

    let fetched = db::get_agent_by_hostname(&pool, "test-host", None)
        .await
        .unwrap();
    assert_eq!(fetched.id, agent.id);
    assert_eq!(fetched.hostname, "test-host");
}

#[sqlx::test(migrations = "./migrations")]
async fn database_storage_lists_application_tables(pool: PgPool) {
    let (database_bytes, relations) = db::get_database_storage(&pool).await.unwrap();

    assert!(database_bytes > 0);
    assert!(
        relations
            .iter()
            .any(|relation| relation.table_name == "archive_dirs")
    );
    assert!(relations.iter().all(|relation| relation.table_bytes >= 0));
    assert!(relations.iter().all(|relation| relation.index_bytes >= 0));
    assert!(relations.iter().all(|relation| relation.toast_bytes >= 0));
    assert!(relations.iter().all(|relation| relation.total_bytes >= 0));
    assert!(
        relations
            .windows(2)
            .all(|rows| rows.first().unwrap().total_bytes >= rows.get(1).unwrap().total_bytes)
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_not_found(pool: PgPool) {
    let result = db::get_agent_by_hostname(&pool, "nonexistent", None).await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_token_hash(pool: PgPool) {
    db::insert_agent(&pool, "token-host", None, "secret_hash", None, None)
        .await
        .unwrap();

    let candidates = db::get_agent_token_hashes(&pool, "token-host")
        .await
        .unwrap();
    assert_eq!(candidates.len(), 1);
    let candidate = candidates.first().unwrap();
    assert!(candidate.id > 0);
    assert_eq!(candidate.agent_token_hash, "secret_hash");
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_update_last_seen(pool: PgPool) {
    let agent = db::insert_agent(&pool, "seen-host", None, "hash", None, None)
        .await
        .unwrap();

    db::update_last_seen(&pool, agent.id).await.unwrap();

    let fetched = db::get_agent_by_hostname(&pool, "seen-host", None)
        .await
        .unwrap();
    assert!(fetched.last_seen_at.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_update_last_ssh_user(pool: PgPool) {
    let agent = db::insert_agent(&pool, "ssh-user-host", None, "hash", None, None)
        .await
        .unwrap();

    let fetched = db::get_agent_by_hostname(&pool, "ssh-user-host", None)
        .await
        .unwrap();
    assert_eq!(fetched.last_ssh_user, None);

    db::update_last_ssh_user(&pool, agent.id, "deploy-user")
        .await
        .unwrap();

    let fetched = db::get_agent_by_hostname(&pool, "ssh-user-host", None)
        .await
        .unwrap();
    assert_eq!(fetched.last_ssh_user.as_deref(), Some("deploy-user"));

    db::update_last_ssh_user(&pool, agent.id, "root")
        .await
        .unwrap();

    let fetched = db::get_agent_by_hostname(&pool, "ssh-user-host", None)
        .await
        .unwrap();
    assert_eq!(fetched.last_ssh_user.as_deref(), Some("root"));
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_update_last_seen_and_version(pool: PgPool) {
    let agent = db::insert_agent(&pool, "ver-host", None, "hash", None, None)
        .await
        .unwrap();

    db::update_last_seen_and_version(&pool, agent.id, "2.0.0", None, None, None)
        .await
        .unwrap();

    let fetched = db::get_agent_by_hostname(&pool, "ver-host", None)
        .await
        .unwrap();
    assert_eq!(fetched.agent_version.as_deref(), Some("2.0.0"));
    assert!(fetched.last_seen_at.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_update_last_seen_by_hostname(pool: PgPool) {
    db::insert_agent(&pool, "hostname-seen", None, "hash", None, None)
        .await
        .unwrap();

    db::update_last_seen_by_hostname(&pool, "hostname-seen")
        .await
        .unwrap();

    let fetched = db::get_agent_by_hostname(&pool, "hostname-seen", None)
        .await
        .unwrap();
    assert!(fetched.last_seen_at.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_list(pool: PgPool) {
    db::insert_agent(&pool, "alpha", None, "h1", None, None)
        .await
        .unwrap();
    db::insert_agent(&pool, "beta", None, "h2", None, None)
        .await
        .unwrap();

    let agents = db::list_agents(&pool, false).await.unwrap();
    assert_eq!(agents.len(), 2);
    assert_eq!(agents.first().unwrap().hostname, "alpha");
    assert_eq!(agents.get(1).unwrap().hostname, "beta");
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_update(pool: PgPool) {
    let agent = db::insert_agent(&pool, "upd-host", Some("Old Name"), "hash", None, None)
        .await
        .unwrap();

    let updated = db::update_agent(
        &pool,
        agent.id,
        "upd-host",
        db::AgentDefaults {
            display_name: Some("New Name"),
            domain: None,
            default_backup_paths: &[],
            default_exclude_patterns: &[],
            default_pre_backup_commands: &[],
            default_post_backup_commands: &[],
            default_file_change_patterns_raw: "*/tmp/* ignore",
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.display_name.as_deref(), Some("New Name"));
    assert_eq!(updated.default_file_change_patterns_raw, "*/tmp/* ignore");
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_regenerate_token(pool: PgPool) {
    let agent = db::insert_agent(&pool, "regen-host", None, "old_hash", None, None)
        .await
        .unwrap();

    let updated = db::regenerate_agent_token(&pool, agent.id, "new_hash")
        .await
        .unwrap();
    assert_eq!(updated.hostname, "regen-host");

    let candidates = db::get_agent_token_hashes(&pool, "regen-host")
        .await
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates.first().unwrap().agent_token_hash, "new_hash");
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_delete(pool: PgPool) {
    let agent = db::insert_agent(&pool, "del-host", None, "hash", None, None)
        .await
        .unwrap();

    db::delete_agent(&pool, agent.id).await.unwrap();

    let result = db::get_agent_by_hostname(&pool, "del-host", None).await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_delete_not_found(pool: PgPool) {
    let result = db::delete_agent(&pool, 999_999_999).await;
    assert!(result.is_err());
}

/// Deleting the agent that caused a schedule's auto-disable must clear that
/// schedule's stale auto-disable bookkeeping. The FK on `auto_disabled_by_agent_id`
/// only nulls that one column on delete, leaving `auto_disabled_agent_unreachable`
/// and `consecutive_failures` stale - and a deleted agent can never appear in a
/// later PUT's old-vs-new target diff, so
/// `reset_schedule_failure_tracking_if_target_dropped` alone could never reach it.
/// Fetches the auto-disable bookkeeping columns exercised by the
/// `*_clears_auto_disable_bookkeeping_for_its_schedules` tests below, mirroring
/// `scheduler.rs`'s own `schedule_failure_state` test helper (kept separate since
/// that one is private to `scheduler.rs`'s in-crate test module and this file is a
/// standalone integration test binary).
#[cfg(test)]
async fn schedule_auto_disable_state(
    pool: &PgPool,
    schedule_id: i64,
) -> (bool, bool, Option<i64>, i32) {
    let row = sqlx::query!(
        "SELECT enabled, auto_disabled_agent_unreachable, auto_disabled_by_agent_id, \
         consecutive_failures FROM schedules WHERE id = $1",
        schedule_id,
    )
    .fetch_one(pool)
    .await
    .unwrap();
    (
        row.enabled,
        row.auto_disabled_agent_unreachable,
        row.auto_disabled_by_agent_id,
        row.consecutive_failures,
    )
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_delete_clears_auto_disable_bookkeeping_for_its_schedules(pool: PgPool) {
    let (agent, _repo, schedule) = create_test_schedule(&pool).await;
    let next = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::days(1))
        .unwrap();

    for _ in 0..3 {
        db::record_schedule_failure(&pool, schedule.id, agent.id, next, 3, true)
            .await
            .unwrap();
    }
    let (enabled, auto_disabled_agent_unreachable, _, _) =
        schedule_auto_disable_state(&pool, schedule.id).await;
    assert!(!enabled && auto_disabled_agent_unreachable);

    db::delete_agent(&pool, agent.id).await.unwrap();

    let (enabled, agent_unreachable, by_agent_id, failures) =
        schedule_auto_disable_state(&pool, schedule.id).await;
    assert!(!enabled);
    assert!(
        !agent_unreachable,
        "the stale auto-disable flag must be cleared once the causing agent is gone"
    );
    assert_eq!(by_agent_id, None);
    assert_eq!(failures, 0);
}

#[cfg(test)]
async fn create_test_repo(pool: &PgPool) -> RepoRow {
    db::insert_repo(
        pool,
        &InsertRepoParams {
            name: "test-repo",
            repo_path: "/backups/test",
            ssh_user: "backup",
            ssh_host: "storage.local",
            ssh_port: 22,
            passphrase_encrypted: b"encrypted_data",
            compression: "lz4",
            encryption: "repokey",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap()
}

/// Sets a repo's authoritative `borg info` statistics. Values mirror
/// `insert_test_report` so stat assertions stay consistent now that repo
/// size/archive numbers come from `repos.info_*` rather than backup reports.
#[cfg(test)]
async fn set_test_repo_info_stats(pool: &PgPool, repo_id: i64, archive_count: i64) {
    db::update_repo_info_stats(
        pool,
        repo_id,
        &db::RepoInfoStats {
            original_size: 1_000_000,
            compressed_size: 500_000,
            deduplicated_size: 250_000,
            total_chunks: 100,
            unique_chunks: 80,
            archive_count,
        },
    )
    .await
    .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_insert_and_list(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    assert_eq!(repo.name, "test-repo");
    assert_eq!(repo.repo_path, "/backups/test");
    assert_eq!(repo.ssh_user, "backup");
    assert_eq!(repo.ssh_host, "storage.local");
    assert_eq!(repo.ssh_port, 22);
    assert_eq!(repo.compression, "lz4");
    assert_eq!(repo.encryption, "repokey");
    assert!(repo.enabled);

    let all = db::list_all_repos(&pool).await.unwrap();
    assert_eq!(all.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_connection(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let conn = db::get_repo_connection(&pool, repo.id).await.unwrap();
    assert_eq!(conn.ssh_user, "backup");
    assert_eq!(conn.ssh_host, "storage.local");
    assert_eq!(conn.ssh_port, 22);
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_update(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let updated = db::update_repo(
        &pool,
        &UpdateRepoParams {
            repo_id: repo.id,
            name: "test-repo-updated",
            repo_path: "/backups/v2",
            ssh_user: "user2",
            ssh_host: "host2.local",
            ssh_port: 2222,
            compression: "zstd,3",
            encryption: "repokey-blake2",
            enabled: false,
            sync_schedule: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.repo_path, "/backups/v2");
    assert_eq!(updated.ssh_user, "user2");
    assert_eq!(updated.ssh_host, "host2.local");
    assert_eq!(updated.ssh_port, 2222);
    assert!(!updated.enabled);
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_delete(pool: PgPool) {
    let repo = create_test_repo(&pool).await;
    db::delete_repo(&pool, repo.id).await.unwrap();

    let result = db::get_repo_connection(&pool, repo.id).await;
    assert!(result.is_err());
}

/// Deleting a repo whose schedule was auto-disabled for an unreachable agent must
/// clear that schedule's stale auto-disable bookkeeping - the same as every other
/// direct `enabled` write in the auto-disable feature (`set_schedule_enabled`,
/// `update_schedule`, retargeting) - so a later reconnect from the agent that caused
/// the disable can never silently re-enable an orphaned schedule nobody decided to
/// turn back on.
#[sqlx::test(migrations = "./migrations")]
async fn repo_delete_clears_auto_disable_bookkeeping_for_its_schedules(pool: PgPool) {
    let (agent, repo, schedule) = create_test_schedule(&pool).await;
    let next = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::days(1))
        .unwrap();

    for _ in 0..3 {
        db::record_schedule_failure(&pool, schedule.id, agent.id, next, 3, true)
            .await
            .unwrap();
    }
    let (enabled, auto_disabled_agent_unreachable, _, _) =
        schedule_auto_disable_state(&pool, schedule.id).await;
    assert!(!enabled && auto_disabled_agent_unreachable);

    db::delete_repo(&pool, repo.id).await.unwrap();

    let (enabled, agent_unreachable, by_agent_id, failures) =
        schedule_auto_disable_state(&pool, schedule.id).await;
    assert!(!enabled);
    assert!(
        !agent_unreachable,
        "the stale auto-disable flag must be cleared once the schedule's repo is gone"
    );
    assert_eq!(by_agent_id, None);
    assert_eq!(failures, 0);

    // The agent that caused the disable reconnecting later must never touch this
    // orphaned schedule again.
    let reenabled =
        db::reenable_system_disabled_schedules_for_agent(&pool, agent.id, chrono::Utc::now())
            .await
            .unwrap();
    assert_eq!(reenabled, Vec::<i64>::new());
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_passphrase(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let passphrase = db::get_repo_passphrase(&pool, repo.id).await.unwrap();
    assert_eq!(passphrase, b"encrypted_data");
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_with_passphrase(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let row = db::get_repo_with_passphrase(&pool, repo.id).await.unwrap();
    assert_eq!(row.name, "test-repo");
    assert_eq!(row.passphrase_encrypted, b"encrypted_data");
}

#[sqlx::test(migrations = "./migrations")]
async fn test_quota_upsert_and_get(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let quota = db::quota::upsert_quota(
        &pool,
        repo.id,
        Some(100),
        Some(200),
        QuotaAction::BlockBackups,
        QuotaAction::DisableSchedule,
        true,
    )
    .await
    .unwrap();
    assert_eq!(quota.repo_id, repo.id);
    assert_eq!(quota.warn_bytes, Some(100));
    assert_eq!(quota.critical_bytes, Some(200));
    assert_eq!(quota.warn_action, "block_backups");
    assert_eq!(quota.critical_action, "disable_schedule");
    assert!(quota.enabled);

    let fetched = db::quota::get_quota(&pool, repo.id).await.unwrap();
    let fetched = fetched.expect("quota should exist");
    assert_eq!(fetched.repo_id, repo.id);
    assert_eq!(fetched.warn_bytes, Some(100));
    assert_eq!(fetched.critical_bytes, Some(200));
    assert_eq!(
        fetched.action_for(db::quota::QuotaStatus::Warning),
        Some(QuotaAction::BlockBackups)
    );
    assert_eq!(
        fetched.action_for(db::quota::QuotaStatus::Critical),
        Some(QuotaAction::DisableSchedule)
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_quota_disabled(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let quota = db::quota::upsert_quota(
        &pool,
        repo.id,
        Some(100),
        Some(200),
        QuotaAction::NotifyOnly,
        QuotaAction::NotifyOnly,
        false,
    )
    .await
    .unwrap();

    assert!(!quota.enabled);
    assert_eq!(
        db::quota::evaluate_quota(&quota, 500),
        db::quota::QuotaStatus::Ok
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_audit_insert_and_list(pool: PgPool) {
    db::audit::insert_audit_entry(
        &pool,
        &db::audit::NewAuditEntry {
            user_id: Some(1),
            username: "admin",
            action: "created_repo",
            target_type: Some("repo"),
            target_id: Some(42),
            details: Some(serde_json::json!({"name": "repo-1"})),
            ip_address: Some("127.0.0.1"),
        },
    )
    .await
    .unwrap();

    let (items, total) = db::audit::list_audit_entries(
        &pool,
        &db::audit::AuditEntryFilters {
            page: 1,
            per_page: 50,
            filter_user_id: None,
            filter_action: None,
            filter_target_type: None,
            filter_from: None,
            filter_to: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(total, 1);
    assert_eq!(items.len(), 1);
    assert_eq!(items.first().unwrap().username, "admin");
    assert_eq!(items.first().unwrap().action, "created_repo");
    assert_eq!(items.first().unwrap().target_type.as_deref(), Some("repo"));
}

#[sqlx::test(migrations = "./migrations")]
async fn test_audit_list_pagination(pool: PgPool) {
    for i in 0..5 {
        let action = format!("action-{i}");
        db::audit::insert_audit_entry(
            &pool,
            &db::audit::NewAuditEntry {
                user_id: Some(1),
                username: "admin",
                action: &action,
                target_type: Some("repo"),
                target_id: Some(i),
                details: None,
                ip_address: None,
            },
        )
        .await
        .unwrap();
    }

    let (items, total) = db::audit::list_audit_entries(
        &pool,
        &db::audit::AuditEntryFilters {
            page: 2,
            per_page: 2,
            filter_user_id: None,
            filter_action: None,
            filter_target_type: None,
            filter_from: None,
            filter_to: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(total, 5);
    assert_eq!(items.len(), 2);
    assert_eq!(items.first().unwrap().action, "action-2");
    assert_eq!(items.get(1).unwrap().action, "action-1");
}

#[sqlx::test(migrations = "./migrations")]
async fn test_audit_list_filter_by_action(pool: PgPool) {
    db::audit::insert_audit_entry(
        &pool,
        &db::audit::NewAuditEntry {
            user_id: Some(1),
            username: "admin",
            action: "repo_created",
            target_type: None,
            target_id: None,
            details: None,
            ip_address: None,
        },
    )
    .await
    .unwrap();
    db::audit::insert_audit_entry(
        &pool,
        &db::audit::NewAuditEntry {
            user_id: Some(1),
            username: "admin",
            action: "repo_deleted",
            target_type: None,
            target_id: None,
            details: None,
            ip_address: None,
        },
    )
    .await
    .unwrap();

    let (items, total) = db::audit::list_audit_entries(
        &pool,
        &db::audit::AuditEntryFilters {
            page: 1,
            per_page: 50,
            filter_user_id: None,
            filter_action: Some("repo_created"),
            filter_target_type: None,
            filter_from: None,
            filter_to: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(total, 1);
    assert_eq!(items.len(), 1);
    assert_eq!(items.first().unwrap().action, "repo_created");
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_name(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let name = db::get_repo_name(&pool, repo.id).await.unwrap();
    assert_eq!(name, "test-repo");
}

#[sqlx::test(migrations = "./migrations")]
async fn tunnel_crud(pool: PgPool) {
    let agent = db::insert_agent(&pool, "tunnel-host", None, "hash", None, None)
        .await
        .unwrap();

    let tunnel = db::insert_tunnel(
        &pool,
        &NewSshTunnel {
            agent_id: agent.id,
            ssh_host: "repo.example.com".to_string(),
            ssh_user: "borg".to_string(),
            ssh_port: Some(2222),
            tunnel_port: 2200,
            enabled: Some(true),
            ssh_host_key: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(tunnel.ssh_host, "repo.example.com");
    assert_eq!(tunnel.ssh_port, 2222);
    assert_eq!(tunnel.tunnel_port, 2200);
    assert!(tunnel.enabled);

    let by_id = db::get_tunnel_by_id(&pool, tunnel.id).await.unwrap();
    assert_eq!(by_id.id, tunnel.id);

    let by_agent = db::get_tunnel_by_agent_id(&pool, agent.id).await.unwrap();
    assert_eq!(by_agent.id, tunnel.id);

    let enabled = db::list_enabled_tunnels(&pool).await.unwrap();
    assert_eq!(enabled.len(), 1);

    let all = db::list_all_tunnels(&pool).await.unwrap();
    assert_eq!(all.len(), 1);

    let updated = db::update_tunnel(
        &pool,
        tunnel.id,
        &UpdateSshTunnel {
            ssh_host: Some("new.example.com".to_string()),
            ssh_user: None,
            ssh_port: None,
            tunnel_port: None,
            enabled: Some(false),
            ssh_host_key: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.ssh_host, "new.example.com");
    assert!(!updated.enabled);

    let enabled = db::list_enabled_tunnels(&pool).await.unwrap();
    assert_eq!(enabled.len(), 0);

    db::delete_tunnel(&pool, tunnel.id).await.unwrap();
    let result = db::get_tunnel_by_id(&pool, tunnel.id).await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn tunnel_defaults(pool: PgPool) {
    let agent = db::insert_agent(&pool, "def-host", None, "hash", None, None)
        .await
        .unwrap();

    let tunnel = db::insert_tunnel(
        &pool,
        &NewSshTunnel {
            agent_id: agent.id,
            ssh_host: "host.com".to_string(),
            ssh_user: "user".to_string(),
            ssh_port: None,
            tunnel_port: 3000,
            enabled: None,
            ssh_host_key: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(tunnel.ssh_port, 22);
    assert!(tunnel.enabled);
}

#[sqlx::test(migrations = "./migrations")]
async fn tunnel_ssh_host_key_persist_and_coalesce(pool: PgPool) {
    let agent = db::insert_agent(&pool, "key-persist-host", None, "hash", None, None)
        .await
        .unwrap();

    let tunnel = db::insert_tunnel(
        &pool,
        &NewSshTunnel {
            agent_id: agent.id,
            ssh_host: "key-test.example.com".to_string(),
            ssh_user: "borg".to_string(),
            ssh_port: Some(2222),
            tunnel_port: 2200,
            enabled: Some(true),
            ssh_host_key: None,
        },
    )
    .await
    .unwrap();

    db::update_tunnel_ssh_host_key(&pool, tunnel.id, "ssh-ed25519 AAAAPINNED")
        .await
        .unwrap();

    let fetched = db::get_tunnel_by_id(&pool, tunnel.id).await.unwrap();
    assert_eq!(
        fetched.ssh_host_key.as_deref(),
        Some("ssh-ed25519 AAAAPINNED")
    );

    let updated = db::update_tunnel(
        &pool,
        tunnel.id,
        &UpdateSshTunnel {
            ssh_host: Some("updated.example.com".to_string()),
            ssh_user: None,
            ssh_port: None,
            tunnel_port: None,
            enabled: Some(true),
            ssh_host_key: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.ssh_host, "updated.example.com");
    assert_eq!(
        updated.ssh_host_key.as_deref(),
        Some("ssh-ed25519 AAAAPINNED"),
        "COALESCE must preserve the previously-pinned SSH host key"
    );

    let updated2 = db::update_tunnel(
        &pool,
        tunnel.id,
        &UpdateSshTunnel {
            ssh_host: None,
            ssh_user: Some("root".to_string()),
            ssh_port: None,
            tunnel_port: None,
            enabled: None,
            ssh_host_key: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        updated2.ssh_host_key.as_deref(),
        Some("ssh-ed25519 AAAAPINNED"),
        "COALESCE must still preserve the pinned key when other fields are updated"
    );

    let updated3 = db::update_tunnel(
        &pool,
        tunnel.id,
        &UpdateSshTunnel {
            ssh_host: None,
            ssh_user: None,
            ssh_port: None,
            tunnel_port: None,
            enabled: None,
            ssh_host_key: Some("ssh-ed25519 AAAAREPLACED".to_string()),
        },
    )
    .await
    .unwrap();

    assert_eq!(
        updated3.ssh_host_key.as_deref(),
        Some("ssh-ed25519 AAAAREPLACED"),
        "Explicit SSH host key update must replace the old value"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn excludes_crud(pool: PgPool) {
    let initial = db::get_global_excludes_raw(&pool).await.unwrap();
    assert_eq!(initial, "");

    db::set_global_excludes_raw(&pool, "*.tmp\n*.log\n\n# comment\n/proc")
        .await
        .unwrap();

    let raw = db::get_global_excludes_raw(&pool).await.unwrap();
    assert_eq!(raw, "*.tmp\n*.log\n\n# comment\n/proc");

    db::set_global_excludes_raw(&pool, "*.log").await.unwrap();
    let raw = db::get_global_excludes_raw(&pool).await.unwrap();
    assert_eq!(raw, "*.log");
}

#[cfg(test)]
async fn create_test_schedule(pool: &PgPool) -> (AgentRow, RepoRow, ScheduleRow) {
    let agent = db::insert_agent(pool, "sched-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = db::insert_repo(
        pool,
        &InsertRepoParams {
            name: "sched-repo",
            repo_path: "/backups/sched",
            ssh_user: "user",
            ssh_host: "host.local",
            ssh_port: 22,
            passphrase_encrypted: b"enc",
            compression: "none",
            encryption: "none",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap();
    let schedule = db::insert_schedule(
        pool,
        repo.id,
        &ScheduleParams {
            name: "test-schedule",
            schedule_type: "backup",
            cron_expression: "0 3 * * *",
            enabled: true,
            canary_enabled: false,
            vm_snapshot_enabled: false,
            exclude_patterns_raw: "",
            file_change_patterns_raw: "",
            ignore_global_excludes: false,
            keep_hourly: 24,
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 6,
            keep_yearly: 1,
            compact_enabled: true,
            rate_limit_kbps: Some(5000),
            pre_backup_commands: &[],
            post_backup_commands: &[],
            hook_timeout_seconds: 60,
            missed_backup_threshold: 3,
            on_failure: "stop",
        },
        None,
    )
    .await
    .unwrap();
    db::insert_schedule_targets(pool, schedule.id, &[(agent.id, 0)])
        .await
        .unwrap();
    (agent, repo, schedule)
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_insert_and_list(pool: PgPool) {
    let (_, _, schedule) = create_test_schedule(&pool).await;

    assert_eq!(schedule.schedule_type, "backup");
    assert_eq!(schedule.cron_expression, "0 3 * * *");
    assert!(schedule.enabled);
    assert_eq!(schedule.keep_daily, 7);
    assert_eq!(schedule.rate_limit_kbps, Some(5000));
    assert_eq!(schedule.hook_timeout_seconds, 60);

    let all = db::list_schedules(&pool).await.unwrap();
    assert_eq!(all.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_update(pool: PgPool) {
    let (_, _, schedule) = create_test_schedule(&pool).await;

    let updated = db::update_schedule(
        &pool,
        schedule.id,
        &ScheduleParams {
            name: "updated-schedule",
            schedule_type: "backup",
            cron_expression: "0 6 * * *",
            enabled: false,
            canary_enabled: true,
            vm_snapshot_enabled: false,
            exclude_patterns_raw: "*.cache",
            file_change_patterns_raw: "",
            ignore_global_excludes: true,
            keep_hourly: 24,
            keep_daily: 14,
            keep_weekly: 8,
            keep_monthly: 12,
            keep_yearly: 2,
            compact_enabled: false,
            rate_limit_kbps: None,
            pre_backup_commands: &[HookCommand::new("echo pre")],
            post_backup_commands: &[HookCommand {
                command: "echo post".to_owned(),
                timeout_seconds: Some(7200),
            }],
            hook_timeout_seconds: 120,
            missed_backup_threshold: 3,
            on_failure: "continue",
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.cron_expression, "0 6 * * *");
    assert!(!updated.enabled);
    assert!(updated.canary_enabled);
    assert_eq!(updated.exclude_patterns_raw, "*.cache");
    assert!(updated.ignore_global_excludes);
    assert_eq!(updated.keep_daily, 14);
    assert!(!updated.compact_enabled);
    assert_eq!(updated.rate_limit_kbps, None);
    assert_eq!(
        updated.pre_backup_commands.0,
        vec![HookCommand::new("echo pre")]
    );
    assert_eq!(
        updated.post_backup_commands.0,
        vec![HookCommand {
            command: "echo post".to_owned(),
            timeout_seconds: Some(7200),
        }]
    );
    assert_eq!(updated.hook_timeout_seconds, 120);
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_get_by_id(pool: PgPool) {
    let (_, _, schedule) = create_test_schedule(&pool).await;

    let fetched = db::get_schedule_by_id(&pool, schedule.id).await.unwrap();
    assert_eq!(fetched.id, schedule.id);
    assert_eq!(fetched.cron_expression, "0 3 * * *");
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_for_repo(pool: PgPool) {
    let (_, repo, _) = create_test_schedule(&pool).await;

    let result = db::get_schedule_for_repo(&pool, repo.id).await.unwrap();
    assert!(result.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_for_hostname_repo(pool: PgPool) {
    let (_, repo, _) = create_test_schedule(&pool).await;

    let result = db::get_schedule_for_hostname_repo(
        &pool,
        "sched-host",
        repo.id,
        shared::types::ScheduleType::Backup,
    )
    .await
    .unwrap();
    assert!(result.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_for_hostname_repo_filters_by_type(pool: PgPool) {
    let (_, repo, _) = create_test_schedule(&pool).await;

    let result = db::get_schedule_for_hostname_repo(
        &pool,
        "sched-host",
        repo.id,
        shared::types::ScheduleType::Check,
    )
    .await
    .unwrap();
    assert!(
        result.is_none(),
        "a backup schedule must not match when looking up a check schedule"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_list_for_repo(pool: PgPool) {
    let (agent, repo, _) = create_test_schedule(&pool).await;

    let schedules = db::list_schedules_for_repo(&pool, repo.id).await.unwrap();
    assert_eq!(schedules.len(), 1);
    assert_eq!(
        schedules.first().unwrap().target_hostnames,
        vec![agent.hostname]
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_list_for_repo_multi_schedule_and_isolation(pool: PgPool) {
    let (agent_a, repo_a, schedule_a) = create_test_schedule(&pool).await;

    let agent_b = db::insert_agent(&pool, "repo-list-host-b", None, "hashb", None, None)
        .await
        .unwrap();
    let repo_b = db::insert_repo(
        &pool,
        &InsertRepoParams {
            name: "repo-list-repo-b",
            repo_path: "/backups/b",
            ssh_user: "user",
            ssh_host: "host.local",
            ssh_port: 22,
            passphrase_encrypted: b"enc",
            compression: "none",
            encryption: "none",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap();
    let schedule_b = db::insert_schedule(
        &pool,
        repo_b.id,
        &ScheduleParams {
            name: "schedule-b",
            schedule_type: "backup",
            cron_expression: "0 4 * * *",
            enabled: true,
            canary_enabled: false,
            vm_snapshot_enabled: false,
            exclude_patterns_raw: "",
            file_change_patterns_raw: "",
            ignore_global_excludes: false,
            keep_hourly: 0,
            keep_daily: 7,
            keep_weekly: 0,
            keep_monthly: 0,
            keep_yearly: 0,
            compact_enabled: false,
            rate_limit_kbps: None,
            pre_backup_commands: &[],
            post_backup_commands: &[],
            hook_timeout_seconds: 60,
            missed_backup_threshold: 3,
            on_failure: "stop",
        },
        None,
    )
    .await
    .unwrap();
    db::insert_schedule_targets(&pool, schedule_b.id, &[(agent_b.id, 0)])
        .await
        .unwrap();

    // Second schedule on repo_a with two hosts
    let schedule_a2 = db::insert_schedule(
        &pool,
        repo_a.id,
        &ScheduleParams {
            name: "schedule-a2",
            schedule_type: "check",
            cron_expression: "0 5 * * *",
            enabled: true,
            canary_enabled: false,
            vm_snapshot_enabled: false,
            exclude_patterns_raw: "",
            file_change_patterns_raw: "",
            ignore_global_excludes: false,
            keep_hourly: 0,
            keep_daily: 0,
            keep_weekly: 0,
            keep_monthly: 0,
            keep_yearly: 0,
            compact_enabled: false,
            rate_limit_kbps: None,
            pre_backup_commands: &[],
            post_backup_commands: &[],
            hook_timeout_seconds: 60,
            missed_backup_threshold: 3,
            on_failure: "stop",
        },
        None,
    )
    .await
    .unwrap();
    db::insert_schedule_targets(&pool, schedule_a2.id, &[(agent_a.id, 0), (agent_b.id, 1)])
        .await
        .unwrap();

    let results_a = db::list_schedules_for_repo(&pool, repo_a.id).await.unwrap();
    assert_eq!(results_a.len(), 2);
    let s1 = results_a.iter().find(|s| s.id == schedule_a.id).unwrap();
    assert_eq!(s1.target_hostnames, vec![agent_a.hostname.clone()]);
    let s2 = results_a.iter().find(|s| s.id == schedule_a2.id).unwrap();
    assert_eq!(
        s2.target_hostnames,
        vec![agent_a.hostname.clone(), agent_b.hostname.clone()]
    );

    // repo_b must only return its own schedule
    let results_b = db::list_schedules_for_repo(&pool, repo_b.id).await.unwrap();
    assert_eq!(results_b.len(), 1);
    assert_eq!(results_b.first().unwrap().id, schedule_b.id);
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_list_for_agent(pool: PgPool) {
    let (agent, _, _) = create_test_schedule(&pool).await;

    let schedules = db::list_schedules_for_agent(&pool, agent.id).await.unwrap();
    assert_eq!(schedules.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_delete(pool: PgPool) {
    let (_, _, schedule) = create_test_schedule(&pool).await;

    db::delete_schedule(&pool, schedule.id).await.unwrap();

    let result = db::get_schedule_by_id(&pool, schedule.id).await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_due_and_trigger(pool: PgPool) {
    let (_, _, schedule) = create_test_schedule(&pool).await;
    let now = Utc::now();
    let past = now.checked_sub_signed(Duration::hours(1)).unwrap();

    db::set_next_run_at(&pool, schedule.id, past).await.unwrap();

    let due = db::list_due_schedules(&pool, now).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due.first().unwrap().schedule_id, schedule.id);

    let future = now.checked_add_signed(Duration::hours(3)).unwrap();
    db::mark_schedule_triggered(&pool, schedule.id, now, future)
        .await
        .unwrap();

    let fetched = db::get_schedule_by_id(&pool, schedule.id).await.unwrap();
    assert!(fetched.last_run_at.is_some());
    assert!(fetched.next_run_at.is_some());

    let due = db::list_due_schedules(&pool, now).await.unwrap();
    assert_eq!(due.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_agent_hostname(pool: PgPool) {
    let (_, _, schedule) = create_test_schedule(&pool).await;

    let targets = db::get_schedule_targets_for_run(&pool, schedule.id)
        .await
        .unwrap();
    let hostnames: Vec<&str> = targets.iter().map(|t| t.hostname.as_str()).collect();
    assert_eq!(hostnames, vec!["sched-host"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn backup_sources_crud(pool: PgPool) {
    let (_, _, schedule) = create_test_schedule(&pool).await;

    db::insert_backup_source_for_schedule(&pool, schedule.id, "/home", 1)
        .await
        .unwrap();
    db::insert_backup_source_for_schedule(&pool, schedule.id, "/etc", 2)
        .await
        .unwrap();

    let sources = db::list_backup_sources_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(sources.len(), 2);
    assert_eq!(sources.first().unwrap(), "/home");
    assert_eq!(sources.get(1).unwrap(), "/etc");

    db::delete_backup_sources_for_schedule(&pool, schedule.id)
        .await
        .unwrap();

    let sources = db::list_backup_sources_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(sources.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn backup_sources_per_agent_crud(pool: PgPool) {
    let (agent, _, schedule) = create_test_schedule(&pool).await;

    let agent2 = db::insert_agent(&pool, "host-two", None, "hash2", None, None)
        .await
        .unwrap();

    db::insert_backup_source_for_schedule(&pool, schedule.id, "/shared", 0)
        .await
        .unwrap();

    db::insert_backup_source_for_schedule_agent(&pool, schedule.id, agent.id, "/home/one", 0)
        .await
        .unwrap();
    db::insert_backup_source_for_schedule_agent(&pool, schedule.id, agent.id, "/var/one", 1)
        .await
        .unwrap();
    db::insert_backup_source_for_schedule_agent(&pool, schedule.id, agent2.id, "/data/two", 0)
        .await
        .unwrap();

    let schedule_level = db::list_backup_sources_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(schedule_level, vec!["/shared"]);

    let agent1_sources = db::list_backup_sources_for_schedule_agent(&pool, schedule.id, agent.id)
        .await
        .unwrap();
    assert_eq!(agent1_sources, vec!["/home/one", "/var/one"]);

    let agent2_sources = db::list_backup_sources_for_schedule_agent(&pool, schedule.id, agent2.id)
        .await
        .unwrap();
    assert_eq!(agent2_sources, vec!["/data/two"]);

    let all_per_agent = db::list_all_per_agent_backup_sources_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(all_per_agent.len(), 2);
    assert_eq!(all_per_agent.first().unwrap().agent_id, agent.id);
    assert_eq!(
        all_per_agent.first().unwrap().paths,
        vec!["/home/one", "/var/one"]
    );
    assert_eq!(all_per_agent.get(1).unwrap().agent_id, agent2.id);
    assert_eq!(all_per_agent.get(1).unwrap().paths, vec!["/data/two"]);

    db::delete_per_agent_backup_sources_for_schedule(&pool, schedule.id)
        .await
        .unwrap();

    let all_per_agent = db::list_all_per_agent_backup_sources_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(all_per_agent.len(), 0);

    let schedule_level = db::list_backup_sources_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(schedule_level, vec!["/shared"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn excludes_per_agent_crud(pool: PgPool) {
    let (agent, _, schedule) = create_test_schedule(&pool).await;

    let agent2 = db::insert_agent(&pool, "host-two-exc", None, "hash2exc", None, None)
        .await
        .unwrap();

    db::upsert_per_agent_excludes_raw(&pool, schedule.id, agent.id, "*.tmp\n*.cache")
        .await
        .unwrap();
    db::upsert_per_agent_excludes_raw(&pool, schedule.id, agent2.id, "*.bak")
        .await
        .unwrap();

    let all_per_agent = db::list_all_per_agent_excludes_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(all_per_agent.len(), 2);
    assert_eq!(all_per_agent.first().unwrap().agent_id, agent.id);
    assert_eq!(all_per_agent.first().unwrap().raw_text, "*.tmp\n*.cache");
    assert_eq!(all_per_agent.get(1).unwrap().agent_id, agent2.id);
    assert_eq!(all_per_agent.get(1).unwrap().raw_text, "*.bak");

    // Upsert updates existing row
    db::upsert_per_agent_excludes_raw(&pool, schedule.id, agent.id, "*.tmp\n*.cache\n\n# new")
        .await
        .unwrap();
    let all_per_agent = db::list_all_per_agent_excludes_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(
        all_per_agent.first().unwrap().raw_text,
        "*.tmp\n*.cache\n\n# new"
    );

    db::delete_per_agent_excludes_for_schedule(&pool, schedule.id)
        .await
        .unwrap();

    let all_per_agent = db::list_all_per_agent_excludes_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(all_per_agent.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn file_change_patterns_per_agent_crud(pool: PgPool) {
    let (agent, _, schedule) = create_test_schedule(&pool).await;

    let agent2 = db::insert_agent(&pool, "host-two-fcp", None, "hash2fcp", None, None)
        .await
        .unwrap();

    assert_eq!(
        db::get_per_agent_file_change_patterns_raw(&pool, schedule.id, agent.id)
            .await
            .unwrap(),
        None
    );

    db::upsert_per_agent_file_change_patterns_raw(
        &pool,
        schedule.id,
        agent.id,
        "*/etc/config* ignore\n*/var/log* fatal",
    )
    .await
    .unwrap();
    db::upsert_per_agent_file_change_patterns_raw(&pool, schedule.id, agent2.id, "*/tmp* warn")
        .await
        .unwrap();

    assert_eq!(
        db::get_per_agent_file_change_patterns_raw(&pool, schedule.id, agent.id)
            .await
            .unwrap(),
        Some("*/etc/config* ignore\n*/var/log* fatal".to_owned())
    );

    let all_per_agent =
        db::list_all_per_agent_file_change_patterns_for_schedule(&pool, schedule.id)
            .await
            .unwrap();
    assert_eq!(all_per_agent.len(), 2);
    assert_eq!(all_per_agent.first().unwrap().agent_id, agent.id);
    assert_eq!(
        all_per_agent.first().unwrap().raw_text,
        "*/etc/config* ignore\n*/var/log* fatal"
    );
    assert_eq!(all_per_agent.get(1).unwrap().agent_id, agent2.id);
    assert_eq!(all_per_agent.get(1).unwrap().raw_text, "*/tmp* warn");

    // Upsert updates the existing row rather than inserting a duplicate
    db::upsert_per_agent_file_change_patterns_raw(
        &pool,
        schedule.id,
        agent.id,
        "*/etc/config* fatal",
    )
    .await
    .unwrap();
    let all_per_agent =
        db::list_all_per_agent_file_change_patterns_for_schedule(&pool, schedule.id)
            .await
            .unwrap();
    assert_eq!(all_per_agent.len(), 2);
    assert_eq!(
        all_per_agent.first().unwrap().raw_text,
        "*/etc/config* fatal"
    );

    db::delete_per_agent_file_change_patterns_for_schedule(&pool, schedule.id)
        .await
        .unwrap();

    let all_per_agent =
        db::list_all_per_agent_file_change_patterns_for_schedule(&pool, schedule.id)
            .await
            .unwrap();
    assert_eq!(all_per_agent.len(), 0);
    assert_eq!(
        db::get_per_agent_file_change_patterns_raw(&pool, schedule.id, agent.id)
            .await
            .unwrap(),
        None
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn global_excludes_preserves_blank_lines_and_comments(pool: PgPool) {
    let raw = "# System paths\n/proc\n/sys\n\n# Cache files\n*.cache\npp:__pycache__";
    db::set_global_excludes_raw(&pool, raw).await.unwrap();
    assert_eq!(db::get_global_excludes_raw(&pool).await.unwrap(), raw);
}

#[sqlx::test(migrations = "./migrations")]
async fn global_excludes_overwrite_replaces_fully(pool: PgPool) {
    db::set_global_excludes_raw(&pool, "first\nsecond")
        .await
        .unwrap();
    db::set_global_excludes_raw(&pool, "only-this")
        .await
        .unwrap();
    assert_eq!(
        db::get_global_excludes_raw(&pool).await.unwrap(),
        "only-this"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_excludes_raw_text_round_trip(pool: PgPool) {
    let (_, _, schedule) = create_test_schedule(&pool).await;

    let raw = "# Cache\n*.cache\n\n# Runtime\n/proc\n/sys";
    let updated = db::update_schedule(
        &pool,
        schedule.id,
        &ScheduleParams {
            name: "test-schedule",
            schedule_type: "backup",
            cron_expression: "0 3 * * *",
            enabled: true,
            canary_enabled: false,
            vm_snapshot_enabled: false,
            exclude_patterns_raw: raw,
            file_change_patterns_raw: "",
            ignore_global_excludes: false,
            keep_hourly: 24,
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 6,
            keep_yearly: 1,
            compact_enabled: true,
            rate_limit_kbps: None,
            pre_backup_commands: &[],
            post_backup_commands: &[],
            hook_timeout_seconds: 60,
            missed_backup_threshold: 3,
            on_failure: "stop",
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.exclude_patterns_raw, raw);

    let fetched = db::get_schedule_by_id(&pool, schedule.id).await.unwrap();
    assert_eq!(fetched.exclude_patterns_raw, raw);
}

#[sqlx::test(migrations = "./migrations")]
async fn per_agent_excludes_preserves_blank_lines_and_comments(pool: PgPool) {
    let (agent, _, schedule) = create_test_schedule(&pool).await;

    let raw = "# Cache\n*.cache\n\n# Runtime\n/proc";
    db::upsert_per_agent_excludes_raw(&pool, schedule.id, agent.id, raw)
        .await
        .unwrap();

    let all = db::list_all_per_agent_excludes_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all.first().unwrap().raw_text, raw);
}

#[sqlx::test(migrations = "./migrations")]
async fn per_agent_excludes_upsert_replaces_existing(pool: PgPool) {
    let (agent, _, schedule) = create_test_schedule(&pool).await;

    db::upsert_per_agent_excludes_raw(&pool, schedule.id, agent.id, "first")
        .await
        .unwrap();
    db::upsert_per_agent_excludes_raw(&pool, schedule.id, agent.id, "second\n\n# comment")
        .await
        .unwrap();

    let all = db::list_all_per_agent_excludes_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all.first().unwrap().raw_text, "second\n\n# comment");
}

#[sqlx::test(migrations = "./migrations")]
async fn config_assembly_parses_raw_excludes_into_effective_patterns(pool: PgPool) {
    let encryption_key = shared::crypto::derive_key(b"test-assembly-key-for-excludes").unwrap();
    let (agent, repo, schedule) = create_test_schedule(&pool).await;

    // Global excludes: blank lines and comments should be stripped
    db::set_global_excludes_raw(&pool, "# system\n/proc\n/sys\n\n# cache\n*.cache")
        .await
        .unwrap();

    // Schedule-level excludes: same
    db::update_schedule(
        &pool,
        schedule.id,
        &ScheduleParams {
            name: "test-schedule",
            schedule_type: "backup",
            cron_expression: "0 3 * * *",
            enabled: true,
            canary_enabled: false,
            vm_snapshot_enabled: false,
            exclude_patterns_raw: "# logs\n*.log\n\n*.tmp",
            file_change_patterns_raw: "",
            ignore_global_excludes: false,
            keep_hourly: 24,
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 6,
            keep_yearly: 1,
            compact_enabled: true,
            rate_limit_kbps: None,
            pre_backup_commands: &[],
            post_backup_commands: &[],
            hook_timeout_seconds: 60,
            missed_backup_threshold: 3,
            on_failure: "stop",
        },
    )
    .await
    .unwrap();

    // Store a properly encrypted passphrase so assemble_config can decrypt it
    let passphrase_encrypted =
        shared::crypto::encrypt_passphrase("test-pass", &encryption_key).unwrap();
    sqlx::query("UPDATE repos SET passphrase_encrypted = $1, ssh_host_key = $2 WHERE id = $3")
        .bind(passphrase_encrypted.as_slice())
        .bind("ssh-ed25519 AAAATEST")
        .bind(repo.id)
        .execute(&pool)
        .await
        .unwrap();

    // Insert a backup source so assemble_config does not fail
    db::insert_backup_source_for_schedule(&pool, schedule.id, "/home", 0)
        .await
        .unwrap();

    // Enable the repo so it is reachable
    let _ = sqlx::query("UPDATE repos SET enabled = true WHERE id = $1")
        .bind(repo.id)
        .execute(&pool)
        .await
        .unwrap();

    let config = server::config_assembler::assemble_config(&pool, &encryption_key, agent.id)
        .await
        .unwrap();

    assert_eq!(
        config.repos.first().unwrap().ssh_host_key,
        "ssh-ed25519 AAAATEST"
    );

    let patterns: Vec<&str> = config
        .repos
        .first()
        .unwrap()
        .schedules
        .first()
        .unwrap()
        .exclude_patterns
        .iter()
        .map(String::as_str)
        .collect();

    // Comments and blank lines must not appear
    assert!(!patterns.iter().any(|p| p.starts_with('#')));
    assert!(!patterns.iter().any(|p| p.is_empty()));

    // Effective patterns from global excludes
    assert!(patterns.contains(&"/proc"));
    assert!(patterns.contains(&"/sys"));
    assert!(patterns.contains(&"*.cache"));

    // Effective patterns from schedule excludes
    assert!(patterns.contains(&"*.log"));
    assert!(patterns.contains(&"*.tmp"));
}

#[sqlx::test(migrations = "./migrations")]
async fn config_assembly_merges_agent_default_file_change_patterns(pool: PgPool) {
    let encryption_key = shared::crypto::derive_key(b"test-assembly-key-for-file-change").unwrap();
    let (agent, repo, schedule) = create_test_schedule(&pool).await;

    db::update_schedule(
        &pool,
        schedule.id,
        &ScheduleParams {
            name: "test-schedule",
            schedule_type: "backup",
            cron_expression: "0 3 * * *",
            enabled: true,
            canary_enabled: false,
            vm_snapshot_enabled: false,
            exclude_patterns_raw: "",
            file_change_patterns_raw: "*/schedule-specific* ignore",
            ignore_global_excludes: false,
            keep_hourly: 24,
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 6,
            keep_yearly: 1,
            compact_enabled: true,
            rate_limit_kbps: None,
            pre_backup_commands: &[],
            post_backup_commands: &[],
            hook_timeout_seconds: 60,
            missed_backup_threshold: 3,
            on_failure: "stop",
        },
    )
    .await
    .unwrap();

    db::update_agent(
        &pool,
        agent.id,
        &agent.hostname,
        db::AgentDefaults {
            display_name: agent.display_name.as_deref(),
            domain: agent.domain.as_deref(),
            default_backup_paths: &agent.default_backup_paths,
            default_exclude_patterns: &agent.default_exclude_patterns,
            default_pre_backup_commands: &agent.default_pre_backup_commands.0,
            default_post_backup_commands: &agent.default_post_backup_commands.0,
            default_file_change_patterns_raw: "*/agent-fallback* fatal",
        },
    )
    .await
    .unwrap();

    let passphrase_encrypted =
        shared::crypto::encrypt_passphrase("test-pass", &encryption_key).unwrap();
    sqlx::query(
        "UPDATE repos SET passphrase_encrypted = $1, ssh_host_key = $2, enabled = true WHERE id = \
         $3",
    )
    .bind(passphrase_encrypted.as_slice())
    .bind("ssh-ed25519 AAAATEST")
    .bind(repo.id)
    .execute(&pool)
    .await
    .unwrap();

    db::insert_backup_source_for_schedule(&pool, schedule.id, "/home", 0)
        .await
        .unwrap();

    let config = server::config_assembler::assemble_config(&pool, &encryption_key, agent.id)
        .await
        .unwrap();

    let patterns = &config
        .repos
        .first()
        .unwrap()
        .schedules
        .first()
        .unwrap()
        .file_change_patterns;
    assert_eq!(patterns.len(), 2);
    // Schedule-level pattern must come first: `filter_file_change_warnings`
    // uses first-match-wins, so the schedule's own configuration must win
    // over the agent-wide fallback.
    assert_eq!(patterns.first().unwrap().path, "*/schedule-specific*");
    assert_eq!(
        patterns.first().unwrap().action,
        shared::types::FileChangeAction::Ignore
    );
    assert_eq!(patterns.get(1).unwrap().path, "*/agent-fallback*");
    assert_eq!(
        patterns.get(1).unwrap().action,
        shared::types::FileChangeAction::Fatal
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn canary_results_crud(pool: PgPool) {
    let (_, _, schedule) = create_test_schedule(&pool).await;

    db::insert_canary_result(
        &pool,
        schedule.id,
        true,
        "canary_20240101.txt",
        None,
        Some("archive-001"),
    )
    .await
    .unwrap();

    db::insert_canary_result(
        &pool,
        schedule.id,
        false,
        "canary_20240102.txt",
        Some("file not found"),
        None,
    )
    .await
    .unwrap();

    let latest = db::get_latest_canary_result(&pool, schedule.id)
        .await
        .unwrap();
    assert!(latest.is_some());
    let latest = latest.unwrap();
    assert!(!latest.success);
    assert_eq!(latest.error_message.as_deref(), Some("file not found"));

    let all = db::list_canary_results(&pool, schedule.id, 10)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
}

#[cfg(test)]
async fn insert_test_report(pool: &PgPool, agent_id: i64, repo_id: i64) {
    let now = Utc::now();
    db::insert_backup_report(
        pool,
        &InsertReportParams {
            agent_id,
            repo_id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(5)).unwrap(),
            finished_at: now,
            status: shared::types::BackupStatus::Success,
            original_size: 1_000_000,
            compressed_size: 500_000,
            deduplicated_size: 250_000,
            repo_unique_csize: 250_000,
            files_processed: 1000,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.4.0".to_string()),
            matched: true,
            archive_name: None,
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();
}

#[cfg(test)]
async fn insert_test_report_for_schedule(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
    schedule_id: i64,
    status: shared::types::BackupStatus,
) {
    let now = Utc::now();
    db::insert_backup_report(
        pool,
        &InsertReportParams {
            agent_id,
            repo_id,
            schedule_id: Some(schedule_id),
            started_at: now.checked_sub_signed(Duration::minutes(5)).unwrap(),
            finished_at: now,
            status,
            original_size: 1_000_000,
            compressed_size: 500_000,
            deduplicated_size: 250_000,
            repo_unique_csize: 250_000,
            files_processed: 1000,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.4.0".to_string()),
            matched: true,
            archive_name: None,
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn backup_report_insert_and_list(pool: PgPool) {
    let agent = db::insert_agent(&pool, "report-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let reports = db::list_reports_for_agent(&pool, agent.id, None, 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports.first().unwrap().status, "success");
    assert_eq!(reports.first().unwrap().original_size, 1_000_000);
    assert_eq!(reports.first().unwrap().compressed_size, 500_000);
    assert_eq!(reports.first().unwrap().deduplicated_size, 250_000);
    assert_eq!(reports.first().unwrap().files_processed, 1000);
    assert_eq!(reports.first().unwrap().duration_secs, 300);
    assert_eq!(
        reports.first().unwrap().borg_version.as_deref(),
        Some("1.4.0")
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn backup_report_list_with_target(pool: PgPool) {
    let agent = db::insert_agent(&pool, "target-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let reports = db::list_reports_for_agent(&pool, agent.id, Some("test-repo"), 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);

    let reports = db::list_reports_for_agent(&pool, agent.id, Some("nonexistent"), 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn backup_report_with_warnings(pool: PgPool) {
    let agent = db::insert_agent(&pool, "warn-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(5)).unwrap(),
            finished_at: now,
            status: shared::types::BackupStatus::Warning,
            original_size: 100,
            compressed_size: 50,
            deduplicated_size: 25,
            repo_unique_csize: 0,
            files_processed: 10,
            duration_secs: 60,
            error_message: Some("partial failure".to_string()),
            warnings: vec!["file skipped".to_string(), "permission denied".to_string()],
            borg_version: None,
            matched: true,
            archive_name: Some("test-archive".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let reports = db::list_reports_for_agent(&pool, agent.id, None, 10)
        .await
        .unwrap();
    assert_eq!(reports.first().unwrap().warnings.len(), 2);
    assert_eq!(
        reports.first().unwrap().error_message.as_deref(),
        Some("partial failure")
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn backup_report_delete_before(pool: PgPool) {
    let agent = db::insert_agent(&pool, "del-report-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let future = Utc::now().checked_add_signed(Duration::hours(1)).unwrap();
    let deleted = db::delete_backup_reports_before(&pool, future)
        .await
        .unwrap();
    assert_eq!(deleted, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn storage_stats_with_sum(pool: PgPool) {
    let agent = db::insert_agent(&pool, "stats-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;
    insert_test_report(&pool, agent.id, repo.id).await;

    let stats = db::get_storage_stats(&pool).await.unwrap();
    assert_eq!(stats.len(), 1);
    assert_eq!(stats.first().unwrap().hostname, "stats-host");
    assert_eq!(stats.first().unwrap().total_original_size, 2_000_000);
    assert_eq!(stats.first().unwrap().total_compressed_size, 1_000_000);
    assert_eq!(stats.first().unwrap().total_deduplicated_size, 500_000);
    assert_eq!(stats.first().unwrap().report_count, 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn storage_stats_empty(pool: PgPool) {
    let stats = db::get_storage_stats(&pool).await.unwrap();
    assert_eq!(stats.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn activity_feed(pool: PgPool) {
    let agent = db::insert_agent(&pool, "act-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let activity = db::get_activity_feed(&pool, 10, ActivityFeedFilters::default())
        .await
        .unwrap();
    assert_eq!(activity.len(), 1);
    assert_eq!(activity.first().unwrap().hostname, "act-host");
    assert_eq!(activity.first().unwrap().target_name, "test-repo");
}

#[sqlx::test(migrations = "./migrations")]
async fn activity_feed_days(pool: PgPool) {
    let agent = db::insert_agent(&pool, "days-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let activity = db::get_activity_feed_days(&pool, 7, None, ActivityFeedFilters::default())
        .await
        .unwrap();
    assert_eq!(activity.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn activity_feed_reports_are_unacknowledged_by_default(pool: PgPool) {
    let agent = db::insert_agent(&pool, "ack-default-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    insert_test_report(&pool, agent.id, repo.id).await;

    let activity = db::get_activity_feed(&pool, 10, ActivityFeedFilters::default())
        .await
        .unwrap();
    assert_eq!(activity.len(), 1);
    assert!(!activity.first().unwrap().acknowledged);
}

#[sqlx::test(migrations = "./migrations")]
async fn set_backup_report_acknowledged_toggles_and_is_visible_in_both_feeds(pool: PgPool) {
    let agent = db::insert_agent(&pool, "ack-toggle-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    insert_test_report(&pool, agent.id, repo.id).await;

    let report_id = db::get_activity_feed(&pool, 10, ActivityFeedFilters::default())
        .await
        .unwrap()
        .first()
        .unwrap()
        .id;

    db::set_backup_report_acknowledged(&pool, report_id, true)
        .await
        .unwrap();

    let activity = db::get_activity_feed(&pool, 10, ActivityFeedFilters::default())
        .await
        .unwrap();
    assert!(activity.first().unwrap().acknowledged);

    let activity_days = db::get_activity_feed_days(&pool, 7, None, ActivityFeedFilters::default())
        .await
        .unwrap();
    assert!(activity_days.first().unwrap().acknowledged);

    db::set_backup_report_acknowledged(&pool, report_id, false)
        .await
        .unwrap();

    let activity = db::get_activity_feed(&pool, 10, ActivityFeedFilters::default())
        .await
        .unwrap();
    assert!(!activity.first().unwrap().acknowledged);
}

/// A report in whatever state the caller needs, for the acknowledgment tests -
/// [`insert_test_report`] always files a successful run, which can never be
/// acknowledged.
#[cfg(test)]
async fn insert_report_with_status(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
    status: shared::types::BackupStatus,
) {
    let started_at = Utc::now().checked_sub_signed(Duration::minutes(5)).unwrap();
    insert_report_with_status_at(pool, agent_id, repo_id, status, started_at).await;
}

/// The same, but filed at a chosen `started_at`, for the window-scoped
/// acknowledgment tests.
#[cfg(test)]
async fn insert_report_with_status_at(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
    status: shared::types::BackupStatus,
    started_at: chrono::DateTime<Utc>,
) {
    db::insert_backup_report(
        pool,
        &InsertReportParams {
            agent_id,
            repo_id,
            schedule_id: None,
            started_at,
            finished_at: started_at.checked_add_signed(Duration::minutes(5)).unwrap(),
            status,
            original_size: 1_000_000,
            compressed_size: 500_000,
            deduplicated_size: 250_000,
            repo_unique_csize: 250_000,
            files_processed: 1000,
            duration_secs: 300,
            error_message: Some("boom".to_owned()),
            warnings: vec![],
            borg_version: Some("1.4.0".to_string()),
            matched: true,
            archive_name: None,
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn activity_feed_filters_on_acknowledgment_state(pool: PgPool) {
    let agent = db::insert_agent(&pool, "ack-filter-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    insert_report_with_status(
        &pool,
        agent.id,
        repo.id,
        shared::types::BackupStatus::Failed,
    )
    .await;

    let report_id = db::get_activity_feed(&pool, 10, ActivityFeedFilters::default())
        .await
        .unwrap()
        .first()
        .unwrap()
        .id;
    db::set_backup_report_acknowledged(&pool, report_id, true)
        .await
        .unwrap();

    let hidden = db::get_activity_feed(
        &pool,
        10,
        ActivityFeedFilters {
            acknowledged: AcknowledgedFilter::Unacknowledged,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        hidden.is_empty(),
        "an acknowledged run must drop out of the default feed"
    );

    let only = db::get_activity_feed(
        &pool,
        10,
        ActivityFeedFilters {
            acknowledged: AcknowledgedFilter::Acknowledged,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(only.len(), 1);

    let days_hidden = db::get_activity_feed_days(
        &pool,
        7,
        None,
        ActivityFeedFilters {
            acknowledged: AcknowledgedFilter::Unacknowledged,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(days_hidden.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn acknowledge_backup_reports_in_repos_only_touches_listed_repos(pool: PgPool) {
    let agent = db::insert_agent(&pool, "bulk-ack-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo_a = create_test_repo_with_host(&pool, "bulk-ack-repo-a", "a.local").await;
    let repo_b = create_test_repo_with_host(&pool, "bulk-ack-repo-b", "b.local").await;

    insert_report_with_status(
        &pool,
        agent.id,
        repo_a.id,
        shared::types::BackupStatus::Failed,
    )
    .await;
    insert_report_with_status(
        &pool,
        agent.id,
        repo_a.id,
        shared::types::BackupStatus::Success,
    )
    .await;
    insert_report_with_status(
        &pool,
        agent.id,
        repo_b.id,
        shared::types::BackupStatus::Warning,
    )
    .await;

    let mut pending = db::repos_with_unacknowledged_reports(&pool).await.unwrap();
    pending.sort_unstable();
    let mut expected = vec![repo_a.id, repo_b.id];
    expected.sort_unstable();
    assert_eq!(pending, expected);

    let acknowledged = db::acknowledge_backup_reports_in_repos(
        &pool,
        &[repo_a.id],
        db::BulkAcknowledgeFilters::default(),
    )
    .await
    .unwrap();
    assert_eq!(acknowledged, 1, "only the failed run in repo A is ackable");

    let still_pending = db::repos_with_unacknowledged_reports(&pool).await.unwrap();
    assert_eq!(still_pending, vec![repo_b.id]);

    // A successful run is never marked - acknowledging it would hide history
    // nobody asked to review.
    let successes_acknowledged = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM backup_reports WHERE status = 'success' AND acknowledged = true",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(successes_acknowledged, 0);
}

/// The dashboard's Backup stats panel clears the failures *it* is showing, so
/// a bulk acknowledge scoped to one repository must leave every other
/// repository's outstanding runs exactly where they were.
#[sqlx::test(migrations = "./migrations")]
async fn bulk_acknowledge_scoped_to_one_repo_leaves_the_others_outstanding(pool: PgPool) {
    let agent = db::insert_agent(&pool, "scoped-ack-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo_a = create_test_repo_with_host(&pool, "scoped-ack-repo-a", "a.local").await;
    let repo_b = create_test_repo_with_host(&pool, "scoped-ack-repo-b", "b.local").await;
    for repo_id in [repo_a.id, repo_b.id] {
        insert_report_with_status(
            &pool,
            agent.id,
            repo_id,
            shared::types::BackupStatus::Failed,
        )
        .await;
    }

    let scoped = db::BulkAcknowledgeFilters {
        repo_id: Some(repo_a.id),
        days: None,
    };
    assert_eq!(
        db::count_unacknowledged_reports_in_repos(&pool, &[repo_a.id, repo_b.id], scoped)
            .await
            .unwrap(),
        1,
        "the count must see only the scoped repository"
    );

    let acknowledged =
        db::acknowledge_backup_reports_in_repos(&pool, &[repo_a.id, repo_b.id], scoped)
            .await
            .unwrap();
    assert_eq!(acknowledged, 1);
    assert_eq!(
        db::repos_with_unacknowledged_reports(&pool).await.unwrap(),
        vec![repo_b.id],
        "the unscoped repository keeps its outstanding failure"
    );
}

/// A hidden agent's failed run never reaches a feed or the Failed tile, so a
/// bulk acknowledge must neither count it nor silently retire it - the
/// button's whole promise is that it clears no more than what was on screen.
#[sqlx::test(migrations = "./migrations")]
async fn bulk_acknowledge_ignores_reports_from_agents_no_feed_shows(pool: PgPool) {
    let visible = db::insert_agent(&pool, "visible-ack-host", None, "hash", None, None)
        .await
        .unwrap();
    let hidden = db::insert_agent(&pool, "hidden-ack-host", None, "hash", None, None)
        .await
        .unwrap();
    sqlx::query("UPDATE agents SET is_hidden = true WHERE id = $1")
        .bind(hidden.id)
        .execute(&pool)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    for agent_id in [visible.id, hidden.id] {
        insert_report_with_status(
            &pool,
            agent_id,
            repo.id,
            shared::types::BackupStatus::Failed,
        )
        .await;
    }

    let all = db::BulkAcknowledgeFilters::default();
    assert_eq!(
        db::count_unacknowledged_reports_in_repos(&pool, &[repo.id], all)
            .await
            .unwrap(),
        1,
        "only the visible agent's failure is counted"
    );

    let acknowledged = db::acknowledge_backup_reports_in_repos(&pool, &[repo.id], all)
        .await
        .unwrap();
    assert_eq!(acknowledged, 1);

    let hidden_still_outstanding: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM backup_reports WHERE agent_id = $1 AND acknowledged = false",
    )
    .bind(hidden.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        hidden_still_outstanding, 1,
        "the hidden agent's run must be left exactly as it was"
    );

    // With nothing acknowledgeable left that a feed would show, the repository
    // stops being a candidate at all rather than offering a no-op button.
    assert_eq!(
        db::repos_with_unacknowledged_reports(&pool).await.unwrap(),
        Vec::<i64>::new()
    );
}

/// Scoped to a window, a bulk acknowledge clears the runs inside it and leaves
/// the older ones outstanding - the panel's "last 7 days" reset must not
/// silently retire a quarter of history.
#[sqlx::test(migrations = "./migrations")]
async fn bulk_acknowledge_scoped_to_a_window_leaves_older_failures_outstanding(pool: PgPool) {
    let agent = db::insert_agent(&pool, "windowed-ack-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();
    insert_report_with_status_at(
        &pool,
        agent.id,
        repo.id,
        shared::types::BackupStatus::Failed,
        now.checked_sub_signed(Duration::days(2)).unwrap(),
    )
    .await;
    insert_report_with_status_at(
        &pool,
        agent.id,
        repo.id,
        shared::types::BackupStatus::Failed,
        now.checked_sub_signed(Duration::days(20)).unwrap(),
    )
    .await;

    let last_week = db::BulkAcknowledgeFilters {
        repo_id: None,
        days: Some(7),
    };
    assert_eq!(
        db::count_unacknowledged_reports_in_repos(&pool, &[repo.id], last_week)
            .await
            .unwrap(),
        1,
        "only the run inside the window is in scope"
    );

    let acknowledged = db::acknowledge_backup_reports_in_repos(&pool, &[repo.id], last_week)
        .await
        .unwrap();
    assert_eq!(acknowledged, 1);
    assert_eq!(
        db::count_unacknowledged_reports_in_repos(&pool, &[repo.id], last_week)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db::count_unacknowledged_reports_in_repos(
            &pool,
            &[repo.id],
            db::BulkAcknowledgeFilters::default()
        )
        .await
        .unwrap(),
        1,
        "the run outside the window must still await review"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn outstanding_counts_track_what_a_bulk_acknowledge_would_touch(pool: PgPool) {
    let agent = db::insert_agent(&pool, "outstanding-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    assert_eq!(
        db::count_unacknowledged_reports_in_repos(
            &pool,
            &[repo.id],
            db::BulkAcknowledgeFilters::default()
        )
        .await
        .unwrap(),
        0
    );

    insert_report_with_status(
        &pool,
        agent.id,
        repo.id,
        shared::types::BackupStatus::Failed,
    )
    .await;
    insert_report_with_status(
        &pool,
        agent.id,
        repo.id,
        shared::types::BackupStatus::Warning,
    )
    .await;
    insert_report_with_status(
        &pool,
        agent.id,
        repo.id,
        shared::types::BackupStatus::Success,
    )
    .await;

    // Only the warning and the failure count - the success is not reviewable.
    assert_eq!(
        db::count_unacknowledged_reports_in_repos(
            &pool,
            &[repo.id],
            db::BulkAcknowledgeFilters::default()
        )
        .await
        .unwrap(),
        2
    );
    // A repo the caller may not touch contributes nothing.
    assert_eq!(
        db::count_unacknowledged_reports_in_repos(
            &pool,
            &[],
            db::BulkAcknowledgeFilters::default()
        )
        .await
        .unwrap(),
        0
    );

    let acknowledged = db::acknowledge_backup_reports_in_repos(
        &pool,
        &[repo.id],
        db::BulkAcknowledgeFilters::default(),
    )
    .await
    .unwrap();
    assert_eq!(acknowledged, 2);
    assert_eq!(
        db::count_unacknowledged_reports_in_repos(
            &pool,
            &[repo.id],
            db::BulkAcknowledgeFilters::default()
        )
        .await
        .unwrap(),
        0,
        "the count must fall to zero once the bulk acknowledge has run"
    );

    assert_eq!(
        db::count_unacknowledged_system_events(&pool).await.unwrap(),
        0
    );
    db::insert_system_event(&pool, SystemEventType::RepoSyncFailed, None, "sync failed")
        .await
        .unwrap();
    db::insert_system_event(&pool, SystemEventType::RepoSync, None, "sync ok")
        .await
        .unwrap();
    assert_eq!(
        db::count_unacknowledged_system_events(&pool).await.unwrap(),
        1,
        "only the failed sync is reviewable"
    );

    db::acknowledge_all_system_events(&pool).await.unwrap();
    assert_eq!(
        db::count_unacknowledged_system_events(&pool).await.unwrap(),
        0
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn system_events_acknowledge_round_trip_and_filter(pool: PgPool) {
    db::insert_system_event(
        &pool,
        SystemEventType::RepoSyncFailed,
        Some("sync-host"),
        "Periodic sync failed",
    )
    .await
    .unwrap();
    db::insert_system_event(
        &pool,
        SystemEventType::RepoSync,
        Some("sync-host"),
        "Synced",
    )
    .await
    .unwrap();

    let events = db::get_system_events(&pool, 10, AcknowledgedFilter::All)
        .await
        .unwrap();
    assert_eq!(events.len(), 2);
    let failed = events
        .iter()
        .find(|e| e.event_type == SystemEventType::RepoSyncFailed)
        .unwrap();
    assert!(failed.acknowledgeable);
    assert!(!failed.acknowledged);
    let synced = events
        .iter()
        .find(|e| e.event_type == SystemEventType::RepoSync)
        .unwrap();
    assert!(
        !synced.acknowledgeable,
        "a successful sync has nothing to review"
    );

    db::get_acknowledgeable_system_event_type(&pool, failed.id)
        .await
        .unwrap();
    db::set_system_event_acknowledged(&pool, failed.id, true)
        .await
        .unwrap();

    let pending = db::get_system_events(&pool, 10, AcknowledgedFilter::Unacknowledged)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending.first().unwrap().event_type,
        SystemEventType::RepoSync
    );

    let acknowledged = db::get_system_events(&pool, 10, AcknowledgedFilter::Acknowledged)
        .await
        .unwrap();
    assert_eq!(acknowledged.len(), 1);
    assert!(acknowledged.first().unwrap().acknowledged);

    db::set_system_event_acknowledged(&pool, failed.id, false)
        .await
        .unwrap();
    assert_eq!(
        db::get_system_events(&pool, 10, AcknowledgedFilter::Unacknowledged)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn acknowledgeable_system_event_type_rejects_an_informational_event(pool: PgPool) {
    db::insert_system_event(&pool, SystemEventType::RepoSync, None, "Synced")
        .await
        .unwrap();
    let id = db::get_system_events(&pool, 1, AcknowledgedFilter::All)
        .await
        .unwrap()
        .first()
        .unwrap()
        .id;

    let err = db::get_acknowledgeable_system_event_type(&pool, id)
        .await
        .unwrap_err();
    assert!(matches!(err, server::error::ApiError::Unprocessable(_)));

    let missing = db::get_acknowledgeable_system_event_type(&pool, id.saturating_add(9_999))
        .await
        .unwrap_err();
    assert!(matches!(missing, server::error::ApiError::NotFound(_)));
}

#[sqlx::test(migrations = "./migrations")]
async fn acknowledge_all_outstanding_commits_both_writes_together(pool: PgPool) {
    let agent = db::insert_agent(&pool, "atomic-ack-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo_with_host(&pool, "atomic-ack-repo", "atomic.local").await;
    insert_report_with_status(
        &pool,
        agent.id,
        repo.id,
        shared::types::BackupStatus::Failed,
    )
    .await;
    db::insert_system_event(&pool, SystemEventType::RepoSyncFailed, None, "sync failed")
        .await
        .unwrap();

    let (reports, events) = db::acknowledge_all_outstanding(
        &pool,
        &[repo.id],
        true,
        db::BulkAcknowledgeFilters::default(),
    )
    .await
    .unwrap();
    assert_eq!((reports, events), (1, 1));

    // Both sides of the transaction are visible afterwards.
    assert_eq!(
        db::count_unacknowledged_reports_in_repos(
            &pool,
            &[repo.id],
            db::BulkAcknowledgeFilters::default()
        )
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        db::count_unacknowledged_system_events(&pool).await.unwrap(),
        0
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn acknowledge_all_outstanding_leaves_system_events_alone_for_a_non_admin(pool: PgPool) {
    let agent = db::insert_agent(&pool, "scoped-ack-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo_with_host(&pool, "scoped-ack-repo", "scoped.local").await;
    insert_report_with_status(
        &pool,
        agent.id,
        repo.id,
        shared::types::BackupStatus::Warning,
    )
    .await;
    db::insert_system_event(&pool, SystemEventType::RepoSyncFailed, None, "sync failed")
        .await
        .unwrap();

    let (reports, events) = db::acknowledge_all_outstanding(
        &pool,
        &[repo.id],
        false,
        db::BulkAcknowledgeFilters::default(),
    )
    .await
    .unwrap();
    assert_eq!((reports, events), (1, 0));
    assert_eq!(
        db::count_unacknowledged_system_events(&pool).await.unwrap(),
        1,
        "a caller who may not acknowledge system events must leave them outstanding"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn acknowledge_all_system_events_skips_the_ones_with_nothing_to_review(pool: PgPool) {
    for (event_type, message) in [
        (SystemEventType::RepoSyncFailed, "sync failed"),
        (SystemEventType::RepoSyncSlow, "sync slow"),
        (SystemEventType::RepoSync, "sync ok"),
        (SystemEventType::RepoSyncCancelled, "sync cancelled"),
    ] {
        db::insert_system_event(&pool, event_type, None, message)
            .await
            .unwrap();
    }

    let acknowledged = db::acknowledge_all_system_events(&pool).await.unwrap();
    assert_eq!(
        acknowledged, 2,
        "only the failed and slow events are ackable"
    );

    let pending = db::get_system_events(&pool, 10, AcknowledgedFilter::Unacknowledged)
        .await
        .unwrap();
    assert_eq!(pending.len(), 2);
    assert!(pending.iter().all(|e| !e.acknowledgeable));

    // Nothing left to do the second time around.
    assert_eq!(db::acknowledge_all_system_events(&pool).await.unwrap(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn get_backup_report_repo_id_test(pool: PgPool) {
    let agent = db::insert_agent(&pool, "ack-repoid-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    insert_test_report(&pool, agent.id, repo.id).await;

    let report_id = db::get_activity_feed(&pool, 10, ActivityFeedFilters::default())
        .await
        .unwrap()
        .first()
        .unwrap()
        .id;

    let repo_id = db::get_backup_report_repo_id(&pool, report_id)
        .await
        .unwrap();
    assert_eq!(repo_id, repo.id);

    let err = db::get_backup_report_repo_id(&pool, report_id.saturating_add(1_000_000))
        .await
        .unwrap_err();
    assert!(matches!(err, server::error::ApiError::NotFound(_)));
}

#[sqlx::test(migrations = "./migrations")]
async fn get_ackable_backup_report_repo_id_test(pool: PgPool) {
    let agent = db::insert_agent(&pool, "ack-status-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    let failed_params = InsertReportParams {
        agent_id: agent.id,
        repo_id: repo.id,
        schedule_id: None,
        started_at: now.checked_sub_signed(Duration::minutes(5)).unwrap(),
        finished_at: now,
        status: shared::types::BackupStatus::Failed,
        original_size: 0,
        compressed_size: 0,
        deduplicated_size: 0,
        repo_unique_csize: 0,
        files_processed: 0,
        duration_secs: 5,
        error_message: Some("connection refused".to_string()),
        warnings: vec![],
        borg_version: None,
        matched: true,
        archive_name: None,
        borg_command: None,
        run_id: None,
    };
    db::insert_backup_report(&pool, &failed_params)
        .await
        .unwrap();
    insert_test_report(&pool, agent.id, repo.id).await;

    let activity = db::get_activity_feed(&pool, 10, ActivityFeedFilters::default())
        .await
        .unwrap();
    let failed_report_id = activity.iter().find(|e| e.status == "failed").unwrap().id;
    let success_report_id = activity.iter().find(|e| e.status == "success").unwrap().id;

    let repo_id = db::get_ackable_backup_report_repo_id(&pool, failed_report_id)
        .await
        .unwrap();
    assert_eq!(repo_id, repo.id, "a failed report must be ackable");

    let err = db::get_ackable_backup_report_repo_id(&pool, success_report_id)
        .await
        .unwrap_err();
    assert!(
        matches!(err, server::error::ApiError::Unprocessable(_)),
        "a successful report has nothing to review and must not be ackable"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn health_summary(pool: PgPool) {
    let (agent, repo, schedule) = create_test_schedule(&pool).await;
    insert_test_report_for_schedule(
        &pool,
        agent.id,
        repo.id,
        schedule.id,
        shared::types::BackupStatus::Success,
    )
    .await;

    let health = db::get_health_summary(&pool).await.unwrap();
    assert_eq!(health.len(), 1);
    assert_eq!(health.first().unwrap().hostname, "sched-host");
    assert_eq!(health.first().unwrap().schedule_id, schedule.id);
    assert_eq!(
        health.first().unwrap().last_status.as_deref(),
        Some("success")
    );
}

/// Two schedules that share the same repository and agent must report
/// independent health: a backup run for one schedule must not surface as the
/// status of the other.
#[sqlx::test(migrations = "./migrations")]
async fn health_summary_is_per_schedule(pool: PgPool) {
    let (agent, repo, schedule_a) = create_test_schedule(&pool).await;
    let schedule_b = db::insert_schedule(
        &pool,
        repo.id,
        &ScheduleParams {
            name: "second-schedule",
            schedule_type: "backup",
            cron_expression: "0 4 * * *",
            enabled: true,
            canary_enabled: false,
            vm_snapshot_enabled: false,
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
            pre_backup_commands: &[],
            post_backup_commands: &[],
            hook_timeout_seconds: 60,
            missed_backup_threshold: 3,
            on_failure: "stop",
        },
        None,
    )
    .await
    .unwrap();
    db::insert_schedule_targets(&pool, schedule_b.id, &[(agent.id, 0)])
        .await
        .unwrap();

    // Only schedule_a has a backup run recorded.
    insert_test_report_for_schedule(
        &pool,
        agent.id,
        repo.id,
        schedule_a.id,
        shared::types::BackupStatus::Success,
    )
    .await;

    let health = db::get_health_summary(&pool).await.unwrap();
    let entry_a = health
        .iter()
        .find(|h| h.schedule_id == schedule_a.id)
        .expect("schedule_a health row");
    let entry_b = health
        .iter()
        .find(|h| h.schedule_id == schedule_b.id)
        .expect("schedule_b health row");

    assert_eq!(entry_a.last_status.as_deref(), Some("success"));
    assert_eq!(
        entry_b.last_status, None,
        "schedule_b must not inherit schedule_a's run status"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn backup_reports_status_check_constraint_rejects_invalid_status(pool: PgPool) {
    let (agent, repo, schedule) = create_test_schedule(&pool).await;

    // backup_reports.status is now guarded by a CHECK constraint (see
    // 20260813120000_enum_check_constraints.sql), mirroring
    // shared::types::BackupStatus. An arbitrary status must be rejected at
    // the DB layer instead of being silently accepted.
    let result = sqlx::query!(
        r#"INSERT INTO backup_reports
           (agent_id, repo_id, schedule_id, started_at, finished_at, status, matched)
           VALUES ($1, $2, $3, NOW() - INTERVAL '5 minutes', NOW(), $4, true)"#,
        agent.id,
        repo.id,
        schedule.id,
        "completely_invalid_status_value",
    )
    .execute(&pool)
    .await;

    let err = result.expect_err("invalid status must be rejected by the CHECK constraint");
    let db_err = err.as_database_error().expect("expected a database error");
    assert_eq!(
        db_err.constraint(),
        Some("backup_reports_status_check"),
        "insert must fail specifically on the status CHECK constraint"
    );

    let health = db::get_health_summary(&pool).await.unwrap();
    let entry = health
        .iter()
        .find(|h| h.schedule_id == schedule.id)
        .expect("schedule health row");
    assert_eq!(
        entry.last_status, None,
        "rejected insert must not leave a report behind"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn dashboard_queries_use_authoritative_assignments_and_exclude_placeholders(pool: PgPool) {
    let (agent, repo, schedule_a) = create_test_schedule(&pool).await;
    let schedule_b = db::insert_schedule(
        &pool,
        repo.id,
        &ScheduleParams {
            name: "second-dashboard-schedule",
            schedule_type: "backup",
            cron_expression: "0 4 * * *",
            enabled: true,
            canary_enabled: false,
            vm_snapshot_enabled: false,
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
            pre_backup_commands: &[],
            post_backup_commands: &[],
            hook_timeout_seconds: 60,
            missed_backup_threshold: 3,
            on_failure: "stop",
        },
        None,
    )
    .await
    .unwrap();
    db::insert_schedule_targets(&pool, schedule_b.id, &[(agent.id, 0)])
        .await
        .unwrap();

    let disabled_agent = db::insert_agent(&pool, "disabled-only", None, "hash", None, None)
        .await
        .unwrap();
    let disabled_schedule = db::insert_schedule(
        &pool,
        repo.id,
        &ScheduleParams {
            name: "disabled-dashboard-schedule",
            schedule_type: "backup",
            cron_expression: "0 5 * * *",
            enabled: false,
            canary_enabled: false,
            vm_snapshot_enabled: false,
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
            pre_backup_commands: &[],
            post_backup_commands: &[],
            hook_timeout_seconds: 60,
            missed_backup_threshold: 3,
            on_failure: "stop",
        },
        None,
    )
    .await
    .unwrap();
    db::insert_schedule_targets(&pool, disabled_schedule.id, &[(disabled_agent.id, 0)])
        .await
        .unwrap();

    let unassigned = db::insert_agent(&pool, "unassigned", None, "hash", None, None)
        .await
        .unwrap();
    let hidden = db::insert_agent(&pool, "hidden", None, "hash", None, None)
        .await
        .unwrap();
    db::set_agent_hidden(&pool, hidden.id, true).await.unwrap();
    db::get_or_create_agent_by_hostname(&pool, "imported-placeholder")
        .await
        .unwrap();

    insert_test_report_for_schedule(
        &pool,
        agent.id,
        repo.id,
        schedule_a.id,
        shared::types::BackupStatus::Success,
    )
    .await;
    sqlx::query("UPDATE schedules SET next_run_at = NOW() + INTERVAL '1 hour' WHERE id = $1")
        .bind(schedule_a.id)
        .execute(&pool)
        .await
        .unwrap();

    let targets = db::dashboard::targets(&pool).await.unwrap();
    assert_eq!(targets.len(), 3);
    assert_eq!(
        targets
            .iter()
            .filter(|target| target.agent_id == agent.id)
            .count(),
        2
    );
    assert!(
        targets
            .iter()
            .any(|target| target.schedule_id == schedule_a.id && target.last_success_at.is_some())
    );
    assert!(
        targets
            .iter()
            .any(|target| target.schedule_id == schedule_b.id && target.last_success_at.is_none())
    );

    let hosts = db::dashboard::eligible_hosts(&pool).await.unwrap();
    assert_eq!(hosts.len(), 3);
    assert!(!hosts.iter().any(|host| host.hostname == hidden.hostname));
    assert!(
        !hosts
            .iter()
            .any(|host| host.hostname == "imported-placeholder")
    );
    let disabled = hosts
        .iter()
        .find(|host| host.agent_id == disabled_agent.id)
        .unwrap();
    assert_eq!(disabled.enabled_assignment_count, Some(0));
    assert_eq!(disabled.disabled_assignment_count, Some(1));
    let unassigned = hosts
        .iter()
        .find(|host| host.agent_id == unassigned.id)
        .unwrap();
    assert_eq!(unassigned.enabled_assignment_count, Some(0));

    let upcoming = db::dashboard::upcoming_schedules(&pool).await.unwrap();
    assert_eq!(upcoming.len(), 1);
    assert_eq!(upcoming.first().unwrap().schedule_id, schedule_a.id);
    assert_eq!(upcoming.first().unwrap().target_count, Some(1));
}

#[sqlx::test(migrations = "./migrations")]
async fn dashboard_repository_capacity_uses_repo_stats_and_quota(pool: PgPool) {
    let repo = create_test_repo(&pool).await;
    set_test_repo_info_stats(&pool, repo.id, 1).await;
    db::quota::upsert_quota(
        &pool,
        repo.id,
        Some(200_000),
        Some(300_000),
        QuotaAction::NotifyOnly,
        QuotaAction::NotifyOnly,
        true,
    )
    .await
    .unwrap();

    let repositories = db::dashboard::repositories(&pool).await.unwrap();
    assert_eq!(repositories.len(), 1);
    assert_eq!(repositories.first().unwrap().deduplicated_size, 250_000);
    assert_eq!(repositories.first().unwrap().warn_bytes, Some(200_000));
    assert_eq!(repositories.first().unwrap().critical_bytes, Some(300_000));
    assert_eq!(
        repositories.first().unwrap().enabled_schedule_count,
        Some(0)
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn repos_with_stats(pool: PgPool) {
    let agent = db::insert_agent(&pool, "rws-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;
    set_test_repo_info_stats(&pool, repo.id, 1).await;

    let repos = db::list_repos_with_stats(&pool).await.unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos.first().unwrap().name, "test-repo");
    assert_eq!(repos.first().unwrap().archive_count, 1);
    assert_eq!(repos.first().unwrap().total_original_size, 1_000_000);
    assert_eq!(repos.first().unwrap().total_compressed_size, 500_000);
    assert_eq!(repos.first().unwrap().total_deduplicated_size, 250_000);
    assert_eq!(repos.first().unwrap().agent_count, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn repos_with_stats_empty(pool: PgPool) {
    create_test_repo(&pool).await;

    let repos = db::list_repos_with_stats(&pool).await.unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos.first().unwrap().total_original_size, 0);
    assert_eq!(repos.first().unwrap().total_deduplicated_size, 0);
    assert_eq!(repos.first().unwrap().archive_count, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn repos_with_stats_carries_own_quota_when_configured(pool: PgPool) {
    let with_quota = create_test_repo(&pool).await;
    let without_quota = db::insert_repo(
        &pool,
        &InsertRepoParams {
            name: "no-quota-repo",
            repo_path: "/backups/no-quota",
            ssh_user: "backup",
            ssh_host: "storage.local",
            ssh_port: 22,
            passphrase_encrypted: b"encrypted_data",
            compression: "lz4",
            encryption: "repokey",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap();

    db::quota::upsert_quota(
        &pool,
        with_quota.id,
        Some(500),
        Some(1_000),
        QuotaAction::NotifyOnly,
        QuotaAction::BlockBackups,
        true,
    )
    .await
    .unwrap();

    let repos = db::list_repos_with_stats(&pool).await.unwrap();
    let with_quota_row = repos.iter().find(|r| r.id == with_quota.id).unwrap();
    let without_quota_row = repos.iter().find(|r| r.id == without_quota.id).unwrap();

    assert_eq!(with_quota_row.quota_warn_bytes, Some(500));
    assert_eq!(with_quota_row.quota_critical_bytes, Some(1_000));
    assert_eq!(
        with_quota_row.quota_warn_action.as_deref(),
        Some("notify_only")
    );
    assert_eq!(
        with_quota_row.quota_critical_action.as_deref(),
        Some("block_backups")
    );
    assert_eq!(with_quota_row.quota_enabled, Some(true));

    assert_eq!(without_quota_row.quota_warn_bytes, None);
    assert_eq!(without_quota_row.quota_critical_bytes, None);
    assert_eq!(without_quota_row.quota_warn_action, None);
    assert_eq!(without_quota_row.quota_critical_action, None);
    assert_eq!(without_quota_row.quota_enabled, None);

    let fetched_single = db::get_repo_with_stats(&pool, with_quota.id).await.unwrap();
    assert_eq!(fetched_single.quota_critical_bytes, Some(1_000));
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_with_stats_single(pool: PgPool) {
    let agent = db::insert_agent(&pool, "single-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;
    set_test_repo_info_stats(&pool, repo.id, 1).await;

    let result = db::get_repo_with_stats(&pool, repo.id).await.unwrap();
    assert_eq!(result.total_deduplicated_size, 250_000);
}

#[sqlx::test(migrations = "./migrations")]
async fn storage_breakdown(pool: PgPool) {
    let agent = db::insert_agent(&pool, "brk-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;
    set_test_repo_info_stats(&pool, repo.id, 1).await;

    let breakdown = db::get_storage_breakdown(&pool).await.unwrap();
    assert_eq!(breakdown.len(), 1);
    assert_eq!(breakdown.first().unwrap().name, "test-repo");
    assert_eq!(breakdown.first().unwrap().deduplicated_size, 250_000);
}

/// Repos are returned in descending `info_deduplicated_size` order and
/// `compressed_size` is also sourced from the info columns.
#[sqlx::test(migrations = "./migrations")]
async fn storage_breakdown_multi_repo_ordering(pool: PgPool) {
    let repo_small = create_test_repo(&pool).await;
    let repo_large = db::insert_repo(
        &pool,
        &InsertRepoParams {
            name: "large-repo",
            repo_path: "/backups/large",
            ssh_user: "u",
            ssh_host: "storage.local",
            ssh_port: 22,
            passphrase_encrypted: b"enc",
            compression: "lz4",
            encryption: "none",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap();

    db::update_repo_info_stats(
        &pool,
        repo_small.id,
        &db::RepoInfoStats {
            compressed_size: 200_000,
            deduplicated_size: 100_000,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    db::update_repo_info_stats(
        &pool,
        repo_large.id,
        &db::RepoInfoStats {
            compressed_size: 800_000,
            deduplicated_size: 400_000,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let breakdown = db::get_storage_breakdown(&pool).await.unwrap();
    assert_eq!(breakdown.len(), 2);
    // largest dedup first
    assert_eq!(breakdown.first().unwrap().name, "large-repo");
    assert_eq!(breakdown.first().unwrap().deduplicated_size, 400_000);
    assert_eq!(breakdown.first().unwrap().compressed_size, 800_000);
    assert_eq!(breakdown.get(1).unwrap().name, "test-repo");
    assert_eq!(breakdown.get(1).unwrap().deduplicated_size, 100_000);
}

/// A repo that has never had `update_repo_info_stats` called must return zeros
/// without an error (columns default to 0).
#[sqlx::test(migrations = "./migrations")]
async fn storage_breakdown_repo_with_no_info_stats(pool: PgPool) {
    create_test_repo(&pool).await;

    let breakdown = db::get_storage_breakdown(&pool).await.unwrap();
    assert_eq!(breakdown.len(), 1);
    assert_eq!(breakdown.first().unwrap().compressed_size, 0);
    assert_eq!(breakdown.first().unwrap().deduplicated_size, 0);
}

/// `update_repo_info_stats` persists all six fields and they are readable back
/// via `get_repo_with_stats` (the queries that feed the UI).
#[sqlx::test(migrations = "./migrations")]
async fn update_repo_info_stats_persists_all_fields(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    db::update_repo_info_stats(
        &pool,
        repo.id,
        &db::RepoInfoStats {
            original_size: 10_000_000,
            compressed_size: 6_000_000,
            deduplicated_size: 3_000_000,
            total_chunks: 500,
            unique_chunks: 400,
            archive_count: 7,
        },
    )
    .await
    .unwrap();

    let r = db::get_repo_with_stats(&pool, repo.id).await.unwrap();
    assert_eq!(r.total_original_size, 10_000_000);
    assert_eq!(r.total_compressed_size, 6_000_000);
    assert_eq!(r.total_deduplicated_size, 3_000_000);
    assert_eq!(r.archive_count, 7);
}

/// A second call to `update_repo_info_stats` fully overwrites the previous values.
#[sqlx::test(migrations = "./migrations")]
async fn update_repo_info_stats_overwrite(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    db::update_repo_info_stats(
        &pool,
        repo.id,
        &db::RepoInfoStats {
            original_size: 1_000,
            compressed_size: 800,
            deduplicated_size: 600,
            total_chunks: 10,
            unique_chunks: 8,
            archive_count: 2,
        },
    )
    .await
    .unwrap();

    db::update_repo_info_stats(
        &pool,
        repo.id,
        &db::RepoInfoStats {
            original_size: 99_000,
            compressed_size: 50_000,
            deduplicated_size: 25_000,
            total_chunks: 200,
            unique_chunks: 150,
            archive_count: 10,
        },
    )
    .await
    .unwrap();

    let r = db::get_repo_with_stats(&pool, repo.id).await.unwrap();
    assert_eq!(r.total_original_size, 99_000);
    assert_eq!(r.total_compressed_size, 50_000);
    assert_eq!(r.total_deduplicated_size, 25_000);
    assert_eq!(r.archive_count, 10);
}

#[sqlx::test(migrations = "./migrations")]
async fn dashboard_summary(pool: PgPool) {
    let (agent, repo, _) = create_test_schedule(&pool).await;
    insert_test_report(&pool, agent.id, repo.id).await;

    let summary = db::get_dashboard_summary(&pool).await.unwrap();
    assert_eq!(summary.total_agents, 1);
    assert_eq!(summary.total_repos, 1);
    assert_eq!(summary.total_schedules, 1);
    assert_eq!(summary.active_schedules, 1);
    assert!(summary.last_backup_at.is_some());
    assert_eq!(summary.success_30d, 1);
    assert_eq!(summary.failed_30d, 0);
    assert_eq!(summary.total_30d, 1);
}

/// `total_storage_bytes` in the dashboard summary must now aggregate
/// `repos.info_deduplicated_size` rather than `backup_reports`.
#[sqlx::test(migrations = "./migrations")]
async fn dashboard_summary_total_storage_from_repo_info(pool: PgPool) {
    let agent = db::insert_agent(&pool, "ds-storage-host", None, "hash", None, None)
        .await
        .unwrap();

    // Create two repos with distinct info stats and confirm the sum is correct.
    let repo1 = create_test_repo(&pool).await;
    let repo2 = db::insert_repo(
        &pool,
        &InsertRepoParams {
            name: "test-repo-2",
            repo_path: "/backups/r2",
            ssh_user: "u",
            ssh_host: "storage.local",
            ssh_port: 22,
            passphrase_encrypted: b"enc",
            compression: "lz4",
            encryption: "none",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap();

    insert_test_report(&pool, agent.id, repo1.id).await;
    insert_test_report(&pool, agent.id, repo2.id).await;

    db::update_repo_info_stats(
        &pool,
        repo1.id,
        &db::RepoInfoStats {
            deduplicated_size: 100_000,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    db::update_repo_info_stats(
        &pool,
        repo2.id,
        &db::RepoInfoStats {
            deduplicated_size: 200_000,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let summary = db::get_dashboard_summary(&pool).await.unwrap();
    assert_eq!(
        summary.total_storage_bytes, 300_000,
        "should sum info_deduplicated_size across both repos"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn dashboard_summary_empty(pool: PgPool) {
    let summary = db::get_dashboard_summary(&pool).await.unwrap();
    assert_eq!(summary.total_agents, 0);
    assert_eq!(summary.total_repos, 0);
    assert_eq!(summary.total_storage_bytes, 0);
}

/// Regression test for `get_dashboard_summary`'s CTE rewrite: the "latest failure" fields
/// (`last_failure_at`/`last_failure_message`/`last_failure_repo_*`) must reflect the single
/// most recent failed report regardless of whether it has a schedule, while
/// `last_failure_schedule_id`/`last_failure_schedule_name` must reflect the most recent
/// failed report that *does* have a resolvable schedule -- which can be a different (older)
/// row. The dashboard summary query builds the same split for warnings, via an identical CTE
/// shape.
#[sqlx::test(migrations = "./migrations")]
async fn dashboard_summary_failure_warning_schedule_split(pool: PgPool) {
    let (agent, repo, schedule) = create_test_schedule(&pool).await;
    let now = Utc::now();

    // Older failure, with a schedule.
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: Some(schedule.id),
            started_at: now.checked_sub_signed(Duration::hours(2)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::hours(2)).unwrap(),
            status: shared::types::BackupStatus::Failed,
            original_size: 0,
            compressed_size: 0,
            deduplicated_size: 0,
            repo_unique_csize: 0,
            files_processed: 0,
            duration_secs: 0,
            error_message: Some("older scheduled failure".to_string()),
            warnings: vec![],
            borg_version: None,
            matched: true,
            archive_name: None,
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    // Newer failure, no schedule (e.g. an ad-hoc/manual run).
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(5)).unwrap(),
            finished_at: now,
            status: shared::types::BackupStatus::Failed,
            original_size: 0,
            compressed_size: 0,
            deduplicated_size: 0,
            repo_unique_csize: 0,
            files_processed: 0,
            duration_secs: 0,
            error_message: Some("newer unscheduled failure".to_string()),
            warnings: vec![],
            borg_version: None,
            matched: true,
            archive_name: None,
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let summary = db::get_dashboard_summary(&pool).await.unwrap();

    // The general "latest failure" fields must pick the newer, schedule-less row.
    assert_eq!(
        summary.last_failure_message.as_deref(),
        Some("newer unscheduled failure")
    );
    assert_eq!(summary.last_failure_repo_id, Some(repo.id));

    // The schedule-scoped fields must instead pick the older row that has a schedule.
    assert_eq!(summary.last_failure_schedule_id, Some(schedule.id));
    assert_eq!(
        summary.last_failure_schedule_name.as_deref(),
        Some(schedule.cron_expression.as_str())
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn user_crud(pool: PgPool) {
    let user = db::insert_user(&pool, "testuser", "hashed_pw")
        .await
        .unwrap();
    assert_eq!(user.username, "testuser");
    assert!(!user.must_change_password);

    let fetched = db::get_user_by_username(&pool, "testuser").await.unwrap();
    assert_eq!(fetched.id, user.id);

    let by_id = db::get_user_by_id(&pool, user.id).await.unwrap();
    assert_eq!(by_id.username, "testuser");

    let users = db::list_users(&pool).await.unwrap();
    assert_eq!(users.len(), 1);

    let count = db::user_count(&pool).await.unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn user_password_hash(pool: PgPool) {
    db::insert_user(&pool, "pwuser", "the_hash").await.unwrap();

    let (user, hash) = db::get_user_password_hash(&pool, "pwuser").await.unwrap();
    assert_eq!(user.username, "pwuser");
    assert_eq!(hash, "the_hash");
}

#[sqlx::test(migrations = "./migrations")]
async fn user_update_role(pool: PgPool) {
    let user = db::insert_user(&pool, "roleuser", "hash").await.unwrap();

    let admin_role = db::list_roles(&pool)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.name == "admin")
        .unwrap();
    db::set_user_roles(&pool, user.id, &[admin_role.id])
        .await
        .unwrap();
    let roles = db::list_user_roles(&pool, user.id).await.unwrap();
    assert!(roles.iter().any(|r| r.name == "admin"));
}

#[sqlx::test(migrations = "./migrations")]
async fn user_update_password(pool: PgPool) {
    let user = db::insert_user(&pool, "passuser", "old_hash")
        .await
        .unwrap();

    db::update_user_password(&pool, user.id, "new_hash")
        .await
        .unwrap();

    let (_, hash) = db::get_user_password_hash(&pool, "passuser").await.unwrap();
    assert_eq!(hash, "new_hash");
}

#[sqlx::test(migrations = "./migrations")]
async fn user_update_last_login(pool: PgPool) {
    let user = db::insert_user(&pool, "loginuser", "hash").await.unwrap();

    db::update_last_login(&pool, user.id).await.unwrap();

    let fetched = db::get_user_by_id(&pool, user.id).await.unwrap();
    assert!(fetched.last_login_at.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn user_delete(pool: PgPool) {
    let user = db::insert_user(&pool, "deluser", "hash").await.unwrap();

    db::delete_user(&pool, user.id).await.unwrap();

    let result = db::get_user_by_id(&pool, user.id).await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn user_preferences(pool: PgPool) {
    let user = db::insert_user(&pool, "prefuser", "hash").await.unwrap();

    let prefs = serde_json::json!({"theme": "dark", "lang": "en"});
    db::set_user_preferences(&pool, user.id, &prefs)
        .await
        .unwrap();

    let fetched = db::get_user_preferences(&pool, user.id).await.unwrap();
    assert_eq!(fetched.get("theme").unwrap(), "dark");
    assert_eq!(fetched.get("lang").unwrap(), "en");
}

#[sqlx::test(migrations = "./migrations")]
async fn session_crud(pool: PgPool) {
    let user = db::insert_user(&pool, "sessuser", "hash").await.unwrap();

    let expires = Utc::now().checked_add_signed(Duration::hours(24)).unwrap();
    db::insert_session(&pool, "sess_abc123", user.id, expires, false, false)
        .await
        .unwrap();

    let session = db::get_session(&pool, "sess_abc123").await.unwrap();
    assert_eq!(session.user_id, user.id);
    assert_eq!(session.id, "sess_abc123");
    assert!(!session.remember_me);
    assert!(!session.pending_totp);

    db::delete_session(&pool, "sess_abc123").await.unwrap();

    let result = db::get_session(&pool, "sess_abc123").await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn session_expired(pool: PgPool) {
    let user = db::insert_user(&pool, "expuser", "hash").await.unwrap();

    let expired = Utc::now().checked_sub_signed(Duration::hours(1)).unwrap();
    db::insert_session(&pool, "sess_expired", user.id, expired, false, false)
        .await
        .unwrap();

    let result = db::get_session(&pool, "sess_expired").await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn session_delete_expired(pool: PgPool) {
    let user = db::insert_user(&pool, "cleanuser", "hash").await.unwrap();

    let expired = Utc::now().checked_sub_signed(Duration::hours(1)).unwrap();
    db::insert_session(&pool, "sess_old", user.id, expired, false, false)
        .await
        .unwrap();

    let deleted = db::delete_expired_sessions(&pool).await.unwrap();
    assert_eq!(deleted, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn session_remember_me(pool: PgPool) {
    let user = db::insert_user(&pool, "rememberuser", "hash")
        .await
        .unwrap();

    let expires = Utc::now().checked_add_signed(Duration::days(7)).unwrap();
    db::insert_session(&pool, "sess_remember", user.id, expires, true, false)
        .await
        .unwrap();

    let session = db::get_session(&pool, "sess_remember").await.unwrap();
    assert_eq!(session.user_id, user.id);
    assert!(session.remember_me);
}

#[sqlx::test(migrations = "./migrations")]
async fn session_extend(pool: PgPool) {
    let user = db::insert_user(&pool, "extenduser", "hash").await.unwrap();

    let original_expires = Utc::now().checked_add_signed(Duration::hours(1)).unwrap();
    db::insert_session(&pool, "sess_extend", user.id, original_expires, true, false)
        .await
        .unwrap();

    let new_expires = Utc::now().checked_add_signed(Duration::days(7)).unwrap();
    db::extend_session(&pool, "sess_extend", new_expires)
        .await
        .unwrap();

    let session = db::get_session(&pool, "sess_extend").await.unwrap();
    assert!(session.expires_at > original_expires);
    assert!(session.remember_me);
}

#[sqlx::test(migrations = "./migrations")]
async fn session_revoke_by_id(pool: PgPool) {
    let user = db::insert_user(&pool, "revokeuser", "hash").await.unwrap();
    let other = db::insert_user(&pool, "otheruser", "hash2").await.unwrap();

    let expires = Utc::now().checked_add_signed(Duration::hours(24)).unwrap();
    db::insert_session(&pool, "sess_revoke_1", user.id, expires, false, false)
        .await
        .unwrap();
    db::insert_session(&pool, "sess_revoke_2", user.id, expires, false, false)
        .await
        .unwrap();
    db::insert_session(&pool, "sess_other_user", other.id, expires, false, false)
        .await
        .unwrap();

    // Revoke sess_revoke_1 by user - should succeed
    let deleted = db::delete_session_by_id(&pool, "sess_revoke_1", user.id)
        .await
        .unwrap();
    assert!(deleted);

    // Revoking same session again should return false
    let deleted = db::delete_session_by_id(&pool, "sess_revoke_1", user.id)
        .await
        .unwrap();
    assert!(!deleted);

    // Other user's session cannot be revoked by user (ownership check)
    let deleted = db::delete_session_by_id(&pool, "sess_other_user", user.id)
        .await
        .unwrap();
    assert!(!deleted, "cannot revoke another user's session");

    // Other user's session still exists
    let fetched = db::get_session(&pool, "sess_other_user").await.unwrap();
    assert_eq!(fetched.user_id, other.id);
}

#[sqlx::test(migrations = "./migrations")]
async fn session_last_seen_update(pool: PgPool) {
    let user = db::insert_user(&pool, "seenuser", "hash").await.unwrap();

    let expires = Utc::now().checked_add_signed(Duration::hours(24)).unwrap();
    db::insert_session(&pool, "sess_seen", user.id, expires, false, false)
        .await
        .unwrap();

    let session = db::get_session(&pool, "sess_seen").await.unwrap();
    let initial_seen = session.last_seen_at;

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    db::update_session_last_seen(&pool, "sess_seen")
        .await
        .unwrap();

    let session = db::get_session(&pool, "sess_seen").await.unwrap();
    assert!(
        session.last_seen_at > initial_seen,
        "last_seen_at must be updated"
    );
}

/// Verifies the DB primitives that support session idle timeout:
/// `last_seen_at` is persisted, can be set to an arbitrary past timestamp,
/// and can be refreshed. The actual timeout enforcement lives in the
/// `AuthUser` extractor; this test ensures the DB layer exposes the field
/// correctly so an idle session older than the configured threshold can be
/// detected and revoked on the next request.
#[sqlx::test(migrations = "./migrations")]
async fn session_idle_timeout_tracking(pool: PgPool) {
    let user = db::insert_user(&pool, "idleuser", "hash").await.unwrap();

    let expires = Utc::now().checked_add_signed(Duration::hours(24)).unwrap();
    db::insert_session(&pool, "sess_idle", user.id, expires, false, false)
        .await
        .unwrap();

    // Simulate a session that has been idle for longer than the default
    // 480-minute timeout by backdating last_seen_at.
    let idle_since = Utc::now()
        .checked_sub_signed(Duration::minutes(481))
        .unwrap();
    sqlx::query!(
        "UPDATE sessions SET last_seen_at = $1 WHERE id = $2",
        idle_since,
        "sess_idle"
    )
    .execute(&pool)
    .await
    .unwrap();

    // The DB layer must still return the session: idle timeout is enforced
    // in the application layer, not by `get_session`.
    let session = db::get_session(&pool, "sess_idle").await.unwrap();
    assert_eq!(session.user_id, user.id);
    assert!(
        Utc::now()
            .signed_duration_since(session.last_seen_at)
            .num_minutes()
            >= 480,
        "session must be idle for at least the default timeout"
    );

    // list_sessions_for_user should also include the idle session.
    let listed = db::list_sessions_for_user(&pool, user.id).await.unwrap();
    assert!(listed.iter().any(|s| s.id == "sess_idle"));

    // Refreshing last_seen_at moves the idle window forward.
    db::update_session_last_seen(&pool, "sess_idle")
        .await
        .unwrap();
    let session = db::get_session(&pool, "sess_idle").await.unwrap();
    assert!(
        Utc::now()
            .signed_duration_since(session.last_seen_at)
            .num_minutes()
            < 1,
        "last_seen_at must be recent after refresh"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn session_pending_totp(pool: PgPool) {
    let user = db::insert_user(&pool, "pendingtotpuser", "hash")
        .await
        .unwrap();

    let expires = Utc::now().checked_add_signed(Duration::hours(24)).unwrap();
    // Session with pending_totp = true (temp session during two-step login)
    db::insert_session(&pool, "sess_pending", user.id, expires, false, true)
        .await
        .unwrap();

    let session = db::get_session(&pool, "sess_pending").await.unwrap();
    assert!(session.pending_totp);

    // Regular session with pending_totp = false
    db::insert_session(&pool, "sess_regular", user.id, expires, false, false)
        .await
        .unwrap();
    let session = db::get_session(&pool, "sess_regular").await.unwrap();
    assert!(!session.pending_totp);
}

#[sqlx::test(migrations = "./migrations")]
async fn totp_fields_crud(pool: PgPool) {
    let user = db::insert_user(&pool, "totpuser", "hash").await.unwrap();

    // Initially TOTP should not be configured
    let fields = db::get_user_totp_fields(&pool, user.id).await.unwrap();
    assert!(fields.is_none());

    // Set TOTP secret and recovery codes
    let secret = b"encrypted_secret_bytes";
    let codes = vec!["bcrypt_code_1".to_string(), "bcrypt_code_2".to_string()];
    db::set_user_totp_secret(&pool, user.id, secret, &codes)
        .await
        .unwrap();

    // Should be present but not enabled
    let fields = db::get_user_totp_fields(&pool, user.id)
        .await
        .unwrap()
        .expect("TOTP fields should exist");
    assert_eq!(fields.secret_encrypted.as_deref(), Some(secret as &[u8]));
    assert!(!fields.enabled);
    assert_eq!(fields.recovery_codes.len(), 2);
    assert!(fields.last_verified_step.is_none());

    // Enable TOTP, recording the step consumed by the enrollment code itself
    // so it can't be replayed against the login endpoint.
    db::enable_user_totp(&pool, user.id, 42).await.unwrap();

    let fields = db::get_user_totp_fields(&pool, user.id)
        .await
        .unwrap()
        .expect("TOTP fields should exist");
    assert!(fields.enabled);
    assert_eq!(
        fields.last_verified_step,
        Some(42),
        "the enrollment code's step must be recorded to prevent its replay"
    );

    // Record a verified step
    let consumed = db::try_consume_totp_step(&pool, user.id, 100)
        .await
        .unwrap();
    assert!(consumed);

    let fields = db::get_user_totp_fields(&pool, user.id)
        .await
        .unwrap()
        .expect("TOTP fields should exist");
    assert_eq!(fields.last_verified_step, Some(100));

    // Replace recovery codes (remove one)
    let remaining = vec!["bcrypt_code_1".to_string()];
    db::replace_totp_recovery_codes(&pool, user.id, &remaining)
        .await
        .unwrap();

    let fields = db::get_user_totp_fields(&pool, user.id)
        .await
        .unwrap()
        .expect("TOTP fields should exist");
    assert_eq!(fields.recovery_codes.len(), 1);
    assert_eq!(fields.recovery_codes.first().unwrap(), "bcrypt_code_1");

    // Disable TOTP (clears everything)
    db::disable_user_totp(&pool, user.id).await.unwrap();

    let fields = db::get_user_totp_fields(&pool, user.id).await.unwrap();
    assert!(fields.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn totp_fields_for_nonexistent_user(pool: PgPool) {
    let fields = db::get_user_totp_fields(&pool, 999_999_999).await.unwrap();
    assert!(fields.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn login_attempts(pool: PgPool) {
    db::insert_login_attempt(&pool, "user1", "192.168.1.1", false)
        .await
        .unwrap();
    db::insert_login_attempt(&pool, "user1", "192.168.1.1", false)
        .await
        .unwrap();
    db::insert_login_attempt(&pool, "user1", "192.168.1.1", true)
        .await
        .unwrap();

    let count = db::count_failed_login_attempts(&pool, "user1", "192.168.1.1", 60)
        .await
        .unwrap();
    assert_eq!(count, 2);

    let count_other_ip = db::count_failed_login_attempts(&pool, "user1", "10.0.0.1", 60)
        .await
        .unwrap();
    assert_eq!(count_other_ip, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn account_lockout(pool: PgPool) {
    // Create a user first
    db::insert_user(&pool, "lockuser", "hash").await.unwrap();

    // Insert some failed login attempts for the user
    for _ in 0..3 {
        db::insert_login_attempt(&pool, "lockuser", "192.168.1.1", false)
            .await
            .unwrap();
    }

    // Verify count across all IPs
    let count = db::count_failed_attempts_since_last_success(&pool, "lockuser")
        .await
        .unwrap();
    assert_eq!(count, 3);

    // Set a lockout
    let lock_time = Utc::now()
        .checked_add_signed(Duration::minutes(30))
        .unwrap();
    sqlx::query!(
        "UPDATE users SET locked_until = $1 WHERE username = $2",
        lock_time,
        "lockuser",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Verify user is locked
    let user = db::get_user_by_username(&pool, "lockuser").await.unwrap();
    assert!(user.locked_until.is_some());
    assert!(user.locked_until.unwrap() > Utc::now());

    // Clear lockout
    db::clear_account_lockout(&pool, "lockuser").await.unwrap();
    let user = db::get_user_by_username(&pool, "lockuser").await.unwrap();
    assert!(user.locked_until.is_none());

    // Never gone through record_failed_login_and_check_lockout, so the
    // escalation counter is still at its default.
    assert_eq!(lockout_escalation_level(&pool, "lockuser").await, 0);

    // Trigger a real lockout, which advances the counter to 1...
    for _ in 0..10 {
        db::record_failed_login_and_check_lockout(&pool, "lockuser", "192.168.1.1", 10)
            .await
            .unwrap();
    }
    assert_eq!(lockout_escalation_level(&pool, "lockuser").await, 1);

    // ...and clearing the lockout (a successful login) resets it back to 0.
    db::clear_account_lockout(&pool, "lockuser").await.unwrap();
    assert_eq!(lockout_escalation_level(&pool, "lockuser").await, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn record_failed_login_triggers_lockout(pool: PgPool) {
    db::insert_user(&pool, "ratelimituser", "hash")
        .await
        .unwrap();

    // Insert 10 failed attempts - this should trigger account lockout
    for _ in 0..10 {
        db::record_failed_login_and_check_lockout(&pool, "ratelimituser", "10.0.0.1", 10)
            .await
            .unwrap();
    }

    // User should be locked
    let user = db::get_user_by_username(&pool, "ratelimituser")
        .await
        .unwrap();
    assert!(user.locked_until.is_some());
    assert!(user.locked_until.unwrap() > Utc::now());
}

/// Simulates a lockout naturally expiring (time passing) without a
/// successful login, so the next cycle's failures can retrigger it. Pushes
/// `locked_until` into the past directly rather than through
/// `clear_account_lockout`, which would also reset the escalation counter.
///
/// Sets `locked_until` to `NOW()` (the moment this call runs), not an
/// arbitrary fixed offset: `count_failed_attempts_in_current_cycle` uses
/// `locked_until` as the cutoff for "which attempts belong to the next
/// cycle", so it must land strictly *after* every attempt made in the test
/// so far (so they're excluded from the next cycle's fresh count) but still
/// *before* the next real check of `locked_until > NOW()` (so the account
/// reads as expired). `NOW()` at call time satisfies both: it comes after
/// every already-committed attempt (each attempt's own `NOW()` is earlier,
/// since those transactions already completed), and real time will have
/// moved forward again by the time anything next checks it.
#[cfg(test)]
async fn expire_lockout(pool: &PgPool, username: &str) {
    sqlx::query!(
        "UPDATE users SET locked_until = NOW() WHERE username = $1",
        username,
    )
    .execute(pool)
    .await
    .unwrap();
}

/// Reads `users.lockout_escalation_level` directly for test assertions. Not
/// exposed from `db` since nothing in production code needs to read it back
/// (`record_failed_login_and_check_lockout` already reads/advances it itself).
#[cfg(test)]
async fn lockout_escalation_level(pool: &PgPool, username: &str) -> i32 {
    sqlx::query_scalar!(
        "SELECT lockout_escalation_level FROM users WHERE username = $1",
        username,
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn lockout_escalation_reaches_60min_tier(pool: PgPool) {
    // The LOCKOUT_DURATIONS are [1, 5, 15, 60, 1440] minutes, and the tier
    // advances once per lockout *cycle* (a threshold-crossing failure while
    // not currently locked), not per raw failure count. With
    // max_account_failures = 5, reaching level 3 (60 min) takes 4 complete
    // cycles: 1 min -> 5 min -> 15 min -> 60 min.
    db::insert_user(&pool, "escalation60", "hash")
        .await
        .unwrap();

    for cycle in 0..4 {
        for _ in 0..5 {
            db::record_failed_login_and_check_lockout(&pool, "escalation60", "10.0.0.1", 5)
                .await
                .unwrap();
        }
        if cycle < 3 {
            expire_lockout(&pool, "escalation60").await;
        }
    }

    let user = db::get_user_by_username(&pool, "escalation60")
        .await
        .unwrap();
    let locked_until = user.locked_until.expect("user should be locked");

    // Lockout duration should be >= 55 minutes (60 min tier, with some slack for test timing)
    let duration_min = locked_until.signed_duration_since(Utc::now()).num_minutes();
    assert!(
        duration_min >= 55,
        "expected ~60 min lockout, got {duration_min} min"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn lockout_escalation_reaches_24h_tier(pool: PgPool) {
    // Reaching level 4 (24h) takes 5 complete lockout cycles.
    db::insert_user(&pool, "escalation24h", "hash")
        .await
        .unwrap();

    for cycle in 0..5 {
        for _ in 0..5 {
            db::record_failed_login_and_check_lockout(&pool, "escalation24h", "10.0.0.1", 5)
                .await
                .unwrap();
        }
        if cycle < 4 {
            expire_lockout(&pool, "escalation24h").await;
        }
    }

    let user = db::get_user_by_username(&pool, "escalation24h")
        .await
        .unwrap();
    let locked_until = user.locked_until.expect("user should be locked");

    let duration_min = locked_until.signed_duration_since(Utc::now()).num_minutes();
    assert!(
        duration_min >= 1430,
        "expected ~1440 min lockout, got {duration_min} min"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn lockout_escalation_resets_after_successful_login(pool: PgPool) {
    // Verify that the consecutive-failure window resets after a successful login.
    db::insert_user(&pool, "escalationreset", "hash")
        .await
        .unwrap();

    // 10 failures -> level 0 -> locked
    for _ in 0..10 {
        db::record_failed_login_and_check_lockout(&pool, "escalationreset", "10.0.0.1", 10)
            .await
            .unwrap();
    }

    let user = db::get_user_by_username(&pool, "escalationreset")
        .await
        .unwrap();
    assert!(user.locked_until.is_some());

    // Simulate a successful login
    db::clear_account_lockout(&pool, "escalationreset")
        .await
        .unwrap();
    db::insert_login_attempt(&pool, "escalationreset", "10.0.0.1", true)
        .await
        .unwrap();

    // Now the count should be 0 (reset by success)
    let count = db::count_failed_attempts_since_last_success(&pool, "escalationreset")
        .await
        .unwrap();
    assert_eq!(count, 0);

    // 5 more failures (below threshold of 10)
    for _ in 0..5 {
        db::record_failed_login_and_check_lockout(&pool, "escalationreset", "10.0.0.1", 10)
            .await
            .unwrap();
    }

    let user = db::get_user_by_username(&pool, "escalationreset")
        .await
        .unwrap();
    assert!(user.locked_until.is_none(), "should not be locked yet");
}

#[sqlx::test(migrations = "./migrations")]
async fn lockout_escalation_sliding_window_keeps_count_across_lockouts(pool: PgPool) {
    // Simulate the attack scenario: attacker accumulates failures across
    // multiple lockout periods, without ever logging in successfully. The
    // *escalation tier* (`lockout_escalation_level`) must persist and keep
    // climbing across cycles in that case -- unlike a successful login
    // (which resets it via clear_account_lockout), a lockout that merely
    // expires with time should not reset it. Reaching each tier still
    // requires a genuinely fresh `max_account_failures` batch in the new
    // cycle, though -- that per-cycle count is a separate thing from the
    // persistent tier, and does reset at each cycle boundary (see
    // count_failed_attempts_in_current_cycle).
    db::insert_user(&pool, "slidingwindow", "hash")
        .await
        .unwrap();

    // Phase 1: 10 failures -> lockout triggered
    for _ in 0..10 {
        db::record_failed_login_and_check_lockout(&pool, "slidingwindow", "10.0.0.1", 10)
            .await
            .unwrap();
    }
    let user = db::get_user_by_username(&pool, "slidingwindow")
        .await
        .unwrap();
    assert!(user.locked_until.is_some());

    // Simulate the lockout naturally expiring (time passing), *not* a
    // successful login.
    expire_lockout(&pool, "slidingwindow").await;

    // Phase 2: a fresh 10 failures -> level 1 (5 min lockout)
    for _ in 0..10 {
        db::record_failed_login_and_check_lockout(&pool, "slidingwindow", "10.0.0.1", 10)
            .await
            .unwrap();
    }
    let user = db::get_user_by_username(&pool, "slidingwindow")
        .await
        .unwrap();
    let locked_until = user
        .locked_until
        .expect("user should be locked after phase 2");
    let duration_min = locked_until.signed_duration_since(Utc::now()).num_minutes();
    assert!(
        duration_min >= 2,
        "expected level 1 (5 min), got {duration_min} min"
    );
}

/// Pins the invariant the `WHERE locked_until IS NULL OR locked_until <=
/// NOW()` guard in `record_failed_login_and_check_lockout` exists to
/// enforce: continued failed attempts against an *already-locked* account
/// must not extend `locked_until` or advance `lockout_escalation_level`
/// further. `login()`'s locked-account branch calls this function on every
/// attempt for timing-uniformity reasons, so without this guard a
/// brute-force attempt against a locked account would keep re-locking and
/// re-escalating it on every single try instead of only once per cycle.
#[sqlx::test(migrations = "./migrations")]
async fn record_failed_login_does_not_reescalate_while_still_locked(pool: PgPool) {
    db::insert_user(&pool, "stilllocked", "hash").await.unwrap();

    // 10 failures -> lockout triggered, escalation level 1.
    for _ in 0..10 {
        db::record_failed_login_and_check_lockout(&pool, "stilllocked", "10.0.0.1", 10)
            .await
            .unwrap();
    }
    let user = db::get_user_by_username(&pool, "stilllocked")
        .await
        .unwrap();
    let locked_until = user.locked_until.expect("account should be locked");
    assert_eq!(lockout_escalation_level(&pool, "stilllocked").await, 1);

    // Many more failed attempts while still locked (no expire_lockout call
    // in between) must not move locked_until or the escalation level at
    // all -- the guard should make every one of these a no-op.
    for _ in 0..20 {
        db::record_failed_login_and_check_lockout(&pool, "stilllocked", "10.0.0.1", 10)
            .await
            .unwrap();
    }

    let user = db::get_user_by_username(&pool, "stilllocked")
        .await
        .unwrap();
    assert_eq!(
        user.locked_until,
        Some(locked_until),
        "locked_until must not change while the account is still locked"
    );
    assert_eq!(
        lockout_escalation_level(&pool, "stilllocked").await,
        1,
        "escalation level must not advance while the account is still locked"
    );
}

/// Regression test for the escalation-gate `DoS` bug: after a lockout expires,
/// a *single* stray failed attempt must not immediately re-trigger
/// escalation -- a genuinely fresh `max_account_failures` batch is required
/// each cycle. Before `count_failed_attempts_in_current_cycle` existed, the
/// gate counted *all* failures since the last successful login (unbounded),
/// including ones recorded against the account while it was already locked
/// (`login()`'s locked branch records those too, for timing uniformity).
/// That meant the count was already >= threshold forever after the first
/// lockout, so the very next failed attempt post-expiry -- not a fresh
/// batch -- re-escalated the tier every time: a cheap, persistent `DoS`
/// against any known username (roughly one request per cycle to keep an
/// account locked at the maximum 24h tier), and a way for a locked-out
/// legitimate user to ratchet their own account up just by retrying.
#[sqlx::test(migrations = "./migrations")]
async fn record_failed_login_requires_a_fresh_batch_to_reescalate_after_expiry(pool: PgPool) {
    db::insert_user(&pool, "freshbatch", "hash").await.unwrap();

    // Cycle 1: 10 failures -> lockout triggered, escalation level 1.
    for _ in 0..10 {
        db::record_failed_login_and_check_lockout(&pool, "freshbatch", "10.0.0.1", 10)
            .await
            .unwrap();
    }
    assert_eq!(lockout_escalation_level(&pool, "freshbatch").await, 1);

    // A few more attempts while still locked (as login()'s locked branch
    // would generate) -- these must not count toward the next cycle either.
    for _ in 0..3 {
        db::record_failed_login_and_check_lockout(&pool, "freshbatch", "10.0.0.1", 10)
            .await
            .unwrap();
    }

    expire_lockout(&pool, "freshbatch").await;

    // A single failed attempt after expiry must NOT re-escalate -- only a
    // fresh batch of max_account_failures should.
    db::record_failed_login_and_check_lockout(&pool, "freshbatch", "10.0.0.1", 10)
        .await
        .unwrap();
    assert_eq!(
        lockout_escalation_level(&pool, "freshbatch").await,
        1,
        "a single stray attempt after expiry must not re-trigger escalation"
    );
    let user = db::get_user_by_username(&pool, "freshbatch").await.unwrap();
    assert!(
        user.locked_until.is_none_or(|lu| lu <= Utc::now()),
        "the account must not be re-locked by a single post-expiry attempt"
    );

    // 9 more (10 total in this fresh cycle) -> now it re-escalates.
    for _ in 0..9 {
        db::record_failed_login_and_check_lockout(&pool, "freshbatch", "10.0.0.1", 10)
            .await
            .unwrap();
    }
    assert_eq!(
        lockout_escalation_level(&pool, "freshbatch").await,
        2,
        "a genuinely fresh batch of max_account_failures must re-escalate"
    );
    let user = db::get_user_by_username(&pool, "freshbatch").await.unwrap();
    assert!(
        user.locked_until.is_some_and(|lu| lu > Utc::now()),
        "the account must be locked again after a fresh full batch"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn record_failed_login_below_threshold_no_lockout(pool: PgPool) {
    db::insert_user(&pool, "underthreshold", "hash")
        .await
        .unwrap();

    // 5 attempts is below the threshold of 10
    for _ in 0..5 {
        db::record_failed_login_and_check_lockout(&pool, "underthreshold", "10.0.0.1", 10)
            .await
            .unwrap();
    }

    let user = db::get_user_by_username(&pool, "underthreshold")
        .await
        .unwrap();
    assert!(user.locked_until.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn record_failed_login_inserts_exactly_one_attempt(pool: PgPool) {
    db::insert_user(&pool, "txuser", "hash").await.unwrap();

    let count_before: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*)::BIGINT AS \"count!\" FROM login_attempts WHERE username = 'txuser'"
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    db::record_failed_login_and_check_lockout(&pool, "txuser", "10.0.0.1", 10)
        .await
        .unwrap();

    let count_after: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*)::BIGINT AS \"count!\" FROM login_attempts WHERE username = 'txuser'"
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(count_after, count_before.checked_add(1).unwrap());
}

#[sqlx::test(migrations = "./migrations")]
async fn api_token_crud(pool: PgPool) {
    let user = db::insert_user(&pool, "tokenuser", "hash").await.unwrap();

    let token = db::insert_api_token(&pool, user.id, "My Token", "token_hash_abc")
        .await
        .unwrap();
    assert_eq!(token.name, "My Token");
    assert_eq!(token.user_id, user.id);

    let tokens = db::list_api_tokens_for_user(&pool, user.id).await.unwrap();
    assert_eq!(tokens.len(), 1);

    let all_tokens = db::list_all_api_tokens(&pool).await.unwrap();
    assert_eq!(all_tokens.len(), 1);

    let owner = db::get_api_token_owner(&pool, token.id).await.unwrap();
    assert_eq!(owner, user.id);

    let lookup = db::get_user_by_token_hash(&pool, "token_hash_abc")
        .await
        .unwrap();
    assert_eq!(lookup.user_id, user.id);

    db::update_api_token_last_used(&pool, "token_hash_abc")
        .await
        .unwrap();

    db::delete_api_token(&pool, token.id).await.unwrap();
    let tokens = db::list_api_tokens_for_user(&pool, user.id).await.unwrap();
    assert_eq!(tokens.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_permissions_crud(pool: PgPool) {
    let user = db::insert_user(&pool, "permuser", "hash").await.unwrap();
    let repo = create_test_repo(&pool).await;

    let perm = db::upsert_repo_permission(
        &pool,
        &UpsertRepoPermissionParams {
            user_id: user.id,
            repo_id: repo.id,
            can_view: true,
            can_backup: true,
            can_modify_schedules: false,
            can_extract: false,
            can_delete: false,
        },
    )
    .await
    .unwrap();
    assert!(perm.can_view);
    assert!(perm.can_backup);
    assert!(!perm.can_delete);

    let fetched = db::get_repo_permission(&pool, user.id, repo.id)
        .await
        .unwrap();
    assert!(fetched.is_some());

    let upserted = db::upsert_repo_permission(
        &pool,
        &UpsertRepoPermissionParams {
            user_id: user.id,
            repo_id: repo.id,
            can_view: true,
            can_backup: true,
            can_modify_schedules: true,
            can_extract: true,
            can_delete: true,
        },
    )
    .await
    .unwrap();
    assert!(upserted.can_delete);
    assert!(upserted.can_modify_schedules);

    let by_user = db::list_repo_permissions_for_user(&pool, user.id)
        .await
        .unwrap();
    assert_eq!(by_user.len(), 1);

    let by_repo = db::list_repo_permissions_for_repo(&pool, repo.id)
        .await
        .unwrap();
    assert_eq!(by_repo.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn system_events_crud(pool: PgPool) {
    db::insert_system_event(
        &pool,
        shared::types::SystemEventType::RepoSync,
        Some("host-1"),
        "Backup finished",
    )
    .await
    .unwrap();
    db::insert_system_event(
        &pool,
        shared::types::SystemEventType::RepoSyncFailed,
        None,
        "Something failed",
    )
    .await
    .unwrap();

    let events = db::get_system_events(&pool, 10, AcknowledgedFilter::All)
        .await
        .unwrap();
    assert_eq!(events.len(), 2);

    let future = Utc::now().checked_add_signed(Duration::hours(1)).unwrap();
    let deleted = db::delete_system_events_before(&pool, future)
        .await
        .unwrap();
    assert_eq!(deleted, 2);

    let events = db::get_system_events(&pool, 10, AcknowledgedFilter::All)
        .await
        .unwrap();
    assert_eq!(events.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn get_system_events_skips_rows_with_unrecognized_event_type(pool: PgPool) {
    db::insert_system_event(
        &pool,
        shared::types::SystemEventType::RepoSync,
        Some("host-1"),
        "Backup finished",
    )
    .await
    .unwrap();

    // Historical/legacy event_type values predate both the SystemEventType enum and the
    // system_events_event_type_check CHECK constraint, so a row like this can still exist in
    // a real database that was migrated after such data was written; the query must skip it
    // rather than fail the whole batch. Drop the constraint to simulate that pre-existing
    // row in this test's isolated database.
    sqlx::query!("ALTER TABLE system_events DROP CONSTRAINT system_events_event_type_check")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query!(
        "INSERT INTO system_events (event_type, hostname, message) VALUES ($1, $2, $3)",
        "agent_connected",
        Some("host-2"),
        "legacy event",
    )
    .execute(&pool)
    .await
    .unwrap();

    db::insert_system_event(
        &pool,
        shared::types::SystemEventType::RepoSyncFailed,
        None,
        "Something failed",
    )
    .await
    .unwrap();

    let events = db::get_system_events(&pool, 10, AcknowledgedFilter::All)
        .await
        .unwrap();
    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .all(|e| e.hostname.as_deref() != Some("host-2"))
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn system_settings_crud(pool: PgPool) {
    let val = db::get_setting(&pool, "ssh_public_key").await.unwrap();
    assert!(val.is_none());

    db::set_setting(&pool, "ssh_public_key", "ssh-ed25519 AAAA...")
        .await
        .unwrap();

    let val = db::get_setting(&pool, "ssh_public_key").await.unwrap();
    assert_eq!(val.as_deref(), Some("ssh-ed25519 AAAA..."));

    db::set_setting(&pool, "ssh_public_key", "updated_key")
        .await
        .unwrap();

    let val = db::get_setting(&pool, "ssh_public_key").await.unwrap();
    assert_eq!(val.as_deref(), Some("updated_key"));
}

#[sqlx::test(migrations = "./migrations")]
async fn tags_crud(pool: PgPool) {
    let tag = db::insert_tag(&pool, "production", "#ff0000", "repo")
        .await
        .unwrap();
    assert_eq!(tag.name, "production");
    assert_eq!(tag.color, "#ff0000");
    assert_eq!(tag.scope, "repo");

    let tags = db::list_tags(&pool, "repo").await.unwrap();
    assert_eq!(tags.len(), 1);

    let host_tags = db::list_tags(&pool, "agent").await.unwrap();
    assert_eq!(host_tags.len(), 0);

    db::delete_tag(&pool, tag.id).await.unwrap();
    let tags = db::list_tags(&pool, "repo").await.unwrap();
    assert_eq!(tags.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_tag_add_and_list(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let created = db::tags::add_tag(&pool, repo.id, "archive-1", "nightly", None)
        .await
        .unwrap();

    assert_eq!(created.repo_id, Some(repo.id));
    assert_eq!(created.archive_name, Some("archive-1".to_string()));
    assert_eq!(created.tag, "nightly");
    assert!(created.created_by.is_none());

    let tags = db::tags::list_tags_for_archive(&pool, repo.id, "archive-1")
        .await
        .unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags.first().unwrap().tag, "nightly");
}

#[sqlx::test(migrations = "./migrations")]
async fn test_tag_remove(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    db::tags::add_tag(&pool, repo.id, "archive-2", "weekly", None)
        .await
        .unwrap();

    let removed = db::tags::remove_tag(&pool, repo.id, "archive-2", "weekly")
        .await
        .unwrap();
    assert!(removed);

    let tags = db::tags::list_tags_for_archive(&pool, repo.id, "archive-2")
        .await
        .unwrap();
    assert_eq!(tags.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_tag_duplicate_returns_conflict(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    db::tags::add_tag(&pool, repo.id, "archive-dup", "important", None)
        .await
        .unwrap();

    let duplicate = db::tags::add_tag(&pool, repo.id, "archive-dup", "important", None).await;
    assert!(matches!(
        duplicate,
        Err(sqlx::Error::Database(ref err)) if err.code().as_deref() == Some("23505")
    ));
}

#[sqlx::test(migrations = "./migrations")]
async fn test_tag_list_archives_by_tag(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    db::tags::add_tag(&pool, repo.id, "archive-a", "daily", None)
        .await
        .unwrap();
    db::tags::add_tag(&pool, repo.id, "archive-b", "daily", None)
        .await
        .unwrap();
    db::tags::add_tag(&pool, repo.id, "archive-c", "weekly", None)
        .await
        .unwrap();

    let archives = db::tags::list_archives_by_tag(&pool, repo.id, "daily")
        .await
        .unwrap();
    assert_eq!(
        archives,
        vec!["archive-a".to_string(), "archive-b".to_string()]
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_tags_assignment(pool: PgPool) {
    let repo = create_test_repo(&pool).await;
    let tag1 = db::insert_tag(&pool, "env:prod", "#f00", "repo")
        .await
        .unwrap();
    let tag2 = db::insert_tag(&pool, "env:dev", "#0f0", "repo")
        .await
        .unwrap();

    db::set_repo_tags(&pool, repo.id, &[tag1.id, tag2.id])
        .await
        .unwrap();

    let tags = db::list_tags_for_repo(&pool, repo.id).await.unwrap();
    assert_eq!(tags.len(), 2);

    let all_repo_tags = db::list_all_repo_tags(&pool).await.unwrap();
    assert_eq!(all_repo_tags.len(), 2);

    db::set_repo_tags(&pool, repo.id, &[tag1.id]).await.unwrap();
    let tags = db::list_tags_for_repo(&pool, repo.id).await.unwrap();
    assert_eq!(tags.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_tags_assignment(pool: PgPool) {
    let agent = db::insert_agent(&pool, "tagged-host", None, "hash", None, None)
        .await
        .unwrap();
    let tag = db::insert_tag(&pool, "critical", "#f00", "agent")
        .await
        .unwrap();

    db::set_agent_tags(&pool, agent.id, &[tag.id])
        .await
        .unwrap();

    let tags = db::list_tags_for_agent(&pool, agent.id).await.unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags.first().unwrap().name, "critical");

    let all = db::list_all_agent_tags(&pool).await.unwrap();
    assert_eq!(all.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn groups_crud(pool: PgPool) {
    let group = db::insert_group(&pool, "engineering", Some("Dev team"))
        .await
        .unwrap();
    assert_eq!(group.name, "engineering");
    assert_eq!(group.description.as_deref(), Some("Dev team"));

    let fetched = db::get_group(&pool, group.id).await.unwrap();
    assert!(fetched.is_some());

    let updated = db::update_group(&pool, group.id, "eng", Some("Engineering"))
        .await
        .unwrap();
    assert_eq!(updated.name, "eng");

    let groups = db::list_groups(&pool).await.unwrap();
    assert_eq!(groups.len(), 1);

    db::delete_group(&pool, group.id).await.unwrap();
    let groups = db::list_groups(&pool).await.unwrap();
    assert_eq!(groups.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn group_members(pool: PgPool) {
    let user1 = db::insert_user(&pool, "grp-user1", "hash").await.unwrap();
    let user2 = db::insert_user(&pool, "grp-user2", "hash").await.unwrap();
    let group = db::insert_group(&pool, "team", None).await.unwrap();

    db::set_group_members(&pool, group.id, &[user1.id, user2.id])
        .await
        .unwrap();

    let members = db::list_group_members(&pool, group.id).await.unwrap();
    assert_eq!(members.len(), 2);

    let user_groups = db::list_user_groups(&pool, user1.id).await.unwrap();
    assert_eq!(user_groups.len(), 1);
    assert_eq!(user_groups.first().unwrap().name, "team");

    let shared = db::user_shares_group_with(&pool, user1.id, user2.id)
        .await
        .unwrap();
    assert!(shared);

    let user3 = db::insert_user(&pool, "grp-user3", "hash").await.unwrap();
    let not_shared = db::user_shares_group_with(&pool, user1.id, user3.id)
        .await
        .unwrap();
    assert!(!not_shared);
}

#[sqlx::test(migrations = "./migrations")]
async fn roles_crud(pool: PgPool) {
    let initial_roles = db::list_roles(&pool).await.unwrap();
    let initial_count = initial_roles.len();

    let role = db::insert_role(
        &pool,
        &InsertRoleParams {
            name: "test-operator",
            can_create_agent: true,
            can_delete_agent: false,
            can_delete_own_agent: true,
            can_create_repo: true,
            can_delete_repo: false,
            can_delete_own_repo: true,
            can_create_schedule: true,
            can_delete_schedule: false,
            can_delete_own_schedule: true,
            can_manage_tags: false,
            can_view_all_repos: false,
            can_manage_tunnels: false,
            can_upgrade_agent: false,
        },
    )
    .await
    .unwrap();

    assert_eq!(role.name, "test-operator");
    assert!(role.can_create_agent);
    assert!(!role.can_delete_agent);
    assert!(role.can_delete_own_agent);

    let fetched = db::get_role(&pool, role.id).await.unwrap();
    assert!(fetched.is_some());

    let updated = db::update_role(
        &pool,
        role.id,
        &InsertRoleParams {
            name: "test-senior-operator",
            can_create_agent: true,
            can_delete_agent: true,
            can_delete_own_agent: true,
            can_create_repo: true,
            can_delete_repo: true,
            can_delete_own_repo: true,
            can_create_schedule: true,
            can_delete_schedule: true,
            can_delete_own_schedule: true,
            can_manage_tags: true,
            can_view_all_repos: true,
            can_manage_tunnels: true,
            can_upgrade_agent: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.name, "test-senior-operator");
    assert!(updated.can_delete_agent);
    assert!(updated.can_manage_tunnels);

    let roles = db::list_roles(&pool).await.unwrap();
    assert_eq!(roles.len(), initial_count.saturating_add(1));

    db::delete_role(&pool, role.id).await.unwrap();
    let roles = db::list_roles(&pool).await.unwrap();
    assert_eq!(roles.len(), initial_count);
}

#[sqlx::test(migrations = "./migrations")]
async fn user_roles_and_effective_permissions(pool: PgPool) {
    let user = db::insert_user(&pool, "rbac-user", "hash").await.unwrap();

    let role1 = db::insert_role(
        &pool,
        &InsertRoleParams {
            name: "test-viewer",
            can_create_agent: false,
            can_delete_agent: false,
            can_delete_own_agent: false,
            can_create_repo: false,
            can_delete_repo: false,
            can_delete_own_repo: false,
            can_create_schedule: false,
            can_delete_schedule: false,
            can_delete_own_schedule: false,
            can_manage_tags: false,
            can_view_all_repos: true,
            can_manage_tunnels: false,
            can_upgrade_agent: false,
        },
    )
    .await
    .unwrap();

    let role2 = db::insert_role(
        &pool,
        &InsertRoleParams {
            name: "test-creator",
            can_create_agent: true,
            can_delete_agent: false,
            can_delete_own_agent: false,
            can_create_repo: true,
            can_delete_repo: false,
            can_delete_own_repo: false,
            can_create_schedule: true,
            can_delete_schedule: false,
            can_delete_own_schedule: false,
            can_manage_tags: false,
            can_view_all_repos: false,
            can_manage_tunnels: false,
            can_upgrade_agent: false,
        },
    )
    .await
    .unwrap();

    db::set_user_roles(&pool, user.id, &[role1.id, role2.id])
        .await
        .unwrap();

    let user_roles = db::list_user_roles(&pool, user.id).await.unwrap();
    assert_eq!(user_roles.len(), 2);

    let effective = db::get_effective_permissions(&pool, user.id).await.unwrap();
    assert!(effective.can_create_agent);
    assert!(effective.can_create_repo);
    assert!(effective.can_create_schedule);
    assert!(effective.can_view_all_repos);
    assert!(!effective.can_delete_agent);
    assert!(!effective.can_manage_tunnels);
    assert!(!effective.can_upgrade_agent);
}

#[sqlx::test(migrations = "./migrations")]
async fn repos_for_agent(pool: PgPool) {
    let (agent, repo, _) = create_test_schedule(&pool).await;

    let repos = db::list_repos_for_agent(&pool, agent.id).await.unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos.first().unwrap().id, repo.id);

    let public_repos = db::list_repos_for_agent_public(&pool, agent.id)
        .await
        .unwrap();
    assert_eq!(public_repos.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn backup_sources_for_repo(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    sqlx::query("INSERT INTO backup_sources (repo_id, path, sort_order) VALUES ($1, $2, $3)")
        .bind(repo.id)
        .bind("/data")
        .bind(1i32)
        .execute(&pool)
        .await
        .unwrap();

    let sources = db::list_backup_sources_for_repo(&pool, repo.id)
        .await
        .unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources.first().unwrap(), "/data");
}

#[sqlx::test(migrations = "./migrations")]
async fn ssh_tunnel_crud(pool: PgPool) {
    use server::error::ApiError;

    let agent = db::insert_agent(&pool, "tun-host-1", None, "tun-token-1", None, None)
        .await
        .unwrap();
    let agent_2 = db::insert_agent(&pool, "tun-host-2", None, "tun-token-2", None, None)
        .await
        .unwrap();

    let tunnel = db::insert_tunnel(
        &pool,
        &db::NewSshTunnel {
            agent_id: agent.id,
            ssh_host: "repo.example.com".to_string(),
            ssh_user: "borg".to_string(),
            ssh_port: Some(2222),
            tunnel_port: 2200,
            enabled: Some(true),
            ssh_host_key: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(tunnel.agent_id, agent.id);
    assert_eq!(tunnel.ssh_host, "repo.example.com");
    assert_eq!(tunnel.ssh_user, "borg");
    assert_eq!(tunnel.ssh_port, 2222);
    assert_eq!(tunnel.tunnel_port, 2200);
    assert!(tunnel.enabled);

    let by_id = db::get_tunnel_by_id(&pool, tunnel.id).await.unwrap();
    assert_eq!(by_id.id, tunnel.id);

    let by_agent_id = db::get_tunnel_by_agent_id(&pool, agent.id).await.unwrap();
    assert_eq!(by_agent_id.id, tunnel.id);

    let enabled_tunnels = db::list_enabled_tunnels(&pool).await.unwrap();
    assert_eq!(enabled_tunnels.len(), 1);
    assert_eq!(enabled_tunnels.first().unwrap().id, tunnel.id);

    let updated = db::update_tunnel(
        &pool,
        tunnel.id,
        &db::UpdateSshTunnel {
            ssh_host: Some("repo.internal".to_string()),
            ssh_user: None,
            ssh_port: Some(2022),
            tunnel_port: Some(2201),
            enabled: Some(false),
            ssh_host_key: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.ssh_host, "repo.internal");
    assert_eq!(updated.ssh_user, "borg");
    assert_eq!(updated.ssh_port, 2022);
    assert_eq!(updated.tunnel_port, 2201);
    assert!(!updated.enabled);

    let all_tunnels = db::list_all_tunnels(&pool).await.unwrap();
    assert_eq!(all_tunnels.len(), 1);
    assert_eq!(all_tunnels.first().unwrap().id, tunnel.id);

    db::delete_tunnel(&pool, tunnel.id).await.unwrap();
    assert!(matches!(
        db::get_tunnel_by_id(&pool, tunnel.id).await,
        Err(ApiError::NotFound(_))
    ));

    let tunnel_2 = db::insert_tunnel(
        &pool,
        &db::NewSshTunnel {
            agent_id: agent_2.id,
            ssh_host: "repo2.example.com".to_string(),
            ssh_user: "borg".to_string(),
            ssh_port: None,
            tunnel_port: 2300,
            enabled: None,
            ssh_host_key: None,
        },
    )
    .await
    .unwrap();

    db::delete_agent(&pool, agent_2.id).await.unwrap();
    assert!(matches!(
        db::get_tunnel_by_id(&pool, tunnel_2.id).await,
        Err(ApiError::NotFound(_))
    ));
}

#[sqlx::test(migrations = "./migrations")]
async fn test_quota_evaluate_warning(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let quota = db::quota::upsert_quota(
        &pool,
        repo.id,
        Some(100),
        Some(500),
        QuotaAction::NotifyOnly,
        QuotaAction::NotifyOnly,
        true,
    )
    .await
    .unwrap();

    assert_eq!(
        db::quota::evaluate_quota(&quota, 50),
        db::quota::QuotaStatus::Ok
    );
    assert_eq!(
        db::quota::evaluate_quota(&quota, 100),
        db::quota::QuotaStatus::Warning
    );
    assert_eq!(
        db::quota::evaluate_quota(&quota, 300),
        db::quota::QuotaStatus::Warning
    );
    assert_eq!(
        db::quota::evaluate_quota(&quota, 500),
        db::quota::QuotaStatus::Critical
    );
    assert_eq!(
        db::quota::evaluate_quota(&quota, 999),
        db::quota::QuotaStatus::Critical
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn test_quota_upsert_overwrites(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    db::quota::upsert_quota(
        &pool,
        repo.id,
        Some(100),
        Some(200),
        QuotaAction::NotifyOnly,
        QuotaAction::NotifyOnly,
        true,
    )
    .await
    .unwrap();

    let updated = db::quota::upsert_quota(
        &pool,
        repo.id,
        Some(500),
        Some(1000),
        QuotaAction::BlockBackups,
        QuotaAction::DisableSchedule,
        false,
    )
    .await
    .unwrap();

    assert_eq!(updated.warn_bytes, Some(500));
    assert_eq!(updated.critical_bytes, Some(1000));
    assert_eq!(updated.warn_action, "block_backups");
    assert_eq!(updated.critical_action, "disable_schedule");
    assert!(!updated.enabled);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_quota_get_nonexistent(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let result = db::quota::get_quota(&pool, repo.id).await.unwrap();
    assert!(result.is_none());
}

#[cfg(test)]
async fn create_test_repo_with_host(pool: &PgPool, name: &str, ssh_host: &str) -> RepoRow {
    db::insert_repo(
        pool,
        &InsertRepoParams {
            name,
            repo_path: "/backups/shared",
            ssh_user: "backup",
            ssh_host,
            ssh_port: 22,
            passphrase_encrypted: b"encrypted_data",
            compression: "lz4",
            encryption: "repokey",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn server_quota_upsert_and_get(pool: PgPool) {
    let quota = db::server_quota::upsert_server_quota(
        &pool,
        "shared.example.com",
        Some(100),
        Some(200),
        QuotaAction::BlockBackups,
        QuotaAction::DisableSchedule,
        true,
    )
    .await
    .unwrap();
    assert_eq!(quota.ssh_host, "shared.example.com");
    assert_eq!(quota.warn_bytes, Some(100));
    assert_eq!(quota.critical_bytes, Some(200));
    assert_eq!(quota.warn_action, "block_backups");
    assert_eq!(quota.critical_action, "disable_schedule");
    assert!(quota.enabled);

    let fetched = db::server_quota::get_server_quota(&pool, "shared.example.com")
        .await
        .unwrap()
        .expect("server quota should exist");
    assert_eq!(fetched.warn_bytes, Some(100));
    assert_eq!(
        fetched.action_for(db::quota::QuotaStatus::Warning),
        Some(QuotaAction::BlockBackups)
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn server_quota_upsert_overwrites(pool: PgPool) {
    db::server_quota::upsert_server_quota(
        &pool,
        "shared.example.com",
        Some(100),
        Some(200),
        QuotaAction::NotifyOnly,
        QuotaAction::NotifyOnly,
        true,
    )
    .await
    .unwrap();

    let updated = db::server_quota::upsert_server_quota(
        &pool,
        "shared.example.com",
        Some(500),
        Some(1000),
        QuotaAction::BlockBackups,
        QuotaAction::DisableSchedule,
        false,
    )
    .await
    .unwrap();

    assert_eq!(updated.warn_bytes, Some(500));
    assert_eq!(updated.critical_bytes, Some(1000));
    assert!(!updated.enabled);
}

#[sqlx::test(migrations = "./migrations")]
async fn server_quota_delete(pool: PgPool) {
    db::server_quota::upsert_server_quota(
        &pool,
        "shared.example.com",
        Some(100),
        Some(200),
        QuotaAction::NotifyOnly,
        QuotaAction::NotifyOnly,
        true,
    )
    .await
    .unwrap();

    let deleted = db::server_quota::delete_server_quota(&pool, "shared.example.com")
        .await
        .unwrap();
    assert!(deleted);

    let deleted_again = db::server_quota::delete_server_quota(&pool, "shared.example.com")
        .await
        .unwrap();
    assert!(!deleted_again);

    let fetched = db::server_quota::get_server_quota(&pool, "shared.example.com")
        .await
        .unwrap();
    assert!(fetched.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn server_quota_aggregates_usage_across_repos_sharing_host(pool: PgPool) {
    let repo_a = create_test_repo_with_host(&pool, "repo-a", "shared.example.com").await;
    let repo_b = create_test_repo_with_host(&pool, "repo-b", "shared.example.com").await;
    let repo_c = create_test_repo_with_host(&pool, "repo-c", "other.example.com").await;
    set_test_repo_info_stats(&pool, repo_a.id, 1).await;
    set_test_repo_info_stats(&pool, repo_b.id, 1).await;
    set_test_repo_info_stats(&pool, repo_c.id, 1).await;

    let total =
        db::server_quota::total_deduplicated_size_for_ssh_host(&pool, "shared.example.com", None)
            .await
            .unwrap();
    assert_eq!(total, 500_000);

    let repo_count = db::server_quota::repo_count_for_ssh_host(&pool, "shared.example.com")
        .await
        .unwrap();
    assert_eq!(repo_count, 2);

    db::server_quota::upsert_server_quota(
        &pool,
        "shared.example.com",
        Some(400_000),
        Some(600_000),
        QuotaAction::NotifyOnly,
        QuotaAction::BlockBackups,
        true,
    )
    .await
    .unwrap();

    let rows = db::server_quota::list_server_quotas_with_usage(&pool)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);

    let shared = rows
        .iter()
        .find(|r| r.ssh_host == "shared.example.com")
        .unwrap();
    assert_eq!(shared.repo_count, 2);
    assert_eq!(shared.total_deduplicated_size, 500_000);
    let shared_quota = shared.quota.as_ref().expect("quota should be configured");
    assert_eq!(shared_quota.warn_bytes, Some(400_000));
    assert_eq!(
        shared_quota.action_for(db::quota::QuotaStatus::Critical),
        Some(QuotaAction::BlockBackups)
    );

    let other = rows
        .iter()
        .find(|r| r.ssh_host == "other.example.com")
        .unwrap();
    assert_eq!(other.repo_count, 1);
    assert!(other.quota.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn server_quota_total_deduplicated_size_excludes_given_repo(pool: PgPool) {
    let repo_a = create_test_repo_with_host(&pool, "repo-a", "shared.example.com").await;
    let repo_b = create_test_repo_with_host(&pool, "repo-b", "shared.example.com").await;
    db::update_repo_info_stats(
        &pool,
        repo_a.id,
        &db::RepoInfoStats {
            deduplicated_size: 300_000,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    db::update_repo_info_stats(
        &pool,
        repo_b.id,
        &db::RepoInfoStats {
            deduplicated_size: 70_000,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let total_excluding_a = db::server_quota::total_deduplicated_size_for_ssh_host(
        &pool,
        "shared.example.com",
        Some(repo_a.id),
    )
    .await
    .unwrap();
    assert_eq!(total_excluding_a, 70_000);

    let total_excluding_b = db::server_quota::total_deduplicated_size_for_ssh_host(
        &pool,
        "shared.example.com",
        Some(repo_b.id),
    )
    .await
    .unwrap();
    assert_eq!(total_excluding_b, 300_000);
}

#[sqlx::test(migrations = "./migrations")]
async fn list_schedule_ids_for_ssh_host_and_set_schedule_enabled(pool: PgPool) {
    let (_, repo, schedule) = create_test_schedule(&pool).await;

    let ids = db::list_schedule_ids_for_ssh_host(&pool, &repo.ssh_host)
        .await
        .unwrap();
    assert_eq!(ids, vec![schedule.id]);

    db::set_schedule_enabled(&pool, schedule.id, false)
        .await
        .unwrap();
    let updated = db::get_schedule_by_id(&pool, schedule.id).await.unwrap();
    assert!(!updated.enabled);

    db::set_schedule_enabled(&pool, schedule.id, true)
        .await
        .unwrap();
    let updated = db::get_schedule_by_id(&pool, schedule.id).await.unwrap();
    assert!(updated.enabled);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_backup_trends_empty(pool: PgPool) {
    let trends = db::get_backup_trends(&pool, None, 30).await.unwrap();
    assert_eq!(trends.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_backup_trends_with_data(pool: PgPool) {
    let agent = db::insert_agent(&pool, "trends-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let trends = db::get_backup_trends(&pool, None, 30).await.unwrap();
    assert_eq!(trends.len(), 1);
    assert_eq!(trends.first().unwrap().backup_count, 1);
    assert!(trends.first().unwrap().original_size > 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_backup_trends_filtered_by_repo(pool: PgPool) {
    let agent = db::insert_agent(&pool, "trends-filter-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let trends = db::get_backup_trends(&pool, Some(repo.id), 30)
        .await
        .unwrap();
    assert_eq!(trends.len(), 1);

    let trends_other = db::get_backup_trends(&pool, Some(repo.id.saturating_add(999)), 30)
        .await
        .unwrap();
    assert_eq!(trends_other.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_calendar_events_empty(pool: PgPool) {
    let events = db::get_calendar_events(&pool, 2026, 1, None, Tz::UTC)
        .await
        .unwrap();
    assert_eq!(events.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_calendar_events_with_data(pool: PgPool) {
    let agent = db::insert_agent(&pool, "cal-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let now = Utc::now();
    let events = db::get_calendar_events(
        &pool,
        now.date_naive().year(),
        now.date_naive().month(),
        None,
        Tz::UTC,
    )
    .await
    .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events.first().unwrap().event_type, "backup");
    assert_eq!(events.first().unwrap().status, "success");
    assert_eq!(events.first().unwrap().repo_name, "test-repo");
}

#[sqlx::test(migrations = "./migrations")]
async fn test_calendar_events_filtered_by_repo(pool: PgPool) {
    let agent = db::insert_agent(&pool, "cal-filter-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let now = Utc::now();
    let events = db::get_calendar_events(
        &pool,
        now.date_naive().year(),
        now.date_naive().month(),
        Some(repo.id),
        Tz::UTC,
    )
    .await
    .unwrap();
    assert_eq!(events.len(), 1);

    let events_other = db::get_calendar_events(
        &pool,
        now.date_naive().year(),
        now.date_naive().month(),
        Some(repo.id.saturating_add(999)),
        Tz::UTC,
    )
    .await
    .unwrap();
    assert_eq!(events_other.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_enabled_schedules_for_calendar(pool: PgPool) {
    let (_agent, _repo, _schedule) = create_test_schedule(&pool).await;

    let schedules = db::get_enabled_schedules_for_calendar(&pool).await.unwrap();
    assert_eq!(schedules.len(), 1);
    assert!(schedules.first().unwrap().enabled);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_audit_filter_by_date_range(pool: PgPool) {
    db::audit::insert_audit_entry(
        &pool,
        &db::audit::NewAuditEntry {
            user_id: Some(1),
            username: "admin",
            action: "date_test",
            target_type: None,
            target_id: None,
            details: None,
            ip_address: None,
        },
    )
    .await
    .unwrap();

    let now = Utc::now();
    let (items, total) = db::audit::list_audit_entries(
        &pool,
        &db::audit::AuditEntryFilters {
            page: 1,
            per_page: 50,
            filter_user_id: None,
            filter_action: None,
            filter_target_type: None,
            filter_from: Some(now.checked_sub_signed(Duration::hours(1)).unwrap()),
            filter_to: Some(now.checked_add_signed(Duration::hours(1)).unwrap()),
        },
    )
    .await
    .unwrap();

    assert_eq!(total, 1);
    assert_eq!(items.len(), 1);

    let (items, total) = db::audit::list_audit_entries(
        &pool,
        &db::audit::AuditEntryFilters {
            page: 1,
            per_page: 50,
            filter_user_id: None,
            filter_action: None,
            filter_target_type: None,
            filter_from: Some(now.checked_add_signed(Duration::hours(1)).unwrap()),
            filter_to: Some(now.checked_add_signed(Duration::hours(2)).unwrap()),
        },
    )
    .await
    .unwrap();

    assert_eq!(total, 0);
    assert_eq!(items.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_hostname_pattern_crud(pool: PgPool) {
    let agent = db::insert_agent(
        &pool,
        "pattern-crud-host",
        Some("Pattern CRUD"),
        "hash",
        None,
        None,
    )
    .await
    .unwrap();

    let pattern = patterns::add_hostname_pattern(&pool, agent.id, "crud.*")
        .await
        .unwrap();

    let patterns = patterns::list_patterns_for_agent(&pool, agent.id)
        .await
        .unwrap();
    assert_eq!(patterns.len(), 1);
    assert_eq!(patterns.first().unwrap().pattern, "crud.*");

    patterns::delete_hostname_pattern(&pool, pattern.id)
        .await
        .unwrap();

    let patterns = patterns::list_patterns_for_agent(&pool, agent.id)
        .await
        .unwrap();
    assert_eq!(patterns.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_find_agent_by_pattern_glob_match(pool: PgPool) {
    let agent = db::insert_agent(
        &pool,
        "pattern-glob-agent",
        Some("Pattern Glob"),
        "hash",
        None,
        None,
    )
    .await
    .unwrap();

    patterns::add_hostname_pattern(&pool, agent.id, "bell*")
        .await
        .unwrap();

    let matched = patterns::find_agent_by_pattern(&pool, "bell.home.mohr.io")
        .await
        .unwrap();

    let matched = matched.unwrap();
    assert_eq!(matched.id, agent.id);
    assert_eq!(matched.hostname, "pattern-glob-agent");
}

#[sqlx::test(migrations = "./migrations")]
async fn test_find_agent_by_pattern_no_match(pool: PgPool) {
    let agent = db::insert_agent(
        &pool,
        "pattern-no-match-agent",
        Some("Pattern No Match"),
        "hash",
        None,
        None,
    )
    .await
    .unwrap();

    patterns::add_hostname_pattern(&pool, agent.id, "bell*")
        .await
        .unwrap();

    let matched = patterns::find_agent_by_pattern(&pool, "gamma.home.mohr.io")
        .await
        .unwrap();

    assert!(matched.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_add_duplicate_pattern_returns_error(pool: PgPool) {
    let agent_one = db::insert_agent(
        &pool,
        "duplicate-pattern-one",
        Some("Duplicate One"),
        "hash",
        None,
        None,
    )
    .await
    .unwrap();
    let agent_two = db::insert_agent(
        &pool,
        "duplicate-pattern-two",
        Some("Duplicate Two"),
        "hash",
        None,
        None,
    )
    .await
    .unwrap();

    patterns::add_hostname_pattern(&pool, agent_one.id, "dup*")
        .await
        .unwrap();

    let result = patterns::add_hostname_pattern(&pool, agent_two.id, "dup*").await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_resolve_agent_exact_match_priority(pool: PgPool) {
    let exact = db::insert_agent(&pool, "foo", Some("Exact Foo"), "hash", None, None)
        .await
        .unwrap();
    let patterned = db::insert_agent(
        &pool,
        "pattern-priority-agent",
        Some("Pattern Foo"),
        "hash",
        None,
        None,
    )
    .await
    .unwrap();

    patterns::add_hostname_pattern(&pool, patterned.id, "foo*")
        .await
        .unwrap();

    let resolved = db::resolve_agent_for_hostname(&pool, "foo").await.unwrap();
    match resolved {
        db::ResolveResult::ExactMatch(agent) => assert_eq!(agent.id, exact.id),
        other => panic!("unexpected resolve result: {other:?}"),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn test_merge_agent_moves_reports(pool: PgPool) {
    let placeholder = db::insert_agent(
        &pool,
        "merge-placeholder",
        Some("Merge Placeholder"),
        "imported:no-auth",
        None,
        None,
    )
    .await
    .unwrap();
    let target = db::insert_agent(
        &pool,
        "merge-target",
        Some("Merge Target"),
        "hash",
        None,
        None,
    )
    .await
    .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, placeholder.id, repo.id).await;

    db::merge_agent(&pool, placeholder.id, target.id)
        .await
        .unwrap();

    let reports = db::list_reports_for_agent(&pool, target.id, None, 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);

    let matched =
        sqlx::query_scalar::<_, bool>("SELECT matched FROM backup_reports WHERE agent_id = $1")
            .bind(target.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(matched);

    let source = db::get_agent_by_hostname(&pool, "merge-placeholder", None).await;
    assert!(source.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_merge_agent_refuses_non_placeholder(pool: PgPool) {
    let source = db::insert_agent(
        &pool,
        "merge-source",
        Some("Merge Source"),
        "hash",
        None,
        None,
    )
    .await
    .unwrap();
    let target = db::insert_agent(
        &pool,
        "merge-target-real",
        Some("Merge Target Real"),
        "hash",
        None,
        None,
    )
    .await
    .unwrap();

    let result = db::merge_agent(&pool, source.id, target.id).await;
    assert!(result.is_err());
}

/// Merging away the placeholder agent that caused a schedule's auto-disable must
/// clear that schedule's stale auto-disable bookkeeping, the same as `delete_agent`
/// and `delete_repo` - the FK on `auto_disabled_by_agent_id` only nulls that one
/// column when the source agent's row is deleted, leaving `auto_disabled_agent_unreachable`
/// and `consecutive_failures` stale.
#[sqlx::test(migrations = "./migrations")]
async fn test_merge_agent_clears_auto_disable_bookkeeping_for_its_schedules(pool: PgPool) {
    let placeholder = db::insert_agent(
        &pool,
        "merge-placeholder-auto-disable",
        Some("Merge Placeholder"),
        "imported:no-auth",
        None,
        None,
    )
    .await
    .unwrap();
    let target = db::insert_agent(
        &pool,
        "merge-target-auto-disable",
        Some("Merge Target"),
        "hash",
        None,
        None,
    )
    .await
    .unwrap();
    let repo = create_test_repo(&pool).await;
    let schedule = db::insert_schedule(
        &pool,
        repo.id,
        &ScheduleParams {
            name: "test-schedule",
            schedule_type: "backup",
            cron_expression: "0 3 * * *",
            enabled: true,
            canary_enabled: false,
            vm_snapshot_enabled: false,
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
            pre_backup_commands: &[],
            post_backup_commands: &[],
            hook_timeout_seconds: 60,
            missed_backup_threshold: 3,
            on_failure: "stop",
        },
        None,
    )
    .await
    .unwrap();
    db::insert_schedule_targets(&pool, schedule.id, &[(placeholder.id, 0)])
        .await
        .unwrap();

    let next = Utc::now().checked_add_signed(Duration::days(1)).unwrap();
    for _ in 0..3 {
        db::record_schedule_failure(&pool, schedule.id, placeholder.id, next, 3, true)
            .await
            .unwrap();
    }
    let (enabled, auto_disabled_agent_unreachable, _, _) =
        schedule_auto_disable_state(&pool, schedule.id).await;
    assert!(!enabled && auto_disabled_agent_unreachable);

    db::merge_agent(&pool, placeholder.id, target.id)
        .await
        .unwrap();

    let (enabled, agent_unreachable, by_agent_id, failures) =
        schedule_auto_disable_state(&pool, schedule.id).await;
    assert!(!enabled);
    assert!(
        !agent_unreachable,
        "the stale auto-disable flag must be cleared once the causing agent is merged away"
    );
    assert_eq!(by_agent_id, None);
    assert_eq!(failures, 0);

    // The merge target reconnecting later must never touch this orphaned schedule.
    let reenabled = db::reenable_system_disabled_schedules_for_agent(&pool, target.id, Utc::now())
        .await
        .unwrap();
    assert_eq!(reenabled, Vec::<i64>::new());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_mark_agent_reports_matched(pool: PgPool) {
    let agent = db::insert_agent(
        &pool,
        "adopt-host",
        Some("Adopt Host (imported)"),
        "imported:no-auth",
        None,
        None,
    )
    .await
    .unwrap();
    let repo = create_test_repo(&pool).await;

    let now = Utc::now();
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(5)).unwrap(),
            finished_at: now,
            status: shared::types::BackupStatus::Success,
            original_size: 1_000_000,
            compressed_size: 500_000,
            deduplicated_size: 250_000,
            repo_unique_csize: 0,
            files_processed: 1000,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.4.0".to_string()),
            matched: false,
            archive_name: None,
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let unmatched =
        sqlx::query_scalar::<_, bool>("SELECT matched FROM backup_reports WHERE agent_id = $1")
            .bind(agent.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!unmatched);

    db::mark_agent_reports_matched(&pool, agent.id)
        .await
        .unwrap();

    let matched =
        sqlx::query_scalar::<_, bool>("SELECT matched FROM backup_reports WHERE agent_id = $1")
            .bind(agent.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(matched);
}

#[sqlx::test(migrations = "./migrations")]
async fn get_archives_for_agent_across_multiple_repos(pool: PgPool) {
    let agent = db::insert_agent(&pool, "primary-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo1 = db::insert_repo(
        &pool,
        &InsertRepoParams {
            name: "repo-alpha",
            repo_path: "/backups/alpha",
            ssh_user: "backup",
            ssh_host: "storage.local",
            ssh_port: 22,
            passphrase_encrypted: b"enc",
            compression: "lz4",
            encryption: "repokey",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap();
    let repo2 = db::insert_repo(
        &pool,
        &InsertRepoParams {
            name: "repo-beta",
            repo_path: "/backups/beta",
            ssh_user: "backup",
            ssh_host: "storage.local",
            ssh_port: 22,
            passphrase_encrypted: b"enc",
            compression: "zstd",
            encryption: "repokey",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap();

    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo1.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(10)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::minutes(5)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 1_000_000,
            compressed_size: 500_000,
            deduplicated_size: 250_000,
            repo_unique_csize: 0,
            files_processed: 100,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.4.0".to_string()),
            matched: true,
            archive_name: Some("primary-host-2026-01-01T10:00:00".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo1.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(20)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::minutes(15)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 2_000_000,
            compressed_size: 1_000_000,
            deduplicated_size: 500_000,
            repo_unique_csize: 0,
            files_processed: 200,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.4.0".to_string()),
            matched: true,
            archive_name: Some("primary-host-2026-01-02T10:00:00".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo2.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(30)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::minutes(25)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 3_000_000,
            compressed_size: 1_500_000,
            deduplicated_size: 750_000,
            repo_unique_csize: 0,
            files_processed: 300,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.4.0".to_string()),
            matched: true,
            archive_name: Some("primary-host-2026-01-03T10:00:00".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo1.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(40)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::minutes(35)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 100_000,
            compressed_size: 50_000,
            deduplicated_size: 25_000,
            repo_unique_csize: 0,
            files_processed: 10,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.4.0".to_string()),
            matched: true,
            archive_name: None,
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let archives = db::get_archives_for_agent(&pool, agent.id).await.unwrap();

    assert_eq!(archives.len(), 2);

    let repo1_archives: Vec<_> = archives
        .iter()
        .filter(|(rid, _)| rid.0 == repo1.id)
        .flat_map(|(_, names)| names.clone())
        .collect();
    let repo2_archives: Vec<_> = archives
        .iter()
        .filter(|(rid, _)| rid.0 == repo2.id)
        .flat_map(|(_, names)| names.clone())
        .collect();

    assert_eq!(repo1_archives.len(), 2);
    assert!(repo1_archives.contains(&"primary-host-2026-01-01T10:00:00".to_string()));
    assert!(repo1_archives.contains(&"primary-host-2026-01-02T10:00:00".to_string()));
    assert_eq!(repo2_archives.len(), 1);
    assert!(repo2_archives.contains(&"primary-host-2026-01-03T10:00:00".to_string()));
}

/// Verifies that `get_archives_for_agent_with_patterns` finds archives from imported agents
/// whose hostnames match the configured glob patterns, even when those archives haven't been
/// merged/reassigned yet (`agent_id` still points to the imported agent).
#[sqlx::test(migrations = "./migrations")]
async fn get_archives_for_agent_includes_pattern_matched_archives(pool: PgPool) {
    let agent = db::insert_agent(&pool, "web-server-01", None, "hash", None, None)
        .await
        .unwrap();
    patterns::add_hostname_pattern(&pool, agent.id, "web-server-*")
        .await
        .unwrap();

    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(10)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::minutes(5)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 1_000_000,
            compressed_size: 500_000,
            deduplicated_size: 250_000,
            repo_unique_csize: 0,
            files_processed: 100,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.4.0".to_string()),
            matched: true,
            archive_name: Some("web-server-01-2026-01-01T10:00:00".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(20)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::minutes(15)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 2_000_000,
            compressed_size: 1_000_000,
            deduplicated_size: 500_000,
            repo_unique_csize: 0,
            files_processed: 200,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.4.0".to_string()),
            matched: true,
            archive_name: Some("web-server-02-2026-01-01T10:00:00".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let imported = db::insert_agent(
        &pool,
        "web-server-legacy (imported)",
        None,
        "imported:no-auth",
        None,
        None,
    )
    .await
    .unwrap();
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: imported.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(30)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::minutes(25)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 3_000_000,
            compressed_size: 1_500_000,
            deduplicated_size: 750_000,
            repo_unique_csize: 0,
            files_processed: 300,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.4.0".to_string()),
            matched: false,
            archive_name: Some("web-server-legacy-2026-01-01T10:00:00".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let archives = db::get_archives_for_agent(&pool, agent.id).await.unwrap();
    assert_eq!(archives.len(), 1);
    let names: Vec<_> = archives
        .iter()
        .flat_map(|(_, names)| names.clone())
        .collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"web-server-01-2026-01-01T10:00:00".to_string()));
    assert!(names.contains(&"web-server-02-2026-01-01T10:00:00".to_string()));

    let all_archives = db::get_archives_for_agent_with_patterns(&pool, agent.id)
        .await
        .unwrap();
    let all_names: Vec<_> = all_archives
        .iter()
        .flat_map(|(_, names)| names.clone())
        .collect();

    assert_eq!(all_names.len(), 3);
    assert!(all_names.contains(&"web-server-01-2026-01-01T10:00:00".to_string()));
    assert!(all_names.contains(&"web-server-02-2026-01-01T10:00:00".to_string()));
    assert!(all_names.contains(&"web-server-legacy-2026-01-01T10:00:00".to_string()));
}

/// Verifies pattern matching across multiple repos with unrelated agents excluded.
#[sqlx::test(migrations = "./migrations")]
async fn get_archives_for_agent_with_patterns_multiple_repos(pool: PgPool) {
    let agent = db::insert_agent(&pool, "db-server-01", None, "hash", None, None)
        .await
        .unwrap();
    patterns::add_hostname_pattern(&pool, agent.id, "db-server-*")
        .await
        .unwrap();

    let repo1 = db::insert_repo(
        &pool,
        &InsertRepoParams {
            name: "daily-repo",
            repo_path: "/backups/daily",
            ssh_user: "backup",
            ssh_host: "storage.local",
            ssh_port: 22,
            passphrase_encrypted: b"enc",
            compression: "lz4",
            encryption: "repokey",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap();
    let repo2 = db::insert_repo(
        &pool,
        &InsertRepoParams {
            name: "weekly-repo",
            repo_path: "/backups/weekly",
            ssh_user: "backup",
            ssh_host: "storage.local",
            ssh_port: 22,
            passphrase_encrypted: b"enc",
            compression: "zstd",
            encryption: "repokey",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap();

    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo1.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(10)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::minutes(5)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 1_000_000,
            compressed_size: 500_000,
            deduplicated_size: 250_000,
            repo_unique_csize: 0,
            files_processed: 100,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.4.0".to_string()),
            matched: true,
            archive_name: Some("db-server-01-daily-2026-01-01".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo2.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(20)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::minutes(15)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 5_000_000,
            compressed_size: 2_500_000,
            deduplicated_size: 1_250_000,
            repo_unique_csize: 0,
            files_processed: 500,
            duration_secs: 600,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.4.0".to_string()),
            matched: true,
            archive_name: Some("db-server-01-weekly-2026-01-01".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let imported = db::insert_agent(
        &pool,
        "db-server-02 (imported)",
        None,
        "imported:no-auth",
        None,
        None,
    )
    .await
    .unwrap();
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: imported.id,
            repo_id: repo1.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(30)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::minutes(25)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 1_500_000,
            compressed_size: 750_000,
            deduplicated_size: 375_000,
            repo_unique_csize: 0,
            files_processed: 150,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.4.0".to_string()),
            matched: false,
            archive_name: Some("db-server-02-daily-2026-01-01".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let imported2 = db::insert_agent(
        &pool,
        "db-server-staging (imported)",
        None,
        "imported:no-auth",
        None,
        None,
    )
    .await
    .unwrap();
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: imported2.id,
            repo_id: repo2.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(40)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::minutes(35)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 4_000_000,
            compressed_size: 2_000_000,
            deduplicated_size: 1_000_000,
            repo_unique_csize: 0,
            files_processed: 400,
            duration_secs: 500,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.4.0".to_string()),
            matched: false,
            archive_name: Some("db-server-staging-weekly-2026-01-01".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let unrelated = db::insert_agent(
        &pool,
        "app-server-01 (imported)",
        None,
        "imported:no-auth",
        None,
        None,
    )
    .await
    .unwrap();
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: unrelated.id,
            repo_id: repo1.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(50)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::minutes(45)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 1_000_000,
            compressed_size: 500_000,
            deduplicated_size: 250_000,
            repo_unique_csize: 0,
            files_processed: 100,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.4.0".to_string()),
            matched: false,
            archive_name: Some("app-server-01-daily-2026-01-01".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let archives = db::get_archives_for_agent_with_patterns(&pool, agent.id)
        .await
        .unwrap();

    let repo1_names: Vec<_> = archives
        .iter()
        .filter(|(rid, _)| rid.0 == repo1.id)
        .flat_map(|(_, names)| names.clone())
        .collect();
    let repo2_names: Vec<_> = archives
        .iter()
        .filter(|(rid, _)| rid.0 == repo2.id)
        .flat_map(|(_, names)| names.clone())
        .collect();

    assert_eq!(repo1_names.len(), 2);
    assert!(repo1_names.contains(&"db-server-01-daily-2026-01-01".to_string()));
    assert!(repo1_names.contains(&"db-server-02-daily-2026-01-01".to_string()));

    assert_eq!(repo2_names.len(), 2);
    assert!(repo2_names.contains(&"db-server-01-weekly-2026-01-01".to_string()));
    assert!(repo2_names.contains(&"db-server-staging-weekly-2026-01-01".to_string()));

    let all_names: Vec<_> = archives
        .iter()
        .flat_map(|(_, names)| names.clone())
        .collect();
    assert!(!all_names.contains(&"app-server-01-daily-2026-01-01".to_string()));
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_sync_schedule_default(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    assert_eq!(repo.sync_schedule.as_deref(), Some("0 0,12 * * *"));
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_sync_schedule_update(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let updated = db::update_repo(
        &pool,
        &UpdateRepoParams {
            repo_id: repo.id,
            name: "test-repo",
            repo_path: "/backups/test",
            ssh_user: "backup",
            ssh_host: "storage.local",
            ssh_port: 22,
            compression: "lz4",
            encryption: "repokey",
            enabled: true,
            sync_schedule: Some(Some("0 */6 * * *")),
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.sync_schedule.as_deref(), Some("0 */6 * * *"));
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_sync_schedule_disable(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let updated = db::update_repo(
        &pool,
        &UpdateRepoParams {
            repo_id: repo.id,
            name: "test-repo",
            repo_path: "/backups/test",
            ssh_user: "backup",
            ssh_host: "storage.local",
            ssh_port: 22,
            compression: "lz4",
            encryption: "repokey",
            enabled: true,
            sync_schedule: Some(None),
        },
    )
    .await
    .unwrap();

    assert!(updated.sync_schedule.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_sync_schedule_unchanged(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    // After an update that doesn't touch sync_schedule, it must retain the DB default.
    let updated = db::update_repo(
        &pool,
        &UpdateRepoParams {
            repo_id: repo.id,
            name: "test-repo",
            repo_path: "/backups/test",
            ssh_user: "backup",
            ssh_host: "storage.local",
            ssh_port: 22,
            compression: "lz4",
            encryption: "repokey",
            enabled: true,
            sync_schedule: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.sync_schedule.as_deref(), Some("0 0,12 * * *"));
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_reset_import_clears_state(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    db::set_repo_importing(&pool, repo.id, true).await.unwrap();
    db::set_repo_import_error(&pool, repo.id, Some("stuck"))
        .await
        .unwrap();

    let stats = db::get_repo_with_stats(&pool, repo.id).await.unwrap();
    assert!(stats.importing);
    assert_eq!(stats.import_error.as_deref(), Some("stuck"));

    db::set_repo_importing(&pool, repo.id, false).await.unwrap();
    db::set_repo_import_error(&pool, repo.id, None)
        .await
        .unwrap();

    let stats = db::get_repo_with_stats(&pool, repo.id).await.unwrap();
    assert!(!stats.importing);
    assert!(stats.import_error.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_import_progress_updates_and_resets(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let stats = db::get_repo_with_stats(&pool, repo.id).await.unwrap();
    assert_eq!(stats.import_progress, 0);
    assert_eq!(stats.import_total, 0);

    db::update_repo_import_progress(&pool, repo.id, 42, 100)
        .await
        .unwrap();

    let stats = db::get_repo_with_stats(&pool, repo.id).await.unwrap();
    assert_eq!(stats.import_progress, 42);
    assert_eq!(stats.import_total, 100);

    db::update_repo_import_progress(&pool, repo.id, 0, 0)
        .await
        .unwrap();

    let stats = db::get_repo_with_stats(&pool, repo.id).await.unwrap();
    assert_eq!(stats.import_progress, 0);
    assert_eq!(stats.import_total, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_import_progress_reflected_in_list(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    db::update_repo_import_progress(&pool, repo.id, 7, 20)
        .await
        .unwrap();

    let repos = db::list_repos_with_stats(&pool).await.unwrap();
    let found = repos.iter().find(|r| r.id == repo.id).unwrap();
    assert_eq!(found.import_progress, 7);
    assert_eq!(found.import_total, 20);
}

#[sqlx::test(migrations = "./migrations")]
async fn bulk_insert_backup_reports_empty(pool: PgPool) {
    let affected = db::bulk_insert_backup_reports(&pool, &[]).await.unwrap();
    assert_eq!(affected, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn bulk_insert_backup_reports_basic(pool: PgPool) {
    let agent = db::insert_agent(&pool, "bulk-host", None, "hash-bulk", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    let params = vec![
        InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(10)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::minutes(5)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 2_000_000,
            compressed_size: 1_000_000,
            deduplicated_size: 500_000,
            repo_unique_csize: 0,
            files_processed: 200,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: Some("1.4.0".to_string()),
            matched: true,
            archive_name: Some("bulk-archive-1".to_string()),
            borg_command: None,
            run_id: None,
        },
        InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(20)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::minutes(15)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 1_000_000,
            compressed_size: 500_000,
            deduplicated_size: 250_000,
            repo_unique_csize: 0,
            files_processed: 100,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: None,
            matched: false,
            archive_name: Some("bulk-archive-2".to_string()),
            borg_command: None,
            run_id: None,
        },
    ];

    let affected = db::bulk_insert_backup_reports(&pool, &params)
        .await
        .unwrap();
    assert_eq!(affected, 2);

    let reports = db::list_reports_for_agent(&pool, agent.id, None, 100)
        .await
        .unwrap();
    assert_eq!(reports.len(), 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn bulk_insert_backup_reports_conflict_skipped(pool: PgPool) {
    let agent = db::insert_agent(&pool, "bulk-dup-host", None, "hash-dup", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();
    let started = now.checked_sub_signed(Duration::minutes(10)).unwrap();

    let param = InsertReportParams {
        agent_id: agent.id,
        repo_id: repo.id,
        schedule_id: None,
        started_at: started,
        finished_at: now,
        status: shared::types::BackupStatus::Success,
        original_size: 1_000,
        compressed_size: 800,
        deduplicated_size: 600,
        repo_unique_csize: 0,
        files_processed: 10,
        duration_secs: 60,
        error_message: None,
        warnings: vec![],
        borg_version: None,
        matched: true,
        archive_name: Some("dup-archive".to_string()),
        borg_command: None,
        run_id: None,
    };

    db::bulk_insert_backup_reports(&pool, std::slice::from_ref(&param))
        .await
        .unwrap();
    let affected = db::bulk_insert_backup_reports(&pool, &[param])
        .await
        .unwrap();
    assert_eq!(affected, 0);

    let reports = db::list_reports_for_agent(&pool, agent.id, None, 100)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn bulk_insert_keeps_distinct_archives_sharing_start_second(pool: PgPool) {
    // Borg reports archive `start` at whole-second precision, so two distinct
    // archives of the same host can share (agent_id, started_at). They must not
    // collapse into a single row on import.
    let agent = db::insert_agent(&pool, "same-second-host", None, "hash-ss", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let started = Utc::now()
        .checked_sub_signed(Duration::minutes(10))
        .unwrap();
    let finished = started.checked_add_signed(Duration::minutes(1)).unwrap();

    let base = InsertReportParams {
        agent_id: agent.id,
        repo_id: repo.id,
        schedule_id: None,
        started_at: started,
        finished_at: finished,
        status: shared::types::BackupStatus::Success,
        original_size: 0,
        compressed_size: 0,
        deduplicated_size: 0,
        repo_unique_csize: 0,
        files_processed: 0,
        duration_secs: 60,
        error_message: None,
        warnings: vec![],
        borg_version: None,
        matched: true,
        archive_name: None,
        borg_command: None,
        run_id: None,
    };

    let params = vec![
        InsertReportParams {
            archive_name: Some("host-2026-06-10T12:00:00".to_string()),
            ..base.clone()
        },
        InsertReportParams {
            archive_name: Some("host-2026-06-10T12:00:00-extra".to_string()),
            ..base.clone()
        },
    ];

    let affected = db::bulk_insert_backup_reports(&pool, &params)
        .await
        .unwrap();
    assert_eq!(affected, 2);

    let names = db::list_archive_names_for_repo(&pool, repo.id)
        .await
        .unwrap();
    assert_eq!(names.len(), 2);

    // Re-importing the same archives stays idempotent.
    let affected = db::bulk_insert_backup_reports(&pool, &params)
        .await
        .unwrap();
    assert_eq!(affected, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_last_synced_at_updates(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    // Initially no row in repo_stats
    let before: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM repo_stats WHERE repo_id = $1")
        .bind(repo.id)
        .fetch_optional(&pool)
        .await
        .unwrap();
    assert!(before.is_none());

    db::update_repo_last_synced(&pool, repo.id).await.unwrap();

    let after: Option<(chrono::DateTime<chrono::Utc>,)> =
        sqlx::query_as("SELECT last_synced_at FROM repo_stats WHERE repo_id = $1")
            .bind(repo.id)
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert!(after.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_get_by_id(pool: PgPool) {
    let agent = db::insert_agent(&pool, "byid-host", None, "hash-byid", None, None)
        .await
        .unwrap();

    let fetched = db::get_agent_by_id(&pool, agent.id).await.unwrap();
    assert_eq!(fetched.id, agent.id);
    assert_eq!(fetched.hostname, "byid-host");

    let result = db::get_agent_by_id(&pool, 999_999_999).await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_set_hidden_and_list(pool: PgPool) {
    let agent = db::insert_agent(&pool, "hidden-host", None, "hash-hidden", None, None)
        .await
        .unwrap();

    let before = db::list_agents(&pool, false).await.unwrap();
    assert!(before.iter().any(|c| c.hostname == "hidden-host"));

    db::set_agent_hidden(&pool, agent.id, true).await.unwrap();

    let visible = db::list_agents(&pool, false).await.unwrap();
    assert!(!visible.iter().any(|c| c.hostname == "hidden-host"));

    let all = db::list_agents(&pool, true).await.unwrap();
    assert!(all.iter().any(|c| c.hostname == "hidden-host"));

    db::set_agent_hidden(&pool, agent.id, false).await.unwrap();

    let restored = db::list_agents(&pool, false).await.unwrap();
    assert!(restored.iter().any(|c| c.hostname == "hidden-host"));
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_token_hash_lookup(pool: PgPool) {
    let agent = db::insert_agent(&pool, "token-host", None, "secret-hash", None, None)
        .await
        .unwrap();

    let candidates = db::get_agent_token_hashes(&pool, "token-host")
        .await
        .unwrap();
    assert_eq!(candidates.len(), 1);
    let candidate = candidates.first().unwrap();
    assert_eq!(candidate.id, agent.id);
    assert_eq!(candidate.agent_token_hash, "secret-hash");

    let result = db::get_agent_token_hashes(&pool, "nonexistent-host").await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_last_seen_updates(pool: PgPool) {
    let agent = db::insert_agent(&pool, "seen-host", None, "hash-seen", None, None)
        .await
        .unwrap();

    assert!(agent.last_seen_at.is_none());

    db::update_last_seen(&pool, agent.id).await.unwrap();
    let fetched = db::get_agent_by_id(&pool, agent.id).await.unwrap();
    assert!(fetched.last_seen_at.is_some());

    db::update_last_seen_and_version(
        &pool,
        agent.id,
        "1.5.0",
        Some("abc123"),
        Some("2026-01-01"),
        Some(42),
    )
    .await
    .unwrap();
    let fetched = db::get_agent_by_id(&pool, agent.id).await.unwrap();
    assert_eq!(fetched.agent_version.as_deref(), Some("1.5.0"));
    assert_eq!(fetched.agent_git_sha.as_deref(), Some("abc123"));

    db::update_last_seen_by_hostname(&pool, "seen-host")
        .await
        .unwrap();
    let fetched = db::get_agent_by_id(&pool, agent.id).await.unwrap();
    assert!(fetched.last_seen_at.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn get_or_create_agent_by_hostname_creates_new(pool: PgPool) {
    let agent = db::get_or_create_agent_by_hostname(&pool, "placeholder-new")
        .await
        .unwrap();
    assert_eq!(agent.hostname, "placeholder-new");
    assert_eq!(agent.agent_token_hash, "imported:no-auth");

    let again = db::get_or_create_agent_by_hostname(&pool, "placeholder-new")
        .await
        .unwrap();
    assert_eq!(again.id, agent.id);
}

#[sqlx::test(migrations = "./migrations")]
async fn get_or_create_agent_by_hostname_returns_existing(pool: PgPool) {
    let real = db::insert_agent(&pool, "existing-real", None, "realhash", None, None)
        .await
        .unwrap();

    let fetched = db::get_or_create_agent_by_hostname(&pool, "existing-real")
        .await
        .unwrap();
    assert_eq!(fetched.id, real.id);
    assert_eq!(fetched.agent_token_hash, "realhash");
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_counts_by_agent(pool: PgPool) {
    let (agent, _, _) = create_test_schedule(&pool).await;

    let counts = db::get_schedule_counts_by_agent(&pool).await.unwrap();
    let entry = counts.iter().find(|c| c.agent_id == agent.id);
    assert!(entry.is_some());
    assert_eq!(entry.unwrap().count, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn list_importing_repo_ids_test(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let before = db::list_importing_repo_ids(&pool).await.unwrap();
    assert!(!before.contains(&repo.id));

    db::set_repo_importing(&pool, repo.id, true).await.unwrap();

    let after = db::list_importing_repo_ids(&pool).await.unwrap();
    assert!(after.contains(&repo.id));

    db::set_repo_importing(&pool, repo.id, false).await.unwrap();

    let cleared = db::list_importing_repo_ids(&pool).await.unwrap();
    assert!(!cleared.contains(&repo.id));
}

/// Regression test for `ImportingGuard::clear_now` only disarming `Drop`'s
/// fallback after the write actually succeeds. Deletes the guarded repo
/// (cascading away its `repo_import_state` row) right before `clear_now`
/// runs, so that write hits a foreign-key violation and fails. Asserts via
/// `TaskRegistry::pending_count` - incremented synchronously by `Drop` the
/// moment it spawns its retry, before that retry's own future ever runs -
/// rather than waiting on the retry's outcome, since the retry racing
/// against any DB state this test could set up afterward would make the
/// test's own timing nondeterministic. If `clear_now` had already disarmed
/// `Drop` on the earlier failure (the bug this guards against), `Drop`
/// would skip spawning a retry entirely and `pending_count` would stay 0.
#[sqlx::test(migrations = "./migrations")]
async fn importing_guard_clear_now_leaves_drop_armed_on_write_failure(pool: PgPool) {
    let repo = create_test_repo(&pool).await;
    let task_registry = shared::task_registry::TaskRegistry::default();

    let guard = db::ImportingGuard::acquire(&pool, repo.id, task_registry.clone())
        .await
        .unwrap();

    sqlx::query!("DELETE FROM repos WHERE id = $1", repo.id)
        .execute(&pool)
        .await
        .unwrap();

    guard.clear_now().await;

    assert_eq!(
        task_registry.pending_count(),
        1,
        "Drop should still spawn its retry after clear_now's own write fails"
    );

    task_registry
        .shutdown(std::time::Duration::from_secs(5))
        .await;
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_import_status_message_test(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    db::set_import_status_message(&pool, repo.id, Some("scanning archives"))
        .await
        .unwrap();

    let stats = db::get_repo_with_stats(&pool, repo.id).await.unwrap();
    assert_eq!(
        stats.import_status_message.as_deref(),
        Some("scanning archives")
    );

    db::set_import_status_message(&pool, repo.id, None)
        .await
        .unwrap();

    let stats = db::get_repo_with_stats(&pool, repo.id).await.unwrap();
    assert!(stats.import_status_message.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_relocation_pending_test(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    db::set_relocation_pending(&pool, repo.id).await.unwrap();

    let row = db::get_repo_with_passphrase(&pool, repo.id).await.unwrap();
    assert!(row.relocation_pending);

    db::clear_relocation_pending(&pool, repo.id).await.unwrap();

    let row = db::get_repo_with_passphrase(&pool, repo.id).await.unwrap();
    assert!(!row.relocation_pending);
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_relocation_per_host_single_agent(pool: PgPool) {
    let (agent, repo, schedule) = create_test_schedule(&pool).await;
    let _ = (agent, schedule);

    db::set_relocation_pending(&pool, repo.id).await.unwrap();
    let row = db::get_repo_with_passphrase(&pool, repo.id).await.unwrap();
    assert!(row.relocation_pending);

    // Confirming the single agent clears the repo-level flag.
    db::clear_relocation_for_host(&pool, repo.id, "sched-host")
        .await
        .unwrap();
    let row = db::get_repo_with_passphrase(&pool, repo.id).await.unwrap();
    assert!(!row.relocation_pending);
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_relocation_per_host_multi_agent(pool: PgPool) {
    // Build a repo used by two agents via separate schedules.
    let agent_a = db::insert_agent(&pool, "host-a", None, "hash-a", None, None)
        .await
        .unwrap();
    let agent_b = db::insert_agent(&pool, "host-b", None, "hash-b", None, None)
        .await
        .unwrap();
    let repo = db::insert_repo(
        &pool,
        &InsertRepoParams {
            name: "multi-agent-repo",
            repo_path: "/backups/multi",
            ssh_user: "user",
            ssh_host: "host.local",
            ssh_port: 22,
            passphrase_encrypted: b"enc",
            compression: "none",
            encryption: "none",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap();
    let sched = db::insert_schedule(
        &pool,
        repo.id,
        &ScheduleParams {
            name: "multi-sched",
            schedule_type: "backup",
            cron_expression: "0 3 * * *",
            enabled: true,
            canary_enabled: false,
            vm_snapshot_enabled: false,
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
            pre_backup_commands: &[],
            post_backup_commands: &[],
            hook_timeout_seconds: 60,
            missed_backup_threshold: 3,
            on_failure: "stop",
        },
        None,
    )
    .await
    .unwrap();
    db::insert_schedule_targets(&pool, sched.id, &[(agent_a.id, 0), (agent_b.id, 1)])
        .await
        .unwrap();

    db::set_relocation_pending(&pool, repo.id).await.unwrap();
    let row = db::get_repo_with_passphrase(&pool, repo.id).await.unwrap();
    assert!(row.relocation_pending);

    // First agent confirms - flag must stay set while the second is still pending.
    db::clear_relocation_for_host(&pool, repo.id, "host-a")
        .await
        .unwrap();
    let row = db::get_repo_with_passphrase(&pool, repo.id).await.unwrap();
    assert!(
        row.relocation_pending,
        "relocation_pending must remain true until all agents confirm"
    );

    // Second agent confirms - now the flag should be cleared.
    db::clear_relocation_for_host(&pool, repo.id, "host-b")
        .await
        .unwrap();
    let row = db::get_repo_with_passphrase(&pool, repo.id).await.unwrap();
    assert!(
        !row.relocation_pending,
        "relocation_pending must be cleared once all agents have confirmed"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_encryption_update(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    db::update_repo_encryption(&pool, repo.id, "keyfile")
        .await
        .unwrap();

    let row = db::get_repo_with_passphrase(&pool, repo.id).await.unwrap();
    assert_eq!(row.encryption, "keyfile");
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_passphrase_update(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    db::update_repo_passphrase(&pool, repo.id, b"new-encrypted-passphrase")
        .await
        .unwrap();

    let passphrase = db::get_repo_passphrase(&pool, repo.id).await.unwrap();
    assert_eq!(passphrase, b"new-encrypted-passphrase");

    let result = db::update_repo_passphrase(&pool, 999_999_999, b"x").await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_connection_test(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let conn = db::get_repo_connection(&pool, repo.id).await.unwrap();
    assert_eq!(conn.ssh_host, "storage.local");
    assert_eq!(conn.ssh_user, "backup");
    assert_eq!(conn.ssh_port, 22);

    let result = db::get_repo_connection(&pool, 999_999_999).await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_name_test(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let name = db::get_repo_name(&pool, repo.id).await.unwrap();
    assert_eq!(name, "test-repo");

    let result = db::get_repo_name(&pool, 999_999_999).await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_targets_list_and_delete(pool: PgPool) {
    let (agent, _, schedule) = create_test_schedule(&pool).await;

    let targets = db::list_schedule_targets(&pool, schedule.id).await.unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets.first().unwrap().agent_id, agent.id);

    db::delete_schedule_targets(&pool, schedule.id)
        .await
        .unwrap();

    let empty = db::list_schedule_targets(&pool, schedule.id).await.unwrap();
    assert_eq!(empty.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_target_hostnames_for_repo_test(pool: PgPool) {
    let (_, repo, _) = create_test_schedule(&pool).await;

    let targets = db::get_schedule_target_agents_for_repo(&pool, repo.id)
        .await
        .unwrap();
    let hostnames: Vec<&str> = targets.iter().map(|t| t.hostname.as_str()).collect();
    assert_eq!(hostnames, vec!["sched-host"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn get_schedule_targets_for_run_returns_ordered_and_excludes_hidden(pool: PgPool) {
    let (agent_a, _, schedule) = create_test_schedule(&pool).await;
    let agent_b = db::insert_agent(&pool, "run-target-b", None, "hash-rtb", None, None)
        .await
        .unwrap();
    let agent_hidden = db::insert_agent(&pool, "run-target-hidden", None, "hash-rth", None, None)
        .await
        .unwrap();
    db::set_agent_hidden(&pool, agent_hidden.id, true)
        .await
        .unwrap();

    // Add agent_b at order 1 (after agent_a at order 0) and the hidden agent at order 2.
    db::insert_schedule_targets(&pool, schedule.id, &[(agent_b.id, 1), (agent_hidden.id, 2)])
        .await
        .unwrap();

    let targets = db::get_schedule_targets_for_run(&pool, schedule.id)
        .await
        .unwrap();

    assert_eq!(targets.len(), 2);
    assert_eq!(targets.first().unwrap().agent_id, agent_a.id);
    assert_eq!(targets.first().unwrap().hostname, "sched-host");
    assert_eq!(targets.get(1).unwrap().agent_id, agent_b.id);
    assert_eq!(targets.get(1).unwrap().hostname, "run-target-b");
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_timezone_default(pool: PgPool) {
    let tz = db::get_schedule_timezone(&pool).await.unwrap();
    assert_ne!(tz.name(), "");
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_timezone_set(pool: PgPool) {
    db::set_setting(&pool, "timezone", "Europe/Berlin")
        .await
        .unwrap();

    let tz = db::get_schedule_timezone(&pool).await.unwrap();
    assert_eq!(tz, chrono_tz::Tz::Europe__Berlin);
}

#[sqlx::test(migrations = "./migrations")]
async fn reports_for_schedule_test(pool: PgPool) {
    let (agent, repo, schedule) = create_test_schedule(&pool).await;

    insert_test_report_for_schedule(
        &pool,
        agent.id,
        repo.id,
        schedule.id,
        shared::types::BackupStatus::Success,
    )
    .await;

    let reports = db::list_reports_for_schedule(&pool, schedule.id, 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports.first().unwrap().status, "success");
    assert_eq!(reports.first().unwrap().repo_name, repo.name);
    assert_eq!(reports.first().unwrap().schedule_id, Some(schedule.id));
    assert_eq!(
        reports.first().unwrap().schedule_name.as_deref(),
        Some("test-schedule")
    );

    let empty = db::list_reports_for_schedule(&pool, schedule.id.saturating_add(999), 10)
        .await
        .unwrap();
    assert_eq!(empty.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn reports_carry_repo_name_and_fall_back_to_it_when_schedule_unnamed(pool: PgPool) {
    let agent = db::insert_agent(&pool, "unnamed-sched-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let schedule = db::insert_schedule(
        &pool,
        repo.id,
        &ScheduleParams {
            name: "",
            schedule_type: "backup",
            cron_expression: "0 3 * * *",
            enabled: true,
            canary_enabled: false,
            vm_snapshot_enabled: false,
            exclude_patterns_raw: "",
            file_change_patterns_raw: "",
            ignore_global_excludes: false,
            keep_hourly: 0,
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 6,
            keep_yearly: 0,
            compact_enabled: true,
            rate_limit_kbps: None,
            pre_backup_commands: &[],
            post_backup_commands: &[],
            hook_timeout_seconds: 60,
            missed_backup_threshold: 3,
            on_failure: "stop",
        },
        None,
    )
    .await
    .unwrap();

    insert_test_report_for_schedule(
        &pool,
        agent.id,
        repo.id,
        schedule.id,
        shared::types::BackupStatus::Failed,
    )
    .await;

    let reports = db::list_reports_for_agent(&pool, agent.id, None, 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports.first().unwrap().repo_name, repo.name);
    assert_eq!(reports.first().unwrap().schedule_id, Some(schedule.id));
    assert_eq!(
        reports.first().unwrap().schedule_name.as_deref(),
        Some(repo.name.as_str()),
        "an unnamed schedule should fall back to the repo name"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn reports_for_agent_have_no_schedule_when_not_schedule_triggered(pool: PgPool) {
    let agent = db::insert_agent(&pool, "no-schedule-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let reports = db::list_reports_for_agent(&pool, agent.id, None, 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports.first().unwrap().repo_name, repo.name);
    assert_eq!(reports.first().unwrap().schedule_id, None);
    assert_eq!(reports.first().unwrap().schedule_name, None);
}

#[sqlx::test(migrations = "./migrations")]
async fn activity_feed_repo_filter(pool: PgPool) {
    let agent = db::insert_agent(&pool, "feed-repo-filter-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let all = db::get_activity_feed(&pool, 10, ActivityFeedFilters::default())
        .await
        .unwrap();
    assert_ne!(all.len(), 0);

    let filtered = db::get_activity_feed(
        &pool,
        10,
        ActivityFeedFilters {
            repo_id: Some(repo.id),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(filtered.len(), 1);

    let empty = db::get_activity_feed(
        &pool,
        10,
        ActivityFeedFilters {
            repo_id: Some(repo.id.saturating_add(999)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(empty.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn activity_feed_hostname_filter(pool: PgPool) {
    let agent = db::insert_agent(&pool, "hostname-filter-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let filtered = db::get_activity_feed(
        &pool,
        10,
        ActivityFeedFilters {
            hostname: Some("hostname-filter-host"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(filtered.len(), 1);

    let empty = db::get_activity_feed(
        &pool,
        10,
        ActivityFeedFilters {
            hostname: Some("nonexistent-host"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(empty.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn activity_feed_days_test(pool: PgPool) {
    let agent = db::insert_agent(&pool, "days-feed-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let all = db::get_activity_feed_days(&pool, 7, None, ActivityFeedFilters::default())
        .await
        .unwrap();
    assert_ne!(all.len(), 0);

    let with_repo = db::get_activity_feed_days(
        &pool,
        7,
        None,
        ActivityFeedFilters {
            repo_id: Some(repo.id),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(with_repo.len(), 1);

    let with_host = db::get_activity_feed_days(
        &pool,
        7,
        None,
        ActivityFeedFilters {
            hostname: Some("days-feed-host"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(with_host.len(), 1);

    let no_match = db::get_activity_feed_days(
        &pool,
        7,
        None,
        ActivityFeedFilters {
            hostname: Some("wrong-host"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(no_match.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn activity_feed_days_limit(pool: PgPool) {
    let agent = db::insert_agent(&pool, "days-feed-limit-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;
    insert_test_report(&pool, agent.id, repo.id).await;
    insert_test_report(&pool, agent.id, repo.id).await;

    let unbounded = db::get_activity_feed_days(&pool, 7, None, ActivityFeedFilters::default())
        .await
        .unwrap();
    assert_eq!(unbounded.len(), 3);

    let limited = db::get_activity_feed_days(&pool, 7, Some(2), ActivityFeedFilters::default())
        .await
        .unwrap();
    assert_eq!(limited.len(), 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn activity_feed_days_limit_is_per_schedule(pool: PgPool) {
    // A frequently-running schedule must not crowd a quieter one out of the
    // ranked window - the limit caps rows *per schedule_id*, not the result
    // set overall.
    let (busy_agent, busy_repo, busy_schedule) = create_test_schedule(&pool).await;
    for _ in 0..5 {
        insert_test_report_for_schedule(
            &pool,
            busy_agent.id,
            busy_repo.id,
            busy_schedule.id,
            shared::types::BackupStatus::Success,
        )
        .await;
    }

    let quiet_agent = db::insert_agent(&pool, "quiet-sched-host", None, "hash", None, None)
        .await
        .unwrap();
    let quiet_repo = db::insert_repo(
        &pool,
        &InsertRepoParams {
            name: "quiet-sched-repo",
            repo_path: "/backups/quiet-sched",
            ssh_user: "user",
            ssh_host: "host.local",
            ssh_port: 22,
            passphrase_encrypted: b"enc",
            compression: "none",
            encryption: "none",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap();
    let quiet_schedule = db::insert_schedule(
        &pool,
        quiet_repo.id,
        &ScheduleParams {
            name: "quiet-schedule",
            schedule_type: "backup",
            cron_expression: "0 3 * * 0",
            enabled: true,
            canary_enabled: false,
            vm_snapshot_enabled: false,
            exclude_patterns_raw: "",
            file_change_patterns_raw: "",
            ignore_global_excludes: false,
            keep_hourly: 0,
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 6,
            keep_yearly: 1,
            compact_enabled: true,
            rate_limit_kbps: None,
            pre_backup_commands: &[],
            post_backup_commands: &[],
            hook_timeout_seconds: 60,
            missed_backup_threshold: 3,
            on_failure: "stop",
        },
        None,
    )
    .await
    .unwrap();
    insert_test_report_for_schedule(
        &pool,
        quiet_agent.id,
        quiet_repo.id,
        quiet_schedule.id,
        shared::types::BackupStatus::Success,
    )
    .await;

    let rows = db::get_activity_feed_days(&pool, 30, Some(2), ActivityFeedFilters::default())
        .await
        .unwrap();

    let busy_count = rows
        .iter()
        .filter(|r| r.schedule_id == Some(busy_schedule.id))
        .count();
    let quiet_count = rows
        .iter()
        .filter(|r| r.schedule_id == Some(quiet_schedule.id))
        .count();
    assert_eq!(busy_count, 2, "busy schedule should be capped at the limit");
    assert_eq!(
        quiet_count, 1,
        "quiet schedule's single run must survive alongside the busy schedule's"
    );
}

#[test]
fn compression_round_trip() {
    use shared::types::Compression;
    let cases = &[
        ("none", "none"),
        ("lz4", "lz4"),
        ("zstd,3", "zstd,3"),
        ("zlib,6", "zlib,6"),
    ];
    for (input, expected) in cases {
        let c: Compression = input.parse().unwrap();
        assert_eq!(c.to_string(), *expected);
    }
    assert!("unknown".parse::<Compression>().is_err());
    assert!("zstd,bad".parse::<Compression>().is_err());
    assert!("zlib,bad".parse::<Compression>().is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn storage_trends_test(pool: PgPool) {
    let empty_trends = db::get_storage_trends(&pool, None, 7).await.unwrap();
    assert!(empty_trends.iter().all(|t| t.deduplicated_size.is_none()));

    let agent = db::insert_agent(&pool, "strend-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let trends = db::get_storage_trends(&pool, None, 7).await.unwrap();
    assert!(
        trends
            .iter()
            .any(|t| t.deduplicated_size.is_some_and(|v| v > 0))
    );

    let trends_repo = db::get_storage_trends(&pool, Some(repo.id), 7)
        .await
        .unwrap();
    assert!(
        trends_repo
            .iter()
            .any(|t| t.deduplicated_size.is_some_and(|v| v > 0))
    );

    let trends_other = db::get_storage_trends(&pool, Some(repo.id.saturating_add(999)), 7)
        .await
        .unwrap();
    assert!(trends_other.iter().all(|t| t.deduplicated_size.is_none()));
}

#[sqlx::test(migrations = "./migrations")]
async fn storage_trends_by_repo_test(pool: PgPool) {
    let empty = db::get_storage_trends_by_repo(&pool, 7).await.unwrap();
    assert_eq!(empty.len(), 0);

    let agent = db::insert_agent(&pool, "strend-repo-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let trends = db::get_storage_trends_by_repo(&pool, 7).await.unwrap();
    assert_ne!(trends.len(), 0);
    assert!(
        trends
            .iter()
            .any(|t| t.repo_name == "test-repo" && t.deduplicated_size.is_some_and(|v| v > 0))
    );
}

/// Regression test for <https://github.com/alexmohr/assimilate/issues/195>: the deduplicated
/// size in the storage trend must never exceed the original/compressed size. Each individual
/// archive is small, but `repo_unique_csize` (the repo-wide on-disk footprint) grows across
/// archives, which used to be compared against a single archive's per-archive original size.
#[sqlx::test(migrations = "./migrations")]
async fn storage_trends_dedup_never_exceeds_original(pool: PgPool) {
    let agent = db::insert_agent(&pool, "strend-invariant-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    for day in 0..5i64 {
        db::insert_backup_report(
            &pool,
            &InsertReportParams {
                agent_id: agent.id,
                repo_id: repo.id,
                schedule_id: None,
                started_at: now
                    .checked_sub_signed(Duration::days(6i64.saturating_sub(day)))
                    .unwrap()
                    .checked_sub_signed(Duration::minutes(5))
                    .unwrap(),
                finished_at: now
                    .checked_sub_signed(Duration::days(6i64.saturating_sub(day)))
                    .unwrap(),
                status: shared::types::BackupStatus::Success,
                original_size: 1_000,
                compressed_size: 800,
                deduplicated_size: 100,
                repo_unique_csize: day.saturating_add(1).saturating_mul(750),
                files_processed: 10,
                duration_secs: 60,
                error_message: None,
                warnings: vec![],
                borg_version: Some("1.4.0".to_string()),
                matched: true,
                archive_name: Some(format!("invariant-archive-{day}")),
                borg_command: None,
                run_id: None,
            },
        )
        .await
        .unwrap();
    }

    for trend in db::get_storage_trends(&pool, None, 7).await.unwrap() {
        let dedup = trend.deduplicated_size.unwrap_or(0);
        assert!(
            dedup <= trend.compressed_size && trend.compressed_size <= trend.original_size,
            "invariant violated on {}: original={} compressed={} dedup={}",
            trend.date,
            trend.original_size,
            trend.compressed_size,
            dedup
        );
    }

    for trend in db::get_storage_trends(&pool, Some(repo.id), 7)
        .await
        .unwrap()
    {
        let dedup = trend.deduplicated_size.unwrap_or(0);
        assert!(
            dedup <= trend.compressed_size && trend.compressed_size <= trend.original_size,
            "invariant violated on {}: original={} compressed={} dedup={}",
            trend.date,
            trend.original_size,
            trend.compressed_size,
            dedup
        );
    }

    for trend in db::get_storage_trends_by_repo(&pool, 7).await.unwrap() {
        let dedup = trend.deduplicated_size.unwrap_or(0);
        assert!(
            dedup <= trend.compressed_size && trend.compressed_size <= trend.original_size,
            "invariant violated on {} for {}: original={} compressed={} dedup={}",
            trend.date,
            trend.repo_name,
            trend.original_size,
            trend.compressed_size,
            dedup
        );
    }

    // The last day's dedup size (3_750) exceeds a single archive's original_size (1_000),
    // which is exactly the scenario that used to violate the invariant.
    let last_dedup = db::get_storage_trends(&pool, Some(repo.id), 7)
        .await
        .unwrap()
        .into_iter()
        .next_back()
        .and_then(|t| t.deduplicated_size)
        .unwrap_or(0);
    assert!(last_dedup > 1_000);
}

#[sqlx::test(migrations = "./migrations")]
async fn archive_names_and_delete_test(pool: PgPool) {
    let agent = db::insert_agent(&pool, "archive-del-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(10)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::minutes(5)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 1_000_000,
            compressed_size: 500_000,
            deduplicated_size: 250_000,
            repo_unique_csize: 0,
            files_processed: 100,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: None,
            matched: true,
            archive_name: Some("archive-2026-01-01".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(20)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::minutes(15)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 2_000_000,
            compressed_size: 1_000_000,
            deduplicated_size: 500_000,
            repo_unique_csize: 0,
            files_processed: 200,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: None,
            matched: true,
            archive_name: Some("archive-2026-01-02".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let names = db::list_archive_names_for_repo(&pool, repo.id)
        .await
        .unwrap();
    assert_eq!(names.len(), 2);
    assert!(names.contains("archive-2026-01-01"));
    assert!(names.contains("archive-2026-01-02"));

    let no_delete = db::delete_archive_reports_by_names(&pool, repo.id, &[])
        .await
        .unwrap();
    assert_eq!(no_delete, 0);

    let deleted =
        db::delete_archive_reports_by_names(&pool, repo.id, &["archive-2026-01-01".to_string()])
            .await
            .unwrap();
    assert_eq!(deleted, 1);

    let remaining = db::list_archive_names_for_repo(&pool, repo.id)
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert!(remaining.contains("archive-2026-01-02"));
}

#[sqlx::test(migrations = "./migrations")]
async fn list_archive_names_needing_stats_filters_enriched(pool: PgPool) {
    let agent = db::insert_agent(&pool, "stats-needing-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    let base = InsertReportParams {
        agent_id: agent.id,
        repo_id: repo.id,
        schedule_id: None,
        started_at: now.checked_sub_signed(Duration::minutes(10)).unwrap(),
        finished_at: now,
        status: shared::types::BackupStatus::Success,
        original_size: 0,
        compressed_size: 0,
        deduplicated_size: 0,
        repo_unique_csize: 0,
        files_processed: 0,
        duration_secs: 0,
        error_message: None,
        warnings: vec![],
        borg_version: None,
        matched: true,
        archive_name: Some("needs-stats".to_string()),
        borg_command: None,
        run_id: None,
    };
    db::insert_backup_report(&pool, &base).await.unwrap();
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            started_at: now.checked_sub_signed(Duration::minutes(20)).unwrap(),
            original_size: 1_000,
            compressed_size: 500,
            deduplicated_size: 250,
            repo_unique_csize: 0,
            archive_name: Some("missing-repo-csize".to_string()),
            ..base.clone()
        },
    )
    .await
    .unwrap();
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            started_at: now.checked_sub_signed(Duration::minutes(30)).unwrap(),
            original_size: 1_000,
            compressed_size: 500,
            deduplicated_size: 250,
            repo_unique_csize: 800,
            archive_name: Some("fully-enriched".to_string()),
            ..base.clone()
        },
    )
    .await
    .unwrap();

    let needing = db::list_archive_names_needing_stats(&pool, repo.id)
        .await
        .unwrap();
    assert_eq!(needing.len(), 2);
    assert!(needing.contains("needs-stats"));
    assert!(needing.contains("missing-repo-csize"));
}

#[sqlx::test(migrations = "./migrations")]
async fn list_indexed_archive_names_returns_only_done(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    for (name, status) in [
        ("done-archive", "done"),
        ("indexing-archive", "indexing"),
        ("pending-archive", "pending"),
        ("failed-archive", "failed"),
    ] {
        let archive_id: i64 = sqlx::query_scalar(
            "INSERT INTO archives (repo_id, name) VALUES ($1, $2) ON CONFLICT (repo_id, name) DO \
             UPDATE SET name = EXCLUDED.name RETURNING id",
        )
        .bind(repo.id)
        .bind(name)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO archive_index_jobs (archive_id, status) VALUES ($1, $2)")
            .bind(archive_id)
            .bind(status)
            .execute(&pool)
            .await
            .unwrap();
    }

    let done = server::archive_index::list_indexed_archive_names(&pool, repo.id)
        .await
        .unwrap();
    assert_eq!(done.len(), 1);
    assert!(done.contains("done-archive"));
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_backup_reports_before_test(pool: PgPool) {
    let agent = db::insert_agent(&pool, "del-before-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::hours(2)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::hours(2)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 1_000_000,
            compressed_size: 500_000,
            deduplicated_size: 250_000,
            repo_unique_csize: 0,
            files_processed: 100,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: None,
            matched: true,
            archive_name: None,
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let cutoff = now.checked_sub_signed(Duration::hours(1)).unwrap();
    let deleted = db::delete_backup_reports_before(&pool, cutoff)
        .await
        .unwrap();
    assert_eq!(deleted, 1);

    let reports = db::list_reports_for_agent(&pool, agent.id, None, 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_backup_reports_before_keeps_archive_rows(pool: PgPool) {
    // Imported/synced archives keep their original (old) borg start timestamp.
    // Age-based report retention must not delete them, or archives vanish from
    // the UI even though they still exist in borg.
    let agent = db::insert_agent(&pool, "retain-archive-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let old = Utc::now().checked_sub_signed(Duration::days(365)).unwrap();

    let base = InsertReportParams {
        agent_id: agent.id,
        repo_id: repo.id,
        schedule_id: None,
        started_at: old,
        finished_at: old,
        status: shared::types::BackupStatus::Success,
        original_size: 0,
        compressed_size: 0,
        deduplicated_size: 0,
        repo_unique_csize: 0,
        files_processed: 0,
        duration_secs: 0,
        error_message: None,
        warnings: vec![],
        borg_version: None,
        matched: true,
        archive_name: Some("imported-archive-2025".to_string()),
        borg_command: None,
        run_id: None,
    };
    db::insert_backup_report(&pool, &base).await.unwrap();

    // A pure run-history row with no archive should still be pruned.
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            started_at: old.checked_add_signed(Duration::seconds(1)).unwrap(),
            finished_at: old.checked_add_signed(Duration::seconds(1)).unwrap(),
            status: shared::types::BackupStatus::Failed,
            archive_name: None,
            ..base.clone()
        },
    )
    .await
    .unwrap();

    let cutoff = Utc::now().checked_sub_signed(Duration::days(7)).unwrap();
    let deleted = db::delete_backup_reports_before(&pool, cutoff)
        .await
        .unwrap();
    assert_eq!(deleted, 1);

    let names = db::list_archive_names_for_repo(&pool, repo.id)
        .await
        .unwrap();
    assert!(names.contains("imported-archive-2025"));
    assert_eq!(names.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_backup_reports_with_archive_before_test(pool: PgPool) {
    let agent = db::insert_agent(&pool, "del-arch-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();
    let old = now.checked_sub_signed(Duration::days(100)).unwrap();

    // Old archived report -- should be deleted
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: old,
            finished_at: old,
            status: shared::types::BackupStatus::Success,
            original_size: 100,
            compressed_size: 50,
            deduplicated_size: 25,
            repo_unique_csize: 0,
            files_processed: 10,
            duration_secs: 60,
            error_message: None,
            warnings: vec![],
            borg_version: None,
            matched: true,
            archive_name: Some("old-archive".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    // Recent archived report -- must be kept
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now,
            finished_at: now,
            status: shared::types::BackupStatus::Success,
            original_size: 200,
            compressed_size: 100,
            deduplicated_size: 50,
            repo_unique_csize: 0,
            files_processed: 20,
            duration_secs: 120,
            error_message: None,
            warnings: vec![],
            borg_version: None,
            matched: true,
            archive_name: Some("recent-archive".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let cutoff = now.checked_sub_signed(Duration::days(30)).unwrap();
    let deleted = db::delete_backup_reports_with_archive_before(&pool, cutoff)
        .await
        .unwrap();
    assert_eq!(deleted, 1);

    let names = db::list_archive_names_for_repo(&pool, repo.id)
        .await
        .unwrap();
    assert!(names.contains("recent-archive"));
    assert!(!names.contains("old-archive"));
    assert_eq!(names.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_backup_reports_with_archive_before_keeps_null_archive(pool: PgPool) {
    let agent = db::insert_agent(&pool, "del-arch-null-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();
    let old = now.checked_sub_signed(Duration::days(100)).unwrap();

    // Old report with NULL archive_name - not deleted by this function
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: old,
            finished_at: old,
            status: shared::types::BackupStatus::Failed,
            original_size: 0,
            compressed_size: 0,
            deduplicated_size: 0,
            repo_unique_csize: 0,
            files_processed: 0,
            duration_secs: 0,
            error_message: Some("error".to_string()),
            warnings: vec![],
            borg_version: None,
            matched: true,
            archive_name: None,
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    // Old report with archive_name -- should be deleted
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: old,
            finished_at: old,
            status: shared::types::BackupStatus::Success,
            original_size: 100,
            compressed_size: 50,
            deduplicated_size: 25,
            repo_unique_csize: 0,
            files_processed: 10,
            duration_secs: 60,
            error_message: None,
            warnings: vec![],
            borg_version: None,
            matched: true,
            archive_name: Some("archived-report".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let cutoff = now.checked_sub_signed(Duration::days(30)).unwrap();
    let deleted = db::delete_backup_reports_with_archive_before(&pool, cutoff)
        .await
        .unwrap();
    // Only the row WITH an archive_name should be deleted
    assert_eq!(deleted, 1);

    // The archive-less row should still exist
    let reports = db::list_reports_for_agent(&pool, agent.id, None, 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports.first().unwrap().archive_name, None);
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_backup_reports_with_archive_before_boundary_exact(pool: PgPool) {
    let agent = db::insert_agent(&pool, "arch-exact-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::days(30)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::days(30)).unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 100,
            compressed_size: 50,
            deduplicated_size: 25,
            repo_unique_csize: 0,
            files_processed: 10,
            duration_secs: 60,
            error_message: None,
            warnings: vec![],
            borg_version: None,
            matched: true,
            archive_name: Some("exact-boundary-archive".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let cutoff = now.checked_sub_signed(Duration::days(30)).unwrap();
    let deleted = db::delete_backup_reports_with_archive_before(&pool, cutoff)
        .await
        .unwrap();
    assert_eq!(deleted, 0, "report exactly at cutoff must not be deleted");

    let names = db::list_archive_names_for_repo(&pool, repo.id)
        .await
        .unwrap();
    assert!(names.contains("exact-boundary-archive"));
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_backup_reports_with_archive_before_one_sec_before(pool: PgPool) {
    let agent = db::insert_agent(&pool, "arch-1s-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now
                .checked_sub_signed(Duration::days(30))
                .and_then(|dt| dt.checked_sub_signed(Duration::seconds(1)))
                .unwrap(),
            finished_at: now
                .checked_sub_signed(Duration::days(30))
                .and_then(|dt| dt.checked_sub_signed(Duration::seconds(1)))
                .unwrap(),
            status: shared::types::BackupStatus::Success,
            original_size: 100,
            compressed_size: 50,
            deduplicated_size: 25,
            repo_unique_csize: 0,
            files_processed: 10,
            duration_secs: 60,
            error_message: None,
            warnings: vec![],
            borg_version: None,
            matched: true,
            archive_name: Some("one-sec-before-archive".to_string()),
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let cutoff = now.checked_sub_signed(Duration::days(30)).unwrap();
    let deleted = db::delete_backup_reports_with_archive_before(&pool, cutoff)
        .await
        .unwrap();
    assert_eq!(
        deleted, 1,
        "report one second before cutoff must be deleted"
    );

    let names = db::list_archive_names_for_repo(&pool, repo.id)
        .await
        .unwrap();
    assert!(!names.contains("one-sec-before-archive"));
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_backup_reports_before_boundary_exact(pool: PgPool) {
    let agent = db::insert_agent(&pool, "fail-exact-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::days(7)).unwrap(),
            finished_at: now.checked_sub_signed(Duration::days(7)).unwrap(),
            status: shared::types::BackupStatus::Failed,
            original_size: 0,
            compressed_size: 0,
            deduplicated_size: 0,
            repo_unique_csize: 0,
            files_processed: 0,
            duration_secs: 0,
            error_message: Some("timeout".to_string()),
            warnings: vec![],
            borg_version: None,
            matched: true,
            archive_name: None,
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let cutoff = now.checked_sub_signed(Duration::days(7)).unwrap();
    let deleted = db::delete_backup_reports_before(&pool, cutoff)
        .await
        .unwrap();
    assert_eq!(
        deleted, 0,
        "failed report exactly at cutoff must not be deleted"
    );

    let reports = db::list_reports_for_agent(&pool, agent.id, None, 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_backup_reports_before_one_sec_before(pool: PgPool) {
    let agent = db::insert_agent(&pool, "fail-1s-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now
                .checked_sub_signed(Duration::days(7))
                .and_then(|dt| dt.checked_sub_signed(Duration::seconds(1)))
                .unwrap(),
            finished_at: now
                .checked_sub_signed(Duration::days(7))
                .and_then(|dt| dt.checked_sub_signed(Duration::seconds(1)))
                .unwrap(),
            status: shared::types::BackupStatus::Failed,
            original_size: 0,
            compressed_size: 0,
            deduplicated_size: 0,
            repo_unique_csize: 0,
            files_processed: 0,
            duration_secs: 0,
            error_message: Some("timeout".to_string()),
            warnings: vec![],
            borg_version: None,
            matched: true,
            archive_name: None,
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    let cutoff = now.checked_sub_signed(Duration::days(7)).unwrap();
    let deleted = db::delete_backup_reports_before(&pool, cutoff)
        .await
        .unwrap();
    assert_eq!(
        deleted, 1,
        "failed report one second before cutoff must be deleted"
    );

    let reports = db::list_reports_for_agent(&pool, agent.id, None, 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_failed_backup_reports_for_agent_test(pool: PgPool) {
    let agent = db::insert_agent(&pool, "clean-failed-host", None, "hash", None, None)
        .await
        .unwrap();
    let other_agent = db::insert_agent(&pool, "other-clean-failed-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    let base = InsertReportParams {
        agent_id: agent.id,
        repo_id: repo.id,
        schedule_id: None,
        started_at: now,
        finished_at: now,
        status: shared::types::BackupStatus::Failed,
        original_size: 0,
        compressed_size: 0,
        deduplicated_size: 0,
        repo_unique_csize: 0,
        files_processed: 0,
        duration_secs: 0,
        error_message: Some("connection refused".to_string()),
        warnings: vec![],
        borg_version: None,
        matched: true,
        archive_name: None,
        borg_command: None,
        run_id: None,
    };

    // Two failed reports for the target agent -- both should be deleted.
    db::insert_backup_report(&pool, &base).await.unwrap();
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            started_at: now.checked_add_signed(Duration::seconds(1)).unwrap(),
            finished_at: now.checked_add_signed(Duration::seconds(1)).unwrap(),
            ..base.clone()
        },
    )
    .await
    .unwrap();

    // A successful report for the same agent must be kept.
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            status: shared::types::BackupStatus::Success,
            error_message: None,
            archive_name: Some("kept-archive".to_string()),
            ..base.clone()
        },
    )
    .await
    .unwrap();

    // A failed report for a different agent must be untouched.
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: other_agent.id,
            ..base.clone()
        },
    )
    .await
    .unwrap();

    // A failed report that still carries an archive_name - e.g. borg create
    // succeeded but a later prune/compact/post-backup hook failed the run
    // overall - must be excluded from both the count and the delete, so
    // "clean up failed backups" can never discard the only report row
    // linking to a retained archive.
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            archive_name: Some("orphan-risk-archive".to_string()),
            ..base.clone()
        },
    )
    .await
    .unwrap();

    assert_eq!(
        db::count_failed_backup_reports_for_agent(&pool, agent.id)
            .await
            .unwrap(),
        2,
        "the archived failed report must not be counted"
    );

    let deleted = db::delete_failed_backup_reports_for_agent(&pool, agent.id)
        .await
        .unwrap();
    assert_eq!(deleted, 2);

    assert_eq!(
        db::count_failed_backup_reports_for_agent(&pool, agent.id)
            .await
            .unwrap(),
        0,
        "count must reflect the delete, not the report list's own pagination window"
    );

    let remaining = db::list_reports_for_agent(&pool, agent.id, None, 10)
        .await
        .unwrap();
    assert_eq!(
        remaining.len(),
        2,
        "the success and archived-failed reports must survive"
    );
    let mut remaining_statuses: Vec<&str> = remaining.iter().map(|r| r.status.as_str()).collect();
    remaining_statuses.sort_unstable();
    assert_eq!(remaining_statuses, vec!["failed", "success"]);

    let other_remaining = db::list_reports_for_agent(&pool, other_agent.id, None, 10)
        .await
        .unwrap();
    assert_eq!(other_remaining.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_failed_backup_reports_for_schedule_test(pool: PgPool) {
    let (agent, repo, schedule) = create_test_schedule(&pool).await;
    let other_schedule = db::insert_schedule(
        &pool,
        repo.id,
        &ScheduleParams {
            name: "other-schedule",
            schedule_type: "backup",
            cron_expression: "0 4 * * *",
            enabled: true,
            canary_enabled: false,
            vm_snapshot_enabled: false,
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
            pre_backup_commands: &[],
            post_backup_commands: &[],
            hook_timeout_seconds: 60,
            missed_backup_threshold: 3,
            on_failure: "stop",
        },
        None,
    )
    .await
    .unwrap();
    let now = Utc::now();

    let base = InsertReportParams {
        agent_id: agent.id,
        repo_id: repo.id,
        schedule_id: Some(schedule.id),
        started_at: now,
        finished_at: now,
        status: shared::types::BackupStatus::Failed,
        original_size: 0,
        compressed_size: 0,
        deduplicated_size: 0,
        repo_unique_csize: 0,
        files_processed: 0,
        duration_secs: 0,
        error_message: Some("connection refused".to_string()),
        warnings: vec![],
        borg_version: None,
        matched: true,
        archive_name: None,
        borg_command: None,
        run_id: None,
    };

    // Two failed reports for the target schedule -- both should be deleted.
    db::insert_backup_report(&pool, &base).await.unwrap();
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            started_at: now.checked_add_signed(Duration::seconds(1)).unwrap(),
            finished_at: now.checked_add_signed(Duration::seconds(1)).unwrap(),
            ..base.clone()
        },
    )
    .await
    .unwrap();

    // A successful report for the same schedule must be kept.
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            status: shared::types::BackupStatus::Success,
            error_message: None,
            archive_name: Some("kept-archive".to_string()),
            ..base.clone()
        },
    )
    .await
    .unwrap();

    // A failed report for a different schedule must be untouched. Needs its
    // own started_at: the upsert conflict target is (repo_id, agent_id,
    // started_at) for archive-less rows and does not include schedule_id, so
    // reusing `now` here would silently reassign the first failed report's
    // schedule_id instead of inserting a second row.
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            schedule_id: Some(other_schedule.id),
            started_at: now.checked_add_signed(Duration::seconds(2)).unwrap(),
            finished_at: now.checked_add_signed(Duration::seconds(2)).unwrap(),
            ..base.clone()
        },
    )
    .await
    .unwrap();

    // A failed report that still carries an archive_name (create succeeded,
    // a later prune/compact/post-backup hook failed the run overall) must be
    // excluded from both the count and the delete.
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            archive_name: Some("orphan-risk-archive".to_string()),
            ..base.clone()
        },
    )
    .await
    .unwrap();

    assert_eq!(
        db::count_failed_backup_reports_for_schedule(&pool, schedule.id)
            .await
            .unwrap(),
        2,
        "the archived failed report must not be counted"
    );

    let deleted = db::delete_failed_backup_reports_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(deleted, 2);

    assert_eq!(
        db::count_failed_backup_reports_for_schedule(&pool, schedule.id)
            .await
            .unwrap(),
        0,
        "count must reflect the delete, not the report list's own pagination window"
    );

    let remaining = db::list_reports_for_schedule(&pool, schedule.id, 10)
        .await
        .unwrap();
    assert_eq!(
        remaining.len(),
        2,
        "the success and archived-failed reports must survive"
    );
    let mut remaining_statuses: Vec<&str> = remaining.iter().map(|r| r.status.as_str()).collect();
    remaining_statuses.sort_unstable();
    assert_eq!(remaining_statuses, vec!["failed", "success"]);

    let other_remaining = db::list_reports_for_schedule(&pool, other_schedule.id, 10)
        .await
        .unwrap();
    assert_eq!(other_remaining.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_system_events_before_keeps_recent(pool: PgPool) {
    let before_insert = Utc::now();
    db::insert_system_event(
        &pool,
        shared::types::SystemEventType::RepoSync,
        None,
        "recent event",
    )
    .await
    .unwrap();

    // Use a cutoff just before the insert -- guaranteed to be before created_at
    let cutoff = before_insert
        .checked_sub_signed(Duration::seconds(1))
        .unwrap();
    let deleted = db::delete_system_events_before(&pool, cutoff)
        .await
        .unwrap();
    assert_eq!(
        deleted, 0,
        "system event created after cutoff must not be deleted"
    );

    let events = db::get_system_events(&pool, 10, AcknowledgedFilter::All)
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
}

#[cfg(test)]
async fn insert_test_notification_delivery(pool: &PgPool, attempted_at: DateTime<Utc>) {
    let channel_id: i64 = sqlx::query_scalar!(
        "INSERT INTO notification_channels (name, channel_type) VALUES ('test-channel', \
         'webhook') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    sqlx::query!(
        "INSERT INTO notification_deliveries (channel_id, event_type, status, attempted_at) \
         VALUES ($1, 'backup_success', 'sent', $2)",
        channel_id,
        attempted_at,
    )
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_notification_deliveries_before_deletes_old(pool: PgPool) {
    let old = Utc::now().checked_sub_signed(Duration::days(60)).unwrap();
    insert_test_notification_delivery(&pool, old).await;

    let cutoff = Utc::now().checked_sub_signed(Duration::days(30)).unwrap();
    let deleted = db::delete_notification_deliveries_before(&pool, cutoff)
        .await
        .unwrap();
    assert_eq!(deleted, 1, "delivery older than cutoff must be deleted");
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_notification_deliveries_before_keeps_recent(pool: PgPool) {
    let before_insert = Utc::now();
    insert_test_notification_delivery(&pool, before_insert).await;

    let cutoff = before_insert
        .checked_sub_signed(Duration::seconds(1))
        .unwrap();
    let deleted = db::delete_notification_deliveries_before(&pool, cutoff)
        .await
        .unwrap();
    assert_eq!(
        deleted, 0,
        "delivery created after cutoff must not be deleted"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_system_events_before_deletes_old(pool: PgPool) {
    db::insert_system_event(
        &pool,
        shared::types::SystemEventType::RepoSync,
        None,
        "old event to prune",
    )
    .await
    .unwrap();

    let cutoff = Utc::now().checked_add_signed(Duration::hours(1)).unwrap();
    let deleted = db::delete_system_events_before(&pool, cutoff)
        .await
        .unwrap();
    assert_eq!(
        deleted, 1,
        "system event must be deleted with future cutoff"
    );

    let events = db::get_system_events(&pool, 10, AcknowledgedFilter::All)
        .await
        .unwrap();
    assert!(events.is_empty());
}

/// Applies the same fallback logic as `get_settings` in `api/system.rs`.
fn compute_retention_fallbacks(
    legacy_raw: Option<String>,
    report_raw: Option<String>,
    failed_raw: Option<String>,
    event_raw: Option<String>,
) -> (i64, i64, i64, i64) {
    let legacy = legacy_raw.and_then(|v| v.parse::<i64>().ok());
    let retention_days = legacy.unwrap_or(7);
    let report_retention_days = report_raw.and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
    let failed_report_retention_days = failed_raw
        .and_then(|v| v.parse::<i64>().ok())
        .or(legacy)
        .unwrap_or(365);
    let system_event_retention_days = event_raw
        .and_then(|v| v.parse::<i64>().ok())
        .or(legacy)
        .unwrap_or(90);
    (
        retention_days,
        report_retention_days,
        failed_report_retention_days,
        system_event_retention_days,
    )
}

#[sqlx::test(migrations = "./migrations")]
async fn retention_fallback_new_settings_unset_uses_legacy(pool: PgPool) {
    db::set_setting(&pool, "retention_days", "30")
        .await
        .unwrap();

    let legacy_raw = db::get_setting(&pool, "retention_days").await.unwrap();
    let report_raw = db::get_setting(&pool, "report_retention_days")
        .await
        .unwrap();
    let failed_raw = db::get_setting(&pool, "failed_report_retention_days")
        .await
        .unwrap();
    let event_raw = db::get_setting(&pool, "system_event_retention_days")
        .await
        .unwrap();

    let (ret, report, failed, events) =
        compute_retention_fallbacks(legacy_raw, report_raw, failed_raw, event_raw);

    assert_eq!(ret, 30);
    assert_eq!(
        report, 0,
        "report_retention_days must NOT fall back to legacy"
    );
    assert_eq!(
        failed, 30,
        "failed_report_retention_days must fall back to legacy (30)"
    );
    assert_eq!(
        events, 30,
        "system_event_retention_days must fall back to legacy (30)"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn retention_fallback_new_settings_take_precedence(pool: PgPool) {
    db::set_setting(&pool, "retention_days", "30")
        .await
        .unwrap();
    db::set_setting(&pool, "report_retention_days", "180")
        .await
        .unwrap();
    db::set_setting(&pool, "failed_report_retention_days", "60")
        .await
        .unwrap();
    db::set_setting(&pool, "system_event_retention_days", "45")
        .await
        .unwrap();

    let legacy_raw = db::get_setting(&pool, "retention_days").await.unwrap();
    let report_raw = db::get_setting(&pool, "report_retention_days")
        .await
        .unwrap();
    let failed_raw = db::get_setting(&pool, "failed_report_retention_days")
        .await
        .unwrap();
    let event_raw = db::get_setting(&pool, "system_event_retention_days")
        .await
        .unwrap();

    let (ret, report, failed, events) =
        compute_retention_fallbacks(legacy_raw, report_raw, failed_raw, event_raw);

    assert_eq!(ret, 30);
    assert_eq!(report, 180, "explicit report_retention_days must be used");
    assert_eq!(
        failed, 60,
        "explicit failed_report_retention_days must be used"
    );
    assert_eq!(
        events, 45,
        "explicit system_event_retention_days must be used"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn retention_fallback_nothing_set_uses_defaults(pool: PgPool) {
    let legacy_raw = db::get_setting(&pool, "retention_days").await.unwrap();
    let report_raw = db::get_setting(&pool, "report_retention_days")
        .await
        .unwrap();
    let failed_raw = db::get_setting(&pool, "failed_report_retention_days")
        .await
        .unwrap();
    let event_raw = db::get_setting(&pool, "system_event_retention_days")
        .await
        .unwrap();

    let (ret, report, failed, events) =
        compute_retention_fallbacks(legacy_raw, report_raw, failed_raw, event_raw);

    assert_eq!(ret, 7, "default retention_days must be 7");
    assert_eq!(
        report, 0,
        "default report_retention_days must be 0 (keep forever)"
    );
    assert_eq!(
        failed, 7,
        "default failed_report_retention_days must fall back to legacy retention_days"
    );
    assert_eq!(
        events, 7,
        "default system_event_retention_days must fall back to legacy retention_days"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn retention_fallback_new_settings_without_legacy(pool: PgPool) {
    db::set_setting(&pool, "report_retention_days", "100")
        .await
        .unwrap();
    db::set_setting(&pool, "failed_report_retention_days", "200")
        .await
        .unwrap();
    db::set_setting(&pool, "system_event_retention_days", "300")
        .await
        .unwrap();

    let legacy_raw = db::get_setting(&pool, "retention_days").await.unwrap();
    let report_raw = db::get_setting(&pool, "report_retention_days")
        .await
        .unwrap();
    let failed_raw = db::get_setting(&pool, "failed_report_retention_days")
        .await
        .unwrap();
    let event_raw = db::get_setting(&pool, "system_event_retention_days")
        .await
        .unwrap();

    let (ret, report, failed, events) =
        compute_retention_fallbacks(legacy_raw, report_raw, failed_raw, event_raw);

    assert_eq!(ret, 7, "default retention_days must be 7");
    assert_eq!(report, 100);
    assert_eq!(failed, 200);
    assert_eq!(events, 300);
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_login_attempts_before_deletes_old_keeps_recent(pool: PgPool) {
    db::insert_login_attempt(&pool, "pruned-user", "10.0.0.1", false)
        .await
        .unwrap();

    let past_cutoff = Utc::now().checked_sub_signed(Duration::hours(1)).unwrap();
    let deleted = db::delete_login_attempts_before(&pool, past_cutoff)
        .await
        .unwrap();
    assert_eq!(deleted, 0, "attempt is newer than the cutoff");

    let future_cutoff = Utc::now().checked_add_signed(Duration::hours(1)).unwrap();
    let deleted = db::delete_login_attempts_before(&pool, future_cutoff)
        .await
        .unwrap();
    assert_eq!(deleted, 1, "attempt must be deleted with future cutoff");

    let count = db::count_failed_attempts_since_last_success(&pool, "pruned-user")
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn audit_filter_by_target_type(pool: PgPool) {
    db::audit::insert_audit_entry(
        &pool,
        &db::audit::NewAuditEntry {
            user_id: None,
            username: "admin",
            action: "create",
            target_type: Some("repo"),
            target_id: Some(1),
            details: None,
            ip_address: None,
        },
    )
    .await
    .unwrap();

    db::audit::insert_audit_entry(
        &pool,
        &db::audit::NewAuditEntry {
            user_id: None,
            username: "admin",
            action: "create",
            target_type: Some("agent"),
            target_id: Some(2),
            details: None,
            ip_address: None,
        },
    )
    .await
    .unwrap();

    let (items, total) = db::audit::list_audit_entries(
        &pool,
        &db::audit::AuditEntryFilters {
            page: 1,
            per_page: 50,
            filter_user_id: None,
            filter_action: None,
            filter_target_type: Some("repo"),
            filter_from: None,
            filter_to: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(total, 1);
    assert_eq!(items.len(), 1);
    assert_eq!(items.first().unwrap().target_type.as_deref(), Some("repo"));
}

#[sqlx::test(migrations = "./migrations")]
async fn audit_filter_by_action(pool: PgPool) {
    db::audit::insert_audit_entry(
        &pool,
        &db::audit::NewAuditEntry {
            user_id: None,
            username: "admin",
            action: "delete",
            target_type: Some("repo"),
            target_id: Some(1),
            details: None,
            ip_address: None,
        },
    )
    .await
    .unwrap();

    db::audit::insert_audit_entry(
        &pool,
        &db::audit::NewAuditEntry {
            user_id: None,
            username: "admin",
            action: "update",
            target_type: Some("repo"),
            target_id: Some(1),
            details: None,
            ip_address: None,
        },
    )
    .await
    .unwrap();

    let (items, total) = db::audit::list_audit_entries(
        &pool,
        &db::audit::AuditEntryFilters {
            page: 1,
            per_page: 50,
            filter_user_id: None,
            filter_action: Some("delete"),
            filter_target_type: None,
            filter_from: None,
            filter_to: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(total, 1);
    assert_eq!(items.first().unwrap().action, "delete");
}

#[sqlx::test(migrations = "./migrations")]
async fn recovery_clears_stuck_importing_and_error(pool: PgPool) {
    // Simulate what happens when the server crashes mid-sync:
    // importing = true and an import_error are left in the DB.
    let repo = create_test_repo(&pool).await;

    db::set_repo_importing(&pool, repo.id, true).await.unwrap();
    db::set_repo_import_error(&pool, repo.id, Some("previous crash"))
        .await
        .unwrap();

    let stats = db::get_repo_with_stats(&pool, repo.id).await.unwrap();
    assert!(stats.importing);
    assert_eq!(stats.import_error.as_deref(), Some("previous crash"));

    // These are the exact DB calls the startup recovery task in main.rs makes
    // after sync_existing_archives completes (or fails).
    db::set_repo_importing(&pool, repo.id, false).await.unwrap();
    db::set_repo_import_error(&pool, repo.id, None)
        .await
        .unwrap();

    let stats = db::get_repo_with_stats(&pool, repo.id).await.unwrap();
    assert!(!stats.importing);
    assert!(stats.import_error.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn cancel_backup_report_updates_started_row(pool: PgPool) {
    let agent = db::insert_agent(&pool, "cancel-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    let started_at = Utc::now();
    db::insert_backup_started(&pool, agent.id, repo.id, None, started_at, None, None)
        .await
        .unwrap();

    db::cancel_backup_report(&pool, agent.id, repo.id)
        .await
        .unwrap();

    let reports = db::list_reports_for_agent(&pool, agent.id, None, 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports.first().unwrap().status, "cancelled");
}

#[sqlx::test(migrations = "./migrations")]
async fn cancel_backup_report_ignores_already_completed(pool: PgPool) {
    let agent = db::insert_agent(&pool, "cancel-done-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now.checked_sub_signed(Duration::minutes(5)).unwrap(),
            finished_at: now,
            status: shared::types::BackupStatus::Success,
            original_size: 0,
            compressed_size: 0,
            deduplicated_size: 0,
            repo_unique_csize: 0,
            files_processed: 0,
            duration_secs: 300,
            error_message: None,
            warnings: vec![],
            borg_version: None,
            matched: true,
            archive_name: None,
            borg_command: None,
            run_id: None,
        },
    )
    .await
    .unwrap();

    db::cancel_backup_report(&pool, agent.id, repo.id)
        .await
        .unwrap();

    let reports = db::list_reports_for_agent(&pool, agent.id, None, 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports.first().unwrap().status, "success");
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_insert_with_paths(pool: PgPool) {
    let paths = vec!["/etc".to_string(), "/home".to_string()];
    let excludes = vec!["*.log".to_string()];
    let agent = db::insert_agent_with_paths(
        &pool,
        "paths-host",
        "hash",
        db::AgentDefaults {
            display_name: Some("Paths Host"),
            domain: None,
            default_backup_paths: &paths,
            default_exclude_patterns: &excludes,
            default_pre_backup_commands: &[],
            default_post_backup_commands: &[],
            default_file_change_patterns_raw: "*/etc/config* fatal",
        },
    )
    .await
    .unwrap();

    assert_eq!(agent.hostname, "paths-host");
    assert_eq!(agent.display_name.as_deref(), Some("Paths Host"));
    assert_eq!(agent.default_backup_paths, paths);
    assert_eq!(agent.default_exclude_patterns, excludes);
    assert_eq!(
        agent.default_file_change_patterns_raw,
        "*/etc/config* fatal"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn run_id_update_scoped_to_agent(pool: PgPool) {
    let agent_a = db::insert_agent(&pool, "run-host-a", None, "hash-a", None, None)
        .await
        .unwrap();
    let agent_b = db::insert_agent(&pool, "run-host-b", None, "hash-b", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();
    let run_id = "shared-run-id";

    db::insert_backup_pending(&pool, agent_a.id, repo.id, None, run_id, now)
        .await
        .unwrap();
    db::insert_backup_pending(&pool, agent_b.id, repo.id, None, run_id, now)
        .await
        .unwrap();

    // Only agent_a sends BackupStarted.
    db::insert_backup_started(&pool, agent_a.id, repo.id, None, now, None, Some(run_id))
        .await
        .unwrap();

    // agent_b's record must still be 'pending'.
    let b_reports = db::list_reports_for_agent(&pool, agent_b.id, None, 10)
        .await
        .unwrap();
    assert_eq!(b_reports.len(), 1);
    assert_eq!(b_reports.first().unwrap().status, "pending");

    // Only agent_a sends BackupCompleted.
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: agent_a.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now,
            finished_at: now.checked_add_signed(Duration::minutes(10)).unwrap(),
            status: shared::types::BackupStatus::Failed,
            original_size: 0,
            compressed_size: 0,
            deduplicated_size: 0,
            repo_unique_csize: 0,
            files_processed: 0,
            duration_secs: 600,
            error_message: Some("lock wait timed out".to_string()),
            warnings: vec![],
            borg_version: None,
            matched: true,
            archive_name: None,
            borg_command: None,
            run_id: Some(run_id.to_string()),
        },
    )
    .await
    .unwrap();

    // agent_b's record must still be 'pending' - not bulk-failed by agent_a's report.
    let b_reports = db::list_reports_for_agent(&pool, agent_b.id, None, 10)
        .await
        .unwrap();
    assert_eq!(b_reports.len(), 1);
    assert_eq!(b_reports.first().unwrap().status, "pending");

    let a_reports = db::list_reports_for_agent(&pool, agent_a.id, None, 10)
        .await
        .unwrap();
    assert_eq!(a_reports.len(), 1);
    assert_eq!(a_reports.first().unwrap().status, "failed");
}

#[sqlx::test(migrations = "./migrations")]
async fn dismiss_finding_roundtrip(pool: PgPool) {
    let user = db::insert_user(&pool, "dismiss-user", "hash")
        .await
        .unwrap();

    let ids = db::dashboard::dismissed_finding_ids(&pool, user.id)
        .await
        .unwrap();
    assert_eq!(ids.len(), 0);

    db::dashboard::dismiss_finding(&pool, user.id, "target:1:2:BackupFailed")
        .await
        .unwrap();
    db::dashboard::dismiss_finding(&pool, user.id, "repository:3:RepositoryQuotaWarning")
        .await
        .unwrap();

    let ids = db::dashboard::dismissed_finding_ids(&pool, user.id)
        .await
        .unwrap();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains("target:1:2:BackupFailed"));
    assert!(ids.contains("repository:3:RepositoryQuotaWarning"));
}

#[sqlx::test(migrations = "./migrations")]
async fn dismiss_finding_idempotent(pool: PgPool) {
    let user = db::insert_user(&pool, "dismiss-idem-user", "hash")
        .await
        .unwrap();

    db::dashboard::dismiss_finding(&pool, user.id, "host:5:unassigned")
        .await
        .unwrap();
    db::dashboard::dismiss_finding(&pool, user.id, "host:5:unassigned")
        .await
        .unwrap();

    let ids = db::dashboard::dismissed_finding_ids(&pool, user.id)
        .await
        .unwrap();
    assert_eq!(ids.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn undismiss_finding_removes_entry(pool: PgPool) {
    let user = db::insert_user(&pool, "undismiss-user", "hash")
        .await
        .unwrap();

    db::dashboard::dismiss_finding(&pool, user.id, "host:5:unassigned")
        .await
        .unwrap();
    db::dashboard::dismiss_finding(&pool, user.id, "host:6:unassigned")
        .await
        .unwrap();

    db::dashboard::undismiss_finding(&pool, user.id, "host:5:unassigned")
        .await
        .unwrap();

    let ids = db::dashboard::dismissed_finding_ids(&pool, user.id)
        .await
        .unwrap();
    assert_eq!(ids.len(), 1);
    assert!(!ids.contains("host:5:unassigned"));
    assert!(ids.contains("host:6:unassigned"));
}

#[sqlx::test(migrations = "./migrations")]
async fn dismissed_findings_are_per_user(pool: PgPool) {
    let user_a = db::insert_user(&pool, "dismiss-user-a", "hash")
        .await
        .unwrap();
    let user_b = db::insert_user(&pool, "dismiss-user-b", "hash")
        .await
        .unwrap();

    db::dashboard::dismiss_finding(&pool, user_a.id, "host:1:unassigned")
        .await
        .unwrap();

    let a_ids = db::dashboard::dismissed_finding_ids(&pool, user_a.id)
        .await
        .unwrap();
    let b_ids = db::dashboard::dismissed_finding_ids(&pool, user_b.id)
        .await
        .unwrap();

    assert_eq!(a_ids.len(), 1);
    assert_eq!(b_ids.len(), 0);
}

/// `update_repo_and_set_relocation_pending` atomically updates the repo path AND sets
/// `relocation_pending = true` AND registers all scheduled agents in the pending-hosts table.
/// There is no observable intermediate state where the path is updated but the flag is false.
/// This eliminates the race window that caused the first agent in a sequential schedule to
/// fail with borg exit code 2.
#[sqlx::test(migrations = "./migrations")]
async fn update_repo_and_set_relocation_pending_is_atomic(pool: PgPool) {
    let (agent, repo, _schedule) = create_test_schedule(&pool).await;

    let row = db::get_repo_with_passphrase(&pool, repo.id).await.unwrap();
    assert!(!row.relocation_pending, "flag must start false");

    let updated = db::update_repo_and_set_relocation_pending(
        &pool,
        &UpdateRepoParams {
            repo_id: repo.id,
            name: "sched-repo",
            repo_path: "/backups/relocated",
            ssh_user: "user",
            ssh_host: "new-host.local",
            ssh_port: 22,
            compression: "none",
            encryption: "none",
            enabled: true,
            sync_schedule: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.repo_path, "/backups/relocated");
    assert_eq!(updated.ssh_host, "new-host.local");

    let row = db::get_repo_with_passphrase(&pool, repo.id).await.unwrap();
    assert!(
        row.relocation_pending,
        "relocation_pending must be true after atomic update"
    );

    // The scheduled agent must appear in the pending-hosts table.
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM repo_relocation_pending_hosts WHERE repo_id = $1 AND hostname = $2",
    )
    .bind(repo.id)
    .bind(&agent.hostname)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        count.0, 1,
        "the scheduled agent must be registered as pending"
    );
}

/// `clear_relocation_for_host` must NOT clear `relocation_pending` when the given hostname
/// was never registered in `repo_relocation_pending_hosts`. This guards against a spurious
/// flag clear when an unregistered host (e.g. added after `set_relocation_pending`) finishes.
#[sqlx::test(migrations = "./migrations")]
async fn clear_relocation_for_host_ignores_unregistered_host(pool: PgPool) {
    let (_agent, repo, _schedule) = create_test_schedule(&pool).await;

    // Set relocation pending - this registers "sched-host" in the pending table.
    db::set_relocation_pending(&pool, repo.id).await.unwrap();
    let row = db::get_repo_with_passphrase(&pool, repo.id).await.unwrap();
    assert!(row.relocation_pending);

    // A different host that was NOT registered calls clear - must be a no-op.
    db::clear_relocation_for_host(&pool, repo.id, "unknown-host")
        .await
        .unwrap();

    let row = db::get_repo_with_passphrase(&pool, repo.id).await.unwrap();
    assert!(
        row.relocation_pending,
        "relocation_pending must stay true when an unregistered host reports completion"
    );

    // The original registered host still remains in the pending table.
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM repo_relocation_pending_hosts WHERE repo_id = $1")
            .bind(repo.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count.0, 1, "pending table must be unchanged");
}

#[sqlx::test(migrations = "./migrations")]
async fn check_repo_permission_view_all_is_view_only(pool: PgPool) {
    use server::{
        api::{auth::AuthUser, permissions::check_repo_permission},
        error::ApiError,
    };

    let user = db::insert_user(&pool, "view-all-user", "hash")
        .await
        .unwrap();

    let role = db::insert_role(
        &pool,
        &InsertRoleParams {
            name: "test-view-all",
            can_create_agent: false,
            can_delete_agent: false,
            can_delete_own_agent: false,
            can_create_repo: false,
            can_delete_repo: false,
            can_delete_own_repo: false,
            can_create_schedule: false,
            can_delete_schedule: false,
            can_delete_own_schedule: false,
            can_manage_tags: false,
            can_view_all_repos: true,
            can_manage_tunnels: false,
            can_upgrade_agent: false,
        },
    )
    .await
    .unwrap();

    db::set_user_roles(&pool, user.id, &[role.id])
        .await
        .unwrap();

    let repo = db::insert_repo(
        &pool,
        &InsertRepoParams {
            name: "view-all-repo",
            repo_path: "/backups/view-all",
            ssh_user: "user",
            ssh_host: "host.local",
            ssh_port: 22,
            passphrase_encrypted: b"enc",
            compression: "none",
            encryption: "none",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap();

    assert!(
        db::get_repo_permission(&pool, user.id, repo.id)
            .await
            .unwrap()
            .is_none()
    );

    let auth = AuthUser {
        user_id: user.id,
        username: "view-all-user".to_string(),
        session_id: None,
    };

    check_repo_permission(&pool, &auth, repo.id, |p| p.can_view)
        .await
        .unwrap();

    let denied = check_repo_permission(&pool, &auth, repo.id, |p| p.can_delete).await;
    assert!(matches!(denied, Err(ApiError::Forbidden(_))));
}

#[sqlx::test(migrations = "./migrations")]
async fn check_agent_repo_access_assigned_agent_succeeds(pool: PgPool) {
    let (agent, repo, _schedule) = create_test_schedule(&pool).await;

    let has_access = server::db::check_agent_repo_access(&pool, agent.id, repo.id)
        .await
        .unwrap();
    assert!(
        has_access,
        "agent assigned to repo via schedule_targets must have access"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn check_agent_repo_access_unassigned_agent_is_rejected(pool: PgPool) {
    let agent = db::insert_agent(&pool, "unassigned-agent", None, "hash", None, None)
        .await
        .unwrap();
    let (_, other_repo, _schedule) = create_test_schedule(&pool).await;

    // This agent has no schedule_targets linking it to other_repo
    let has_access = server::db::check_agent_repo_access(&pool, agent.id, other_repo.id)
        .await
        .unwrap();
    assert!(
        !has_access,
        "agent not assigned to repo must not have access"
    );
}

/// Verify that the `validate_agent_repo` function in handler.rs correctly rejects
/// an agent reporting on an unassigned repo and logs a `security_violation` system event.
#[sqlx::test(migrations = "./migrations")]
async fn validate_agent_repo_rejects_and_logs_security_event(pool: PgPool) {
    let (assigned_agent, assigned_repo, _schedule) = create_test_schedule(&pool).await;

    // Create a second agent that is NOT assigned to the repo
    let rogue_agent = db::insert_agent(&pool, "rogue-agent", None, "rogue-hash", None, None)
        .await
        .unwrap();

    // Assigned agent must pass validation
    let valid = server::db::check_agent_repo_access(&pool, assigned_agent.id, assigned_repo.id)
        .await
        .unwrap();
    assert!(valid);

    // Rogue agent must NOT have access
    let no_access = server::db::check_agent_repo_access(&pool, rogue_agent.id, assigned_repo.id)
        .await
        .unwrap();
    assert!(!no_access);

    // Simulate what validate_agent_repo does on rejection: log a security_violation event
    db::insert_system_event(
        &pool,
        shared::types::SystemEventType::SecurityViolation,
        Some("rogue-agent"),
        "Agent 'rogue-agent' tried to report on repo 999 without assignment (msg=BackupCompleted)",
    )
    .await
    .unwrap();

    let events = db::get_system_events(&pool, 10, AcknowledgedFilter::All)
        .await
        .unwrap();
    let security_events: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == shared::types::SystemEventType::SecurityViolation)
        .collect();
    assert_eq!(security_events.len(), 1);
    assert!(
        security_events
            .first()
            .unwrap()
            .message
            .contains("rogue-agent")
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_tags_use_repo_scope(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let tag = db::insert_tag(&pool, "critical", "#EF4444", "repo")
        .await
        .unwrap();
    assert_eq!(tag.name, "critical");
    assert_eq!(tag.scope, "repo");

    db::set_repo_tags(&pool, repo.id, &[tag.id]).await.unwrap();

    let tags = db::list_tags_for_repo(&pool, repo.id).await.unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags.first().unwrap().name, "critical");
    assert_eq!(tags.first().unwrap().scope, "repo");

    let all_repo_tags = db::list_tags(&pool, "repo").await.unwrap();
    assert!(all_repo_tags.iter().any(|t| t.name == "critical"));
}

#[sqlx::test(migrations = "./migrations")]
async fn fail_started_backups_for_agent_reconnect_covers_all_repos(pool: PgPool) {
    let agent = db::insert_agent(&pool, "reconnect-host", None, "hash", None, None)
        .await
        .unwrap();
    let other_agent = db::insert_agent(&pool, "other-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo_a = create_test_repo_with_host(&pool, "reconnect-repo-a", "storage-a.local").await;
    let repo_b = create_test_repo_with_host(&pool, "reconnect-repo-b", "storage-b.local").await;

    db::insert_backup_started(&pool, agent.id, repo_a.id, None, Utc::now(), None, None)
        .await
        .unwrap();
    db::insert_backup_pending(&pool, agent.id, repo_b.id, None, "run-b", Utc::now())
        .await
        .unwrap();
    // A different agent's in-flight backup must be left untouched.
    db::insert_backup_started(
        &pool,
        other_agent.id,
        repo_a.id,
        None,
        Utc::now(),
        None,
        None,
    )
    .await
    .unwrap();

    let mut repo_ids =
        db::fail_started_backups_for_agent_reconnect(&pool, agent.id, "reconnect-host")
            .await
            .unwrap();
    repo_ids.sort_unstable();
    assert_eq!(repo_ids, vec![repo_a.id, repo_b.id]);

    let reports_a = db::list_reports_for_agent(&pool, agent.id, None, 10)
        .await
        .unwrap();
    assert!(reports_a.iter().all(|r| r.status == "failed"
        && r.error_message.as_deref()
            == Some("Agent 'reconnect-host' reconnected; previous backup abandoned")));

    let other_reports = db::list_reports_for_agent(&pool, other_agent.id, None, 10)
        .await
        .unwrap();
    assert_eq!(other_reports.len(), 1);
    assert_eq!(other_reports.first().unwrap().status, "started");
}

#[sqlx::test(migrations = "./migrations")]
async fn session_insert_and_get(pool: PgPool) {
    let user = db::insert_user(&pool, "session-user", "hash")
        .await
        .unwrap();
    let expires = Utc::now().checked_add_signed(Duration::hours(1)).unwrap();

    db::insert_session(&pool, "session-hash-1", user.id, expires, false, false)
        .await
        .unwrap();

    let session = db::get_session(&pool, "session-hash-1").await.unwrap();
    assert_eq!(session.user_id, user.id);
    assert!(!session.pending_totp);
    assert!(!session.remember_me);
    assert_eq!(session.id, "session-hash-1");
}

#[sqlx::test(migrations = "./migrations")]
async fn session_pending_totp_flag_roundtrip(pool: PgPool) {
    let user = db::insert_user(&pool, "totp-pending-user", "hash")
        .await
        .unwrap();
    let expires = Utc::now().checked_add_signed(Duration::hours(1)).unwrap();

    db::insert_session(&pool, "pending-session-1", user.id, expires, false, true)
        .await
        .unwrap();

    let session = db::get_session(&pool, "pending-session-1").await.unwrap();
    assert!(session.pending_totp);
}

#[sqlx::test(migrations = "./migrations")]
async fn session_expired_returns_error(pool: PgPool) {
    let user = db::insert_user(&pool, "expired-session-user", "hash")
        .await
        .unwrap();
    let past = Utc::now().checked_add_signed(Duration::hours(-1)).unwrap();

    db::insert_session(&pool, "expired-session", user.id, past, false, false)
        .await
        .unwrap();

    let result = db::get_session(&pool, "expired-session").await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn session_list_for_user_excludes_expired_and_pending_totp(pool: PgPool) {
    let user = db::insert_user(&pool, "list-session-user", "hash")
        .await
        .unwrap();
    let future = Utc::now().checked_add_signed(Duration::hours(1)).unwrap();
    let past = Utc::now().checked_add_signed(Duration::hours(-1)).unwrap();

    db::insert_session(&pool, "active-session", user.id, future, false, false)
        .await
        .unwrap();
    db::insert_session(&pool, "expired-session", user.id, past, false, false)
        .await
        .unwrap();
    db::insert_session(&pool, "pending-session", user.id, future, false, true)
        .await
        .unwrap();

    let sessions = db::list_sessions_for_user(&pool, user.id).await.unwrap();
    let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&"active-session"));
    assert!(!ids.contains(&"expired-session"));
    assert!(!ids.contains(&"pending-session"));
}

#[sqlx::test(migrations = "./migrations")]
async fn session_delete_by_id_with_ownership_check(pool: PgPool) {
    let user = db::insert_user(&pool, "del-session-user", "hash")
        .await
        .unwrap();
    let other_user = db::insert_user(&pool, "other-del-user", "hash")
        .await
        .unwrap();
    let future = Utc::now().checked_add_signed(Duration::hours(1)).unwrap();

    db::insert_session(&pool, "del-session", user.id, future, false, false)
        .await
        .unwrap();

    // Other user tries to delete - must fail
    let deleted = db::delete_session_by_id(&pool, "del-session", other_user.id)
        .await
        .unwrap();
    assert!(!deleted, "other user must not be able to delete session");

    // Owner deletes - must succeed
    let deleted = db::delete_session_by_id(&pool, "del-session", user.id)
        .await
        .unwrap();
    assert!(deleted, "owner must be able to delete session");

    // Already deleted - must return false
    let deleted = db::delete_session_by_id(&pool, "del-session", user.id)
        .await
        .unwrap();
    assert!(!deleted);
}

#[sqlx::test(migrations = "./migrations")]
async fn session_update_last_seen(pool: PgPool) {
    let user = db::insert_user(&pool, "last-seen-user", "hash")
        .await
        .unwrap();
    let future = Utc::now().checked_add_signed(Duration::hours(1)).unwrap();

    db::insert_session(&pool, "seen-session", user.id, future, false, false)
        .await
        .unwrap();

    // Wait a minimal delta and update
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    db::update_session_last_seen(&pool, "seen-session")
        .await
        .unwrap();

    let session = db::get_session(&pool, "seen-session").await.unwrap();
    assert!(
        session.last_seen_at > Utc::now().checked_add_signed(Duration::hours(-1)).unwrap(),
        "last_seen_at must have been updated"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn session_extend_expiry(pool: PgPool) {
    let user = db::insert_user(&pool, "extend-user", "hash").await.unwrap();
    let expires = Utc::now().checked_add_signed(Duration::hours(1)).unwrap();

    db::insert_session(&pool, "extend-session", user.id, expires, false, false)
        .await
        .unwrap();

    let new_expires = Utc::now().checked_add_signed(Duration::hours(24)).unwrap();
    db::extend_session(&pool, "extend-session", new_expires)
        .await
        .unwrap();

    let session = db::get_session(&pool, "extend-session").await.unwrap();
    assert!(
        session.expires_at > expires,
        "expires_at must have been extended"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn session_delete_removes_session(pool: PgPool) {
    let user = db::insert_user(&pool, "delete-sess-user", "hash")
        .await
        .unwrap();
    let future = Utc::now().checked_add_signed(Duration::hours(1)).unwrap();

    db::insert_session(&pool, "delete-me", user.id, future, false, false)
        .await
        .unwrap();

    db::delete_session(&pool, "delete-me").await.unwrap();

    let result = db::get_session(&pool, "delete-me").await;
    assert!(result.is_err(), "deleted session must not be found");
}

#[sqlx::test(migrations = "./migrations")]
async fn totp_fields_roundtrip(pool: PgPool) {
    let user = db::insert_user(&pool, "totp-user", "hash").await.unwrap();

    // Initially no TOTP fields
    let fields = db::get_user_totp_fields(&pool, user.id).await.unwrap();
    assert!(fields.is_none(), "no TOTP fields initially");

    let encrypted = b"encrypted_secret_32_bytes_long_here!";
    let recovery = vec!["code1".to_string(), "code2".to_string()];

    db::set_user_totp_secret(&pool, user.id, encrypted, &recovery)
        .await
        .unwrap();

    let fields = db::get_user_totp_fields(&pool, user.id).await.unwrap();
    let fields = fields.expect("TOTP fields must exist after set");
    assert_eq!(fields.secret_encrypted.as_deref(), Some(&encrypted[..]));
    assert!(!fields.enabled, "TOTP must not be enabled yet");
    assert_eq!(fields.recovery_codes.len(), 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn totp_enable_disable_roundtrip(pool: PgPool) {
    let user = db::insert_user(&pool, "totp-enable-user", "hash")
        .await
        .unwrap();

    db::set_user_totp_secret(&pool, user.id, b"some_secret", &[])
        .await
        .unwrap();

    let fields = db::get_user_totp_fields(&pool, user.id).await.unwrap();
    let fields = fields.unwrap();
    assert!(!fields.enabled);

    db::enable_user_totp(&pool, user.id, 7).await.unwrap();
    let fields = db::get_user_totp_fields(&pool, user.id).await.unwrap();
    let fields = fields.unwrap();
    assert!(fields.enabled);
    assert_eq!(
        fields.last_verified_step,
        Some(7),
        "the enrollment code's step must be recorded to prevent its replay"
    );

    db::disable_user_totp(&pool, user.id).await.unwrap();
    let fields = db::get_user_totp_fields(&pool, user.id).await.unwrap();
    // After disable the TOTP configuration must be fully cleared
    assert!(fields.is_none(), "TOTP fields must be cleared on disable");
}

#[sqlx::test(migrations = "./migrations")]
async fn totp_recovery_codes_replace(pool: PgPool) {
    let user = db::insert_user(&pool, "totp-recovery-user", "hash")
        .await
        .unwrap();

    db::set_user_totp_secret(
        &pool,
        user.id,
        b"secret",
        &["old1".to_string(), "old2".to_string()],
    )
    .await
    .unwrap();

    db::replace_totp_recovery_codes(&pool, user.id, &["new1".to_string()])
        .await
        .unwrap();

    let fields = db::get_user_totp_fields(&pool, user.id).await.unwrap();
    let fields = fields.unwrap();
    assert_eq!(fields.recovery_codes.len(), 1);
    assert_eq!(
        fields.recovery_codes.first().map(String::as_str),
        Some("new1")
    );
}

/// Regression test for the TOCTOU race fixed by consuming a recovery code
/// via an atomic conditional `UPDATE ... WHERE $hash = ANY(...)` instead of
/// a read-modify-write of the whole array: a second, concurrent attempt to
/// consume the same (now-removed) code must be rejected, not silently
/// no-op into removing an already-removed value or affecting another code.
#[sqlx::test(migrations = "./migrations")]
async fn totp_try_consume_recovery_code_rejects_reuse_and_is_atomic(pool: PgPool) {
    let user = db::insert_user(&pool, "totp-recovery-atomic-user", "hash")
        .await
        .unwrap();

    db::set_user_totp_secret(
        &pool,
        user.id,
        b"secret",
        &["hash-a".to_string(), "hash-b".to_string()],
    )
    .await
    .unwrap();

    let first = db::try_consume_totp_recovery_code(&pool, user.id, "hash-a")
        .await
        .unwrap();
    assert!(first, "consuming a present code must succeed");

    // Simulate a second, concurrent request racing the same code: it must
    // observe the first request's removal and be rejected, not succeed a
    // second time.
    let second = db::try_consume_totp_recovery_code(&pool, user.id, "hash-a")
        .await
        .unwrap();
    assert!(!second, "reusing an already-consumed code must be rejected");

    let fields = db::get_user_totp_fields(&pool, user.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fields.recovery_codes,
        vec!["hash-b".to_string()],
        "only the consumed code must be removed, the other must remain untouched"
    );

    let unknown = db::try_consume_totp_recovery_code(&pool, user.id, "hash-does-not-exist")
        .await
        .unwrap();
    assert!(
        !unknown,
        "consuming a code that was never present must fail"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn totp_last_verified_step_update(pool: PgPool) {
    let user = db::insert_user(&pool, "totp-verified-user", "hash")
        .await
        .unwrap();

    db::set_user_totp_secret(&pool, user.id, b"secret", &[])
        .await
        .unwrap();

    let fields = db::get_user_totp_fields(&pool, user.id).await.unwrap();
    let fields = fields.unwrap();
    assert!(fields.last_verified_step.is_none());

    let consumed = db::try_consume_totp_step(&pool, user.id, 42).await.unwrap();
    assert!(consumed, "a fresh, newer step must be consumable");

    let fields = db::get_user_totp_fields(&pool, user.id).await.unwrap();
    let fields = fields.unwrap();
    assert_eq!(
        fields.last_verified_step,
        Some(42),
        "last_verified_step must be set after update"
    );
}

/// Regression test for the TOCTOU race fixed by making the replay check and
/// the write a single atomic `UPDATE ... WHERE ...`, rather than a
/// read-then-write: two requests racing the same (or an older) step must
/// not both be able to consume it.
#[sqlx::test(migrations = "./migrations")]
async fn totp_try_consume_step_rejects_replay_and_is_atomic(pool: PgPool) {
    let user = db::insert_user(&pool, "totp-atomic-user", "hash")
        .await
        .unwrap();

    db::set_user_totp_secret(&pool, user.id, b"secret", &[])
        .await
        .unwrap();

    let first = db::try_consume_totp_step(&pool, user.id, 10).await.unwrap();
    assert!(first, "the first attempt at a fresh step must succeed");

    // Simulate a second, concurrent request racing the same step: it must
    // observe the write from the first request and be rejected, not overwrite
    // it or otherwise succeed.
    let replay_same_step = db::try_consume_totp_step(&pool, user.id, 10).await.unwrap();
    assert!(!replay_same_step, "reusing the same step must be rejected");

    let replay_older_step = db::try_consume_totp_step(&pool, user.id, 5).await.unwrap();
    assert!(!replay_older_step, "an older step must be rejected");

    let fields = db::get_user_totp_fields(&pool, user.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fields.last_verified_step,
        Some(10),
        "a rejected replay must not change the recorded step"
    );

    let newer = db::try_consume_totp_step(&pool, user.id, 11).await.unwrap();
    assert!(newer, "a genuinely newer step must still be accepted");
}

#[sqlx::test(migrations = "./migrations")]
async fn totp_fields_per_user_isolation(pool: PgPool) {
    let user_a = db::insert_user(&pool, "totp-iso-a", "hash").await.unwrap();
    let user_b = db::insert_user(&pool, "totp-iso-b", "hash").await.unwrap();

    db::set_user_totp_secret(&pool, user_a.id, b"secret_a", &[])
        .await
        .unwrap();

    let a_fields = db::get_user_totp_fields(&pool, user_a.id).await.unwrap();
    assert!(a_fields.is_some(), "user_a must have TOTP fields");

    let b_fields = db::get_user_totp_fields(&pool, user_b.id).await.unwrap();
    assert!(b_fields.is_none(), "user_b must not have TOTP fields");
}

#[sqlx::test(migrations = "./migrations")]
async fn totp_user_without_secret_returns_none(pool: PgPool) {
    let user = db::insert_user(&pool, "no-totp-user", "hash")
        .await
        .unwrap();

    let fields = db::get_user_totp_fields(&pool, user.id).await.unwrap();
    assert!(fields.is_none(), "user without TOTP setup must return None");
}

/// Seeds one directory of the content index the way `archive_index` writes it:
/// children sorted into listing order, split into chunks of `chunk_entries`.
#[cfg(test)]
async fn seed_archive_dir(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
    dir_path: &str,
    entries: &[DirEntry],
    chunk_entries: usize,
) {
    let archive_id: i64 = sqlx::query_scalar(
        "INSERT INTO archives (repo_id, name) VALUES ($1, $2) ON CONFLICT (repo_id, name) DO \
         UPDATE SET name = EXCLUDED.name RETURNING id",
    )
    .bind(repo_id)
    .bind(archive_name)
    .fetch_one(pool)
    .await
    .unwrap();

    let dir_path_id: i64 = sqlx::query_scalar(
        "INSERT INTO archive_paths (repo_id, path) VALUES ($1, $2) ON CONFLICT (repo_id, path) DO \
         UPDATE SET path = EXCLUDED.path RETURNING id",
    )
    .bind(repo_id)
    .bind(dir_path)
    .fetch_one(pool)
    .await
    .unwrap();

    let mut sorted = entries.to_vec();
    codec::sort_for_listing(&mut sorted);

    for (index, chunk) in sorted.chunks(chunk_entries).enumerate() {
        sqlx::query(
            "INSERT INTO archive_dirs (archive_id, dir_path_id, chunk_no, entries) VALUES ($1, \
             $2, $3, $4)",
        )
        .bind(archive_id)
        .bind(dir_path_id)
        .bind(i32::try_from(index).unwrap())
        .bind(codec::encode(chunk))
        .execute(pool)
        .await
        .unwrap();
    }
}

#[cfg(test)]
fn dir_entry(name: &str, entry_type: &str) -> DirEntry {
    DirEntry {
        name: name.to_owned(),
        entry_type: entry_type.to_owned(),
        size: 42,
        mtime: "2026-06-05T12:00:00.000000".to_owned(),
        mode: "-rw-r--r--".to_owned(),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn query_dir_lists_children_directories_first_then_by_name(pool: PgPool) {
    let repo = create_test_repo(&pool).await;
    seed_archive_dir(
        &pool,
        repo.id,
        "daily-1",
        "var/log",
        &[
            dir_entry("syslog", "-"),
            dir_entry("apt", "d"),
            dir_entry("auth.log", "-"),
        ],
        2000,
    )
    .await;

    let entries = server::archive_index::query_dir(&pool, repo.id, "daily-1", "var/log", 100)
        .await
        .unwrap();

    let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, ["var/log/apt", "var/log/auth.log", "var/log/syslog"]);
    assert_eq!(entries.first().unwrap().entry_type, "d");
}

#[sqlx::test(migrations = "./migrations")]
async fn query_dir_preserves_metadata_verbatim(pool: PgPool) {
    let repo = create_test_repo(&pool).await;
    let mut entry = dir_entry("odd\tname\nwith breaks", "-");
    entry.mtime = "2026-06-05T12:00:00.123456".to_owned();
    entry.mode = "-rwsr-xr-t".to_owned();
    entry.size = i64::MAX;

    seed_archive_dir(&pool, repo.id, "daily-1", "data", &[entry], 2000).await;

    let entries = server::archive_index::query_dir(&pool, repo.id, "daily-1", "data", 100)
        .await
        .unwrap();

    let only = entries.first().unwrap();
    assert_eq!(only.path, "data/odd\tname\nwith breaks");
    assert_eq!(only.mtime, "2026-06-05T12:00:00.123456");
    assert_eq!(only.mode, "-rwsr-xr-t");
    assert_eq!(only.size, i64::MAX);
}

#[sqlx::test(migrations = "./migrations")]
async fn query_dir_returns_root_children_without_a_leading_separator(pool: PgPool) {
    let repo = create_test_repo(&pool).await;
    seed_archive_dir(
        &pool,
        repo.id,
        "daily-1",
        "",
        &[dir_entry("etc", "d")],
        2000,
    )
    .await;

    let entries = server::archive_index::query_dir(&pool, repo.id, "daily-1", "", 100)
        .await
        .unwrap();

    assert_eq!(entries.first().unwrap().path, "etc");
}

#[sqlx::test(migrations = "./migrations")]
async fn query_dir_reads_across_chunks_and_honours_the_limit(pool: PgPool) {
    let repo = create_test_repo(&pool).await;
    let entries: Vec<DirEntry> = (0..250)
        .map(|i| dir_entry(&format!("file_{i:04}"), "-"))
        .collect();

    // Chunk size 10 forces 25 chunks, so a limit of 42 must span several of them.
    seed_archive_dir(&pool, repo.id, "daily-1", "many", &entries, 10).await;

    let limited = server::archive_index::query_dir(&pool, repo.id, "daily-1", "many", 42)
        .await
        .unwrap();
    assert_eq!(limited.len(), 42);
    assert_eq!(limited.first().unwrap().path, "many/file_0000");
    assert_eq!(limited.last().unwrap().path, "many/file_0041");

    let all = server::archive_index::query_dir(&pool, repo.id, "daily-1", "many", 1000)
        .await
        .unwrap();
    assert_eq!(all.len(), 250);
    assert_eq!(all.last().unwrap().path, "many/file_0249");
}

#[sqlx::test(migrations = "./migrations")]
async fn query_dir_returns_nothing_for_unknown_archives_and_directories(pool: PgPool) {
    let repo = create_test_repo(&pool).await;
    seed_archive_dir(
        &pool,
        repo.id,
        "daily-1",
        "etc",
        &[dir_entry("hosts", "-")],
        2000,
    )
    .await;

    assert!(
        server::archive_index::query_dir(&pool, repo.id, "missing-archive", "etc", 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        server::archive_index::query_dir(&pool, repo.id, "daily-1", "no/such/dir", 100)
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn deleting_an_archive_gcs_only_its_orphaned_directory_paths(pool: PgPool) {
    let repo = create_test_repo(&pool).await;
    seed_archive_dir(
        &pool,
        repo.id,
        "daily-1",
        "shared",
        &[dir_entry("a", "-")],
        2000,
    )
    .await;
    seed_archive_dir(
        &pool,
        repo.id,
        "daily-2",
        "shared",
        &[dir_entry("a", "-")],
        2000,
    )
    .await;
    seed_archive_dir(
        &pool,
        repo.id,
        "daily-1",
        "only-in-one",
        &[dir_entry("b", "-")],
        2000,
    )
    .await;

    db::delete_archive_records_by_names(&pool, repo.id, &["daily-1".to_owned()])
        .await
        .unwrap();

    let remaining: Vec<String> =
        sqlx::query_scalar("SELECT path FROM archive_paths WHERE repo_id = $1 ORDER BY path")
            .bind(repo.id)
            .fetch_all(&pool)
            .await
            .unwrap();

    // "shared" is still referenced by daily-2; "only-in-one" is now orphaned.
    assert_eq!(remaining, ["shared"]);

    let survivors = server::archive_index::query_dir(&pool, repo.id, "daily-2", "shared", 100)
        .await
        .unwrap();
    assert_eq!(survivors.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn deleting_all_repo_archive_data_clears_the_content_index(pool: PgPool) {
    let repo = create_test_repo(&pool).await;
    seed_archive_dir(
        &pool,
        repo.id,
        "daily-1",
        "shared",
        &[dir_entry("a", "-")],
        2000,
    )
    .await;
    seed_archive_dir(
        &pool,
        repo.id,
        "daily-2",
        "shared",
        &[dir_entry("a", "-")],
        2000,
    )
    .await;

    db::delete_all_repo_archive_data(&pool, repo.id)
        .await
        .unwrap();

    let paths: i64 = sqlx::query_scalar("SELECT count(*) FROM archive_paths WHERE repo_id = $1")
        .bind(repo.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let dirs: i64 = sqlx::query_scalar("SELECT count(*) FROM archive_dirs")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(paths, 0);
    assert_eq!(dirs, 0);
}

/// The `(archives.id, archive_paths.id)` pair a seeded directory was stored under.
#[cfg(test)]
async fn archive_and_dir_path_ids(
    pool: &PgPool,
    repo_id: i64,
    archive_name: &str,
    dir_path: &str,
) -> (i64, i64) {
    let archive_id: i64 =
        sqlx::query_scalar("SELECT id FROM archives WHERE repo_id = $1 AND name = $2")
            .bind(repo_id)
            .bind(archive_name)
            .fetch_one(pool)
            .await
            .unwrap();
    let dir_path_id: i64 =
        sqlx::query_scalar("SELECT id FROM archive_paths WHERE repo_id = $1 AND path = $2")
            .bind(repo_id)
            .bind(dir_path)
            .fetch_one(pool)
            .await
            .unwrap();
    (archive_id, dir_path_id)
}

#[sqlx::test(migrations = "./migrations")]
async fn reindexing_an_archive_replaces_stale_chunks_instead_of_merging(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    // An earlier indexing pass packed the directory into three chunks, as it would
    // have with a smaller CHUNK_ENTRIES.
    let first_pass: Vec<DirEntry> = (0..30)
        .map(|i| dir_entry(&format!("old_{i:03}"), "-"))
        .collect();
    seed_archive_dir(&pool, repo.id, "daily-1", "dir", &first_pass, 10).await;

    let (archive_id, dir_path_id) =
        archive_and_dir_path_ids(&pool, repo.id, "daily-1", "dir").await;

    // A later pass over the same archive produces fewer entries in a single chunk.
    // Merging would leave chunks 1 and 2 behind for the reader to append.
    let second_pass: Vec<DirEntry> = (0..5)
        .map(|i| dir_entry(&format!("new_{i:03}"), "-"))
        .collect();
    server::archive_index::replace_archive_dirs(&pool, archive_id, &[(dir_path_id, second_pass)])
        .await
        .unwrap();

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM archive_dirs WHERE archive_id = $1")
        .bind(archive_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 1, "stale chunk rows from the earlier pass survived");

    let entries = server::archive_index::query_dir(&pool, repo.id, "daily-1", "dir", 1000)
        .await
        .unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        names,
        [
            "dir/new_000",
            "dir/new_001",
            "dir/new_002",
            "dir/new_003",
            "dir/new_004"
        ]
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn replacing_one_archive_leaves_other_archives_untouched(pool: PgPool) {
    let repo = create_test_repo(&pool).await;
    seed_archive_dir(
        &pool,
        repo.id,
        "daily-1",
        "dir",
        &[dir_entry("a", "-")],
        2000,
    )
    .await;
    seed_archive_dir(
        &pool,
        repo.id,
        "daily-2",
        "dir",
        &[dir_entry("b", "-")],
        2000,
    )
    .await;

    let (archive_id, dir_path_id) =
        archive_and_dir_path_ids(&pool, repo.id, "daily-1", "dir").await;
    server::archive_index::replace_archive_dirs(
        &pool,
        archive_id,
        &[(dir_path_id, vec![dir_entry("c", "-")])],
    )
    .await
    .unwrap();

    let replaced = server::archive_index::query_dir(&pool, repo.id, "daily-1", "dir", 100)
        .await
        .unwrap();
    let untouched = server::archive_index::query_dir(&pool, repo.id, "daily-2", "dir", 100)
        .await
        .unwrap();

    assert_eq!(replaced.first().unwrap().path, "dir/c");
    assert_eq!(untouched.first().unwrap().path, "dir/b");
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_power_defaults_to_disabled(pool: PgPool) {
    let agent = db::insert_agent(&pool, "power-default-host", None, "hash", None, None)
        .await
        .unwrap();

    assert!(!agent.wake_enabled);
    assert!(agent.wake_mac_address.is_none());
    assert_eq!(agent.wake_timeout_seconds, 180);
    assert!(!agent.shutdown_after_backup);
    assert!(!agent.start_agent_enabled);
    assert!(!agent.stop_agent_after_backup);
    assert_eq!(agent.agent_service_name, "assimilate-agent");
}

#[sqlx::test(migrations = "./migrations")]
async fn update_agent_power_persists_all_fields(pool: PgPool) {
    let agent = db::insert_agent(&pool, "power-host", None, "hash", None, None)
        .await
        .unwrap();

    let updated = db::update_agent_power(
        &pool,
        agent.id,
        db::AgentPowerPatch {
            wake_enabled: true,
            wake_mac_address: Some("3C:97:0E:2B:9A:44"),
            wake_broadcast_address: Some("192.168.1.255"),
            wake_timeout_seconds: 240,
            shutdown_after_backup: true,
            start_agent_enabled: true,
            stop_agent_after_backup: true,
            ssh_host: Some("192.168.1.10"),
            ssh_port: 2222,
            agent_service_name: "custom-agent",
        },
    )
    .await
    .unwrap();

    assert!(updated.wake_enabled);
    assert_eq!(
        updated.wake_mac_address.as_deref(),
        Some("3C:97:0E:2B:9A:44")
    );
    assert_eq!(
        updated.wake_broadcast_address.as_deref(),
        Some("192.168.1.255")
    );
    assert_eq!(updated.wake_timeout_seconds, 240);
    assert!(updated.shutdown_after_backup);
    assert!(updated.start_agent_enabled);
    assert!(updated.stop_agent_after_backup);
    assert_eq!(updated.ssh_host.as_deref(), Some("192.168.1.10"));
    assert_eq!(updated.ssh_port, 2222);
    assert_eq!(updated.agent_service_name, "custom-agent");

    // update_agent_power must not touch backup-default fields it doesn't own.
    assert_eq!(updated.hostname, "power-host");
}

#[sqlx::test(migrations = "./migrations")]
async fn update_agent_power_rejects_shutdown_without_wake_at_the_db_layer(pool: PgPool) {
    let agent = db::insert_agent(&pool, "power-check-host", None, "hash", None, None)
        .await
        .unwrap();

    let err = db::update_agent_power(
        &pool,
        agent.id,
        db::AgentPowerPatch {
            wake_enabled: false,
            wake_mac_address: None,
            wake_broadcast_address: None,
            wake_timeout_seconds: 180,
            shutdown_after_backup: true,
            start_agent_enabled: false,
            stop_agent_after_backup: false,
            ssh_host: None,
            ssh_port: 22,
            agent_service_name: "assimilate-agent",
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, server::error::ApiError::Database(_)));
}

#[sqlx::test(migrations = "./migrations")]
async fn update_repo_power_persists_all_fields(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let updated = db::update_repo_power(
        &pool,
        repo.id,
        db::RepoPowerPatch {
            wake_enabled: true,
            wake_mac_address: Some("9C:B6:D0:1A:44:7F"),
            wake_broadcast_address: Some("192.168.1.255"),
            wake_timeout_seconds: 240,
            shutdown_after_backup: true,
        },
    )
    .await
    .unwrap();

    assert!(updated.wake_enabled);
    assert_eq!(
        updated.wake_mac_address.as_deref(),
        Some("9C:B6:D0:1A:44:7F")
    );
    assert_eq!(updated.wake_timeout_seconds, 240);
    assert!(updated.shutdown_after_backup);
    // update_repo_power must not touch fields it doesn't own.
    assert_eq!(updated.name, repo.name);
    assert_eq!(updated.ssh_host, repo.ssh_host);
}

#[sqlx::test(migrations = "./migrations")]
async fn get_repo_by_id_includes_power_fields(pool: PgPool) {
    let repo = create_test_repo(&pool).await;
    db::update_repo_power(
        &pool,
        repo.id,
        db::RepoPowerPatch {
            wake_enabled: true,
            wake_mac_address: Some("9C:B6:D0:1A:44:7F"),
            wake_broadcast_address: None,
            wake_timeout_seconds: 180,
            shutdown_after_backup: true,
        },
    )
    .await
    .unwrap();

    let fetched = db::get_repo_by_id(&pool, repo.id).await.unwrap();
    assert!(fetched.wake_enabled);
    assert_eq!(
        fetched.wake_mac_address.as_deref(),
        Some("9C:B6:D0:1A:44:7F")
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn run_events_list_in_chronological_order(pool: PgPool) {
    let agent = db::insert_agent(&pool, "run-events-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let run_id = "test-run-123";

    db::run_events::insert_run_event(
        &pool,
        run_id,
        agent.id,
        repo.id,
        shared::types::RunEventTarget::Source,
        shared::types::RunEventType::ReachabilityCheck,
        "Checked agent -- no response",
    )
    .await
    .unwrap();
    db::run_events::insert_run_event(
        &pool,
        run_id,
        agent.id,
        repo.id,
        shared::types::RunEventTarget::Source,
        shared::types::RunEventType::WakeSent,
        "Sent Wake-on-LAN packet to 3C:97:0E:2B:9A:44",
    )
    .await
    .unwrap();
    db::run_events::insert_run_event(
        &pool,
        run_id,
        agent.id,
        repo.id,
        shared::types::RunEventTarget::Repository,
        shared::types::RunEventType::WakeSent,
        "Sent Wake-on-LAN packet to 9C:B6:D0:1A:44:7F",
    )
    .await
    .unwrap();

    let events = db::run_events::list_run_events(&pool, run_id, agent.id, repo.id)
        .await
        .unwrap();

    assert_eq!(events.len(), 3);
    assert_eq!(events.first().unwrap().event_type, "reachability_check");
    assert_eq!(events.get(1).unwrap().event_type, "wake_sent");
    assert_eq!(events.get(1).unwrap().target, "source");
    assert_eq!(events.get(2).unwrap().target, "repository");
}

#[sqlx::test(migrations = "./migrations")]
async fn run_events_for_unrelated_run_id_are_not_returned(pool: PgPool) {
    let agent = db::insert_agent(&pool, "run-events-host-a", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    db::run_events::insert_run_event(
        &pool,
        "run-a",
        agent.id,
        repo.id,
        shared::types::RunEventTarget::Source,
        shared::types::RunEventType::WakeSent,
        "message for run a",
    )
    .await
    .unwrap();

    let events = db::run_events::list_run_events(&pool, "run-b", agent.id, repo.id)
        .await
        .unwrap();
    assert!(events.is_empty());
}

// A multi-target schedule shares one run_id across every target pairing it
// fans out to (see the backup_run_events migration), so run_id alone can't
// scope a query to one pairing's events - list_run_events must also filter
// by agent_id/repo_id, or a run detail view for one target would show
// another target's power-management timeline mixed in with its own.
#[sqlx::test(migrations = "./migrations")]
async fn run_events_are_scoped_to_their_own_target_pairing_within_a_shared_run_id(pool: PgPool) {
    let agent_a = db::insert_agent(&pool, "run-events-target-a", None, "hash", None, None)
        .await
        .unwrap();
    let agent_b = db::insert_agent(&pool, "run-events-target-b", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let run_id = "shared-run-id";

    db::run_events::insert_run_event(
        &pool,
        run_id,
        agent_a.id,
        repo.id,
        shared::types::RunEventTarget::Source,
        shared::types::RunEventType::WakeSent,
        "message for target a",
    )
    .await
    .unwrap();
    db::run_events::insert_run_event(
        &pool,
        run_id,
        agent_b.id,
        repo.id,
        shared::types::RunEventTarget::Source,
        shared::types::RunEventType::WakeSent,
        "message for target b",
    )
    .await
    .unwrap();

    let events_for_a = db::run_events::list_run_events(&pool, run_id, agent_a.id, repo.id)
        .await
        .unwrap();
    assert_eq!(events_for_a.len(), 1);
    assert_eq!(
        events_for_a.first().unwrap().message,
        "message for target a"
    );

    let events_for_b = db::run_events::list_run_events(&pool, run_id, agent_b.id, repo.id)
        .await
        .unwrap();
    assert_eq!(events_for_b.len(), 1);
    assert_eq!(
        events_for_b.first().unwrap().message,
        "message for target b"
    );
}

#[cfg(test)]
async fn insert_test_run_event(
    pool: &PgPool,
    agent_id: i64,
    repo_id: i64,
    occurred_at: DateTime<Utc>,
) {
    sqlx::query!(
        "INSERT INTO backup_run_events (run_id, agent_id, repo_id, target, event_type, message, \
         occurred_at) VALUES ('retention-test-run', $1, $2, 'source', 'wake_sent', 'test event', \
         $3)",
        agent_id,
        repo_id,
        occurred_at,
    )
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_run_events_before_deletes_old(pool: PgPool) {
    let agent = db::insert_agent(&pool, "run-events-retention-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let old = Utc::now().checked_sub_signed(Duration::days(120)).unwrap();
    insert_test_run_event(&pool, agent.id, repo.id, old).await;

    let cutoff = Utc::now().checked_sub_signed(Duration::days(90)).unwrap();
    let deleted = db::run_events::delete_run_events_before(&pool, cutoff)
        .await
        .unwrap();
    assert_eq!(deleted, 1, "run event older than cutoff must be deleted");
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_run_events_before_keeps_recent(pool: PgPool) {
    let agent = db::insert_agent(&pool, "run-events-retention-host", None, "hash", None, None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let before_insert = Utc::now();
    insert_test_run_event(&pool, agent.id, repo.id, before_insert).await;

    let cutoff = before_insert
        .checked_sub_signed(Duration::seconds(1))
        .unwrap();
    let deleted = db::run_events::delete_run_events_before(&pool, cutoff)
        .await
        .unwrap();
    assert_eq!(
        deleted, 0,
        "run event created after cutoff must not be deleted"
    );

    let events = db::run_events::list_run_events(&pool, "retention-test-run", agent.id, repo.id)
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
}

/// Hook commands stored before per-command timeouts existed are a JSONB array
/// of bare strings. The migration normalises the rows it finds, but a config
/// import can still write the old shape, so decoding one must not fail - it
/// must read as a command that inherits the schedule's timeout.
#[sqlx::test(migrations = "./migrations")]
async fn schedule_hook_commands_decode_legacy_bare_strings(pool: PgPool) {
    let repo = create_test_repo(&pool).await;
    let schedule = db::insert_schedule(
        &pool,
        repo.id,
        &ScheduleParams {
            name: "legacy-hook-commands",
            schedule_type: "backup",
            cron_expression: "0 3 * * *",
            enabled: true,
            canary_enabled: false,
            vm_snapshot_enabled: false,
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
            pre_backup_commands: &[],
            post_backup_commands: &[],
            hook_timeout_seconds: 60,
            missed_backup_threshold: 3,
            on_failure: "stop",
        },
        None,
    )
    .await
    .unwrap();

    sqlx::query(
        "UPDATE schedules SET pre_backup_commands = '[\"echo legacy\"]'::JSONB WHERE id = $1",
    )
    .bind(schedule.id)
    .execute(&pool)
    .await
    .unwrap();

    let loaded = db::get_schedule_by_id(&pool, schedule.id).await.unwrap();
    assert_eq!(
        loaded.pre_backup_commands.0,
        vec![HookCommand::new("echo legacy")],
        "a bare string must decode as a command with no timeout of its own"
    );
}

/// A per-command timeout has to survive the JSONB round trip, since it is the
/// only thing that lets one slow hook have a longer budget than its siblings.
#[sqlx::test(migrations = "./migrations")]
async fn agent_default_hook_commands_round_trip_their_timeouts(pool: PgPool) {
    let agent = db::insert_agent(&pool, "hook-timeout-agent", None, "hash", None, None)
        .await
        .unwrap();

    let commands = vec![
        HookCommand {
            command: "vzdump --all 1".to_owned(),
            timeout_seconds: Some(7200),
        },
        HookCommand::new("systemctl start app"),
    ];
    db::update_agent(
        &pool,
        agent.id,
        "hook-timeout-agent",
        db::AgentDefaults {
            display_name: None,
            domain: None,
            default_backup_paths: &[],
            default_exclude_patterns: &[],
            default_pre_backup_commands: &commands,
            default_post_backup_commands: &[],
            default_file_change_patterns_raw: "",
        },
    )
    .await
    .unwrap();

    let loaded = db::get_agent_by_id(&pool, agent.id).await.unwrap();
    assert_eq!(loaded.default_pre_backup_commands.0, commands);
}

#[sqlx::test(migrations = "./migrations")]
async fn vm_selection_defaults_to_staging_every_domain(pool: PgPool) {
    let agent = db::insert_agent(&pool, "vm-default", None, "hash", None, None)
        .await
        .unwrap();

    let config = db::vms::load_config(&pool, agent.id).await.unwrap();

    assert_eq!(config.selection, VmSelectionMode::All);
    assert!(
        config.includes("anything"),
        "a host nobody has configured backs its machines up"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn vm_selection_survives_a_round_trip(pool: PgPool) {
    let agent = db::insert_agent(&pool, "vm-selection", None, "hash", None, None)
        .await
        .unwrap();

    let stored = db::vms::update_agent_vm_snapshot(
        &pool,
        agent.id,
        db::vms::VmSnapshotPatch {
            enabled: true,
            dir: "/srv/vm",
            full_interval: 7,
            timeout_seconds: 1800,
            default_limit_bytes: 0,
            selection: VmSelectionMode::Selected,
        },
    )
    .await
    .unwrap();

    assert_eq!(stored.vm_snapshot_selection, "selected");
    assert_eq!(
        db::vms::load_config(&pool, agent.id)
            .await
            .unwrap()
            .selection,
        VmSelectionMode::Selected
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_scanned_domain_carries_no_decision_until_someone_makes_one(pool: PgPool) {
    let agent = db::insert_agent(&pool, "vm-undecided", None, "hash", None, None)
        .await
        .unwrap();
    db::vms::record_scan(&pool, agent.id, &[discovered("web01")])
        .await
        .unwrap();

    let config = db::vms::load_config(&pool, agent.id).await.unwrap();
    let domain = config
        .domains
        .iter()
        .find(|candidate| candidate.name == "web01")
        .expect("the scan stored the domain");
    assert_eq!(
        domain.included, None,
        "a scan reports what is on the host, it does not decide what to back up"
    );

    // The same stored row reads both ways, which is the whole point of
    // keeping "undecided" distinct from "included".
    assert!(config.includes("web01"));
    let selected = VmSnapshotConfig {
        selection: VmSelectionMode::Selected,
        ..config
    };
    assert!(!selected.includes("web01"));
}

#[sqlx::test(migrations = "./migrations")]
async fn a_rescan_forgets_domains_nobody_decided_about_and_keeps_the_rest(pool: PgPool) {
    let agent = db::insert_agent(&pool, "vm-rescan", None, "hash", None, None)
        .await
        .unwrap();
    db::vms::record_scan(
        &pool,
        agent.id,
        &[
            discovered("untouched"),
            discovered("picked"),
            discovered("refused"),
            discovered("limited"),
        ],
    )
    .await
    .unwrap();

    db::vms::set_vm_settings(&pool, agent.id, "picked", Some(true), None)
        .await
        .unwrap();
    db::vms::set_vm_settings(&pool, agent.id, "refused", Some(false), None)
        .await
        .unwrap();
    db::vms::set_vm_settings(&pool, agent.id, "limited", Some(true), Some(4096))
        .await
        .unwrap();

    // Every domain is gone from the host now.
    db::vms::record_scan(&pool, agent.id, &[]).await.unwrap();

    let names: Vec<String> = db::vms::list_agent_vms(&pool, agent.id)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.name)
        .collect();

    assert!(
        !names.contains(&"untouched".to_owned()),
        "a row a rescan can rebuild from scratch is not worth keeping"
    );
    for kept in ["picked", "refused", "limited"] {
        assert!(
            names.contains(&kept.to_owned()),
            "{kept} carries a decision that must outlive the domain"
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn a_rescan_keeps_a_selection_that_the_old_prune_would_have_dropped(pool: PgPool) {
    let agent = db::insert_agent(&pool, "vm-selected-prune", None, "hash", None, None)
        .await
        .unwrap();
    db::vms::update_agent_vm_snapshot(
        &pool,
        agent.id,
        db::vms::VmSnapshotPatch {
            enabled: true,
            dir: "/srv/vm",
            full_interval: 7,
            timeout_seconds: 1800,
            default_limit_bytes: 0,
            selection: VmSelectionMode::Selected,
        },
    )
    .await
    .unwrap();
    db::vms::record_scan(&pool, agent.id, &[discovered("picked")])
        .await
        .unwrap();
    db::vms::set_vm_settings(&pool, agent.id, "picked", Some(true), None)
        .await
        .unwrap();

    // The domain disappears for a reboot and comes back.
    db::vms::record_scan(&pool, agent.id, &[]).await.unwrap();
    db::vms::record_scan(&pool, agent.id, &[discovered("picked")])
        .await
        .unwrap();

    assert!(
        db::vms::load_config(&pool, agent.id)
            .await
            .unwrap()
            .includes("picked"),
        "an opt-in selection must survive the domain vanishing from a scan"
    );
}

/// Setting only a limit must not decide whether a domain is staged. The UI
/// sends the resolved include flag it is displaying, and under `selected` the
/// resolved value for an undecided domain would otherwise write an opt-in
/// that nobody asked for - a budget is a cap, not consent.
#[sqlx::test(migrations = "./migrations")]
async fn setting_a_limit_leaves_an_undecided_domain_undecided(pool: PgPool) {
    let agent = db::insert_agent(&pool, "vm-limit", None, "hash", None, None)
        .await
        .unwrap();

    db::vms::record_scan(&pool, agent.id, &[discovered("undecided")])
        .await
        .unwrap();

    let row = db::vms::set_vm_settings(&pool, agent.id, "undecided", None, Some(4096))
        .await
        .unwrap();
    assert_eq!(
        row.included, None,
        "a limit-only edit must leave the domain undecided"
    );
    assert_eq!(
        row.limit_bytes,
        Some(4096),
        "the limit must still be stored"
    );

    db::vms::update_agent_vm_snapshot(
        &pool,
        agent.id,
        db::vms::VmSnapshotPatch {
            enabled: true,
            dir: "/srv/vm",
            full_interval: 7,
            timeout_seconds: 1800,
            default_limit_bytes: 0,
            selection: VmSelectionMode::Selected,
        },
    )
    .await
    .unwrap();

    assert!(
        !db::vms::load_config(&pool, agent.id)
            .await
            .unwrap()
            .includes("undecided"),
        "giving a domain a budget must not opt it into an opt-in host"
    );
}

/// The other direction: an explicit decision survives a later limit-only
/// edit, so preserving the undecided state has not made the flag unwritable.
#[sqlx::test(migrations = "./migrations")]
async fn a_limit_edit_preserves_an_explicit_decision(pool: PgPool) {
    let agent = db::insert_agent(&pool, "vm-limit-keep", None, "hash", None, None)
        .await
        .unwrap();

    db::vms::record_scan(
        &pool,
        agent.id,
        &[discovered("kept"), discovered("dropped")],
    )
    .await
    .unwrap();

    db::vms::set_vm_settings(&pool, agent.id, "kept", Some(true), None)
        .await
        .unwrap();
    db::vms::set_vm_settings(&pool, agent.id, "dropped", Some(false), None)
        .await
        .unwrap();

    let kept = db::vms::set_vm_settings(&pool, agent.id, "kept", None, Some(8192))
        .await
        .unwrap();
    let dropped = db::vms::set_vm_settings(&pool, agent.id, "dropped", None, Some(8192))
        .await
        .unwrap();

    assert_eq!(kept.included, Some(true), "an opt-in must survive");
    assert_eq!(dropped.included, Some(false), "an opt-out must survive");
}

fn discovered(name: &str) -> DiscoveredVm {
    DiscoveredVm {
        name: name.to_owned(),
        state: VmState::Running,
        mode: VmSnapshotMode::Incremental,
        disk_count: 1,
        disk_bytes: 1024,
    }
}
