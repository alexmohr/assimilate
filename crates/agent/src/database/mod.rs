// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Database dumps streamed into a backup's archive.
//!
//! The agent finds database workloads in its Kubernetes cluster, and for a
//! backup that asks for them creates one named pipe per database below the
//! host's dump directory. borg archives each pipe's content (`--read-special`)
//! while the agent copies a dump running inside the database's pod into it,
//! so nothing is staged on disk. A dump that fails stops borg before it
//! commits: an archive must never hold a truncated dump while reporting
//! success.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use shared::database::{
    DatabaseBackupConfig, DatabaseEngine, DatabaseManifest, DatabaseManifestEntry,
    DiscoveredDatabaseSource, MANIFEST_VERSION,
};

mod discovery;
mod exec;
#[cfg(unix)]
mod pipes;
mod postgres;

pub use self::{
    discovery::{SourceCandidate, find_sources},
    exec::{ExecError, KubeExec, PodExec, PodTarget},
};

/// What a backup that streamed dumps produced.
#[derive(Debug)]
pub struct StreamOutcome<T> {
    /// What the borg run returned.
    pub borg: T,
    /// Dumps borg never opened, by their path in the archive: an exclude
    /// pattern matched them, or borg stopped early. The manifest still lists
    /// them, so a run that leaves any out must say so.
    pub unread: Vec<String>,
}

/// A dump that failed, which stopped the backup.
#[derive(Debug, thiserror::Error)]
#[error("dumping database {database} of {source} failed: {error}")]
pub struct DumpFailure {
    pub source: String,
    pub database: String,
    #[source]
    pub error: ExecError,
}

/// The dumps one backup streams into its archive, ready for borg.
pub struct PreparedDumps<E> {
    exec: E,
    #[cfg(unix)]
    pipes: pipes::DumpPipes,
    dir: PathBuf,
}

impl<E: PodExec> PreparedDumps<E> {
    /// Creates `dir` with the manifest and one pipe per job.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory or a pipe cannot be created, and
    /// always on a platform without named pipes.
    pub async fn create(exec: E, dir: PathBuf, jobs: Vec<DumpJob>) -> std::io::Result<Self> {
        #[cfg(unix)]
        {
            let pipes = pipes::DumpPipes::create(dir.clone(), jobs).await?;
            Ok(Self { exec, pipes, dir })
        }
        #[cfg(not(unix))]
        {
            let _ = (exec, dir, jobs);
            Err(std::io::Error::other(
                "database dumps reach borg through named pipes, which this platform does not have",
            ))
        }
    }

    /// The directory borg must archive.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Runs `borg` while feeding every dump into its pipe. A failed dump stops
    /// borg before it commits; see [`pipes::DumpPipes::stream`].
    ///
    /// # Errors
    ///
    /// Returns the first dump that failed.
    pub async fn stream<F: Future>(
        &self,
        borg: F,
        hold: Duration,
    ) -> Result<StreamOutcome<F::Output>, Box<DumpFailure>> {
        #[cfg(unix)]
        {
            self.pipes.stream(&self.exec, borg, hold).await
        }
        #[cfg(not(unix))]
        {
            let _ = (&self.exec, hold);
            Ok(StreamOutcome {
                borg: borg.await,
                unread: Vec::new(),
            })
        }
    }

    /// Removes the dump directory once the backup is over.
    pub async fn remove(self) {
        if let Err(e) = tokio::fs::remove_dir_all(&self.dir).await {
            tracing::warn!(
                "could not remove the dump directory {}: {e}",
                self.dir.display()
            );
        }
    }
}

/// One database to dump during a backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpJob {
    /// What the manifest says about the dump.
    pub entry: DatabaseManifestEntry,
    /// Where the dump runs.
    pub target: PodTarget,
    /// The role the dump connects as.
    pub user: String,
}

impl DumpJob {
    /// The command that writes this dump to stdout.
    #[must_use]
    pub fn command(&self) -> Vec<String> {
        match self.entry.engine {
            DatabaseEngine::Postgresql => postgres::dump(&self.user, &self.entry.database),
        }
    }
}

