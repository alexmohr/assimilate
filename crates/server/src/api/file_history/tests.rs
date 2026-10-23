// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use super::*;

fn archive(name: &str) -> FileHistoryArchive {
    FileHistoryArchive {
        name: name.to_owned(),
        start: format!("{name}-start"),
    }
}

fn version(size: i64, mtime: &str) -> Seen {
    Seen::Version {
        size,
        mtime: mtime.to_owned(),
    }
}

fn names(archives: &[FileHistoryArchive]) -> Vec<&str> {
    archives.iter().map(|a| a.name.as_str()).collect()
}

#[test]
fn consecutive_archives_with_the_same_version_are_one_entry() {
    let entries = group(vec![
        (archive("a5"), version(20, "t2")),
        (archive("a4"), version(20, "t2")),
        (archive("a3"), version(10, "t1")),
        (archive("a2"), Seen::Absent),
        (archive("a1"), version(10, "t1")),
        (archive("a0"), Seen::NotIndexed),
    ]);

    assert_eq!(entries.len(), 5);
    assert!(matches!(
        entries.first(),
        Some(FileHistoryEntry::Version { archives, size: 20, mtime })
            if names(archives) == ["a5", "a4"] && mtime == "t2"
    ));
    assert!(matches!(
        entries.get(1),
        Some(FileHistoryEntry::Version { archives, size: 10, .. }) if names(archives) == ["a3"]
    ));
    assert!(matches!(
        entries.get(2),
        Some(FileHistoryEntry::Absent { archives }) if names(archives) == ["a2"]
    ));
    // The same version again after a gap is its own entry: it came back.
    assert!(matches!(
        entries.get(3),
        Some(FileHistoryEntry::Version { archives, size: 10, .. }) if names(archives) == ["a1"]
    ));
    assert!(matches!(
        entries.get(4),
        Some(FileHistoryEntry::NotIndexed { archives }) if names(archives) == ["a0"]
    ));
}

#[test]
fn no_archives_is_no_history() {
    assert_eq!(group(Vec::new()), Vec::new());
}

#[test]
fn only_a_regular_file_is_a_version() {
    let entry = |entry_type: &str| DirEntry {
        name: "x".to_owned(),
        entry_type: entry_type.to_owned(),
        size: 3,
        mtime: "t".to_owned(),
        mode: "-rw-r--r--".to_owned(),
    };
    assert_eq!(Seen::from(Some(&entry("-"))), version(3, "t"));
    assert_eq!(Seen::from(Some(&entry("d"))), Seen::Absent);
    assert_eq!(Seen::from(Some(&entry("l"))), Seen::Absent);
    assert_eq!(Seen::from(None), Seen::Absent);
}

#[test]
fn a_path_splits_into_its_directory_and_name() {
    assert_eq!(
        split_path("etc/nginx/nginx.conf"),
        ("etc/nginx", "nginx.conf")
    );
    assert_eq!(split_path("README"), ("", "README"));
    assert_eq!(index_path("/etc/hosts"), "etc/hosts");
    assert_eq!(index_path("./etc/hosts"), "etc/hosts");
}

mod with_database {
    use sqlx::PgPool;

    use super::*;
    use crate::{
        db,
        test_support::{build_test_state, insert_auth_user},
    };

    struct Fixture {
        state: AppState,
        user: AuthUser,
        repo_id: i64,
        web: i64,
        db_host: i64,
    }

