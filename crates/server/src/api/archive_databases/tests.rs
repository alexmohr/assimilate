// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use shared::{
    database::{
        DatabaseEngine, DatabaseManifestEntry, DatabaseSourceId, DatabaseSourceKind, DumpFormat,
    },
    protocol::ServerToAgent,
    types::RestoreRunStatus,
};
use sqlx::PgPool;
use tokio::sync::mpsc;

use super::*;
use crate::db::restore_runs::RestoreOutcome;

const DUMP_DIR: &str = "var/lib/assimilate/database-dumps";

fn entry(
    runtime: DatabaseRuntime,
    namespace: &str,
    name: &str,
    database: &str,
    path: &str,
) -> DatabaseManifestEntry {
    DatabaseManifestEntry {
        source: DatabaseSourceId {
            runtime,
            namespace: namespace.to_owned(),
            name: name.to_owned(),
        },
        kind: DatabaseSourceKind::Container,
        engine: DatabaseEngine::Postgresql,
        pod: format!("{name}-0"),
        database: database.to_owned(),
        path: path.to_owned(),
        format: DumpFormat::PgCustom,
    }
}

fn manifest_json(version: u32, dumps: Vec<DatabaseManifestEntry>) -> String {
    serde_json::to_string(&DatabaseManifest { version, dumps }).unwrap()
}

fn orders() -> DatabaseManifestEntry {
    entry(
        DatabaseRuntime::Kubernetes,
        "shop",
        "orders-db",
        "orders",
        "kubernetes/shop/orders-db/orders.pgdump",
    )
}

fn gitea() -> DatabaseManifestEntry {
    entry(
        DatabaseRuntime::Docker,
        "gitea",
        "db",
        "gitea",
        "docker/gitea/db/gitea.pgdump",
    )
}

fn inventory() -> DatabaseManifestEntry {
    entry(
        DatabaseRuntime::Host,
        "",
        "postgresql",
        "inventory",
        "host/postgresql/inventory.pgdump",
    )
}

#[test]
fn only_a_run_directory_holds_a_manifest() {
    assert!(matches!("manual".parse(), Ok(DumpRunDir::Manual)));
    assert!(matches!("schedule-7".parse(), Ok(DumpRunDir::Schedule)));
    for dir in ["schedule-", "schedule-x", "schedule-7a", "Manual", "dumps"] {
        assert!(dir.parse::<DumpRunDir>().is_err(), "{dir}");
    }

    assert_eq!(
        manifest_dir(&format!("{DUMP_DIR}/schedule-7/databases.json")),
        Some(&*format!("{DUMP_DIR}/schedule-7"))
    );
    assert_eq!(
        manifest_dir("manual/databases.json"),
        Some("manual"),
        "a dump directory at the archive root"
    );
    for path in [
        "manual",
        "etc/databases.json",
        "manual/databases.json.bak",
        "manual/other.json",
        "schedule-7/nested/databases.json",
        "schedule-x/databases.json",
        "/manual/databases.json",
        "-x/manual/databases.json",
        "databases.json",
    ] {
        assert_eq!(manifest_dir(path), None, "{path}");
    }
}

#[test]
fn the_listing_keeps_only_manifests() {
    assert_eq!(
        manifest_patterns(),
        vec![
            "+re:(^|/)(schedule-[0-9]+|manual)/databases\\.json$".to_owned(),
            "-sh:**".to_owned(),
        ]
    );
}

#[test]
fn a_dump_path_must_stay_below_its_manifest() {
    for path in [
        "kubernetes/shop/orders-db/orders.pgdump",
        "host/postgresql/inventory.pgdump",
        "orders.pgdump",
    ] {
        assert!(is_safe_entry_path(path), "{path}");
    }
    for path in [
        "",
        "/etc/shadow",
        "../../etc/shadow",
        "docker/../../../etc/shadow",
        "docker//gitea.pgdump",
        "docker/gitea/",
        "./gitea.pgdump",
        "docker/./gitea.pgdump",
        "-gitea.pgdump",
        "docker/gitea\0.pgdump",
    ] {
        assert!(!is_safe_entry_path(path), "{path:?}");
    }
}