/// The databases `candidate` holds: the ones its annotation names, or every
/// database that accepts connections, asked of the pod itself.
///
/// # Errors
///
/// Returns an error when the pod cannot be asked.
pub async fn list_databases<E: PodExec>(
    exec: &E,
    candidate: &SourceCandidate,
) -> Result<Vec<String>, ExecError> {
    if let Some(databases) = &candidate.databases {
        return Ok(databases.clone());
    }
    let command = match candidate.engine {
        DatabaseEngine::Postgresql => postgres::list_databases(&candidate.user),
    };
    let mut output = Vec::new();
    exec.run(&target_of(candidate), command, &mut output)
        .await?;
    Ok(match candidate.engine {
        DatabaseEngine::Postgresql => {
            postgres::parse_database_list(&String::from_utf8_lossy(&output))
        }
    })
}

/// What a scan reports for each source: its databases, or why they could not
/// be listed. One unreachable pod does not hide the others.
pub async fn describe_sources<E: PodExec>(
    exec: &E,
    candidates: &[SourceCandidate],
) -> Vec<DiscoveredDatabaseSource> {
    let mut sources = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let (databases, error) = match list_databases(exec, candidate).await {
            Ok(databases) => (databases, None),
            Err(e) => (Vec::new(), Some(e.to_string())),
        };
        sources.push(DiscoveredDatabaseSource {
            id: candidate.id.clone(),
            kind: candidate.kind,
            engine: candidate.engine,
            pod: candidate.pod.clone(),
            container: candidate.container.clone(),
            databases,
            error,
        });
    }
    sources
}

/// The dumps a backup makes: every database of every source `config`
/// includes.
///
/// # Errors
///
/// Returns an error when an included source's databases cannot be listed,
/// since a backup that silently skipped a database is worse than one that
/// fails.
pub async fn plan_dumps<E: PodExec>(
    exec: &E,
    config: &DatabaseBackupConfig,
    candidates: &[SourceCandidate],
) -> Result<Vec<DumpJob>, ExecError> {
    let mut jobs = Vec::new();
    for candidate in candidates.iter().filter(|c| config.includes(&c.id)) {
        let databases = list_databases(exec, candidate).await?;
        jobs.extend(databases.into_iter().map(|database| DumpJob {
            entry: DatabaseManifestEntry {
                path: dump_path(candidate, &database),
                source: candidate.id.clone(),
                kind: candidate.kind,
                engine: candidate.engine,
                pod: candidate.pod.clone(),
                database,
                format: candidate.engine.dump_format(),
            },
            target: target_of(candidate),
            user: candidate.user.clone(),
        }));
    }
    Ok(jobs)
}

/// The manifest describing `jobs`.
#[must_use]
pub fn manifest(jobs: &[DumpJob]) -> DatabaseManifest {
    DatabaseManifest {
        version: MANIFEST_VERSION,
        dumps: jobs.iter().map(|job| job.entry.clone()).collect(),
    }
}

/// The directory one schedule's dumps are streamed from. Fixed per schedule
/// rather than per run, so successive archives store each dump under the same
/// path and archive diffs line up.
#[must_use]
pub fn schedule_dir(dump_dir: &str, schedule_id: Option<i64>) -> PathBuf {
    Path::new(dump_dir)
        .join(schedule_id.map_or_else(|| "manual".to_owned(), |id| format!("schedule-{id}")))
}

fn target_of(candidate: &SourceCandidate) -> PodTarget {
    PodTarget {
        namespace: candidate.id.namespace.clone(),
        pod: candidate.pod.clone(),
        container: candidate.container.clone(),
    }
}

/// Where a dump sits, relative to the schedule's dump directory. Kubernetes
/// names are safe path components already; a database name may hold anything,
/// so every byte outside a conservative set is percent-encoded.
fn dump_path(candidate: &SourceCandidate, database: &str) -> String {
    format!(
        "{}/{}/{}.{}",
        candidate.id.namespace,
        candidate.id.name,
        encode_component(database),
        candidate.engine.dump_extension()
    )
}

