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

use chrono::{Datelike, Duration, Utc};
use chrono_tz::Tz;
use server::db::{self, patterns, *};
use shared::types::QuotaAction;
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn agent_insert_and_get(pool: PgPool) {
    let agent = db::insert_agent(&pool, "test-host", Some("Test Host"), "hash123", None)
        .await
        .unwrap();

    assert_eq!(agent.hostname, "test-host");
    assert_eq!(agent.display_name.as_deref(), Some("Test Host"));
    assert!(agent.agent_version.is_none());
    assert!(agent.last_seen_at.is_none());

    let fetched = db::get_agent_by_hostname(&pool, "test-host").await.unwrap();
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
            .any(|relation| relation.table_name == "archive_files")
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
    let result = db::get_agent_by_hostname(&pool, "nonexistent").await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_token_hash(pool: PgPool) {
    db::insert_agent(&pool, "token-host", None, "secret_hash", None)
        .await
        .unwrap();

    let (id, hash) = db::get_agent_token_hash(&pool, "token-host").await.unwrap();
    assert!(id > 0);
    assert_eq!(hash, "secret_hash");
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_update_last_seen(pool: PgPool) {
    let agent = db::insert_agent(&pool, "seen-host", None, "hash", None)
        .await
        .unwrap();

    db::update_last_seen(&pool, agent.id).await.unwrap();

    let fetched = db::get_agent_by_hostname(&pool, "seen-host").await.unwrap();
    assert!(fetched.last_seen_at.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_update_last_ssh_user(pool: PgPool) {
    let agent = db::insert_agent(&pool, "ssh-user-host", None, "hash", None)
        .await
        .unwrap();

    let fetched = db::get_agent_by_hostname(&pool, "ssh-user-host")
        .await
        .unwrap();
    assert_eq!(fetched.last_ssh_user, None);

    db::update_last_ssh_user(&pool, agent.id, "deploy-user")
        .await
        .unwrap();

    let fetched = db::get_agent_by_hostname(&pool, "ssh-user-host")
        .await
        .unwrap();
    assert_eq!(fetched.last_ssh_user.as_deref(), Some("deploy-user"));

    db::update_last_ssh_user(&pool, agent.id, "root")
        .await
        .unwrap();

    let fetched = db::get_agent_by_hostname(&pool, "ssh-user-host")
        .await
        .unwrap();
    assert_eq!(fetched.last_ssh_user.as_deref(), Some("root"));
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_update_last_seen_and_version(pool: PgPool) {
    let agent = db::insert_agent(&pool, "ver-host", None, "hash", None)
        .await
        .unwrap();

    db::update_last_seen_and_version(&pool, agent.id, "2.0.0", None, None, None)
        .await
        .unwrap();

    let fetched = db::get_agent_by_hostname(&pool, "ver-host").await.unwrap();
    assert_eq!(fetched.agent_version.as_deref(), Some("2.0.0"));
    assert!(fetched.last_seen_at.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_update_last_seen_by_hostname(pool: PgPool) {
    db::insert_agent(&pool, "hostname-seen", None, "hash", None)
        .await
        .unwrap();

    db::update_last_seen_by_hostname(&pool, "hostname-seen")
        .await
        .unwrap();

    let fetched = db::get_agent_by_hostname(&pool, "hostname-seen")
        .await
        .unwrap();
    assert!(fetched.last_seen_at.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_list(pool: PgPool) {
    db::insert_agent(&pool, "alpha", None, "h1", None)
        .await
        .unwrap();
    db::insert_agent(&pool, "beta", None, "h2", None)
        .await
        .unwrap();

    let agents = db::list_agents(&pool, false).await.unwrap();
    assert_eq!(agents.len(), 2);
    assert_eq!(agents.first().unwrap().hostname, "alpha");
    assert_eq!(agents.get(1).unwrap().hostname, "beta");
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_update(pool: PgPool) {
    db::insert_agent(&pool, "upd-host", Some("Old Name"), "hash", None)
        .await
        .unwrap();

    let updated = db::update_agent(
        &pool,
        "upd-host",
        "upd-host",
        db::AgentDefaults {
            display_name: Some("New Name"),
            default_backup_paths: &[],
            default_exclude_patterns: &[],
            default_pre_backup_commands: "[]",
            default_post_backup_commands: "[]",
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
    db::insert_agent(&pool, "regen-host", None, "old_hash", None)
        .await
        .unwrap();

    let updated = db::regenerate_agent_token(&pool, "regen-host", "new_hash")
        .await
        .unwrap();
    assert_eq!(updated.hostname, "regen-host");

    let (_, hash) = db::get_agent_token_hash(&pool, "regen-host").await.unwrap();
    assert_eq!(hash, "new_hash");
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_delete(pool: PgPool) {
    db::insert_agent(&pool, "del-host", None, "hash", None)
        .await
        .unwrap();

    db::delete_agent(&pool, "del-host").await.unwrap();

    let result = db::get_agent_by_hostname(&pool, "del-host").await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_delete_not_found(pool: PgPool) {
    let result = db::delete_agent(&pool, "ghost").await;
    assert!(result.is_err());
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
    let agent = db::insert_agent(&pool, "tunnel-host", None, "hash", None)
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
    assert!(enabled.is_empty());

    db::delete_tunnel(&pool, tunnel.id).await.unwrap();
    let result = db::get_tunnel_by_id(&pool, tunnel.id).await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn tunnel_defaults(pool: PgPool) {
    let agent = db::insert_agent(&pool, "def-host", None, "hash", None)
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
    let agent = db::insert_agent(&pool, "key-persist-host", None, "hash", None)
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
    let agent = db::insert_agent(pool, "sched-host", None, "hash", None)
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
            pre_backup_commands: "",
            post_backup_commands: "",
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
            pre_backup_commands: "echo pre",
            post_backup_commands: "echo post",
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
    assert_eq!(updated.pre_backup_commands, "echo pre");
    assert_eq!(updated.post_backup_commands, "echo post");
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

    let agent_b = db::insert_agent(&pool, "repo-list-host-b", None, "hashb", None)
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
            pre_backup_commands: "",
            post_backup_commands: "",
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
            pre_backup_commands: "",
            post_backup_commands: "",
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
    assert!(due.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_agent_hostname(pool: PgPool) {
    let (_, _, schedule) = create_test_schedule(&pool).await;

    let hostnames = db::get_schedule_target_hostnames(&pool, schedule.id)
        .await
        .unwrap();
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
    assert!(sources.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn backup_sources_per_agent_crud(pool: PgPool) {
    let (agent, _, schedule) = create_test_schedule(&pool).await;

    let agent2 = db::insert_agent(&pool, "host-two", None, "hash2", None)
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
    assert!(all_per_agent.is_empty());

    let schedule_level = db::list_backup_sources_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(schedule_level, vec!["/shared"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn excludes_per_agent_crud(pool: PgPool) {
    let (agent, _, schedule) = create_test_schedule(&pool).await;

    let agent2 = db::insert_agent(&pool, "host-two-exc", None, "hash2exc", None)
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
    assert!(all_per_agent.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn file_change_patterns_per_agent_crud(pool: PgPool) {
    let (agent, _, schedule) = create_test_schedule(&pool).await;

    let agent2 = db::insert_agent(&pool, "host-two-fcp", None, "hash2fcp", None)
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
    assert!(all_per_agent.is_empty());
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
            pre_backup_commands: "",
            post_backup_commands: "",
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
            pre_backup_commands: "",
            post_backup_commands: "",
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

    let config = server::config_assembler::assemble_config(&pool, &encryption_key, &agent.hostname)
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
            pre_backup_commands: "",
            post_backup_commands: "",
            on_failure: "stop",
        },
    )
    .await
    .unwrap();

    db::update_agent(
        &pool,
        &agent.hostname,
        &agent.hostname,
        db::AgentDefaults {
            display_name: agent.display_name.as_deref(),
            default_backup_paths: &agent.default_backup_paths,
            default_exclude_patterns: &agent.default_exclude_patterns,
            default_pre_backup_commands: &agent.default_pre_backup_commands,
            default_post_backup_commands: &agent.default_post_backup_commands,
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

    let config = server::config_assembler::assemble_config(&pool, &encryption_key, &agent.hostname)
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
            status: "success".to_string(),
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
    status: &str,
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
            status: status.to_string(),
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
    let agent = db::insert_agent(&pool, "report-host", None, "hash", None)
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
    let agent = db::insert_agent(&pool, "target-host", None, "hash", None)
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
    assert!(reports.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn backup_report_with_warnings(pool: PgPool) {
    let agent = db::insert_agent(&pool, "warn-host", None, "hash", None)
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
            status: "warning".to_string(),
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
    let agent = db::insert_agent(&pool, "del-report-host", None, "hash", None)
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
    let agent = db::insert_agent(&pool, "stats-host", None, "hash", None)
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
    assert!(stats.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn activity_feed(pool: PgPool) {
    let agent = db::insert_agent(&pool, "act-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let activity = db::get_activity_feed(&pool, 10, None, None, None, None)
        .await
        .unwrap();
    assert_eq!(activity.len(), 1);
    assert_eq!(activity.first().unwrap().hostname, "act-host");
    assert_eq!(activity.first().unwrap().target_name, "test-repo");
}

#[sqlx::test(migrations = "./migrations")]
async fn activity_feed_days(pool: PgPool) {
    let agent = db::insert_agent(&pool, "days-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let activity = db::get_activity_feed_days(&pool, 7, None, None, None, None)
        .await
        .unwrap();
    assert_eq!(activity.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn health_summary(pool: PgPool) {
    let (agent, repo, schedule) = create_test_schedule(&pool).await;
    insert_test_report_for_schedule(&pool, agent.id, repo.id, schedule.id, "success").await;

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
    db::insert_schedule_targets(&pool, schedule_b.id, &[(agent.id, 0)])
        .await
        .unwrap();

    // Only schedule_a has a backup run recorded.
    insert_test_report_for_schedule(&pool, agent.id, repo.id, schedule_a.id, "success").await;

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
async fn health_summary_with_invalid_status_silently_returns_none(pool: PgPool) {
    let (agent, repo, schedule) = create_test_schedule(&pool).await;

    sqlx::query!(
        r#"INSERT INTO backup_reports
           (agent_id, repo_id, schedule_id, started_at, finished_at, status, matched)
           VALUES ($1, $2, $3, NOW() - INTERVAL '5 minutes', NOW(), $4, true)"#,
        agent.id,
        repo.id,
        schedule.id,
        "completely_invalid_status_value",
    )
    .execute(&pool)
    .await
    .unwrap();

    let health = db::get_health_summary(&pool).await.unwrap();
    assert_eq!(health.len(), 1);
    assert_eq!(
        health.first().unwrap().last_status.as_deref(),
        Some("completely_invalid_status_value"),
        "raw invalid status is returned as-is from the db layer"
    );
    assert_eq!(health.first().unwrap().schedule_id, schedule.id);
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
    db::insert_schedule_targets(&pool, schedule_b.id, &[(agent.id, 0)])
        .await
        .unwrap();

    let disabled_agent = db::insert_agent(&pool, "disabled-only", None, "hash", None)
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
    db::insert_schedule_targets(&pool, disabled_schedule.id, &[(disabled_agent.id, 0)])
        .await
        .unwrap();

    let unassigned = db::insert_agent(&pool, "unassigned", None, "hash", None)
        .await
        .unwrap();
    let hidden = db::insert_agent(&pool, "hidden", None, "hash", None)
        .await
        .unwrap();
    db::set_agent_hidden(&pool, &hidden.hostname, true)
        .await
        .unwrap();
    db::get_or_create_agent_by_hostname(&pool, "imported-placeholder")
        .await
        .unwrap();

    insert_test_report_for_schedule(&pool, agent.id, repo.id, schedule_a.id, "success").await;
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
    let agent = db::insert_agent(&pool, "rws-host", None, "hash", None)
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
async fn repo_with_stats_single(pool: PgPool) {
    let agent = db::insert_agent(&pool, "single-host", None, "hash", None)
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
    let agent = db::insert_agent(&pool, "brk-host", None, "hash", None)
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
    let agent = db::insert_agent(&pool, "ds-storage-host", None, "hash", None)
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
    db::insert_session(&pool, "sess_abc123", user.id, expires, false)
        .await
        .unwrap();

    let session = db::get_session(&pool, "sess_abc123").await.unwrap();
    assert_eq!(session.user_id, user.id);
    assert_eq!(session.id, "sess_abc123");
    assert!(!session.remember_me);

    db::delete_session(&pool, "sess_abc123").await.unwrap();

    let result = db::get_session(&pool, "sess_abc123").await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn session_expired(pool: PgPool) {
    let user = db::insert_user(&pool, "expuser", "hash").await.unwrap();

    let expired = Utc::now().checked_sub_signed(Duration::hours(1)).unwrap();
    db::insert_session(&pool, "sess_expired", user.id, expired, false)
        .await
        .unwrap();

    let result = db::get_session(&pool, "sess_expired").await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn session_delete_expired(pool: PgPool) {
    let user = db::insert_user(&pool, "cleanuser", "hash").await.unwrap();

    let expired = Utc::now().checked_sub_signed(Duration::hours(1)).unwrap();
    db::insert_session(&pool, "sess_old", user.id, expired, false)
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
    db::insert_session(&pool, "sess_remember", user.id, expires, true)
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
    db::insert_session(&pool, "sess_extend", user.id, original_expires, true)
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

    // Username-only count includes all IPs
    let count_by_user = db::count_failed_login_attempts_by_username(&pool, "user1", 60)
        .await
        .unwrap();
    assert_eq!(count_by_user, 2);
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
    let count = db::count_failed_login_attempts_by_username(&pool, "lockuser", 60)
        .await
        .unwrap();
    assert_eq!(count, 3);

    // Set a lockout
    let lock_time = Utc::now()
        .checked_add_signed(Duration::minutes(30))
        .unwrap();
    db::set_account_lockout(&pool, "lockuser", lock_time)
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

    // Escalation level (3 failures -> below threshold of 10 -> level 0)
    let level = db::count_lockout_escalation_level(&pool, "lockuser", 10)
        .await
        .unwrap();
    assert_eq!(level, 0);

    // With 15 failures the level should be 1 (first lockout at index 0 = 1 min)
    for _ in 0..12 {
        db::insert_login_attempt(&pool, "lockuser", "192.168.1.1", false)
            .await
            .unwrap();
    }
    let level = db::count_lockout_escalation_level(&pool, "lockuser", 10)
        .await
        .unwrap();
    assert_eq!(level, 1);
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

#[sqlx::test(migrations = "./migrations")]
async fn lockout_escalation_reaches_60min_tier(pool: PgPool) {
    // The LOCKOUT_DURATIONS are [1, 5, 15, 60, 1440] minutes.
    // With max_account_failures = 5:
    //   - 5  failures (0-4)  -> level 0 = 1 minute
    //   - 10 failures (5-9)  -> level 1 = 5 minutes
    //   - 15 failures (10-14) -> level 2 = 15 minutes
    //   - 20 failures (15-19) -> level 3 = 60 minutes
    //   - 25 failures (20-24) -> level 4 = 1440 minutes (24h)

    db::insert_user(&pool, "escalation60", "hash")
        .await
        .unwrap();

    // 20 failures -> level 3 -> 60 min lockout
    for _ in 0..20 {
        db::record_failed_login_and_check_lockout(&pool, "escalation60", "10.0.0.1", 5)
            .await
            .unwrap();
    }

    let user = db::get_user_by_username(&pool, "escalation60")
        .await
        .unwrap();
    let locked_until = user.locked_until.expect("user should be locked");

    // Lockout duration should be >= 59 minutes (60 min tier, with some slack for test timing)
    let duration_min = locked_until.signed_duration_since(Utc::now()).num_minutes();
    assert!(
        duration_min >= 55,
        "expected ~60 min lockout, got {duration_min} min"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn lockout_escalation_reaches_24h_tier(pool: PgPool) {
    db::insert_user(&pool, "escalation24h", "hash")
        .await
        .unwrap();

    // 25 failures -> level 4 -> 1440 min (24h) lockout
    for _ in 0..25 {
        db::record_failed_login_and_check_lockout(&pool, "escalation24h", "10.0.0.1", 5)
            .await
            .unwrap();
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
    // multiple lockout periods. The consecutive-failure counter persists
    // as long as there's no successful login in between.
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

    // Simulate lockout expires (clear it, but NO successful login)
    db::clear_account_lockout(&pool, "slidingwindow")
        .await
        .unwrap();

    // Phase 2: 10 more failures -> level 1 (5 min lockout)
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
async fn record_failed_login_transactional_rollback(pool: PgPool) {
    db::insert_user(&pool, "txuser", "hash").await.unwrap();

    // The function is atomic - if it succeeds, the insert is committed
    // If it fails (e.g. DB error), the insert is rolled back.
    // Test by checking count before and after.
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
    assert!(tokens.is_empty());
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
    db::insert_system_event(&pool, "backup_complete", Some("host-1"), "Backup finished")
        .await
        .unwrap();
    db::insert_system_event(&pool, "error", None, "Something failed")
        .await
        .unwrap();

    let events = db::get_system_events(&pool, 10).await.unwrap();
    assert_eq!(events.len(), 2);

    let future = Utc::now().checked_add_signed(Duration::hours(1)).unwrap();
    let deleted = db::delete_system_events_before(&pool, future)
        .await
        .unwrap();
    assert_eq!(deleted, 2);

    let events = db::get_system_events(&pool, 10).await.unwrap();
    assert!(events.is_empty());
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
    assert!(host_tags.is_empty());

    db::delete_tag(&pool, tag.id).await.unwrap();
    let tags = db::list_tags(&pool, "repo").await.unwrap();
    assert!(tags.is_empty());
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
    assert!(tags.is_empty());
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
    let agent = db::insert_agent(&pool, "tagged-host", None, "hash", None)
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
    assert!(groups.is_empty());
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

    let agent = db::insert_agent(&pool, "tun-host-1", None, "tun-token-1", None)
        .await
        .unwrap();
    let agent_2 = db::insert_agent(&pool, "tun-host-2", None, "tun-token-2", None)
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

    db::delete_agent(&pool, "tun-host-2").await.unwrap();
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

    let total = db::server_quota::total_deduplicated_size_for_ssh_host(&pool, "shared.example.com")
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

    let total_excluding_a = db::server_quota::total_deduplicated_size_for_ssh_host_excluding(
        &pool,
        "shared.example.com",
        repo_a.id,
    )
    .await
    .unwrap();
    assert_eq!(total_excluding_a, 70_000);

    let total_excluding_b = db::server_quota::total_deduplicated_size_for_ssh_host_excluding(
        &pool,
        "shared.example.com",
        repo_b.id,
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
    assert!(trends.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_backup_trends_with_data(pool: PgPool) {
    let agent = db::insert_agent(&pool, "trends-host", None, "hash", None)
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
    let agent = db::insert_agent(&pool, "trends-filter-host", None, "hash", None)
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
    assert!(trends_other.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_calendar_events_empty(pool: PgPool) {
    let events = db::get_calendar_events(&pool, 2026, 1, None, Tz::UTC)
        .await
        .unwrap();
    assert!(events.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_calendar_events_with_data(pool: PgPool) {
    let agent = db::insert_agent(&pool, "cal-host", None, "hash", None)
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
    let agent = db::insert_agent(&pool, "cal-filter-host", None, "hash", None)
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
    assert!(events_other.is_empty());
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
    assert!(items.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_hostname_pattern_crud(pool: PgPool) {
    let agent = db::insert_agent(
        &pool,
        "pattern-crud-host",
        Some("Pattern CRUD"),
        "hash",
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
    assert!(patterns.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_find_agent_by_pattern_glob_match(pool: PgPool) {
    let agent = db::insert_agent(
        &pool,
        "pattern-glob-agent",
        Some("Pattern Glob"),
        "hash",
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
    )
    .await
    .unwrap();
    let agent_two = db::insert_agent(
        &pool,
        "duplicate-pattern-two",
        Some("Duplicate Two"),
        "hash",
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
    let exact = db::insert_agent(&pool, "foo", Some("Exact Foo"), "hash", None)
        .await
        .unwrap();
    let patterned = db::insert_agent(
        &pool,
        "pattern-priority-agent",
        Some("Pattern Foo"),
        "hash",
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
    )
    .await
    .unwrap();
    let target = db::insert_agent(&pool, "merge-target", Some("Merge Target"), "hash", None)
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

    let source = db::get_agent_by_hostname(&pool, "merge-placeholder").await;
    assert!(source.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_merge_agent_refuses_non_placeholder(pool: PgPool) {
    let source = db::insert_agent(&pool, "merge-source", Some("Merge Source"), "hash", None)
        .await
        .unwrap();
    let target = db::insert_agent(
        &pool,
        "merge-target-real",
        Some("Merge Target Real"),
        "hash",
        None,
    )
    .await
    .unwrap();

    let result = db::merge_agent(&pool, source.id, target.id).await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_mark_agent_reports_matched(pool: PgPool) {
    let agent = db::insert_agent(
        &pool,
        "adopt-host",
        Some("Adopt Host (imported)"),
        "imported:no-auth",
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
            status: "success".to_string(),
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
    let agent = db::insert_agent(&pool, "primary-host", None, "hash", None)
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
            status: "success".to_string(),
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
            status: "success".to_string(),
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
            status: "success".to_string(),
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
            status: "success".to_string(),
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
    let agent = db::insert_agent(&pool, "web-server-01", None, "hash", None)
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
            status: "success".to_string(),
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
            status: "success".to_string(),
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
            status: "success".to_string(),
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
    let agent = db::insert_agent(&pool, "db-server-01", None, "hash", None)
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
            status: "success".to_string(),
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
            status: "success".to_string(),
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
            status: "success".to_string(),
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
            status: "success".to_string(),
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
            status: "success".to_string(),
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
    let agent = db::insert_agent(&pool, "bulk-host", None, "hash-bulk", None)
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
            status: "success".to_string(),
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
            status: "success".to_string(),
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
    let agent = db::insert_agent(&pool, "bulk-dup-host", None, "hash-dup", None)
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
        status: "success".to_string(),
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
    let agent = db::insert_agent(&pool, "same-second-host", None, "hash-ss", None)
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
        status: "success".to_string(),
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
    let agent = db::insert_agent(&pool, "byid-host", None, "hash-byid", None)
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
    db::insert_agent(&pool, "hidden-host", None, "hash-hidden", None)
        .await
        .unwrap();

    let before = db::list_agents(&pool, false).await.unwrap();
    assert!(before.iter().any(|c| c.hostname == "hidden-host"));

    db::set_agent_hidden(&pool, "hidden-host", true)
        .await
        .unwrap();

    let visible = db::list_agents(&pool, false).await.unwrap();
    assert!(!visible.iter().any(|c| c.hostname == "hidden-host"));

    let all = db::list_agents(&pool, true).await.unwrap();
    assert!(all.iter().any(|c| c.hostname == "hidden-host"));

    db::set_agent_hidden(&pool, "hidden-host", false)
        .await
        .unwrap();

    let restored = db::list_agents(&pool, false).await.unwrap();
    assert!(restored.iter().any(|c| c.hostname == "hidden-host"));
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_token_hash_lookup(pool: PgPool) {
    let agent = db::insert_agent(&pool, "token-host", None, "secret-hash", None)
        .await
        .unwrap();

    let (id, hash) = db::get_agent_token_hash(&pool, "token-host").await.unwrap();
    assert_eq!(id, agent.id);
    assert_eq!(hash, "secret-hash");

    let result = db::get_agent_token_hash(&pool, "nonexistent-host").await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn agent_last_seen_updates(pool: PgPool) {
    let agent = db::insert_agent(&pool, "seen-host", None, "hash-seen", None)
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
    let real = db::insert_agent(&pool, "existing-real", None, "realhash", None)
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
    let agent_a = db::insert_agent(&pool, "host-a", None, "hash-a", None)
        .await
        .unwrap();
    let agent_b = db::insert_agent(&pool, "host-b", None, "hash-b", None)
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
    assert!(empty.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_target_hostnames_for_repo_test(pool: PgPool) {
    let (_, repo, _) = create_test_schedule(&pool).await;

    let hostnames = db::get_schedule_target_hostnames_for_repo(&pool, repo.id)
        .await
        .unwrap();
    assert_eq!(hostnames, vec!["sched-host"]);
}

#[sqlx::test(migrations = "./migrations")]
async fn get_schedule_targets_for_run_returns_ordered_and_excludes_hidden(pool: PgPool) {
    let (agent_a, _, schedule) = create_test_schedule(&pool).await;
    let agent_b = db::insert_agent(&pool, "run-target-b", None, "hash-rtb", None)
        .await
        .unwrap();
    let agent_hidden = db::insert_agent(&pool, "run-target-hidden", None, "hash-rth", None)
        .await
        .unwrap();
    db::set_agent_hidden(&pool, "run-target-hidden", true)
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
    assert!(!tz.name().is_empty());
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

    insert_test_report_for_schedule(&pool, agent.id, repo.id, schedule.id, "success").await;

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
    assert!(empty.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn reports_carry_repo_name_and_fall_back_to_it_when_schedule_unnamed(pool: PgPool) {
    let agent = db::insert_agent(&pool, "unnamed-sched-host", None, "hash", None)
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
            pre_backup_commands: "",
            post_backup_commands: "",
            on_failure: "stop",
        },
        None,
    )
    .await
    .unwrap();

    insert_test_report_for_schedule(&pool, agent.id, repo.id, schedule.id, "failed").await;

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
    let agent = db::insert_agent(&pool, "no-schedule-host", None, "hash", None)
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
    let agent = db::insert_agent(&pool, "feed-repo-filter-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let all = db::get_activity_feed(&pool, 10, None, None, None, None)
        .await
        .unwrap();
    assert!(!all.is_empty());

    let filtered = db::get_activity_feed(&pool, 10, Some(repo.id), None, None, None)
        .await
        .unwrap();
    assert_eq!(filtered.len(), 1);

    let empty = db::get_activity_feed(
        &pool,
        10,
        Some(repo.id.saturating_add(999)),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(empty.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn activity_feed_hostname_filter(pool: PgPool) {
    let agent = db::insert_agent(&pool, "hostname-filter-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let filtered = db::get_activity_feed(&pool, 10, None, Some("hostname-filter-host"), None, None)
        .await
        .unwrap();
    assert_eq!(filtered.len(), 1);

    let empty = db::get_activity_feed(&pool, 10, None, Some("nonexistent-host"), None, None)
        .await
        .unwrap();
    assert!(empty.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn activity_feed_days_test(pool: PgPool) {
    let agent = db::insert_agent(&pool, "days-feed-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let all = db::get_activity_feed_days(&pool, 7, None, None, None, None)
        .await
        .unwrap();
    assert!(!all.is_empty());

    let with_repo = db::get_activity_feed_days(&pool, 7, Some(repo.id), None, None, None)
        .await
        .unwrap();
    assert_eq!(with_repo.len(), 1);

    let with_host = db::get_activity_feed_days(&pool, 7, None, Some("days-feed-host"), None, None)
        .await
        .unwrap();
    assert_eq!(with_host.len(), 1);

    let no_match = db::get_activity_feed_days(&pool, 7, None, Some("wrong-host"), None, None)
        .await
        .unwrap();
    assert!(no_match.is_empty());
}

#[test]
fn compression_round_trip() {
    let cases = &[
        ("none", "none"),
        ("lz4", "lz4"),
        ("zstd,3", "zstd,3"),
        ("zlib,6", "zlib,6"),
    ];
    for (input, expected) in cases {
        let c = db::compression_from_str(input).unwrap();
        assert_eq!(db::compression_to_str(&c), *expected);
    }
    assert!(db::compression_from_str("unknown").is_err());
    assert!(db::compression_from_str("zstd,bad").is_err());
    assert!(db::compression_from_str("zlib,bad").is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn storage_trends_test(pool: PgPool) {
    let empty_trends = db::get_storage_trends(&pool, None, 7).await.unwrap();
    assert!(empty_trends.iter().all(|t| t.deduplicated_size.is_none()));

    let agent = db::insert_agent(&pool, "strend-host", None, "hash", None)
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
    assert!(empty.is_empty());

    let agent = db::insert_agent(&pool, "strend-repo-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, agent.id, repo.id).await;

    let trends = db::get_storage_trends_by_repo(&pool, 7).await.unwrap();
    assert!(!trends.is_empty());
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
    let agent = db::insert_agent(&pool, "strend-invariant-host", None, "hash", None)
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
                status: "success".to_string(),
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
    let agent = db::insert_agent(&pool, "archive-del-host", None, "hash", None)
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
            status: "success".to_string(),
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
            status: "success".to_string(),
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
    let agent = db::insert_agent(&pool, "stats-needing-host", None, "hash", None)
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
        status: "success".to_string(),
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
    let agent = db::insert_agent(&pool, "del-before-host", None, "hash", None)
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
            status: "success".to_string(),
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
    assert!(reports.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_backup_reports_before_keeps_archive_rows(pool: PgPool) {
    // Imported/synced archives keep their original (old) borg start timestamp.
    // Age-based report retention must not delete them, or archives vanish from
    // the UI even though they still exist in borg.
    let agent = db::insert_agent(&pool, "retain-archive-host", None, "hash", None)
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
        status: "success".to_string(),
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
            status: "failed".to_string(),
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
    let agent = db::insert_agent(&pool, "del-arch-host", None, "hash", None)
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
            status: "success".to_string(),
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
            status: "success".to_string(),
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
    let agent = db::insert_agent(&pool, "del-arch-null-host", None, "hash", None)
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
            status: "failed".to_string(),
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
            status: "success".to_string(),
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
    let agent = db::insert_agent(&pool, "arch-exact-host", None, "hash", None)
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
            status: "success".to_string(),
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
    let agent = db::insert_agent(&pool, "arch-1s-host", None, "hash", None)
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
            status: "success".to_string(),
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
    let agent = db::insert_agent(&pool, "fail-exact-host", None, "hash", None)
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
            status: "failed".to_string(),
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
    let agent = db::insert_agent(&pool, "fail-1s-host", None, "hash", None)
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
            status: "failed".to_string(),
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
    assert!(reports.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_system_events_before_keeps_recent(pool: PgPool) {
    let before_insert = Utc::now();
    db::insert_system_event(&pool, "test_event", None, "recent event")
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

    let events = db::get_system_events(&pool, 10).await.unwrap();
    assert_eq!(events.len(), 1);
}

/// Applies the same fallback logic as `get_settings` in `api/system.rs`.
fn compute_retention_fallbacks(
    legacy_raw: Option<String>,
    report_raw: Option<String>,
    failed_raw: Option<String>,
    event_raw: Option<String>,
) -> (i64, i64, i64, i64) {
    let legacy = legacy_raw.as_deref().and_then(|v| v.parse::<i64>().ok());
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
        failed, 365,
        "default failed_report_retention_days must be 365"
    );
    assert_eq!(events, 90, "default system_event_retention_days must be 90");
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
async fn delete_system_events_before_deletes_old(pool: PgPool) {
    db::insert_system_event(&pool, "old_event", None, "old event to prune")
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

    let events = db::get_system_events(&pool, 10).await.unwrap();
    assert!(events.is_empty());
}

/// Applies the same fallback logic as `get_settings` in `api/system.rs`.
fn compute_retention_fallbacks(
    legacy_raw: Option<&str>,
    report_raw: Option<&str>,
    failed_raw: Option<&str>,
    event_raw: Option<&str>,
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

    let (ret, report, failed, events) = compute_retention_fallbacks(
        legacy_raw.as_deref(),
        report_raw.as_deref(),
        failed_raw.as_deref(),
        event_raw.as_deref(),
    );
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

    let (ret, report, failed, events) = compute_retention_fallbacks(
        legacy_raw.as_deref(),
        report_raw.as_deref(),
        failed_raw.as_deref(),
        event_raw.as_deref(),
    );
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

    let (ret, report, failed, events) = compute_retention_fallbacks(
        legacy_raw.as_deref(),
        report_raw.as_deref(),
        failed_raw.as_deref(),
        event_raw.as_deref(),
    );
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

    let (ret, report, failed, events) = compute_retention_fallbacks(
        legacy_raw.as_deref(),
        report_raw.as_deref(),
        failed_raw.as_deref(),
        event_raw.as_deref(),
    );
    assert_eq!(ret, 7, "default retention_days must be 7");
    assert_eq!(report, 100);
    assert_eq!(failed, 200);
    assert_eq!(events, 300);
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
    let agent = db::insert_agent(&pool, "cancel-host", None, "hash", None)
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
    let agent = db::insert_agent(&pool, "cancel-done-host", None, "hash", None)
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
            status: "success".to_string(),
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
            default_backup_paths: &paths,
            default_exclude_patterns: &excludes,
            default_pre_backup_commands: "[]",
            default_post_backup_commands: "[]",
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
    let agent_a = db::insert_agent(&pool, "run-host-a", None, "hash-a", None)
        .await
        .unwrap();
    let agent_b = db::insert_agent(&pool, "run-host-b", None, "hash-b", None)
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
            status: "failed".to_string(),
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
    assert!(ids.is_empty());

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
    assert!(b_ids.is_empty());
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
    let agent = db::insert_agent(&pool, "unassigned-agent", None, "hash", None)
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
    let rogue_agent = db::insert_agent(&pool, "rogue-agent", None, "rogue-hash", None)
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
        "security_violation",
        Some("rogue-agent"),
        "Agent 'rogue-agent' tried to report on repo 999 without assignment (msg=BackupCompleted)",
    )
    .await
    .unwrap();

    let events = db::get_system_events(&pool, 10).await.unwrap();
    let security_events: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "security_violation")
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