#[test]
fn a_dump_path_is_joined_to_its_manifest_directory() {
    assert_eq!(
        dump_archive_path(
            "var/lib/assimilate/database-dumps/schedule-7",
            "docker/gitea/db/gitea.pgdump"
        ),
        "var/lib/assimilate/database-dumps/schedule-7/docker/gitea/db/gitea.pgdump"
    );
}

#[test]
fn a_manifest_lists_its_safe_dumps_in_order() {
    let manifest_path = format!("{DUMP_DIR}/schedule-7/databases.json");
    let mut escaping = orders();
    escaping.path = "../../../etc/shadow".to_owned();
    let contents = manifest_json(MANIFEST_VERSION, vec![orders(), escaping, gitea()]);

    let dumps = dumps_in_manifest(&manifest_path, contents.as_bytes());

    assert_eq!(
        dumps,
        vec![
            ArchiveDatabaseDump {
                manifest_path: manifest_path.clone(),
                archive_path: format!(
                    "{DUMP_DIR}/schedule-7/kubernetes/shop/orders-db/orders.pgdump"
                ),
                entry: orders(),
            },
            ArchiveDatabaseDump {
                manifest_path,
                archive_path: format!("{DUMP_DIR}/schedule-7/docker/gitea/db/gitea.pgdump"),
                entry: gitea(),
            },
        ]
    );
}

#[test]
fn a_manifest_that_cannot_be_read_lists_nothing() {
    let manifest_path = format!("{DUMP_DIR}/manual/databases.json");
    let newer = manifest_json(MANIFEST_VERSION + 1, vec![gitea()]);
    let current = manifest_json(MANIFEST_VERSION, vec![gitea()]);

    assert_eq!(dumps_in_manifest(&manifest_path, b"not json"), Vec::new());
    assert_eq!(
        dumps_in_manifest(&manifest_path, newer.as_bytes()),
        Vec::new()
    );
    assert_eq!(
        dumps_in_manifest("etc/databases.json", current.as_bytes()),
        Vec::new(),
        "a file that is no run's manifest is not read as one"
    );
}

#[test]
fn a_dump_is_restored_only_through_a_runtime_the_agent_searches() {
    let runtimes = DatabaseRuntimes {
        kubernetes: true,
        docker: false,
        host: true,
        ..DatabaseRuntimes::default()
    };

    assert!(runtime_enabled(&runtimes, DatabaseRuntime::Kubernetes));
    assert!(!runtime_enabled(&runtimes, DatabaseRuntime::Docker));
    assert!(runtime_enabled(&runtimes, DatabaseRuntime::Host));
}

#[test]
fn a_new_database_needs_a_plain_name() {
    let named = |name: &str| DatabaseRestoreTarget::NewDatabase {
        name: name.to_owned(),
    };

    assert!(validate_target(&DatabaseRestoreTarget::Original).is_ok());
    assert!(validate_target(&named("orders_restored")).is_ok());
    assert!(matches!(
        validate_target(&named("pg_orders")),
        Err(ApiError::BadRequest(message)) if message.contains("must not start with")
    ));
    assert!(matches!(
        validate_target(&named("orders; DROP DATABASE orders")),
        Err(ApiError::BadRequest(_))
    ));
}

/// A fake borg that lists `listing` and extracts each of `files` as the
/// output of its shell command; any other path is missing. Every call's
/// arguments are appended to `calls` next to it.
fn fake_borg(listing: &[String], files: &[(String, String)]) -> String {
    let listing = listing
        .iter()
        .map(|path| format!(" '{}'", serde_json::json!({"type": "-", "path": path})))
        .collect::<Vec<_>>()
        .concat();
    let files = files
        .iter()
        .map(|(path, command)| format!("    '{path}') {command} ;;\n"))
        .collect::<Vec<_>>()
        .concat();
    format!(
        "#!/bin/sh\nfor arg; do last=\"$arg\"; done\necho \"$@\" >> \"$(dirname \
         \"$0\")/calls\"\ncase \"$1\" in\n  list) printf '%s\\n'{listing} ;;\n  extract)\n    \
         case \"$last\" in\n{files}    *) echo \"$last: not found in archive\" >&2; exit 1 ;;\n    \
         esac ;;\nesac\n"
    )
}

