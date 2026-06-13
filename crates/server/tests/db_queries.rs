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
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn client_insert_and_get(pool: PgPool) {
    let client = db::insert_agent(&pool, "test-host", Some("Test Host"), "hash123", None)
        .await
        .unwrap();

    assert_eq!(client.hostname, "test-host");
    assert_eq!(client.display_name.as_deref(), Some("Test Host"));
    assert!(client.agent_version.is_none());
    assert!(client.last_seen_at.is_none());

    let fetched = db::get_agent_by_hostname(&pool, "test-host").await.unwrap();
    assert_eq!(fetched.id, client.id);
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
            .all(|rows| rows[0].total_bytes >= rows[1].total_bytes)
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn client_not_found(pool: PgPool) {
    let result = db::get_agent_by_hostname(&pool, "nonexistent").await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn client_token_hash(pool: PgPool) {
    db::insert_agent(&pool, "token-host", None, "secret_hash", None)
        .await
        .unwrap();

    let (id, hash) = db::get_agent_token_hash(&pool, "token-host").await.unwrap();
    assert!(id > 0);
    assert_eq!(hash, "secret_hash");
}

#[sqlx::test(migrations = "./migrations")]
async fn client_update_last_seen(pool: PgPool) {
    let client = db::insert_agent(&pool, "seen-host", None, "hash", None)
        .await
        .unwrap();

    db::update_last_seen(&pool, client.id).await.unwrap();

    let fetched = db::get_agent_by_hostname(&pool, "seen-host").await.unwrap();
    assert!(fetched.last_seen_at.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn client_update_last_seen_and_version(pool: PgPool) {
    let client = db::insert_agent(&pool, "ver-host", None, "hash", None)
        .await
        .unwrap();

    db::update_last_seen_and_version(&pool, client.id, "2.0.0", None, None, None)
        .await
        .unwrap();

    let fetched = db::get_agent_by_hostname(&pool, "ver-host").await.unwrap();
    assert_eq!(fetched.agent_version.as_deref(), Some("2.0.0"));
    assert!(fetched.last_seen_at.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn client_update_last_seen_by_hostname(pool: PgPool) {
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
async fn client_list(pool: PgPool) {
    db::insert_agent(&pool, "alpha", None, "h1", None)
        .await
        .unwrap();
    db::insert_agent(&pool, "beta", None, "h2", None)
        .await
        .unwrap();

    let clients = db::list_agents(&pool, false).await.unwrap();
    assert_eq!(clients.len(), 2);
    assert_eq!(clients[0].hostname, "alpha");
    assert_eq!(clients[1].hostname, "beta");
}

#[sqlx::test(migrations = "./migrations")]
async fn client_update(pool: PgPool) {
    db::insert_agent(&pool, "upd-host", Some("Old Name"), "hash", None)
        .await
        .unwrap();

    let updated = db::update_agent(&pool, "upd-host", "upd-host", Some("New Name"), &[], &[])
        .await
        .unwrap();
    assert_eq!(updated.display_name.as_deref(), Some("New Name"));
}

#[sqlx::test(migrations = "./migrations")]
async fn client_regenerate_token(pool: PgPool) {
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
async fn client_delete(pool: PgPool) {
    db::insert_agent(&pool, "del-host", None, "hash", None)
        .await
        .unwrap();

    db::delete_agent(&pool, "del-host").await.unwrap();

    let result = db::get_agent_by_hostname(&pool, "del-host").await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn client_delete_not_found(pool: PgPool) {
    let result = db::delete_agent(&pool, "ghost").await;
    assert!(result.is_err());
}

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
        },
    )
    .await
    .unwrap()
}

/// Sets a repo's authoritative `borg info` statistics. Values mirror
/// `insert_test_report` so stat assertions stay consistent now that repo
/// size/archive numbers come from `repos.info_*` rather than backup reports.
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

    let quota = db::quota::upsert_quota(&pool, repo.id, Some(100), Some(200), true)
        .await
        .unwrap();
    assert_eq!(quota.repo_id, repo.id);
    assert_eq!(quota.warn_bytes, Some(100));
    assert_eq!(quota.critical_bytes, Some(200));
    assert!(quota.enabled);

    let fetched = db::quota::get_quota(&pool, repo.id).await.unwrap();
    let fetched = fetched.expect("quota should exist");
    assert_eq!(fetched.repo_id, repo.id);
    assert_eq!(fetched.warn_bytes, Some(100));
    assert_eq!(fetched.critical_bytes, Some(200));
}

#[sqlx::test(migrations = "./migrations")]
async fn test_quota_disabled(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let quota = db::quota::upsert_quota(&pool, repo.id, Some(100), Some(200), false)
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
    assert_eq!(items[0].username, "admin");
    assert_eq!(items[0].action, "created_repo");
    assert_eq!(items[0].target_type.as_deref(), Some("repo"));
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
    assert_eq!(items[0].action, "action-2");
    assert_eq!(items[1].action, "action-1");
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
    assert_eq!(items[0].action, "repo_created");
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_name(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let name = db::get_repo_name(&pool, repo.id).await.unwrap();
    assert_eq!(name, "test-repo");
}

#[sqlx::test(migrations = "./migrations")]
async fn tunnel_crud(pool: PgPool) {
    let client = db::insert_agent(&pool, "tunnel-host", None, "hash", None)
        .await
        .unwrap();

    let tunnel = db::insert_tunnel(
        &pool,
        &NewSshTunnel {
            agent_id: client.id,
            ssh_host: "repo.example.com".to_string(),
            ssh_user: "borg".to_string(),
            ssh_port: Some(2222),
            tunnel_port: 2200,
            enabled: Some(true),
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

    let by_client = db::get_tunnel_by_agent_id(&pool, client.id).await.unwrap();
    assert_eq!(by_client.id, tunnel.id);

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
    let client = db::insert_agent(&pool, "def-host", None, "hash", None)
        .await
        .unwrap();

    let tunnel = db::insert_tunnel(
        &pool,
        &NewSshTunnel {
            agent_id: client.id,
            ssh_host: "host.com".to_string(),
            ssh_user: "user".to_string(),
            ssh_port: None,
            tunnel_port: 3000,
            enabled: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(tunnel.ssh_port, 22);
    assert!(tunnel.enabled);
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

async fn create_test_schedule(pool: &PgPool) -> (AgentRow, RepoRow, ScheduleRow) {
    let client = db::insert_agent(pool, "sched-host", None, "hash", None)
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
    db::insert_schedule_targets(pool, schedule.id, &[(client.id, 0)])
        .await
        .unwrap();
    (client, repo, schedule)
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

    let result = db::get_backup_schedule_for_hostname_repo(&pool, "sched-host", repo.id)
        .await
        .unwrap();
    assert!(result.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_list_for_repo(pool: PgPool) {
    let (_, repo, _) = create_test_schedule(&pool).await;

    let schedules = db::list_schedules_for_repo(&pool, repo.id).await.unwrap();
    assert_eq!(schedules.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn schedule_list_for_client(pool: PgPool) {
    let (client, _, _) = create_test_schedule(&pool).await;

    let schedules = db::list_schedules_for_agent(&pool, client.id)
        .await
        .unwrap();
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
    let past = now - Duration::hours(1);

    db::set_next_run_at(&pool, schedule.id, past).await.unwrap();

    let due = db::list_due_schedules(&pool, now).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].schedule_id, schedule.id);

    let future = now + Duration::hours(3);
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
async fn schedule_client_hostname(pool: PgPool) {
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
    assert_eq!(sources[0], "/home");
    assert_eq!(sources[1], "/etc");

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
    let (client, _, schedule) = create_test_schedule(&pool).await;

    let client2 = db::insert_agent(&pool, "host-two", None, "hash2", None)
        .await
        .unwrap();

    db::insert_backup_source_for_schedule(&pool, schedule.id, "/shared", 0)
        .await
        .unwrap();

    db::insert_backup_source_for_schedule_agent(&pool, schedule.id, client.id, "/home/one", 0)
        .await
        .unwrap();
    db::insert_backup_source_for_schedule_agent(&pool, schedule.id, client.id, "/var/one", 1)
        .await
        .unwrap();
    db::insert_backup_source_for_schedule_agent(&pool, schedule.id, client2.id, "/data/two", 0)
        .await
        .unwrap();

    let schedule_level = db::list_backup_sources_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(schedule_level, vec!["/shared"]);

    let client1_sources = db::list_backup_sources_for_schedule_agent(&pool, schedule.id, client.id)
        .await
        .unwrap();
    assert_eq!(client1_sources, vec!["/home/one", "/var/one"]);

    let client2_sources =
        db::list_backup_sources_for_schedule_agent(&pool, schedule.id, client2.id)
            .await
            .unwrap();
    assert_eq!(client2_sources, vec!["/data/two"]);

    let all_per_agent = db::list_all_per_agent_backup_sources_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(all_per_agent.len(), 2);
    assert_eq!(all_per_agent[0].agent_id, client.id);
    assert_eq!(all_per_agent[0].paths, vec!["/home/one", "/var/one"]);
    assert_eq!(all_per_agent[1].agent_id, client2.id);
    assert_eq!(all_per_agent[1].paths, vec!["/data/two"]);

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
    let (client, _, schedule) = create_test_schedule(&pool).await;

    let client2 = db::insert_agent(&pool, "host-two-exc", None, "hash2exc", None)
        .await
        .unwrap();

    db::upsert_per_agent_excludes_raw(&pool, schedule.id, client.id, "*.tmp\n*.cache")
        .await
        .unwrap();
    db::upsert_per_agent_excludes_raw(&pool, schedule.id, client2.id, "*.bak")
        .await
        .unwrap();

    let all_per_agent = db::list_all_per_agent_excludes_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(all_per_agent.len(), 2);
    assert_eq!(all_per_agent[0].agent_id, client.id);
    assert_eq!(all_per_agent[0].raw_text, "*.tmp\n*.cache");
    assert_eq!(all_per_agent[1].agent_id, client2.id);
    assert_eq!(all_per_agent[1].raw_text, "*.bak");

    // Upsert updates existing row
    db::upsert_per_agent_excludes_raw(&pool, schedule.id, client.id, "*.tmp\n*.cache\n\n# new")
        .await
        .unwrap();
    let all_per_agent = db::list_all_per_agent_excludes_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(all_per_agent[0].raw_text, "*.tmp\n*.cache\n\n# new");

    db::delete_per_agent_excludes_for_schedule(&pool, schedule.id)
        .await
        .unwrap();

    let all_per_agent = db::list_all_per_agent_excludes_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert!(all_per_agent.is_empty());
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
    let (client, _, schedule) = create_test_schedule(&pool).await;

    let raw = "# Cache\n*.cache\n\n# Runtime\n/proc";
    db::upsert_per_agent_excludes_raw(&pool, schedule.id, client.id, raw)
        .await
        .unwrap();

    let all = db::list_all_per_agent_excludes_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].raw_text, raw);
}

#[sqlx::test(migrations = "./migrations")]
async fn per_agent_excludes_upsert_replaces_existing(pool: PgPool) {
    let (client, _, schedule) = create_test_schedule(&pool).await;

    db::upsert_per_agent_excludes_raw(&pool, schedule.id, client.id, "first")
        .await
        .unwrap();
    db::upsert_per_agent_excludes_raw(&pool, schedule.id, client.id, "second\n\n# comment")
        .await
        .unwrap();

    let all = db::list_all_per_agent_excludes_for_schedule(&pool, schedule.id)
        .await
        .unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].raw_text, "second\n\n# comment");
}

#[sqlx::test(migrations = "./migrations")]
async fn config_assembly_parses_raw_excludes_into_effective_patterns(pool: PgPool) {
    let encryption_key = shared::crypto::derive_key(b"test-assembly-key-for-excludes").unwrap();
    let (client, repo, _schedule) = create_test_schedule(&pool).await;

    // Global excludes: blank lines and comments should be stripped
    db::set_global_excludes_raw(&pool, "# system\n/proc\n/sys\n\n# cache\n*.cache")
        .await
        .unwrap();

    // Schedule-level excludes: same
    db::update_schedule(
        &pool,
        _schedule.id,
        &ScheduleParams {
            name: "test-schedule",
            schedule_type: "backup",
            cron_expression: "0 3 * * *",
            enabled: true,
            canary_enabled: false,
            exclude_patterns_raw: "# logs\n*.log\n\n*.tmp",
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
    db::insert_backup_source_for_schedule(&pool, _schedule.id, "/home", 0)
        .await
        .unwrap();

    // Enable the repo so it is reachable
    let _ = sqlx::query("UPDATE repos SET enabled = true WHERE id = $1")
        .bind(repo.id)
        .execute(&pool)
        .await
        .unwrap();

    let config =
        server::config_assembler::assemble_config(&pool, &encryption_key, &client.hostname)
            .await
            .unwrap();

    assert_eq!(config.repos[0].ssh_host_key, "ssh-ed25519 AAAATEST");

    let patterns: Vec<&str> = config.repos[0].schedules[0]
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

async fn insert_test_report(pool: &PgPool, agent_id: i64, repo_id: i64) {
    let now = Utc::now();
    db::insert_backup_report(
        pool,
        &InsertReportParams {
            agent_id,
            repo_id,
            schedule_id: None,
            started_at: now - Duration::minutes(5),
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
            started_at: now - Duration::minutes(5),
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
    let client = db::insert_agent(&pool, "report-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;

    let reports = db::list_reports_for_agent(&pool, client.id, None, 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].status, "success");
    assert_eq!(reports[0].original_size, 1_000_000);
    assert_eq!(reports[0].compressed_size, 500_000);
    assert_eq!(reports[0].deduplicated_size, 250_000);
    assert_eq!(reports[0].files_processed, 1000);
    assert_eq!(reports[0].duration_secs, 300);
    assert_eq!(reports[0].borg_version.as_deref(), Some("1.4.0"));
}

#[sqlx::test(migrations = "./migrations")]
async fn backup_report_list_with_target(pool: PgPool) {
    let client = db::insert_agent(&pool, "target-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;

    let reports = db::list_reports_for_agent(&pool, client.id, Some("test-repo"), 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);

    let reports = db::list_reports_for_agent(&pool, client.id, Some("nonexistent"), 10)
        .await
        .unwrap();
    assert!(reports.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn backup_report_with_warnings(pool: PgPool) {
    let client = db::insert_agent(&pool, "warn-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: client.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now - Duration::minutes(5),
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

    let reports = db::list_reports_for_agent(&pool, client.id, None, 10)
        .await
        .unwrap();
    assert_eq!(reports[0].warnings.len(), 2);
    assert_eq!(reports[0].error_message.as_deref(), Some("partial failure"));
}

#[sqlx::test(migrations = "./migrations")]
async fn backup_report_delete_before(pool: PgPool) {
    let client = db::insert_agent(&pool, "del-report-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;

    let future = Utc::now() + Duration::hours(1);
    let deleted = db::delete_backup_reports_before(&pool, future)
        .await
        .unwrap();
    assert_eq!(deleted, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn storage_stats_with_sum(pool: PgPool) {
    let client = db::insert_agent(&pool, "stats-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;
    insert_test_report(&pool, client.id, repo.id).await;

    let stats = db::get_storage_stats(&pool).await.unwrap();
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].hostname, "stats-host");
    assert_eq!(stats[0].total_original_size, 2_000_000);
    assert_eq!(stats[0].total_compressed_size, 1_000_000);
    assert_eq!(stats[0].total_deduplicated_size, 500_000);
    assert_eq!(stats[0].report_count, 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn storage_stats_empty(pool: PgPool) {
    let stats = db::get_storage_stats(&pool).await.unwrap();
    assert!(stats.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn activity_feed(pool: PgPool) {
    let client = db::insert_agent(&pool, "act-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;

    let activity = db::get_activity_feed(&pool, 10, None, None, None, None)
        .await
        .unwrap();
    assert_eq!(activity.len(), 1);
    assert_eq!(activity[0].hostname, "act-host");
    assert_eq!(activity[0].target_name, "test-repo");
}

#[sqlx::test(migrations = "./migrations")]
async fn activity_feed_days(pool: PgPool) {
    let client = db::insert_agent(&pool, "days-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;

    let activity = db::get_activity_feed_days(&pool, 7, None, None, None, None)
        .await
        .unwrap();
    assert_eq!(activity.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn health_summary(pool: PgPool) {
    let (client, repo, schedule) = create_test_schedule(&pool).await;
    insert_test_report_for_schedule(&pool, client.id, repo.id, schedule.id, "success").await;

    let health = db::get_health_summary(&pool).await.unwrap();
    assert_eq!(health.len(), 1);
    assert_eq!(health[0].hostname, "sched-host");
    assert_eq!(health[0].schedule_id, schedule.id);
    assert_eq!(health[0].last_status.as_deref(), Some("success"));
}

/// Two schedules that share the same repository and client must report
/// independent health: a backup run for one schedule must not surface as the
/// status of the other.
#[sqlx::test(migrations = "./migrations")]
async fn health_summary_is_per_schedule(pool: PgPool) {
    let (client, repo, schedule_a) = create_test_schedule(&pool).await;
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
    db::insert_schedule_targets(&pool, schedule_b.id, &[(client.id, 0)])
        .await
        .unwrap();

    // Only schedule_a has a backup run recorded.
    insert_test_report_for_schedule(&pool, client.id, repo.id, schedule_a.id, "success").await;

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
async fn dashboard_queries_use_authoritative_assignments_and_exclude_placeholders(pool: PgPool) {
    let (client, repo, schedule_a) = create_test_schedule(&pool).await;
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
    db::insert_schedule_targets(&pool, schedule_b.id, &[(client.id, 0)])
        .await
        .unwrap();

    let disabled_client = db::insert_agent(&pool, "disabled-only", None, "hash", None)
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
    db::insert_schedule_targets(&pool, disabled_schedule.id, &[(disabled_client.id, 0)])
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

    insert_test_report_for_schedule(&pool, client.id, repo.id, schedule_a.id, "success").await;
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
            .filter(|target| target.agent_id == client.id)
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
        .find(|host| host.agent_id == disabled_client.id)
        .unwrap();
    assert_eq!(disabled.enabled_assignment_count, 0);
    assert_eq!(disabled.disabled_assignment_count, 1);
    let unassigned = hosts
        .iter()
        .find(|host| host.agent_id == unassigned.id)
        .unwrap();
    assert_eq!(unassigned.enabled_assignment_count, 0);

    let upcoming = db::dashboard::upcoming_schedules(&pool).await.unwrap();
    assert_eq!(upcoming.len(), 1);
    assert_eq!(upcoming[0].schedule_id, schedule_a.id);
    assert_eq!(upcoming[0].target_count, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn dashboard_repository_capacity_uses_repo_stats_and_quota(pool: PgPool) {
    let repo = create_test_repo(&pool).await;
    set_test_repo_info_stats(&pool, repo.id, 1).await;
    db::quota::upsert_quota(&pool, repo.id, Some(200_000), Some(300_000), true)
        .await
        .unwrap();

    let repositories = db::dashboard::repositories(&pool).await.unwrap();
    assert_eq!(repositories.len(), 1);
    assert_eq!(repositories[0].deduplicated_size, 250_000);
    assert_eq!(repositories[0].warn_bytes, Some(200_000));
    assert_eq!(repositories[0].critical_bytes, Some(300_000));
    assert_eq!(repositories[0].enabled_schedule_count, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn repos_with_stats(pool: PgPool) {
    let client = db::insert_agent(&pool, "rws-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;
    set_test_repo_info_stats(&pool, repo.id, 1).await;

    let repos = db::list_repos_with_stats(&pool).await.unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].name, "test-repo");
    assert_eq!(repos[0].archive_count, 1);
    assert_eq!(repos[0].total_original_size, 1_000_000);
    assert_eq!(repos[0].total_compressed_size, 500_000);
    assert_eq!(repos[0].total_deduplicated_size, 250_000);
    assert_eq!(repos[0].client_count, 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn repos_with_stats_empty(pool: PgPool) {
    create_test_repo(&pool).await;

    let repos = db::list_repos_with_stats(&pool).await.unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].total_original_size, 0);
    assert_eq!(repos[0].total_deduplicated_size, 0);
    assert_eq!(repos[0].archive_count, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn repo_with_stats_single(pool: PgPool) {
    let client = db::insert_agent(&pool, "single-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;
    set_test_repo_info_stats(&pool, repo.id, 1).await;

    let result = db::get_repo_with_stats(&pool, repo.id).await.unwrap();
    assert_eq!(result.total_deduplicated_size, 250_000);
}

#[sqlx::test(migrations = "./migrations")]
async fn storage_breakdown(pool: PgPool) {
    let client = db::insert_agent(&pool, "brk-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;
    set_test_repo_info_stats(&pool, repo.id, 1).await;

    let breakdown = db::get_storage_breakdown(&pool).await.unwrap();
    assert_eq!(breakdown.len(), 1);
    assert_eq!(breakdown[0].name, "test-repo");
    assert_eq!(breakdown[0].deduplicated_size, 250_000);
}

/// Repos are returned in descending `info_deduplicated_size` order and
/// compressed_size is also sourced from the info columns.
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
    assert_eq!(breakdown[0].name, "large-repo");
    assert_eq!(breakdown[0].deduplicated_size, 400_000);
    assert_eq!(breakdown[0].compressed_size, 800_000);
    assert_eq!(breakdown[1].name, "test-repo");
    assert_eq!(breakdown[1].deduplicated_size, 100_000);
}

/// A repo that has never had `update_repo_info_stats` called must return zeros
/// without an error (columns default to 0).
#[sqlx::test(migrations = "./migrations")]
async fn storage_breakdown_repo_with_no_info_stats(pool: PgPool) {
    create_test_repo(&pool).await;

    let breakdown = db::get_storage_breakdown(&pool).await.unwrap();
    assert_eq!(breakdown.len(), 1);
    assert_eq!(breakdown[0].compressed_size, 0);
    assert_eq!(breakdown[0].deduplicated_size, 0);
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
    let (client, repo, _) = create_test_schedule(&pool).await;
    insert_test_report(&pool, client.id, repo.id).await;

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
/// `repos.info_deduplicated_size` rather than backup_reports.
#[sqlx::test(migrations = "./migrations")]
async fn dashboard_summary_total_storage_from_repo_info(pool: PgPool) {
    let client = db::insert_agent(&pool, "ds-storage-host", None, "hash", None)
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
        },
    )
    .await
    .unwrap();

    insert_test_report(&pool, client.id, repo1.id).await;
    insert_test_report(&pool, client.id, repo2.id).await;

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
    let user = db::insert_user(&pool, "testuser", "hashed_pw", "admin")
        .await
        .unwrap();
    assert_eq!(user.username, "testuser");
    assert_eq!(user.role, "admin");
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
    db::insert_user(&pool, "pwuser", "the_hash", "user")
        .await
        .unwrap();

    let (user, hash) = db::get_user_password_hash(&pool, "pwuser").await.unwrap();
    assert_eq!(user.username, "pwuser");
    assert_eq!(hash, "the_hash");
}

#[sqlx::test(migrations = "./migrations")]
async fn user_update_role(pool: PgPool) {
    let user = db::insert_user(&pool, "roleuser", "hash", "user")
        .await
        .unwrap();

    let updated = db::update_user_role(&pool, user.id, "admin").await.unwrap();
    assert_eq!(updated.role, "admin");
}

#[sqlx::test(migrations = "./migrations")]
async fn user_update_password(pool: PgPool) {
    let user = db::insert_user(&pool, "passuser", "old_hash", "user")
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
    let user = db::insert_user(&pool, "loginuser", "hash", "user")
        .await
        .unwrap();

    db::update_last_login(&pool, user.id).await.unwrap();

    let fetched = db::get_user_by_id(&pool, user.id).await.unwrap();
    assert!(fetched.last_login_at.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn user_delete(pool: PgPool) {
    let user = db::insert_user(&pool, "deluser", "hash", "user")
        .await
        .unwrap();

    db::delete_user(&pool, user.id).await.unwrap();

    let result = db::get_user_by_id(&pool, user.id).await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn user_preferences(pool: PgPool) {
    let user = db::insert_user(&pool, "prefuser", "hash", "user")
        .await
        .unwrap();

    let prefs = serde_json::json!({"theme": "dark", "lang": "en"});
    db::set_user_preferences(&pool, user.id, &prefs)
        .await
        .unwrap();

    let fetched = db::get_user_preferences(&pool, user.id).await.unwrap();
    assert_eq!(fetched["theme"], "dark");
    assert_eq!(fetched["lang"], "en");
}

#[sqlx::test(migrations = "./migrations")]
async fn session_crud(pool: PgPool) {
    let user = db::insert_user(&pool, "sessuser", "hash", "user")
        .await
        .unwrap();

    let expires = Utc::now() + Duration::hours(24);
    db::insert_session(&pool, "sess_abc123", user.id, expires)
        .await
        .unwrap();

    let session = db::get_session(&pool, "sess_abc123").await.unwrap();
    assert_eq!(session.user_id, user.id);
    assert_eq!(session.id, "sess_abc123");

    db::delete_session(&pool, "sess_abc123").await.unwrap();

    let result = db::get_session(&pool, "sess_abc123").await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn session_expired(pool: PgPool) {
    let user = db::insert_user(&pool, "expuser", "hash", "user")
        .await
        .unwrap();

    let expired = Utc::now() - Duration::hours(1);
    db::insert_session(&pool, "sess_expired", user.id, expired)
        .await
        .unwrap();

    let result = db::get_session(&pool, "sess_expired").await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn session_delete_expired(pool: PgPool) {
    let user = db::insert_user(&pool, "cleanuser", "hash", "user")
        .await
        .unwrap();

    let expired = Utc::now() - Duration::hours(1);
    db::insert_session(&pool, "sess_old", user.id, expired)
        .await
        .unwrap();

    let deleted = db::delete_expired_sessions(&pool).await.unwrap();
    assert_eq!(deleted, 1);
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
async fn api_token_crud(pool: PgPool) {
    let user = db::insert_user(&pool, "tokenuser", "hash", "user")
        .await
        .unwrap();

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
    let user = db::insert_user(&pool, "permuser", "hash", "user")
        .await
        .unwrap();
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

    let future = Utc::now() + Duration::hours(1);
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

    assert_eq!(created.repo_id, repo.id);
    assert_eq!(created.archive_name, "archive-1");
    assert_eq!(created.tag, "nightly");
    assert!(created.created_by.is_none());

    let tags = db::tags::list_tags_for_archive(&pool, repo.id, "archive-1")
        .await
        .unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].tag, "nightly");
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
    let client = db::insert_agent(&pool, "tagged-host", None, "hash", None)
        .await
        .unwrap();
    let tag = db::insert_tag(&pool, "critical", "#f00", "agent")
        .await
        .unwrap();

    db::set_agent_tags(&pool, client.id, &[tag.id])
        .await
        .unwrap();

    let tags = db::list_tags_for_agent(&pool, client.id).await.unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].name, "critical");

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
    let user1 = db::insert_user(&pool, "grp-user1", "hash", "user")
        .await
        .unwrap();
    let user2 = db::insert_user(&pool, "grp-user2", "hash", "user")
        .await
        .unwrap();
    let group = db::insert_group(&pool, "team", None).await.unwrap();

    db::set_group_members(&pool, group.id, &[user1.id, user2.id])
        .await
        .unwrap();

    let members = db::list_group_members(&pool, group.id).await.unwrap();
    assert_eq!(members.len(), 2);

    let user_groups = db::list_user_groups(&pool, user1.id).await.unwrap();
    assert_eq!(user_groups.len(), 1);
    assert_eq!(user_groups[0].name, "team");

    let shared = db::user_shares_group_with(&pool, user1.id, user2.id)
        .await
        .unwrap();
    assert!(shared);

    let user3 = db::insert_user(&pool, "grp-user3", "hash", "user")
        .await
        .unwrap();
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
    assert_eq!(roles.len(), initial_count + 1);

    db::delete_role(&pool, role.id).await.unwrap();
    let roles = db::list_roles(&pool).await.unwrap();
    assert_eq!(roles.len(), initial_count);
}

#[sqlx::test(migrations = "./migrations")]
async fn user_roles_and_effective_permissions(pool: PgPool) {
    let user = db::insert_user(&pool, "rbac-user", "hash", "user")
        .await
        .unwrap();

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
async fn repos_for_client(pool: PgPool) {
    let (client, repo, _) = create_test_schedule(&pool).await;

    let repos = db::list_repos_for_agent(&pool, client.id).await.unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].id, repo.id);

    let public_repos = db::list_repos_for_agent_public(&pool, client.id)
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
        .bind(1_i32)
        .execute(&pool)
        .await
        .unwrap();

    let sources = db::list_backup_sources_for_repo(&pool, repo.id)
        .await
        .unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0], "/data");
}

#[sqlx::test(migrations = "./migrations")]
async fn ssh_tunnel_crud(pool: PgPool) {
    use server::error::ApiError;

    let client = db::insert_agent(&pool, "tun-host-1", None, "tun-token-1", None)
        .await
        .unwrap();
    let client_2 = db::insert_agent(&pool, "tun-host-2", None, "tun-token-2", None)
        .await
        .unwrap();

    let tunnel = db::insert_tunnel(
        &pool,
        &db::NewSshTunnel {
            agent_id: client.id,
            ssh_host: "repo.example.com".to_string(),
            ssh_user: "borg".to_string(),
            ssh_port: Some(2222),
            tunnel_port: 2200,
            enabled: Some(true),
        },
    )
    .await
    .unwrap();

    assert_eq!(tunnel.agent_id, client.id);
    assert_eq!(tunnel.ssh_host, "repo.example.com");
    assert_eq!(tunnel.ssh_user, "borg");
    assert_eq!(tunnel.ssh_port, 2222);
    assert_eq!(tunnel.tunnel_port, 2200);
    assert!(tunnel.enabled);

    let by_id = db::get_tunnel_by_id(&pool, tunnel.id).await.unwrap();
    assert_eq!(by_id.id, tunnel.id);

    let by_agent_id = db::get_tunnel_by_agent_id(&pool, client.id).await.unwrap();
    assert_eq!(by_agent_id.id, tunnel.id);

    let enabled_tunnels = db::list_enabled_tunnels(&pool).await.unwrap();
    assert_eq!(enabled_tunnels.len(), 1);
    assert_eq!(enabled_tunnels[0].id, tunnel.id);

    let updated = db::update_tunnel(
        &pool,
        tunnel.id,
        &db::UpdateSshTunnel {
            ssh_host: Some("repo.internal".to_string()),
            ssh_user: None,
            ssh_port: Some(2022),
            tunnel_port: Some(2201),
            enabled: Some(false),
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
    assert_eq!(all_tunnels[0].id, tunnel.id);

    db::delete_tunnel(&pool, tunnel.id).await.unwrap();
    assert!(matches!(
        db::get_tunnel_by_id(&pool, tunnel.id).await,
        Err(ApiError::NotFound(_))
    ));

    let tunnel_2 = db::insert_tunnel(
        &pool,
        &db::NewSshTunnel {
            agent_id: client_2.id,
            ssh_host: "repo2.example.com".to_string(),
            ssh_user: "borg".to_string(),
            ssh_port: None,
            tunnel_port: 2300,
            enabled: None,
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

    let quota = db::quota::upsert_quota(&pool, repo.id, Some(100), Some(500), true)
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

    db::quota::upsert_quota(&pool, repo.id, Some(100), Some(200), true)
        .await
        .unwrap();

    let updated = db::quota::upsert_quota(&pool, repo.id, Some(500), Some(1000), false)
        .await
        .unwrap();

    assert_eq!(updated.warn_bytes, Some(500));
    assert_eq!(updated.critical_bytes, Some(1000));
    assert!(!updated.enabled);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_quota_get_nonexistent(pool: PgPool) {
    let repo = create_test_repo(&pool).await;

    let result = db::quota::get_quota(&pool, repo.id).await.unwrap();
    assert!(result.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_backup_trends_empty(pool: PgPool) {
    let trends = db::get_backup_trends(&pool, None, 30).await.unwrap();
    assert!(trends.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_backup_trends_with_data(pool: PgPool) {
    let client = db::insert_agent(&pool, "trends-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;

    let trends = db::get_backup_trends(&pool, None, 30).await.unwrap();
    assert_eq!(trends.len(), 1);
    assert_eq!(trends[0].backup_count, 1);
    assert!(trends[0].original_size > 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn test_backup_trends_filtered_by_repo(pool: PgPool) {
    let client = db::insert_agent(&pool, "trends-filter-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;

    let trends = db::get_backup_trends(&pool, Some(repo.id), 30)
        .await
        .unwrap();
    assert_eq!(trends.len(), 1);

    let trends_other = db::get_backup_trends(&pool, Some(repo.id + 999), 30)
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
    let client = db::insert_agent(&pool, "cal-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;

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
    assert_eq!(events[0].event_type, "backup");
    assert_eq!(events[0].status, "success");
    assert_eq!(events[0].repo_name, "test-repo");
}

#[sqlx::test(migrations = "./migrations")]
async fn test_calendar_events_filtered_by_repo(pool: PgPool) {
    let client = db::insert_agent(&pool, "cal-filter-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;

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
        Some(repo.id + 999),
        Tz::UTC,
    )
    .await
    .unwrap();
    assert!(events_other.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_enabled_schedules_for_calendar(pool: PgPool) {
    let (_client, _repo, _schedule) = create_test_schedule(&pool).await;

    let schedules = db::get_enabled_schedules_for_calendar(&pool).await.unwrap();
    assert_eq!(schedules.len(), 1);
    assert!(schedules[0].enabled);
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
            filter_from: Some(now - Duration::hours(1)),
            filter_to: Some(now + Duration::hours(1)),
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
            filter_from: Some(now + Duration::hours(1)),
            filter_to: Some(now + Duration::hours(2)),
        },
    )
    .await
    .unwrap();

    assert_eq!(total, 0);
    assert!(items.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_hostname_pattern_crud(pool: PgPool) {
    let client = db::insert_agent(
        &pool,
        "pattern-crud-host",
        Some("Pattern CRUD"),
        "hash",
        None,
    )
    .await
    .unwrap();

    let pattern = patterns::add_hostname_pattern(&pool, client.id, "crud.*")
        .await
        .unwrap();

    let patterns = patterns::list_patterns_for_agent(&pool, client.id)
        .await
        .unwrap();
    assert_eq!(patterns.len(), 1);
    assert_eq!(patterns[0].pattern, "crud.*");

    patterns::delete_hostname_pattern(&pool, pattern.id)
        .await
        .unwrap();

    let patterns = patterns::list_patterns_for_agent(&pool, client.id)
        .await
        .unwrap();
    assert!(patterns.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_find_client_by_pattern_glob_match(pool: PgPool) {
    let client = db::insert_agent(
        &pool,
        "pattern-glob-client",
        Some("Pattern Glob"),
        "hash",
        None,
    )
    .await
    .unwrap();

    patterns::add_hostname_pattern(&pool, client.id, "bell*")
        .await
        .unwrap();

    let matched = patterns::find_agent_by_pattern(&pool, "bell.home.mohr.io")
        .await
        .unwrap();

    let matched = matched.unwrap();
    assert_eq!(matched.id, client.id);
    assert_eq!(matched.hostname, "pattern-glob-client");
}

#[sqlx::test(migrations = "./migrations")]
async fn test_find_client_by_pattern_no_match(pool: PgPool) {
    let client = db::insert_agent(
        &pool,
        "pattern-no-match-client",
        Some("Pattern No Match"),
        "hash",
        None,
    )
    .await
    .unwrap();

    patterns::add_hostname_pattern(&pool, client.id, "bell*")
        .await
        .unwrap();

    let matched = patterns::find_agent_by_pattern(&pool, "gamma.home.mohr.io")
        .await
        .unwrap();

    assert!(matched.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_add_duplicate_pattern_returns_error(pool: PgPool) {
    let client_one = db::insert_agent(
        &pool,
        "duplicate-pattern-one",
        Some("Duplicate One"),
        "hash",
        None,
    )
    .await
    .unwrap();
    let client_two = db::insert_agent(
        &pool,
        "duplicate-pattern-two",
        Some("Duplicate Two"),
        "hash",
        None,
    )
    .await
    .unwrap();

    patterns::add_hostname_pattern(&pool, client_one.id, "dup*")
        .await
        .unwrap();

    let result = patterns::add_hostname_pattern(&pool, client_two.id, "dup*").await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn test_resolve_client_exact_match_priority(pool: PgPool) {
    let exact = db::insert_agent(&pool, "foo", Some("Exact Foo"), "hash", None)
        .await
        .unwrap();
    let patterned = db::insert_agent(
        &pool,
        "pattern-priority-client",
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
        db::ResolveResult::ExactMatch(client) => assert_eq!(client.id, exact.id),
        other => panic!("unexpected resolve result: {other:?}"),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn test_merge_client_moves_reports(pool: PgPool) {
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
async fn test_merge_client_refuses_non_placeholder(pool: PgPool) {
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
async fn test_mark_client_reports_matched(pool: PgPool) {
    let client = db::insert_agent(
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
            agent_id: client.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now - Duration::minutes(5),
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
            .bind(client.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!unmatched);

    db::mark_agent_reports_matched(&pool, client.id)
        .await
        .unwrap();

    let matched =
        sqlx::query_scalar::<_, bool>("SELECT matched FROM backup_reports WHERE agent_id = $1")
            .bind(client.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(matched);
}

#[sqlx::test(migrations = "./migrations")]
async fn get_archives_for_client_across_multiple_repos(pool: PgPool) {
    let client = db::insert_agent(&pool, "primary-host", None, "hash", None)
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
        },
    )
    .await
    .unwrap();

    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: client.id,
            repo_id: repo1.id,
            schedule_id: None,
            started_at: now - Duration::minutes(10),
            finished_at: now - Duration::minutes(5),
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
            agent_id: client.id,
            repo_id: repo1.id,
            schedule_id: None,
            started_at: now - Duration::minutes(20),
            finished_at: now - Duration::minutes(15),
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
            agent_id: client.id,
            repo_id: repo2.id,
            schedule_id: None,
            started_at: now - Duration::minutes(30),
            finished_at: now - Duration::minutes(25),
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
            agent_id: client.id,
            repo_id: repo1.id,
            schedule_id: None,
            started_at: now - Duration::minutes(40),
            finished_at: now - Duration::minutes(35),
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

    let archives = db::get_archives_for_agent(&pool, client.id).await.unwrap();

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

/// Verifies that `get_archives_for_client_with_patterns` finds archives from imported clients
/// whose hostnames match the configured glob patterns, even when those archives haven't been
/// merged/reassigned yet (agent_id still points to the imported client).
#[sqlx::test(migrations = "./migrations")]
async fn get_archives_for_client_includes_pattern_matched_archives(pool: PgPool) {
    let client = db::insert_agent(&pool, "web-server-01", None, "hash", None)
        .await
        .unwrap();
    patterns::add_hostname_pattern(&pool, client.id, "web-server-*")
        .await
        .unwrap();

    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: client.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now - Duration::minutes(10),
            finished_at: now - Duration::minutes(5),
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
            agent_id: client.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now - Duration::minutes(20),
            finished_at: now - Duration::minutes(15),
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
            started_at: now - Duration::minutes(30),
            finished_at: now - Duration::minutes(25),
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

    let archives = db::get_archives_for_agent(&pool, client.id).await.unwrap();
    assert_eq!(archives.len(), 1);
    let names: Vec<_> = archives
        .iter()
        .flat_map(|(_, names)| names.clone())
        .collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"web-server-01-2026-01-01T10:00:00".to_string()));
    assert!(names.contains(&"web-server-02-2026-01-01T10:00:00".to_string()));

    let all_archives = db::get_archives_for_agent_with_patterns(&pool, client.id)
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

/// Verifies pattern matching across multiple repos with unrelated clients excluded.
#[sqlx::test(migrations = "./migrations")]
async fn get_archives_for_client_with_patterns_multiple_repos(pool: PgPool) {
    let client = db::insert_agent(&pool, "db-server-01", None, "hash", None)
        .await
        .unwrap();
    patterns::add_hostname_pattern(&pool, client.id, "db-server-*")
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
        },
    )
    .await
    .unwrap();

    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: client.id,
            repo_id: repo1.id,
            schedule_id: None,
            started_at: now - Duration::minutes(10),
            finished_at: now - Duration::minutes(5),
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
            agent_id: client.id,
            repo_id: repo2.id,
            schedule_id: None,
            started_at: now - Duration::minutes(20),
            finished_at: now - Duration::minutes(15),
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
            started_at: now - Duration::minutes(30),
            finished_at: now - Duration::minutes(25),
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
            started_at: now - Duration::minutes(40),
            finished_at: now - Duration::minutes(35),
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
            started_at: now - Duration::minutes(50),
            finished_at: now - Duration::minutes(45),
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

    let archives = db::get_archives_for_agent_with_patterns(&pool, client.id)
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
    assert!(repo.last_synced_at.is_none());
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
            sync_schedule: Some("0 */6 * * *"),
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
            sync_schedule: None,
        },
    )
    .await
    .unwrap();

    assert!(updated.sync_schedule.is_none());
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
    let client = db::insert_agent(&pool, "bulk-host", None, "hash-bulk", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    let params = vec![
        InsertReportParams {
            agent_id: client.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now - Duration::minutes(10),
            finished_at: now - Duration::minutes(5),
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
            agent_id: client.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now - Duration::minutes(20),
            finished_at: now - Duration::minutes(15),
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

    let reports = db::list_reports_for_agent(&pool, client.id, None, 100)
        .await
        .unwrap();
    assert_eq!(reports.len(), 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn bulk_insert_backup_reports_conflict_skipped(pool: PgPool) {
    let client = db::insert_agent(&pool, "bulk-dup-host", None, "hash-dup", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();
    let started = now - Duration::minutes(10);

    let param = InsertReportParams {
        agent_id: client.id,
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

    let reports = db::list_reports_for_agent(&pool, client.id, None, 100)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn bulk_insert_keeps_distinct_archives_sharing_start_second(pool: PgPool) {
    // Borg reports archive `start` at whole-second precision, so two distinct
    // archives of the same host can share (client_id, started_at). They must not
    // collapse into a single row on import.
    let client = db::insert_agent(&pool, "same-second-host", None, "hash-ss", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let started = Utc::now() - Duration::minutes(10);
    let finished = started + Duration::minutes(1);

    let base = InsertReportParams {
        agent_id: client.id,
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
    assert!(repo.last_synced_at.is_none());

    db::update_repo_last_synced(&pool, repo.id).await.unwrap();

    let all = db::list_all_repos(&pool).await.unwrap();
    let updated = all.iter().find(|r| r.id == repo.id).unwrap();
    assert!(updated.last_synced_at.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn client_get_by_id(pool: PgPool) {
    let client = db::insert_agent(&pool, "byid-host", None, "hash-byid", None)
        .await
        .unwrap();

    let fetched = db::get_agent_by_id(&pool, client.id).await.unwrap();
    assert_eq!(fetched.id, client.id);
    assert_eq!(fetched.hostname, "byid-host");

    let result = db::get_agent_by_id(&pool, 999_999_999).await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn client_set_hidden_and_list(pool: PgPool) {
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
async fn client_token_hash_lookup(pool: PgPool) {
    let client = db::insert_agent(&pool, "token-host", None, "secret-hash", None)
        .await
        .unwrap();

    let (id, hash) = db::get_agent_token_hash(&pool, "token-host").await.unwrap();
    assert_eq!(id, client.id);
    assert_eq!(hash, "secret-hash");

    let result = db::get_agent_token_hash(&pool, "nonexistent-host").await;
    assert!(result.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn client_last_seen_updates(pool: PgPool) {
    let client = db::insert_agent(&pool, "seen-host", None, "hash-seen", None)
        .await
        .unwrap();

    assert!(client.last_seen_at.is_none());

    db::update_last_seen(&pool, client.id).await.unwrap();
    let fetched = db::get_agent_by_id(&pool, client.id).await.unwrap();
    assert!(fetched.last_seen_at.is_some());

    db::update_last_seen_and_version(
        &pool,
        client.id,
        "1.5.0",
        Some("abc123"),
        Some("2026-01-01"),
        Some(42),
    )
    .await
    .unwrap();
    let fetched = db::get_agent_by_id(&pool, client.id).await.unwrap();
    assert_eq!(fetched.agent_version.as_deref(), Some("1.5.0"));
    assert_eq!(fetched.agent_git_sha.as_deref(), Some("abc123"));

    db::update_last_seen_by_hostname(&pool, "seen-host")
        .await
        .unwrap();
    let fetched = db::get_agent_by_id(&pool, client.id).await.unwrap();
    assert!(fetched.last_seen_at.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn get_or_create_client_by_hostname_creates_new(pool: PgPool) {
    let client = db::get_or_create_agent_by_hostname(&pool, "placeholder-new")
        .await
        .unwrap();
    assert_eq!(client.hostname, "placeholder-new");
    assert_eq!(client.agent_token_hash, "imported:no-auth");

    let again = db::get_or_create_agent_by_hostname(&pool, "placeholder-new")
        .await
        .unwrap();
    assert_eq!(again.id, client.id);
}

#[sqlx::test(migrations = "./migrations")]
async fn get_or_create_client_by_hostname_returns_existing(pool: PgPool) {
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
async fn schedule_counts_by_client(pool: PgPool) {
    let (client, _, _) = create_test_schedule(&pool).await;

    let counts = db::get_schedule_counts_by_agent(&pool).await.unwrap();
    let entry = counts.iter().find(|c| c.agent_id == client.id);
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
    let (client, _, schedule) = create_test_schedule(&pool).await;

    let targets = db::list_schedule_targets(&pool, schedule.id).await.unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].agent_id, client.id);

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
    let (client_a, _, schedule) = create_test_schedule(&pool).await;
    let client_b = db::insert_agent(&pool, "run-target-b", None, "hash-rtb", None)
        .await
        .unwrap();
    let client_hidden = db::insert_agent(&pool, "run-target-hidden", None, "hash-rth", None)
        .await
        .unwrap();
    db::set_agent_hidden(&pool, "run-target-hidden", true)
        .await
        .unwrap();

    // Add client_b at order 1 (after client_a at order 0) and the hidden client at order 2.
    db::insert_schedule_targets(
        &pool,
        schedule.id,
        &[(client_b.id, 1), (client_hidden.id, 2)],
    )
    .await
    .unwrap();

    let targets = db::get_schedule_targets_for_run(&pool, schedule.id)
        .await
        .unwrap();

    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0].agent_id, client_a.id);
    assert_eq!(targets[0].hostname, "sched-host");
    assert_eq!(targets[1].agent_id, client_b.id);
    assert_eq!(targets[1].hostname, "run-target-b");
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
    let (client, repo, schedule) = create_test_schedule(&pool).await;

    insert_test_report_for_schedule(&pool, client.id, repo.id, schedule.id, "success").await;

    let reports = db::list_reports_for_schedule(&pool, schedule.id, 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].status, "success");

    let empty = db::list_reports_for_schedule(&pool, schedule.id + 999, 10)
        .await
        .unwrap();
    assert!(empty.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn activity_feed_repo_filter(pool: PgPool) {
    let client = db::insert_agent(&pool, "feed-repo-filter-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;

    let all = db::get_activity_feed(&pool, 10, None, None, None, None)
        .await
        .unwrap();
    assert!(!all.is_empty());

    let filtered = db::get_activity_feed(&pool, 10, Some(repo.id), None, None, None)
        .await
        .unwrap();
    assert_eq!(filtered.len(), 1);

    let empty = db::get_activity_feed(&pool, 10, Some(repo.id + 999), None, None, None)
        .await
        .unwrap();
    assert!(empty.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn activity_feed_hostname_filter(pool: PgPool) {
    let client = db::insert_agent(&pool, "hostname-filter-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;

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
    let client = db::insert_agent(&pool, "days-feed-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;

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
    assert!(empty_trends.iter().all(|t| t.deduplicated_size == 0));

    let client = db::insert_agent(&pool, "strend-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;

    let trends = db::get_storage_trends(&pool, None, 7).await.unwrap();
    assert!(trends.iter().any(|t| t.deduplicated_size > 0));

    let trends_repo = db::get_storage_trends(&pool, Some(repo.id), 7)
        .await
        .unwrap();
    assert!(trends_repo.iter().any(|t| t.deduplicated_size > 0));

    let trends_other = db::get_storage_trends(&pool, Some(repo.id + 999), 7)
        .await
        .unwrap();
    assert!(trends_other.iter().all(|t| t.deduplicated_size == 0));
}

#[sqlx::test(migrations = "./migrations")]
async fn storage_trends_by_repo_test(pool: PgPool) {
    let empty = db::get_storage_trends_by_repo(&pool, 7).await.unwrap();
    assert!(empty.is_empty());

    let client = db::insert_agent(&pool, "strend-repo-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    insert_test_report(&pool, client.id, repo.id).await;

    let trends = db::get_storage_trends_by_repo(&pool, 7).await.unwrap();
    assert!(!trends.is_empty());
    assert!(
        trends
            .iter()
            .any(|t| t.repo_name == "test-repo" && t.deduplicated_size > 0)
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn archive_names_and_delete_test(pool: PgPool) {
    let client = db::insert_agent(&pool, "archive-del-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: client.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now - Duration::minutes(10),
            finished_at: now - Duration::minutes(5),
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
            agent_id: client.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now - Duration::minutes(20),
            finished_at: now - Duration::minutes(15),
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
    let client = db::insert_agent(&pool, "stats-needing-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    let base = InsertReportParams {
        agent_id: client.id,
        repo_id: repo.id,
        schedule_id: None,
        started_at: now - Duration::minutes(10),
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
            started_at: now - Duration::minutes(20),
            original_size: 1_000,
            compressed_size: 500,
            deduplicated_size: 250,
            archive_name: Some("already-enriched".to_string()),
            ..base.clone()
        },
    )
    .await
    .unwrap();

    let needing = db::list_archive_names_needing_stats(&pool, repo.id)
        .await
        .unwrap();
    assert_eq!(needing.len(), 1);
    assert!(needing.contains("needs-stats"));
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
    let client = db::insert_agent(&pool, "del-before-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: client.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now - Duration::hours(2),
            finished_at: now - Duration::hours(2),
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

    let cutoff = now - Duration::hours(1);
    let deleted = db::delete_backup_reports_before(&pool, cutoff)
        .await
        .unwrap();
    assert_eq!(deleted, 1);

    let reports = db::list_reports_for_agent(&pool, client.id, None, 10)
        .await
        .unwrap();
    assert!(reports.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_backup_reports_before_keeps_archive_rows(pool: PgPool) {
    // Imported/synced archives keep their original (old) borg start timestamp.
    // Age-based report retention must not delete them, or archives vanish from
    // the UI even though they still exist in borg.
    let client = db::insert_agent(&pool, "retain-archive-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let old = Utc::now() - Duration::days(365);

    let base = InsertReportParams {
        agent_id: client.id,
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
            started_at: old + Duration::seconds(1),
            finished_at: old + Duration::seconds(1),
            status: "failed".to_string(),
            archive_name: None,
            ..base.clone()
        },
    )
    .await
    .unwrap();

    let cutoff = Utc::now() - Duration::days(7);
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
            target_type: Some("client"),
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
    assert_eq!(items[0].target_type.as_deref(), Some("repo"));
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
    assert_eq!(items[0].action, "delete");
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
    let client = db::insert_agent(&pool, "cancel-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;

    let started_at = Utc::now();
    db::insert_backup_started(&pool, client.id, repo.id, None, started_at, None, None)
        .await
        .unwrap();

    db::cancel_backup_report(&pool, client.id, repo.id)
        .await
        .unwrap();

    let reports = db::list_reports_for_agent(&pool, client.id, None, 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].status, "cancelled");
}

#[sqlx::test(migrations = "./migrations")]
async fn cancel_backup_report_ignores_already_completed(pool: PgPool) {
    let client = db::insert_agent(&pool, "cancel-done-host", None, "hash", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();

    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: client.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now - Duration::minutes(5),
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

    db::cancel_backup_report(&pool, client.id, repo.id)
        .await
        .unwrap();

    let reports = db::list_reports_for_agent(&pool, client.id, None, 10)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].status, "success");
}

#[sqlx::test(migrations = "./migrations")]
async fn client_insert_with_paths(pool: PgPool) {
    let paths = vec!["/etc".to_string(), "/home".to_string()];
    let excludes = vec!["*.log".to_string()];
    let client = db::insert_agent_with_paths(
        &pool,
        "paths-host",
        Some("Paths Host"),
        "hash",
        &paths,
        &excludes,
    )
    .await
    .unwrap();

    assert_eq!(client.hostname, "paths-host");
    assert_eq!(client.display_name.as_deref(), Some("Paths Host"));
    assert_eq!(client.default_backup_paths, paths);
    assert_eq!(client.default_exclude_patterns, excludes);
}

#[sqlx::test(migrations = "./migrations")]
async fn run_id_update_scoped_to_client(pool: PgPool) {
    let client_a = db::insert_agent(&pool, "run-host-a", None, "hash-a", None)
        .await
        .unwrap();
    let client_b = db::insert_agent(&pool, "run-host-b", None, "hash-b", None)
        .await
        .unwrap();
    let repo = create_test_repo(&pool).await;
    let now = Utc::now();
    let run_id = "shared-run-id";

    db::insert_backup_pending(&pool, client_a.id, repo.id, None, run_id, now)
        .await
        .unwrap();
    db::insert_backup_pending(&pool, client_b.id, repo.id, None, run_id, now)
        .await
        .unwrap();

    // Only client_a sends BackupStarted.
    db::insert_backup_started(&pool, client_a.id, repo.id, None, now, None, Some(run_id))
        .await
        .unwrap();

    // client_b's record must still be 'pending'.
    let b_reports = db::list_reports_for_agent(&pool, client_b.id, None, 10)
        .await
        .unwrap();
    assert_eq!(b_reports.len(), 1);
    assert_eq!(b_reports[0].status, "pending");

    // Only client_a sends BackupCompleted.
    db::insert_backup_report(
        &pool,
        &InsertReportParams {
            agent_id: client_a.id,
            repo_id: repo.id,
            schedule_id: None,
            started_at: now,
            finished_at: now + Duration::minutes(10),
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

    // client_b's record must still be 'pending' - not bulk-failed by client_a's report.
    let b_reports = db::list_reports_for_agent(&pool, client_b.id, None, 10)
        .await
        .unwrap();
    assert_eq!(b_reports.len(), 1);
    assert_eq!(b_reports[0].status, "pending");

    let a_reports = db::list_reports_for_agent(&pool, client_a.id, None, 10)
        .await
        .unwrap();
    assert_eq!(a_reports.len(), 1);
    assert_eq!(a_reports[0].status, "failed");
}

#[sqlx::test(migrations = "./migrations")]
async fn dismiss_finding_roundtrip(pool: PgPool) {
    let user = db::insert_user(&pool, "dismiss-user", "hash", "admin")
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
    let user = db::insert_user(&pool, "dismiss-idem-user", "hash", "admin")
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
    let user = db::insert_user(&pool, "undismiss-user", "hash", "admin")
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
    let user_a = db::insert_user(&pool, "dismiss-user-a", "hash", "admin")
        .await
        .unwrap();
    let user_b = db::insert_user(&pool, "dismiss-user-b", "hash", "admin")
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
