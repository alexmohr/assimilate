// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Named pipes that carry database dumps into `borg create`.
//!
//! borg opens the pipes one at a time, in whatever order it walks the
//! directory, so every pipe gets a writer that waits for borg to open it and
//! only then starts its dump. A dump that fails keeps its pipe open while borg
//! is stopped: closing it would hand borg an end of file, and borg would
//! commit the truncated dump.

use std::{
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    time::Duration,
};

use futures_util::{StreamExt as _, stream::FuturesUnordered};
use nix::{fcntl::OFlag, sys::stat::Mode};
use shared::database::MANIFEST_FILE_NAME;
use tokio::sync::watch;

use super::{CommandExec, DumpFailure, DumpJob, ExecError, StreamOutcome, manifest};

/// The dump pipes of one backup, below one schedule's dump directory.
#[derive(Debug)]
pub(super) struct DumpPipes {
    pipes: Vec<(DumpJob, PathBuf)>,
}

enum Fed {
    Dumped,
    Unread(String),
}

impl Fed {
    fn unread(self) -> Option<String> {
        match self {
            Self::Unread(path) => Some(path),
            Self::Dumped => None,
        }
    }
}

/// A failed dump, with its pipe still open so borg never sees the dump end.
type HeldFailure = Box<(DumpFailure, Option<tokio::fs::File>)>;

impl DumpPipes {
    /// Creates `dir` afresh with the manifest and one pipe per job. Anything
    /// already there is left over from an earlier run and removed first.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory, the manifest or a pipe cannot be
    /// created.
    pub async fn create(dir: PathBuf, jobs: Vec<DumpJob>) -> std::io::Result<Self> {
        match tokio::fs::remove_dir_all(&dir).await {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            Ok(()) | Err(_) => {}
        }
        create_private_dir(&dir).await?;
        let manifest =
            serde_json::to_vec_pretty(&manifest(&jobs)).map_err(std::io::Error::other)?;
        tokio::fs::write(dir.join(MANIFEST_FILE_NAME), manifest).await?;

        let mut pipes = Vec::with_capacity(jobs.len());
        for job in jobs {
            let path = dir.join(&job.entry.path);
            // Every entry lies below `dir`, so it always has a parent.
            create_private_dir(path.parent().unwrap_or(&dir)).await?;
            let fifo = path.clone();
            tokio::task::spawn_blocking(move || {
                nix::unistd::mkfifo(&fifo, Mode::S_IRUSR | Mode::S_IWUSR)
            })
            .await
            .map_err(std::io::Error::other)?
            .map_err(std::io::Error::from)?;
            pipes.push((job, path));
        }
        Ok(Self { pipes })
    }

    /// Runs `borg` while feeding every pipe from its dump.
    ///
    /// When a dump fails, `borg` is dropped while the failed dump's pipe is
    /// still open, which stops borg through its graceful termination, and the
    /// pipe is only closed `hold` later, once borg is gone.
    ///
    /// # Errors
    ///
    /// Returns the first dump that failed. borg was stopped before it could
    /// commit the archive.
    pub async fn stream<E, F>(
        &self,
        exec: &E,
        borg: F,
        hold: Duration,
    ) -> Result<StreamOutcome<F::Output>, Box<DumpFailure>>
    where
        E: CommandExec,
        F: Future,
    {
        let (done_tx, done_rx) = watch::channel(false);
        let mut feeders: FuturesUnordered<_> = self
            .pipes
            .iter()
            .map(|(job, path)| feed(exec, job, path, done_rx.clone()))
            .collect();
        let mut borg = Box::pin(borg);
        let mut settled = Vec::new();

        let first = loop {
            tokio::select! {
                result = &mut borg => break Ok(result),
                Some(fed) = feeders.next() => match fed {
                    Ok(fed) => settled.push(fed),
                    Err(failure) => break Err(failure),
                },
            }
        };

        match first {
            Ok(result) => {
                let _ = done_tx.send(true);
                self.release_waiting_writers().await;
                while let Some(fed) = feeders.next().await {
                    match fed {
                        Ok(fed) => settled.push(fed),
                        // borg is done, so the archive is decided either way;
                        // a dump that broke off after it ended changes nothing.
                        Err(held) => tracing::warn!("{}", held.0),
                    }
                }
                Ok(StreamOutcome {
                    borg: result,
                    unread: settled.into_iter().filter_map(Fed::unread).collect(),
                })
            }
            Err(held) => {
                let (failure, held_pipe) = *held;
                tracing::error!("{failure}; stopping borg before it commits the archive");
                drop(borg);
                let _ = done_tx.send(true);
                tokio::time::sleep(hold).await;
                drop(held_pipe);
                self.release_waiting_writers().await;
                while feeders.next().await.is_some() {}
                Err(Box::new(failure))
            }
        }
    }