/// The command printing `contents`.
fn print(contents: &str) -> String {
    format!("cat <<'MANIFEST'\n{contents}\nMANIFEST\n")
}

/// The archive the handler tests read: a scheduled and a manual run's
/// manifests, a manifest that does not parse, one from a newer agent, one
/// too large to read, and a stray `databases.json` outside any run.
fn archive_script() -> String {
    let schedule_7 = format!("{DUMP_DIR}/schedule-7/databases.json");
    let manual = format!("{DUMP_DIR}/manual/databases.json");
    let broken = format!("{DUMP_DIR}/schedule-8/databases.json");
    let newer = format!("{DUMP_DIR}/schedule-9/databases.json");
    let huge = format!("{DUMP_DIR}/schedule-10/databases.json");
    fake_borg(
        &[
            schedule_7.clone(),
            format!("./{manual}"),
            broken.clone(),
            newer.clone(),
            huge.clone(),
            "etc/databases.json".to_owned(),
        ],
        &[
            (
                schedule_7,
                print(&manifest_json(MANIFEST_VERSION, vec![orders()])),
            ),
            (
                manual,
                print(&manifest_json(MANIFEST_VERSION, vec![gitea(), inventory()])),
            ),
            (broken, print("{\"version\": 1, \"dumps\": [")),
            (
                newer,
                print(&manifest_json(MANIFEST_VERSION + 1, vec![orders()])),
            ),
            (
                huge,
                format!("head -c {} /dev/zero", MANIFEST_READ_LIMIT + 1),
            ),
        ],
    )
}

struct Fixture {
    state: AppState,
    admin: AuthUser,
    viewer: AuthUser,
    stranger: AuthUser,
    agent_id: i64,
    repo_id: i64,
}

async fn auth_user(pool: &PgPool, username: &str) -> AuthUser {
    let user = db::insert_user(pool, username, "hash").await.unwrap();
    AuthUser {
        user_id: user.id,
        username: username.to_owned(),
        session_id: None,
    }
}

async fn fixture(pool: &PgPool) -> Fixture {
    let state = crate::test_support::build_test_state(pool.clone(), b"archive-databases-test-key");
    let admin = auth_user(pool, "db-restorer").await;
    let viewer = auth_user(pool, "db-viewer").await;
    let stranger = auth_user(pool, "db-stranger").await;
    let agent = db::insert_agent(pool, "db-host", None, "hash", None, None)
        .await
        .unwrap();
    db::databases::update_agent_database_backup(
        pool,
        agent.id,
        db::databases::DatabaseBackupPatch {
            enabled: true,
            dump_dir: "/var/lib/assimilate/database-dumps",
            runtimes: &DatabaseRuntimes {
                kubernetes: true,
                docker: true,
                host: false,
                ..DatabaseRuntimes::default()
            },
            selection: None,
        },
    )
    .await
    .unwrap();
    let passphrase = shared::crypto::encrypt_passphrase("secret", &state.encryption_key).unwrap();
    let repo = db::insert_repo(
        pool,
        &db::InsertRepoParams {
            name: "db-repo",
            repo_path: "/backups/db",
            ssh_user: "backup",
            ssh_host: "storage.local",
            ssh_port: 22,
            passphrase_encrypted: &passphrase,
            compression: "lz4",
            encryption: "repokey",
            owner_id: None,
            sync_schedule: None,
        },
    )
    .await
    .unwrap();
    db::upsert_repo_permission(
        pool,
        &db::UpsertRepoPermissionParams {
            user_id: viewer.user_id,
            repo_id: repo.id,
            can_view: true,
            can_backup: false,
            can_modify_schedules: false,
            can_extract: false,
            can_delete: false,
        },
    )
    .await
    .unwrap();
    Fixture {
        state,
        admin,
        viewer,
        stranger,
        agent_id: agent.id,
        repo_id: repo.id,
    }
}