    async fn fixture(pool: PgPool) -> Fixture {
        let state = build_test_state(pool, b"file-history-test-key");
        let user = insert_auth_user(&state.pool, "history-reader").await;
        let passphrase =
            shared::crypto::encrypt_passphrase("secret", &state.encryption_key).unwrap();
        let repo_id = db::insert_repo(
            &state.pool,
            &db::InsertRepoParams {
                name: "nightly",
                repo_path: "/backups/nightly",
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
        .unwrap()
        .id;
        db::upsert_repo_permission(
            &state.pool,
            &db::UpsertRepoPermissionParams {
                user_id: user.user_id,
                repo_id,
                can_view: true,
                can_backup: false,
                can_modify_schedules: false,
                can_extract: false,
                can_delete: false,
            },
        )
        .await
        .unwrap();
        let web = db::insert_agent(&state.pool, "web-01", None, "hash", None, None)
            .await
            .unwrap()
            .id;
        let db_host = db::insert_agent(&state.pool, "db-01", None, "hash", None, None)
            .await
            .unwrap()
            .id;
        Fixture {
            state,
            user,
            repo_id,
            web,
            db_host,
        }
    }

    impl Fixture {
        /// Records a successful backup by `agent_id` that wrote `archive`,
        /// `hours_ago` hours ago.
        async fn backup(&self, agent_id: i64, archive: &str, hours_ago: i32) {
            sqlx::query!(
                "INSERT INTO backup_reports (agent_id, repo_id, started_at, finished_at, status, \
                 archive_name) VALUES ($1, $2, now() - make_interval(hours => $3), now() - \
                 make_interval(hours => $3), 'success', $4)",
                agent_id,
                self.repo_id,
                hours_ago,
                archive,
            )
            .execute(&self.state.pool)
            .await
            .unwrap();
        }

        /// Indexes `archive` as holding `etc/app.conf` with `size` and
        /// `mtime`, or not holding it when `size` is `None`.
        async fn index(&self, archive: &str, file: Option<(i64, &str)>) {
            let archive_id =
                archive_index::ensure_index_job(&self.state.pool, self.repo_id, archive)
                    .await
                    .unwrap();
            let ids = archive_index::ensure_archive_paths(
                &self.state.pool,
                self.repo_id,
                &["etc".to_owned()],
            )
            .await
            .unwrap();
            let entries = std::iter::once(DirEntry {
                name: "other.conf".to_owned(),
                entry_type: "-".to_owned(),
                size: 1,
                mtime: "t0".to_owned(),
                mode: "-rw-r--r--".to_owned(),
            })
            .chain(file.map(|(size, mtime)| DirEntry {
                name: "app.conf".to_owned(),
                entry_type: "-".to_owned(),
                size,
                mtime: mtime.to_owned(),
                mode: "-rw-r--r--".to_owned(),
            }))
            .collect();
            archive_index::replace_archive_dirs(
                &self.state.pool,
                archive_id,
                &[(*ids.get("etc").unwrap(), entries)],
            )
            .await
            .unwrap();
            sqlx::query!(
                "UPDATE archive_index_jobs SET status = 'done' WHERE archive_id = $1",
                archive_id
            )
            .execute(&self.state.pool)
            .await
            .unwrap();
        }

        async fn history(&self, archive: &str) -> Result<FileHistoryResponse, ApiError> {
            file_history(
                State(self.state.clone()),
                self.user.clone(),
                AxumPath((self.repo_id, archive.to_owned())),
                Query(FileHistoryQuery {
                    path: "etc/app.conf".to_owned(),
                }),
            )
            .await
            .map(|Json(response)| response)
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn the_history_covers_only_the_archives_of_the_same_host(pool: PgPool) {
        let fixture = fixture(pool).await;
        fixture.backup(fixture.web, "web-4", 1).await;
        fixture.backup(fixture.web, "web-3", 2).await;
        fixture.backup(fixture.web, "web-2", 3).await;
        fixture.backup(fixture.web, "web-1", 4).await;
        fixture.backup(fixture.web, "web-0", 5).await;
        fixture.backup(fixture.db_host, "db-1", 1).await;
        fixture.index("web-4", Some((20, "t2"))).await;
        fixture.index("web-3", Some((20, "t2"))).await;
        fixture.index("web-2", Some((10, "t1"))).await;
        fixture.index("web-1", None).await;
        fixture.index("db-1", Some((99, "t9"))).await;

        let history = fixture.history("web-3").await.unwrap();

        assert_eq!(history.hostname, "web-01");
        assert_eq!(history.path, "etc/app.conf");
        assert!(!history.agent_online);
        assert_eq!(history.entries.len(), 4);
        assert!(matches!(
            history.entries.first(),
            Some(FileHistoryEntry::Version { archives, size: 20, .. })
                if names(archives) == ["web-4", "web-3"]
        ));
        assert!(matches!(
            history.entries.get(1),
            Some(FileHistoryEntry::Version { archives, size: 10, .. })
                if names(archives) == ["web-2"]
        ));
        assert!(matches!(
            history.entries.get(2),
            Some(FileHistoryEntry::Absent { archives }) if names(archives) == ["web-1"]
        ));
        assert!(matches!(
            history.entries.get(3),
            Some(FileHistoryEntry::NotIndexed { archives }) if names(archives) == ["web-0"]
        ));
    }

    /// A directory the index has never seen holds nothing, and one stored
    /// in several chunks is read until the file turns up.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn entries_are_found_in_chunked_directories_only(pool: PgPool) {
        let fixture = fixture(pool).await;
        let archive_id =
            archive_index::ensure_index_job(&fixture.state.pool, fixture.repo_id, "web-1")
                .await
                .unwrap();
        let ids = archive_index::ensure_archive_paths(
            &fixture.state.pool,
            fixture.repo_id,
            &["etc".to_owned()],
        )
        .await
        .unwrap();
        let file = |name: String| DirEntry {
            name,
            entry_type: "-".to_owned(),
            size: 1,
            mtime: "t".to_owned(),
            mode: "-rw-r--r--".to_owned(),
        };
        // Enough files after app.conf to spill into a second chunk.
        let entries = std::iter::once(file("app.conf".to_owned()))
            .chain((0..2500).map(|i| file(format!("z{i:04}.conf"))))
            .collect();
        archive_index::replace_archive_dirs(
            &fixture.state.pool,
            archive_id,
            &[(*ids.get("etc").unwrap(), entries)],
        )
        .await
        .unwrap();
        let archives = ["web-1".to_owned()];

        let found = archive_index::find_entries(
            &fixture.state.pool,
            fixture.repo_id,
            &archives,
            "etc",
            "app.conf",
        )
        .await
        .unwrap();
        let unindexed = archive_index::find_entries(
            &fixture.state.pool,
            fixture.repo_id,
            &archives,
            "srv",
            "app.conf",
        )
        .await
        .unwrap();

        assert_eq!(
            found.get("web-1").map(|entry| entry.name.as_str()),
            Some("app.conf")
        );
        assert_eq!(unindexed.len(), 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn an_archive_no_backup_recorded_has_no_history(pool: PgPool) {
        let fixture = fixture(pool).await;
        assert!(matches!(
            fixture.history("nowhere").await,
            Err(ApiError::NotFound(_))
        ));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn a_user_who_cannot_view_the_repository_gets_no_history(pool: PgPool) {
        let fixture = fixture(pool).await;
        fixture.backup(fixture.web, "web-1", 1).await;
        let stranger = insert_auth_user(&fixture.state.pool, "history-stranger").await;

        let result = file_history(
            State(fixture.state.clone()),
            stranger,
            AxumPath((fixture.repo_id, "web-1".to_owned())),
            Query(FileHistoryQuery {
                path: "etc/app.conf".to_owned(),
            }),
        )
        .await;

        assert!(matches!(result, Err(ApiError::Forbidden(_))));
    }

    /// Only the archives with no index yet are queued, and only the host's.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn indexing_queues_the_hosts_unindexed_archives(pool: PgPool) {
        let fixture = fixture(pool).await;
        fixture.backup(fixture.web, "web-2", 1).await;
        fixture.backup(fixture.web, "web-1", 2).await;
        fixture.backup(fixture.db_host, "db-1", 1).await;
        fixture.index("web-2", Some((1, "t"))).await;

        let Json(response) = index_file_history(
            State(fixture.state.clone()),
            fixture.user.clone(),
            AxumPath((fixture.repo_id, "web-2".to_owned())),
        )
        .await
        .unwrap();

        assert_eq!(response.queued, 1);
        let statuses = archive_index::index_statuses(
            &fixture.state.pool,
            fixture.repo_id,
            &["web-1".to_owned(), "db-1".to_owned()],
        )
        .await
        .unwrap();
        assert!(statuses.contains_key("web-1"));
        assert!(!statuses.contains_key("db-1"));
    }
}