/// Percent-encodes everything but letters, digits, `_` and `-`. Encoding the
/// dot too means no name can become a hidden file, `.` or `..`.
fn encode_component(name: &str) -> String {
    name.bytes()
        .map(|byte| match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' => char::from(byte).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod fake {
    //! A [`PodExec`] that answers from a script instead of a cluster.

    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use tokio::io::{AsyncWrite, AsyncWriteExt as _};

    use super::{ExecError, PodExec, PodTarget};

    /// What one command does when run.
    #[derive(Clone)]
    pub enum Script {
        /// Writes these bytes and succeeds.
        Output(Vec<u8>),
        /// Writes these bytes, then fails.
        FailAfter(Vec<u8>),
    }

    #[derive(Clone, Default)]
    pub struct FakeExec {
        /// Keyed by `<pod> <last argument>`.
        pub scripts: Arc<Mutex<HashMap<String, Script>>>,
        pub calls: Arc<Mutex<Vec<String>>>,
    }

    impl FakeExec {
        pub fn script(&self, pod: &str, last_arg: &str, script: Script) {
            self.scripts
                .lock()
                .unwrap()
                .insert(format!("{pod} {last_arg}"), script);
        }
    }

    impl PodExec for FakeExec {
        async fn run<W>(
            &self,
            target: &PodTarget,
            command: Vec<String>,
            out: &mut W,
        ) -> Result<(), ExecError>
        where
            W: AsyncWrite + Unpin + Send,
        {
            let key = format!(
                "{} {}",
                target.pod,
                command.last().cloned().unwrap_or_default()
            );
            self.calls.lock().unwrap().push(key.clone());
            let script = self.scripts.lock().unwrap().get(&key).cloned();
            match script {
                Some(Script::Output(bytes)) => {
                    out.write_all(&bytes).await?;
                    Ok(())
                }
                Some(Script::FailAfter(bytes)) => {
                    out.write_all(&bytes).await?;
                    Err(ExecError::Failed {
                        command: command.first().cloned().unwrap_or_default(),
                        target: target.to_string(),
                        message: "exit status 1".to_owned(),
                    })
                }
                None => Err(ExecError::Failed {
                    command: key,
                    target: target.to_string(),
                    message: "no such command in the script".to_owned(),
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use shared::database::{
        DatabaseSelectionMode, DatabaseSourceConfig, DatabaseSourceId, DatabaseSourceKind,
        DumpFormat,
    };

    use super::{
        fake::{FakeExec, Script},
        *,
    };

    fn candidate(name: &str, databases: Option<&[&str]>) -> SourceCandidate {
        SourceCandidate {
            id: DatabaseSourceId {
                namespace: "apps".to_owned(),
                name: name.to_owned(),
            },
            kind: DatabaseSourceKind::CnpgCluster,
            engine: DatabaseEngine::Postgresql,
            pod: format!("{name}-1"),
            container: "postgres".to_owned(),
            user: "postgres".to_owned(),
            databases: databases.map(|list| list.iter().map(|d| (*d).to_owned()).collect()),
        }
    }

    const LISTING: &str =
        "SELECT datname FROM pg_database WHERE datallowconn AND NOT datistemplate ORDER BY datname";

    #[tokio::test]
    async fn a_source_without_a_list_is_asked_for_its_databases() {
        let exec = FakeExec::default();
        exec.script("pg-1", LISTING, Script::Output(b"app\nshop\n".to_vec()));
        let databases = list_databases(&exec, &candidate("pg", None)).await.unwrap();
        assert_eq!(databases, ["app", "shop"]);

        let named = list_databases(&exec, &candidate("pg", Some(&["only"])))
            .await
            .unwrap();
        assert_eq!(named, ["only"]);
        assert_eq!(exec.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_scan_reports_a_failing_source_without_hiding_the_others() {
        let exec = FakeExec::default();
        exec.script("ok-1", LISTING, Script::Output(b"app\n".to_vec()));
        let sources =
            describe_sources(&exec, &[candidate("broken", None), candidate("ok", None)]).await;
        assert_eq!(sources.len(), 2);
        let broken = sources.first().unwrap();
        assert_eq!(broken.databases.len(), 0);
        assert!(
            broken
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("broken-1")
        );
        let ok = sources.last().unwrap();
        assert_eq!(ok.databases, ["app"]);
        assert_eq!(ok.error, None);
        assert_eq!(ok.pod, "ok-1");
    }

    #[tokio::test]
    async fn a_plan_covers_every_database_of_every_included_source() {
        let exec = FakeExec::default();
        let config = DatabaseBackupConfig {
            enabled: true,
            selection: DatabaseSelectionMode::All,
            sources: vec![DatabaseSourceConfig {
                id: candidate("skip", None).id,
                included: Some(false),
            }],
            ..DatabaseBackupConfig::default()
        };
        let jobs = plan_dumps(
            &exec,
            &config,
            &[
                candidate("pg", Some(&["app", "my db/x"])),
                candidate("skip", Some(&["never"])),
            ],
        )
        .await
        .unwrap();
        let paths: Vec<&str> = jobs.iter().map(|job| job.entry.path.as_str()).collect();
        assert_eq!(paths, ["apps/pg/app.pgdump", "apps/pg/my%20db%2Fx.pgdump"]);
        let job = jobs.first().unwrap();
        assert_eq!(job.entry.format, DumpFormat::PgCustom);
        assert_eq!(job.entry.pod, "pg-1");
        assert_eq!(job.target.container, "postgres");
        assert_eq!(
            job.command().last().map(String::as_str),
            Some("--dbname=app")
        );

        let manifest = manifest(&jobs);
        assert_eq!(manifest.version, MANIFEST_VERSION);
        assert_eq!(manifest.dumps.len(), 2);
    }

    #[tokio::test]
    async fn a_plan_fails_when_an_included_source_cannot_be_listed() {
        let exec = FakeExec::default();
        let config = DatabaseBackupConfig {
            enabled: true,
            ..DatabaseBackupConfig::default()
        };
        let err = plan_dumps(&exec, &config, &[candidate("pg", None)])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("pg-1"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn prepared_dumps_stream_into_borg_and_clean_up_after() {
        use tokio::io::AsyncReadExt as _;

        let root = tempfile::tempdir().unwrap();
        let exec = FakeExec::default();
        exec.script("pg-1", "--dbname=app", Script::Output(b"dump".to_vec()));
        let jobs = plan_dumps(
            &exec,
            &DatabaseBackupConfig {
                enabled: true,
                ..DatabaseBackupConfig::default()
            },
            &[candidate("pg", Some(&["app"]))],
        )
        .await
        .unwrap();
        let dir = root.path().join("schedule-3");
        let dumps = PreparedDumps::create(exec, dir.clone(), jobs)
            .await
            .unwrap();
        assert_eq!(dumps.dir(), dir);

        let pipe = dir.join("apps/pg/app.pgdump");
        let borg = async move {
            let mut content = Vec::new();
            tokio::fs::File::open(&pipe)
                .await
                .unwrap()
                .read_to_end(&mut content)
                .await
                .unwrap();
            content
        };
        let outcome = dumps.stream(borg, Duration::from_millis(10)).await.unwrap();
        assert_eq!(outcome.borg, b"dump");
        dumps.remove().await;
        assert!(tokio::fs::metadata(&dir).await.is_err());
    }

    #[test]
    fn hidden_and_relative_names_cannot_escape_their_directory() {
        assert_eq!(encode_component(".."), "%2E%2E");
        assert_eq!(encode_component(".hidden"), "%2Ehidden");
        assert_eq!(encode_component("\u{fc}n\u{ef}"), "%C3%BCn%C3%AF");
        assert_eq!(encode_component("plain_name-1"), "plain_name-1");
    }

    #[test]
    fn each_schedule_streams_from_its_own_directory() {
        assert_eq!(
            schedule_dir("/var/lib/dumps", Some(7)),
            Path::new("/var/lib/dumps/schedule-7")
        );
        assert_eq!(
            schedule_dir("/var/lib/dumps", None),
            Path::new("/var/lib/dumps/manual")
        );
    }
}
