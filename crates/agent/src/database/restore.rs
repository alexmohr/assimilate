// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Restoring a dump into the database server its source runs on now.
//!
//! The dump streams into the restore client's stdin; nothing is staged on
//! disk. A restore into a new database creates it first and drops it again
//! when the restore fails, so a failed restore leaves nothing behind. A
//! restore over the original database runs in one transaction, so a failed
//! one leaves it as it was.

use shared::database::{DatabaseRestore, DatabaseRestoreTarget};
use tokio::io::AsyncRead;
use tracing::{info, warn};

use super::{
    CommandExec, SourceCandidate, commands::FormatMismatch, exec::ExecError, stdin::Stdin,
    target::DumpTarget,
};

/// Why a restore failed.
#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    #[error(transparent)]
    Format(#[from] FormatMismatch),
    #[error("could not create the database {database}: {error}")]
    Create {
        database: String,
        #[source]
        error: Box<ExecError>,
    },
    #[error("restoring the database {database} failed: {error}")]
    Restore {
        database: String,
        #[source]
        error: Box<ExecError>,
    },
}

/// Restores the dump `dump` yields into `restore`'s target database on
/// `candidate`'s server.
///
/// # Errors
///
/// Returns an error when the new database cannot be created or the restore
/// fails. A new database the restore created is dropped again first.
pub async fn restore_dump<E: CommandExec>(
    exec: &E,
    candidate: &SourceCandidate,
    restore: &DatabaseRestore,
    dump: &mut (dyn AsyncRead + Unpin + Send),
) -> Result<(), RestoreError> {
    let target = &candidate.target;
    let client = candidate.client();
    let database = restore.target_database();
    let command = client.restore(restore)?;
    let created = match &restore.target {
        DatabaseRestoreTarget::NewDatabase { name } => {
            run(exec, target, client.create_database(name), Stdin::Closed)
                .await
                .map_err(|error| RestoreError::Create {
                    database: name.clone(),
                    error: Box::new(error),
                })?;
            info!("Created the database {name} in {target} to restore into");
            Some(name)
        }
        DatabaseRestoreTarget::Original => None,
    };
    info!(
        "Restoring database {} of {} into {database} in {target}",
        restore.database, restore.source
    );
    let Err(error) = run(exec, target, command, Stdin::From(dump)).await else {
        info!("Restored database {database} in {target}");
        return Ok(());
    };
    if let Some(name) = created {
        drop_created(exec, candidate, name).await;
    }
    Err(RestoreError::Restore {
        database: database.to_owned(),
        error: Box::new(error),
    })
}

/// Drops the database `name` a failed restore created. Best effort: a
/// failure is logged, and the restore's own error is what gets reported.
async fn drop_created<E: CommandExec>(exec: &E, candidate: &SourceCandidate, name: &str) {
    let command = candidate.client().drop_database(name);
    match run(exec, &candidate.target, command, Stdin::Closed).await {
        Ok(()) => info!("Dropped the database {name} the failed restore had created"),
        Err(e) => warn!("could not drop the database {name} the failed restore had created: {e}"),
    }
}

/// Runs `command`, discarding what it writes to stdout.
async fn run<E: CommandExec>(
    exec: &E,
    target: &DumpTarget,
    command: Vec<String>,
    input: Stdin<'_>,
) -> Result<(), ExecError> {
    exec.run(target, command, input, &mut tokio::io::sink())
        .await
}

#[cfg(all(test, unix))]
mod tests {
    use shared::database::{DatabaseEngine, DatabaseRuntime, DatabaseSourceId, DumpFormat};

    use super::{
        super::{
            HostExec, PodTarget, RuntimeClients,
            fake::{FakeExec, Script},
            host_source,
        },
        *,
    };
    use crate::tests::host_postgres::HostPostgres;

    const DUMP: &[u8] = b"PGDMP custom dump";

    fn restoring(target: DatabaseRestoreTarget) -> DatabaseRestore {
        DatabaseRestore {
            source: DatabaseSourceId {
                runtime: DatabaseRuntime::Host,
                namespace: String::new(),
                name: "postgresql".to_owned(),
            },
            database: "shop".to_owned(),
            archive_path: "dumps/host/postgresql/shop.pgdump".to_owned(),
            format: DumpFormat::PgCustom,
            target,
        }
    }

    fn into_new() -> DatabaseRestore {
        restoring(DatabaseRestoreTarget::NewDatabase {
            name: "shop_restored".to_owned(),
        })
    }

