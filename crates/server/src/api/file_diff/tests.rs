// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use shared::responses::FileDiffLineKind;
use sqlx::PgPool;

use super::*;
use crate::{
    db,
    test_support::{build_test_state, insert_auth_user, install_fake_borg},
};

/// A borg whose `extract --stdout` prints a different version of the file for
/// each archive name: `old` and `new` differ in one line, `binary` holds a NUL
/// byte, `huge` is past the limit, and `gone` does not hold the path at all.
const FAKE_BORG: &str = "#!/bin/sh\ncase \"$5\" in\n  *::old) printf 'user \
                         www;\\nworker_processes auto;\\n' ;;\n  *::new) printf 'user \
                         www;\\nworker_processes 4;\\n' ;;\n  *::binary) printf 'ELF\\000' ;;\n  \
                         *::huge) head -c 1100000 /dev/zero ;;\n  *::gone) echo \"Include pattern \
                         '$7' never matched.\" >&2; exit 1 ;;\nesac\n";

struct Fixture {
    state: AppState,
    reader: AuthUser,
    viewer: AuthUser,
    repo_id: i64,
    other_repo_id: i64,
}

async fn insert_repo(state: &AppState, name: &str) -> i64 {
    let passphrase = shared::crypto::encrypt_passphrase("secret", &state.encryption_key).unwrap();
    db::insert_repo(
        &state.pool,
        &db::InsertRepoParams {
            name,
            repo_path: &format!("/backups/{name}"),
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
    .id
}

async fn grant(state: &AppState, user: &AuthUser, repo_id: i64, can_extract: bool) {
    db::upsert_repo_permission(
        &state.pool,
        &db::UpsertRepoPermissionParams {
            user_id: user.user_id,
            repo_id,
            can_view: true,
            can_backup: false,
            can_modify_schedules: false,
            can_extract,
            can_delete: false,
        },
    )
    .await
    .unwrap();
}

async fn fixture(pool: PgPool) -> Fixture {
    let state = build_test_state(pool, b"file-diff-test-key");
    let reader = insert_auth_user(&state.pool, "diff-reader").await;
    let viewer = insert_auth_user(&state.pool, "diff-viewer").await;
    let repo_id = insert_repo(&state, "nightly").await;
    let other_repo_id = insert_repo(&state, "offsite").await;
    grant(&state, &reader, repo_id, true).await;
    grant(&state, &viewer, repo_id, false).await;
    Fixture {
        state,
        reader,
        viewer,
        repo_id,
        other_repo_id,
    }
}

impl Fixture {
    async fn diff(
        &self,
        user: &AuthUser,
        archive: &str,
        against: &str,
        against_repo_id: Option<i64>,
    ) -> Result<FileDiffResponse, ApiError> {
        diff_file(
            State(self.state.clone()),
            user.clone(),
            AxumPath((self.repo_id, archive.to_owned())),
            Query(FileDiffQuery {
                path: "etc/nginx/nginx.conf".to_owned(),
                against: against.to_owned(),
                against_repo_id,
                whole_file: false,
            }),
        )
        .await
        .map(|Json(response)| response)
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn two_versions_of_a_text_file_are_compared_line_by_line(pool: PgPool) {
    let fixture = fixture(pool).await;
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = install_fake_borg(FAKE_BORG).await;

    let response = fixture
        .diff(&fixture.reader, "new", "old", None)
        .await
        .unwrap();

    let FileDiffResponse::Text {
        added,
        removed,
        hunks,
    } = response
    else {
        panic!("expected a text diff, got {response:?}");
    };
    assert_eq!((added, removed), (1, 1));
    let kinds: Vec<_> = hunks
        .iter()
        .flat_map(|h| &h.lines)
        .map(|l| l.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            FileDiffLineKind::Context,
            FileDiffLineKind::Removed,
            FileDiffLineKind::Added,
        ]
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn a_binary_version_is_reported_with_its_side(pool: PgPool) {
    let fixture = fixture(pool).await;
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = install_fake_borg(FAKE_BORG).await;

    let result = fixture.diff(&fixture.reader, "binary", "old", None).await;

    assert!(
        matches!(
            result,
            Ok(FileDiffResponse::Binary {
                side: FileDiffSide::Target
            })
        ),
        "got {result:?}"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn a_version_past_the_limit_is_too_large_to_compare(pool: PgPool) {
    let fixture = fixture(pool).await;
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = install_fake_borg(FAKE_BORG).await;

    let result = fixture.diff(&fixture.reader, "new", "huge", None).await;

    assert!(
        matches!(
            result,
            Ok(FileDiffResponse::TooLarge {
                side: FileDiffSide::Base,
                limit_bytes: FILE_DIFF_MAX_BYTES,
            })
        ),
        "got {result:?}"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn a_file_missing_from_an_archive_is_not_found(pool: PgPool) {
    let fixture = fixture(pool).await;
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = install_fake_borg(FAKE_BORG).await;

    let result = fixture.diff(&fixture.reader, "new", "gone", None).await;

    assert!(
        matches!(result, Err(ApiError::NotFound(_))),
        "got {result:?}"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn a_borg_that_cannot_start_fails_the_comparison(pool: PgPool) {
    let fixture = fixture(pool).await;
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let missing = tempfile::tempdir().expect("create temp dir");
    let _guard = crate::borg::override_binary_for_tests(missing.path().join("borg"));

    let result = fixture.diff(&fixture.reader, "new", "old", None).await;

    assert!(
        matches!(&result, Err(ApiError::Internal(message)) if message.contains("spawn")),
        "got {result:?}"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn an_archive_cannot_be_compared_with_itself(pool: PgPool) {
    let fixture = fixture(pool).await;

    let result = fixture.diff(&fixture.reader, "new", "new", None).await;

    assert!(
        matches!(result, Err(ApiError::BadRequest(_))),
        "got {result:?}"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn comparing_needs_extract_permission(pool: PgPool) {
    let fixture = fixture(pool).await;

    let result = fixture.diff(&fixture.viewer, "new", "old", None).await;

    assert!(
        matches!(result, Err(ApiError::Forbidden(_))),
        "got {result:?}"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn comparing_across_repositories_needs_extract_permission_on_both(pool: PgPool) {
    let fixture = fixture(pool).await;

    let refused = fixture
        .diff(&fixture.reader, "new", "old", Some(fixture.other_repo_id))
        .await;
    assert!(
        matches!(refused, Err(ApiError::Forbidden(_))),
        "got {refused:?}"
    );

    grant(&fixture.state, &fixture.reader, fixture.other_repo_id, true).await;
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = install_fake_borg(FAKE_BORG).await;
    let allowed = fixture
        .diff(&fixture.reader, "new", "old", Some(fixture.other_repo_id))
        .await;
    assert!(
        matches!(
            allowed,
            Ok(FileDiffResponse::Text {
                added: 1,
                removed: 1,
                ..
            })
        ),
        "got {allowed:?}"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn the_same_archive_name_in_another_repository_can_be_compared(pool: PgPool) {
    let fixture = fixture(pool).await;
    grant(&fixture.state, &fixture.reader, fixture.other_repo_id, true).await;
    let _gate = crate::borg::acquire_test_binary_gate().await;
    let (_borg_dir, _guard) = install_fake_borg(FAKE_BORG).await;

    let result = fixture
        .diff(&fixture.reader, "new", "new", Some(fixture.other_repo_id))
        .await;

    assert!(
        matches!(result, Ok(FileDiffResponse::Text { added: 0, removed: 0, ref hunks }) if hunks.is_empty()),
        "got {result:?}"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL"]
async fn an_invalid_path_is_refused(pool: PgPool) {
    let fixture = fixture(pool).await;

    let result = diff_file(
        State(fixture.state.clone()),
        fixture.reader.clone(),
        AxumPath((fixture.repo_id, "new".to_owned())),
        Query(FileDiffQuery {
            path: "../etc/shadow".to_owned(),
            against: "old".to_owned(),
            against_repo_id: None,
            whole_file: false,
        }),
    )
    .await;

    assert!(
        matches!(result, Err(ApiError::BadRequest(_))),
        "got {result:?}"
    );
}

#[test]
fn utf8_without_nul_bytes_is_text() {
    assert!(matches!(
        Version::from(b"listen 80;\n".to_vec()),
        Version::Text(text) if text == "listen 80;\n"
    ));
}

#[test]
fn a_nul_byte_makes_a_version_binary() {
    assert!(matches!(
        Version::from(b"ELF\0\x01".to_vec()),
        Version::Binary
    ));
}

#[test]
fn invalid_utf8_makes_a_version_binary() {
    assert!(matches!(
        Version::from(vec![0xFF, 0xFE, b'a']),
        Version::Binary
    ));
}

#[test]
fn a_version_that_is_not_text_says_which_side_it_is() {
    assert_eq!(
        Version::Binary.into_text(FileDiffSide::Target),
        Err(FileDiffResponse::Binary {
            side: FileDiffSide::Target
        })
    );
    assert_eq!(
        Version::TooLarge.into_text(FileDiffSide::Base),
        Err(FileDiffResponse::TooLarge {
            side: FileDiffSide::Base,
            limit_bytes: FILE_DIFF_MAX_BYTES,
        })
    );
}

/// The live comparison: the archived version through borg, the current one
/// through a fake connected agent.
mod live {
    use shared::live_file::{LiveFile, LiveReadRefusal, LiveText};
    use tokio::sync::mpsc;

    use super::*;
    use crate::pending::Claim;

    async fn web_agent(fixture: &Fixture) -> i64 {
        let agent_id = db::insert_agent(&fixture.state.pool, "web-01", None, "hash", None, None)
            .await
            .unwrap()
            .id;
        sqlx::query!(
            "INSERT INTO backup_reports (agent_id, repo_id, started_at, finished_at, status, \
             archive_name) VALUES ($1, $2, now(), now(), 'success', 'old')",
            agent_id,
            fixture.repo_id,
        )
        .execute(&fixture.state.pool)
        .await
        .unwrap();
        agent_id
    }

    /// Connects `agent_id` and answers its first live read with `file`,
    /// returning the path it was asked for.
    async fn answer_with(
        fixture: &Fixture,
        agent_id: i64,
        file: LiveFile,
    ) -> tokio::task::JoinHandle<String> {
        let (tx, mut rx) = mpsc::channel(4);
        fixture
            .state
            .registry
            .register(agent_id, tx, false, None, None)
            .await;
        let pending = fixture.state.pending_live_reads.clone();
        tokio::spawn(async move {
            let Some(ServerToAgent::ReadLiveFile { request_id, path }) = rx.recv().await else {
                panic!("expected a live read");
            };
            let Claim::Claimed(answer) = pending.claim(&request_id, agent_id).await else {
                panic!("the read is pending for this agent");
            };
            answer.send(file).unwrap();
            path
        })
    }

    async fn diff_live(fixture: &Fixture, user: &AuthUser) -> Result<FileDiffResponse, ApiError> {
        diff_file_live(
            State(fixture.state.clone()),
            user.clone(),
            AxumPath((fixture.repo_id, "old".to_owned())),
            Query(LiveFileDiffQuery {
                path: "etc/nginx/nginx.conf".to_owned(),
                whole_file: false,
            }),
        )
        .await
        .map(|Json(response)| response)
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn an_archive_is_compared_with_the_file_on_its_host_now(pool: PgPool) {
        let fixture = fixture(pool).await;
        let _gate = crate::borg::acquire_test_binary_gate().await;
        let (_borg_dir, _guard) = install_fake_borg(FAKE_BORG).await;
        let agent_id = web_agent(&fixture).await;
        let agent = answer_with(
            &fixture,
            agent_id,
            LiveFile::Text {
                text: LiveText("user www;\nworker_processes 8;\n".to_owned()),
                size: 30,
                modified: None,
            },
        )
        .await;

        let response = diff_live(&fixture, &fixture.reader).await.unwrap();

        assert_eq!(agent.await.unwrap(), "etc/nginx/nginx.conf");
        assert!(matches!(
            response,
            FileDiffResponse::Text {
                added: 1,
                removed: 1,
                ..
            }
        ));
        let events = crate::test_support::audit_events(&fixture.state.pool).await;
        assert!(events.contains(&AuditEvent::ReadLiveFile {
            hostname: "web-01".to_owned(),
            path: "etc/nginx/nginx.conf".to_owned(),
        }));
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn a_refused_read_says_why(pool: PgPool) {
        let fixture = fixture(pool).await;
        let _gate = crate::borg::acquire_test_binary_gate().await;
        let (_borg_dir, _guard) = install_fake_borg(FAKE_BORG).await;
        let agent_id = web_agent(&fixture).await;
        let agent = answer_with(
            &fixture,
            agent_id,
            LiveFile::Refused {
                reason: LiveReadRefusal::NotInSources,
            },
        )
        .await;

        let response = diff_live(&fixture, &fixture.reader).await.unwrap();

        agent.await.unwrap();
        assert_eq!(
            response,
            FileDiffResponse::LiveUnavailable {
                reason: LiveReadRefusal::NotInSources
            }
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn an_offline_host_cannot_be_read(pool: PgPool) {
        let fixture = fixture(pool).await;
        let _gate = crate::borg::acquire_test_binary_gate().await;
        let (_borg_dir, _guard) = install_fake_borg(FAKE_BORG).await;
        web_agent(&fixture).await;

        let response = diff_live(&fixture, &fixture.reader).await.unwrap();

        assert_eq!(
            response,
            FileDiffResponse::LiveUnavailable {
                reason: LiveReadRefusal::AgentOffline
            }
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn a_large_live_file_is_not_compared(pool: PgPool) {
        let fixture = fixture(pool).await;
        let _gate = crate::borg::acquire_test_binary_gate().await;
        let (_borg_dir, _guard) = install_fake_borg(FAKE_BORG).await;
        let agent_id = web_agent(&fixture).await;
        let agent = answer_with(&fixture, agent_id, LiveFile::TooLarge { size: 5_000_000 }).await;

        let response = diff_live(&fixture, &fixture.reader).await.unwrap();

        agent.await.unwrap();
        assert_eq!(
            response,
            FileDiffResponse::TooLarge {
                side: FileDiffSide::Target,
                limit_bytes: FILE_DIFF_MAX_BYTES,
            }
        );
    }

    /// Reading a file live needs extract permission, as downloading it does.
    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires DATABASE_URL"]
    async fn a_viewer_cannot_read_a_file_live(pool: PgPool) {
        let fixture = fixture(pool).await;
        web_agent(&fixture).await;

        let result = diff_live(&fixture, &fixture.viewer).await;

        assert!(matches!(result, Err(ApiError::Forbidden(_))));
        assert_eq!(
            crate::test_support::audit_events(&fixture.state.pool).await,
            Vec::new()
        );
    }
}