    /// Opens, and at once closes, the read end of every pipe, so a writer
    /// still waiting for borg wakes up, sees the backup is over and leaves.
    /// A pipe that cannot be opened has no writer waiting on it.
    async fn release_waiting_writers(&self) {
        for (_, path) in &self.pipes {
            drop(
                tokio::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(OFlag::O_NONBLOCK.bits())
                    .open(path)
                    .await,
            );
        }
    }
}

/// Waits for borg to open `path`, then streams `job`'s dump into it.
async fn feed<E: CommandExec>(
    exec: &E,
    job: &DumpJob,
    path: &Path,
    done: watch::Receiver<bool>,
) -> Result<Fed, HeldFailure> {
    let failure = |error: ExecError| DumpFailure {
        source: job.entry.source.to_string(),
        database: job.entry.database.clone(),
        error,
    };
    let mut pipe = tokio::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .await
        .map_err(|e| Box::new((failure(ExecError::Io(e)), None)))?;
    if *done.borrow() {
        return Ok(Fed::Unread(job.entry.path.clone()));
    }
    tracing::info!(
        "Dumping database {} of {} into the archive",
        job.entry.database,
        job.entry.source
    );
    match exec.run(&job.target, job.command(), &mut pipe).await {
        Ok(()) => Ok(Fed::Dumped),
        Err(e) => Err(Box::new((failure(e), Some(pipe)))),
    }
}

async fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    tokio::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).await
}

#[cfg(test)]
mod tests {
    use shared::database::{
        DatabaseEngine, DatabaseManifest, DatabaseManifestEntry, DatabaseRuntime, DatabaseSourceId,
        DatabaseSourceKind, DumpFormat,
    };
    use tokio::io::AsyncReadExt as _;

    use super::{
        super::{
            DumpTarget, PodTarget,
            fake::{FakeExec, Script},
        },
        *,
    };

    const HOLD: Duration = Duration::from_millis(50);

    fn job(database: &str) -> DumpJob {
        DumpJob {
            entry: DatabaseManifestEntry {
                source: DatabaseSourceId {
                    runtime: DatabaseRuntime::Kubernetes,
                    namespace: "apps".to_owned(),
                    name: "pg".to_owned(),
                },
                kind: DatabaseSourceKind::CnpgCluster,
                engine: DatabaseEngine::Postgresql,
                pod: "pg-1".to_owned(),
                database: database.to_owned(),
                path: format!("apps/pg/{database}.pgdump"),
                format: DumpFormat::PgCustom,
            },
            target: DumpTarget::Pod(PodTarget {
                namespace: "apps".to_owned(),
                pod: "pg-1".to_owned(),
                container: "postgres".to_owned(),
            }),
            user: "postgres".to_owned(),
        }
    }