    /// Restores [`DUMP`] as `restore` into the host's server, whose client
    /// programs `pg` stands in for.
    async fn restore_on_host(
        pg: &HostPostgres,
        restore: &DatabaseRestore,
    ) -> Result<(), RestoreError> {
        let runtimes = pg.runtimes();
        let clients = RuntimeClients {
            host: Some(HostExec::new(runtimes.host_bin_dir.as_deref())),
            ..RuntimeClients::default()
        };
        let mut dump = DUMP;
        restore_dump(&clients, &host_source(&runtimes), restore, &mut dump).await
    }

    #[tokio::test]
    async fn a_dump_is_restored_into_a_new_database_created_for_it() {
        let pg = HostPostgres::install(&[]).await;

        restore_on_host(&pg, &into_new()).await.unwrap();

        assert_eq!(pg.programs().await, ["createdb", "pg_restore"]);
        let log = pg.log().await;
        assert!(log.contains("--port=5433 --username="), "{log}");
        assert!(
            log.contains("--maintenance-db=postgres shop_restored"),
            "{log}"
        );
        assert!(log.contains("--dbname=shop_restored"), "{log}");
        assert_eq!(pg.restored().await, DUMP);
    }

    #[tokio::test]
    async fn a_failed_restore_drops_the_database_it_created() {
        let pg = HostPostgres::install(&["pg_restore"]).await;

        let (result, logs) =
            crate::tests::logs::logs_while(restore_on_host(&pg, &into_new())).await;

        let err = result.unwrap_err();
        assert!(matches!(err, RestoreError::Restore { .. }));
        let message = err.to_string();
        assert!(
            message.starts_with("restoring the database shop_restored failed"),
            "{message}"
        );
        assert!(message.contains("pg_restore: error: it broke"), "{message}");
        assert_eq!(pg.programs().await, ["createdb", "pg_restore", "dropdb"]);
        assert!(
            logs.contains("Dropped the database shop_restored"),
            "{logs}"
        );
    }

    #[tokio::test]
    async fn a_database_that_cannot_be_dropped_again_is_logged() {
        let pg = HostPostgres::install(&["pg_restore", "dropdb"]).await;

        let (result, logs) =
            crate::tests::logs::logs_while(restore_on_host(&pg, &into_new())).await;

        assert!(matches!(result, Err(RestoreError::Restore { .. })));
        assert!(
            logs.contains("could not drop the database shop_restored"),
            "{logs}"
        );
    }

    #[tokio::test]
    async fn a_new_database_that_cannot_be_created_is_never_restored_into() {
        let pg = HostPostgres::install(&["createdb"]).await;

        let err = restore_on_host(&pg, &into_new()).await.unwrap_err();

        assert!(matches!(err, RestoreError::Create { .. }));
        let message = err.to_string();
        assert!(
            message.starts_with("could not create the database shop_restored"),
            "{message}"
        );
        assert!(message.contains("createdb: error: it broke"), "{message}");
        assert_eq!(pg.programs().await, ["createdb"]);
    }

    #[tokio::test]
    async fn a_failed_restore_over_the_original_drops_nothing() {
        let pg = HostPostgres::install(&["pg_restore"]).await;

        let err = restore_on_host(&pg, &restoring(DatabaseRestoreTarget::Original))
            .await
            .unwrap_err();

        assert!(err.to_string().contains("database shop failed"), "{err}");
        assert_eq!(pg.programs().await, ["pg_restore"]);
    }

    #[tokio::test]
    async fn a_restore_over_the_original_in_a_pod_reads_the_dump() {
        let exec = FakeExec::default();
        exec.script("pg-1", "--dbname=shop", Script::Output(Vec::new()));
        let candidate = SourceCandidate {
            id: DatabaseSourceId {
                runtime: DatabaseRuntime::Kubernetes,
                namespace: "apps".to_owned(),
                name: "pg".to_owned(),
            },
            kind: shared::database::DatabaseSourceKind::CnpgCluster,
            engine: DatabaseEngine::Postgresql,
            target: DumpTarget::Pod(PodTarget {
                namespace: "apps".to_owned(),
                pod: "pg-1".to_owned(),
                container: "postgres".to_owned(),
            }),
            user: "postgres".to_owned(),
            password_env: None,
            databases: None,
        };
        let mut dump = DUMP;

        restore_dump(
            &exec,
            &candidate,
            &restoring(DatabaseRestoreTarget::Original),
            &mut dump,
        )
        .await
        .unwrap();

        assert_eq!(*exec.inputs.lock().unwrap(), [DUMP.to_vec()]);
        let commands = exec.commands.lock().unwrap();
        let command = commands.first().unwrap();
        assert_eq!(command.first().map(String::as_str), Some("pg_restore"));
        assert!(command.contains(&"--clean".to_owned()));
    }
}
