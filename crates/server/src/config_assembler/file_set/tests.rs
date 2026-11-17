// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use shared::{
    hooks::HookCommand,
    types::{FileChangeAction, FileChangePattern, ScheduleFilesMode},
};
use sqlx::PgPool;

use super::{FileSet, resolve};
use crate::db;

fn cmd(command: &str) -> HookCommand {
    HookCommand {
        command: command.to_owned(),
        timeout_seconds: None,
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// A host whose Sources > Files has every field set.
async fn host(pool: &PgPool, hostname: &str) -> db::AgentRow {
    db::insert_agent_with_paths(
        pool,
        hostname,
        "hash",
        db::AgentDefaults {
            display_name: None,
            domain: None,
            default_backup_paths: &strings(&["/etc", "/home"]),
            default_exclude_patterns: &strings(&["*.cache"]),
            default_include_patterns: &strings(&["/home/keep"]),
            default_pre_backup_commands: &[cmd("systemctl stop app")],
            default_post_backup_commands: &[cmd("systemctl start app")],
            default_file_change_patterns_raw: "/var/lib/app/wal/** ignore",
        },
    )
    .await
    .unwrap()
}

async fn repo(pool: &PgPool) -> i64 {
    db::insert_repo(
        pool,
        &db::InsertRepoParams {
            name: "file-set-repo",
            repo_path: "/backup/file-set",
            ssh_user: "borg",
            ssh_host: "127.0.0.1",
            ssh_port: 1,
            passphrase_encrypted: b"irrelevant",
            compression: "lz4",
            encryption: "none",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap()
    .id
}

/// A schedule of `mode` whose own fields are all set too, so a test can tell
/// which of the two sets a backup read.
async fn schedule(
    pool: &PgPool,
    mode: ScheduleFilesMode,
    run_host_commands: bool,
) -> db::ScheduleRow {
    let repo_id = repo(pool).await;
    let pre = [cmd("pg_dump app")];
    let post = [cmd("rm /tmp/dump")];
    let params = db::ScheduleParams {
        files_mode: mode,
        run_host_commands,
        exclude_patterns_raw: "# own\n*.tmp",
        include_patterns_raw: "/srv/keep",
        file_change_patterns_raw: "/srv/** fatal",
        pre_backup_commands: &pre,
        post_backup_commands: &post,
        ..db::ScheduleParams::for_test("files", "0 2 * * *")
    };
    let schedule = db::insert_schedule(pool, repo_id, &params, None)
        .await
        .unwrap();
    db::insert_backup_source_for_schedule(pool, schedule.id, "/srv", 0)
        .await
        .unwrap();
    schedule
}

fn host_set() -> FileSet {
    FileSet {
        paths: strings(&["/etc", "/home"]),
        excludes: strings(&["*.cache"]),
        includes: strings(&["/home/keep"]),
        file_change_patterns: vec![FileChangePattern {
            path: "/var/lib/app/wal/**".to_owned(),
            action: FileChangeAction::Ignore,
        }],
        pre_backup_commands: vec![cmd("systemctl stop app")],
        post_backup_commands: vec![cmd("systemctl start app")],
    }
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_schedule_taking_each_hosts_files_reads_the_hosts_set_only(pool: PgPool) {
    let agent = host(&pool, "fs-host").await;
    let schedule = schedule(&pool, ScheduleFilesMode::Host, true).await;

    assert_eq!(resolve(&pool, &schedule, &agent).await.unwrap(), host_set());
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_schedule_can_take_the_hosts_files_without_their_commands(pool: PgPool) {
    let agent = host(&pool, "fs-host-quiet").await;
    let schedule = schedule(&pool, ScheduleFilesMode::Host, false).await;

    let files = resolve(&pool, &schedule, &agent).await.unwrap();

    assert_eq!(files, host_set().without_commands());
    assert!(files.pre_backup_commands.is_empty() && files.post_backup_commands.is_empty());
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn own_files_replace_the_hosts_set_as_a_whole(pool: PgPool) {
    let agent = host(&pool, "fs-own").await;
    let schedule = schedule(&pool, ScheduleFilesMode::Own, true).await;

    let files = resolve(&pool, &schedule, &agent).await.unwrap();

    assert_eq!(
        files,
        FileSet {
            paths: strings(&["/srv"]),
            excludes: strings(&["*.tmp"]),
            includes: strings(&["/srv/keep"]),
            file_change_patterns: vec![FileChangePattern {
                path: "/srv/**".to_owned(),
                action: FileChangeAction::Fatal,
            }],
            pre_backup_commands: vec![cmd("pg_dump app")],
            post_backup_commands: vec![cmd("rm /tmp/dump")],
        }
    );
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn own_files_take_a_hosts_own_values_where_the_schedule_splits_them(pool: PgPool) {
    let agent = host(&pool, "fs-split").await;
    let schedule = schedule(&pool, ScheduleFilesMode::Own, true).await;
    db::insert_backup_source_for_schedule_agent(&pool, schedule.id, agent.id, "/opt", 0)
        .await
        .unwrap();
    db::upsert_per_agent_excludes_raw(&pool, schedule.id, agent.id, "*.log")
        .await
        .unwrap();
    db::upsert_per_agent_commands(&pool, schedule.id, agent.id, &[cmd("sync")], &[])
        .await
        .unwrap();

    let files = resolve(&pool, &schedule, &agent).await.unwrap();

    assert_eq!(files.paths, strings(&["/opt"]));
    assert_eq!(files.excludes, strings(&["*.log"]));
    assert_eq!(files.pre_backup_commands, vec![cmd("sync")]);
    assert_eq!(files.post_backup_commands, Vec::<HookCommand>::new());
    assert_eq!(files.includes, strings(&["/srv/keep"]));
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_schedule_with_files_off_reads_nothing_and_runs_no_commands(pool: PgPool) {
    let agent = host(&pool, "fs-off").await;
    let schedule = schedule(&pool, ScheduleFilesMode::Off, true).await;

    assert_eq!(
        resolve(&pool, &schedule, &agent).await.unwrap(),
        FileSet::default()
    );
}

#[test]
fn excludes_put_the_global_list_first_and_drop_repeats() {
    let files = FileSet {
        excludes: strings(&["*.cache", "node_modules"]),
        ..FileSet::default()
    };
    let global = strings(&["node_modules", "/proc"]);

    assert_eq!(
        files.exclude_patterns(&global, false),
        strings(&["node_modules", "/proc", "*.cache"])
    );
    assert_eq!(
        files.exclude_patterns(&global, true),
        strings(&["*.cache", "node_modules"])
    );
}