    /// Reads `paths` the way borg would: one after another, each to its end.
    async fn read_like_borg(paths: Vec<PathBuf>) -> Vec<Vec<u8>> {
        let mut contents = Vec::new();
        for path in paths {
            let mut file = tokio::fs::File::open(&path).await.unwrap();
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).await.unwrap();
            contents.push(buf);
        }
        contents
    }

    #[tokio::test]
    async fn every_dump_reaches_borg_and_the_manifest_lists_them() {
        let root = tempfile::tempdir().unwrap();
        let exec = FakeExec::default();
        exec.script("pg-1", "--dbname=app", Script::Output(b"app dump".to_vec()));
        exec.script(
            "pg-1",
            "--dbname=shop",
            Script::Output(b"shop dump".to_vec()),
        );
        let dir = root.path().join("schedule-1");
        let pipes = DumpPipes::create(dir.clone(), vec![job("app"), job("shop")])
            .await
            .unwrap();
        assert_eq!(pipes.pipes.len(), 2);

        let manifest: DatabaseManifest =
            serde_json::from_slice(&tokio::fs::read(dir.join(MANIFEST_FILE_NAME)).await.unwrap())
                .unwrap();
        assert_eq!(manifest.dumps.len(), 2);

        // borg walks the directory in its own order, not the job order.
        let borg = read_like_borg(vec![
            dir.join("apps/pg/shop.pgdump"),
            dir.join("apps/pg/app.pgdump"),
        ]);
        let outcome = pipes.stream(&exec, borg, HOLD).await.unwrap();
        assert_eq!(outcome.borg, [b"shop dump".to_vec(), b"app dump".to_vec()]);
        assert_eq!(outcome.unread.len(), 0);
    }

    #[tokio::test]
    async fn a_dump_borg_never_opens_is_reported_and_never_runs() {
        let root = tempfile::tempdir().unwrap();
        let exec = FakeExec::default();
        exec.script("pg-1", "--dbname=app", Script::Output(b"app".to_vec()));
        let dir = root.path().join("s");
        let pipes = DumpPipes::create(dir.clone(), vec![job("app"), job("excluded")])
            .await
            .unwrap();
        let borg = read_like_borg(vec![dir.join("apps/pg/app.pgdump")]);
        let outcome = pipes.stream(&exec, borg, HOLD).await.unwrap();
        assert_eq!(outcome.unread, ["apps/pg/excluded.pgdump"]);
        assert_eq!(*exec.calls.lock().unwrap(), ["pg-1 --dbname=app"]);
    }

    #[tokio::test]
    async fn a_failed_dump_stops_borg_before_it_sees_the_end() {
        let root = tempfile::tempdir().unwrap();
        let exec = FakeExec::default();
        exec.script(
            "pg-1",
            "--dbname=app",
            Script::FailAfter(b"half a dump".to_vec()),
        );
        let dir = root.path().join("s");
        let pipes = DumpPipes::create(dir.clone(), vec![job("app"), job("other")])
            .await
            .unwrap();
        // borg reads the pipe to its end and would then commit; an error
        // back means it was stopped before it ever saw that end.
        let borg = read_like_borg(vec![dir.join("apps/pg/app.pgdump")]);
        let failure = pipes.stream(&exec, borg, HOLD).await.unwrap_err();
        assert_eq!(failure.database, "app");
        assert_eq!(failure.source, "apps/pg");
        assert!(failure.to_string().contains("exit status 1"));
        // The other dump never started.
        assert_eq!(*exec.calls.lock().unwrap(), ["pg-1 --dbname=app"]);
    }

    #[tokio::test]
    async fn a_dump_directory_that_cannot_be_cleared_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("file");
        tokio::fs::write(&file, b"not a directory").await.unwrap();
        assert!(
            DumpPipes::create(file.join("s"), vec![job("app")])
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_dump_cut_off_by_borg_ending_early_is_only_logged() {
        let root = tempfile::tempdir().unwrap();
        let exec = FakeExec::default();
        // Far more than a pipe holds, so writing it outlasts the reader.
        exec.script("pg-1", "--dbname=app", Script::Output(vec![b'x'; 1 << 20]));
        let dir = root.path().join("s");
        let pipes = DumpPipes::create(dir.clone(), vec![job("app")])
            .await
            .unwrap();
        let path = dir.join("apps/pg/app.pgdump");
        // borg opens the pipe, reads a little and gives up.
        let borg = async move {
            let mut file = tokio::fs::File::open(&path).await.unwrap();
            let mut first = [0u8; 16];
            file.read_exact(&mut first).await.unwrap();
        };
        let (outcome, logs) = crate::tests::logs::logs_while(pipes.stream(&exec, borg, HOLD)).await;
        assert!(outcome.is_ok(), "borg's own result decides the backup");
        assert!(
            logs.contains("dumping database app of apps/pg failed"),
            "{logs}"
        );
    }

    #[tokio::test]
    async fn leftovers_from_an_earlier_run_are_replaced() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("s");
        tokio::fs::create_dir_all(dir.join("stale")).await.unwrap();
        let pipes = DumpPipes::create(dir.clone(), vec![job("app")])
            .await
            .unwrap();
        assert_eq!(pipes.pipes.len(), 1);
        assert!(tokio::fs::metadata(dir.join("stale")).await.is_err());
        let mode = tokio::fs::metadata(&dir)
            .await
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        let fifo = tokio::fs::metadata(dir.join("apps/pg/app.pgdump"))
            .await
            .unwrap();
        assert!(std::os::unix::fs::FileTypeExt::is_fifo(&fifo.file_type()));
    }

    #[tokio::test]
    async fn a_backup_without_dumps_just_runs_borg() {
        let root = tempfile::tempdir().unwrap();
        let pipes = DumpPipes::create(root.path().join("s"), Vec::new())
            .await
            .unwrap();
        assert_eq!(pipes.pipes.len(), 0);
        let outcome = pipes
            .stream(&FakeExec::default(), async { 7 }, HOLD)
            .await
            .unwrap();
        assert_eq!(outcome.borg, 7);
    }
}