impl Fixture {
    async fn list(&self, user: &AuthUser) -> Result<Json<ArchiveDatabasesResponse>, ApiError> {
        list_archive_databases(
            State(self.state.clone()),
            user.clone(),
            AxumPath((self.repo_id, "nightly".to_owned())),
        )
        .await
    }

    async fn restore(
        &self,
        repo_id: i64,
        archive_path: &str,
        target: DatabaseRestoreTarget,
    ) -> Result<(StatusCode, Json<RestoreRun>), ApiError> {
        restore_database(
            State(self.state.clone()),
            RequireAdmin(self.admin.clone()),
            AxumPath((repo_id, "nightly".to_owned())),
            Json(RestoreDatabaseRequest {
                archive_path: archive_path.to_owned(),
                target,
                hostname: "db-host".to_owned(),
                domain: None,
            }),
        )
        .await
    }

    async fn connect(&self) -> mpsc::Receiver<ServerToAgent> {
        let (tx, rx) = mpsc::channel(8);
        self.state
            .registry
            .register(
                self.agent_id,
                tx,
                false,
                None,
                Some("instance-a".to_owned()),
            )
            .await;
        rx
    }
}

fn restored_as(name: &str) -> DatabaseRestoreTarget {
    DatabaseRestoreTarget::NewDatabase {
        name: name.to_owned(),
    }
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn an_archive_lists_the_dumps_of_its_readable_manifests(pool: PgPool) {
    let fx = fixture(&pool).await;
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (borg_dir, _guard) = crate::test_support::install_fake_borg(&archive_script()).await;

    let Json(listed) = fx.list(&fx.viewer).await.unwrap();

    let manual = format!("{DUMP_DIR}/manual/databases.json");
    let schedule_7 = format!("{DUMP_DIR}/schedule-7/databases.json");
    assert_eq!(
        listed.dumps,
        vec![
            ArchiveDatabaseDump {
                manifest_path: manual.clone(),
                archive_path: format!("{DUMP_DIR}/manual/docker/gitea/db/gitea.pgdump"),
                entry: gitea(),
            },
            ArchiveDatabaseDump {
                manifest_path: manual,
                archive_path: format!("{DUMP_DIR}/manual/host/postgresql/inventory.pgdump"),
                entry: inventory(),
            },
            ArchiveDatabaseDump {
                manifest_path: schedule_7,
                archive_path: format!(
                    "{DUMP_DIR}/schedule-7/kubernetes/shop/orders-db/orders.pgdump"
                ),
                entry: orders(),
            },
        ]
    );
    let calls = tokio::fs::read_to_string(borg_dir.path().join("calls"))
        .await
        .unwrap();
    assert!(
        calls.lines().next().is_some_and(|list| list.contains(
            "--pattern +re:(^|/)(schedule-[0-9]+|manual)/databases\\.json$ --pattern -sh:**"
        )),
        "{calls}"
    );
    assert!(!calls.contains("-- etc/databases.json"), "{calls}");
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn listing_an_archives_dumps_needs_view_permission(pool: PgPool) {
    let fx = fixture(&pool).await;

    let result = fx.list(&fx.stranger).await;

    assert!(matches!(result, Err(ApiError::Forbidden(_))));
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_listing_borg_cannot_run_is_its_error(pool: PgPool) {
    let fx = fixture(&pool).await;
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = crate::test_support::install_fake_borg(
        "#!/bin/sh\necho 'ssh: connect to host storage.local port 22: Connection refused' \
         >&2\nexit 2\n",
    )
    .await;

    let result = fx.list(&fx.viewer).await;

    assert!(matches!(result, Err(ApiError::BadGateway(_))));
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_manifest_borg_cannot_extract_fails_the_listing(pool: PgPool) {
    let fx = fixture(&pool).await;
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = crate::test_support::install_fake_borg(&fake_borg(
        &[format!("{DUMP_DIR}/manual/databases.json")],
        &[],
    ))
    .await;

    let result = fx.list(&fx.viewer).await;

    assert!(matches!(result, Err(ApiError::Internal(_))));
}

/// The restore is built from the archive's manifest, handed to the
/// connected agent as a database restore, audited, and settled by the
/// agent's answer like a file restore.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_dump_is_restored_through_its_agent(pool: PgPool) {
    let fx = fixture(&pool).await;
    let mut agent_rx = fx.connect().await;
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = crate::test_support::install_fake_borg(&archive_script()).await;
    let archive_path = format!("{DUMP_DIR}/manual/docker/gitea/db/gitea.pgdump");

    let (status, Json(run)) = fx
        .restore(fx.repo_id, &archive_path, restored_as("gitea_restored"))
        .await
        .unwrap();

    let expected = DatabaseRestore {
        source: gitea().source,
        database: "gitea".to_owned(),
        archive_path: archive_path.clone(),
        format: DumpFormat::PgCustom,
        target: restored_as("gitea_restored"),
    };
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(run.status, RestoreRunStatus::Running);
    assert_eq!(run.hostname, "db-host");
    assert_eq!(run.paths, vec![archive_path.clone()]);
    assert_eq!(run.target_path, None);
    assert_eq!(run.database.as_ref(), Some(&expected));
    assert!(matches!(
        agent_rx.try_recv(),
        Ok(ServerToAgent::RestoreDatabase { request_id, repo_id, archive_name, restore })
            if request_id == run.id
                && repo_id.0 == fx.repo_id
                && archive_name == "nightly"
                && restore == expected
    ));
    assert!(matches!(
        crate::test_support::audit_events(&pool).await.as_slice(),
        [AuditEvent::RestoreDatabase {
            archive,
            archive_path: audited,
            source,
            database,
            target,
            hostname,
        }]
            if archive == "nightly"
                && audited == &archive_path
                && source == &gitea().source
                && database == "gitea"
                && target == &restored_as("gitea_restored")
                && hostname == "db-host"
    ));

    let settled = crate::restore_runs::settle(
        &fx.state,
        fx.agent_id,
        &run.id,
        RestoreOutcome {
            success: true,
            files_restored: 1,
            error_message: None,
        },
    )
    .await;

    assert_eq!(settled, crate::restore_runs::Settlement::Recorded);
    let id = uuid::Uuid::parse_str(&run.id).unwrap();
    let finished = db::restore_runs::get_restore_run(&pool, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(finished.status, RestoreRunStatus::Success);
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_restore_for_an_offline_agent_waits_for_it(pool: PgPool) {
    let fx = fixture(&pool).await;
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = crate::test_support::install_fake_borg(&archive_script()).await;

    let (status, Json(run)) = fx
        .restore(
            fx.repo_id,
            &format!("{DUMP_DIR}/schedule-7/kubernetes/shop/orders-db/orders.pgdump"),
            DatabaseRestoreTarget::Original,
        )
        .await
        .unwrap();

    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(run.status, RestoreRunStatus::Pending);
    assert_eq!(
        run.database.map(|restore| restore.target),
        Some(DatabaseRestoreTarget::Original)
    );
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_restore_of_a_dump_the_archive_does_not_list_is_not_found(pool: PgPool) {
    let fx = fixture(&pool).await;
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = crate::test_support::install_fake_borg(&archive_script()).await;

    // Listed in a manifest from a newer agent, which is not read.
    let unread = fx
        .restore(
            fx.repo_id,
            &format!("{DUMP_DIR}/schedule-9/kubernetes/shop/orders-db/orders.pgdump"),
            DatabaseRestoreTarget::Original,
        )
        .await;
    let unknown = fx
        .restore(fx.repo_id, "etc/passwd", DatabaseRestoreTarget::Original)
        .await;

    assert!(matches!(unread, Err(ApiError::NotFound(_))));
    assert!(matches!(unknown, Err(ApiError::NotFound(message)) if message.contains("etc/passwd")));
    assert_eq!(crate::test_support::audit_events(&pool).await, Vec::new());
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_restore_request_is_checked_before_the_archive_is_read(pool: PgPool) {
    let fx = fixture(&pool).await;
    let dump = format!("{DUMP_DIR}/manual/docker/gitea/db/gitea.pgdump");

    let bad_name = fx.restore(fx.repo_id, &dump, restored_as("pg_gitea")).await;
    let bad_path = fx
        .restore(fx.repo_id, "../etc/shadow", DatabaseRestoreTarget::Original)
        .await;
    let unknown_repo = fx
        .restore(987_654, &dump, DatabaseRestoreTarget::Original)
        .await;

    assert!(matches!(bad_name, Err(ApiError::BadRequest(_))));
    assert!(matches!(bad_path, Err(ApiError::BadRequest(_))));
    assert!(matches!(unknown_repo, Err(ApiError::NotFound(_))));
}

#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_dump_from_a_runtime_the_agent_does_not_search_is_refused(pool: PgPool) {
    let fx = fixture(&pool).await;
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = crate::test_support::install_fake_borg(&archive_script()).await;

    let result = fx
        .restore(
            fx.repo_id,
            &format!("{DUMP_DIR}/manual/host/postgresql/inventory.pgdump"),
            DatabaseRestoreTarget::Original,
        )
        .await;

    assert!(matches!(
        result,
        Err(ApiError::BadRequest(message))
            if message.contains("does not look for databases in host")
    ));
    assert_eq!(
        db::restore_runs::list_restore_runs(&pool, 10)
            .await
            .unwrap(),
        Vec::new()
    );
}

/// An audit log that rejects the entry does not undo or block the restore.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn a_restore_goes_ahead_when_the_audit_log_rejects_it(pool: PgPool) {
    let fx = fixture(&pool).await;
    sqlx::query!(
        "ALTER TABLE audit_log ADD CONSTRAINT test_rejects_database_restores CHECK (action <> \
         'restore_database') NOT VALID"
    )
    .execute(&pool)
    .await
    .unwrap();
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = crate::test_support::install_fake_borg(&archive_script()).await;

    let (status, Json(run)) = fx
        .restore(
            fx.repo_id,
            &format!("{DUMP_DIR}/manual/docker/gitea/db/gitea.pgdump"),
            DatabaseRestoreTarget::Original,
        )
        .await
        .unwrap();

    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(run.status, RestoreRunStatus::Pending);
    assert_eq!(crate::test_support::audit_events(&pool).await, Vec::new());
}

/// Browsing an archive whose index failed lists its contents with `borg
/// list`, the same helper the dump listing uses.
#[ignore = "requires DATABASE_URL"]
#[sqlx::test(migrations = "./migrations")]
async fn an_archive_whose_index_failed_is_browsed_with_borg_list(pool: PgPool) {
    let fx = fixture(&pool).await;
    sqlx::query(
        "WITH archive AS (INSERT INTO archives (repo_id, name) VALUES ($1, 'nightly') RETURNING \
         id) INSERT INTO archive_index_jobs (archive_id, status) SELECT id, 'failed' FROM archive",
    )
    .bind(fx.repo_id)
    .execute(&pool)
    .await
    .unwrap();
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (borg_dir, _guard) =
        crate::test_support::install_fake_borg(&fake_borg(&["etc/hosts".to_owned()], &[])).await;

    let Json(contents) = crate::api::archives::list_contents(
        State(fx.state.clone()),
        fx.viewer.clone(),
        AxumPath((fx.repo_id, "nightly".to_owned())),
        axum::extract::Query(crate::api::archives::ContentsQuery {
            path: Some("etc".to_owned()),
            limit: None,
        }),
    )
    .await
    .unwrap();

    assert_eq!(contents.index_status, shared::types::IndexStatus::Failed);
    let paths: Vec<_> = contents.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, ["etc/hosts"]);
    let calls = tokio::fs::read_to_string(borg_dir.path().join("calls"))
        .await
        .unwrap();
    assert!(calls.contains("--pattern +sh:etc/**"), "{calls}");
}
