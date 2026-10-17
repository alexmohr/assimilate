// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Database dumps streamed into a backup's archive.
//!
//! The agent finds database workloads in its Kubernetes cluster, in Docker or
//! Podman containers and on the host itself, and for a backup that asks for
//! them creates one named pipe per database below the host's dump directory.
//! borg archives each pipe's content (`--read-special`) while the agent copies
//! a dump running next to the database into it, so nothing is staged on disk.
//! A dump that fails stops borg before it commits: an archive must never hold
//! a truncated dump while reporting success.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use shared::database::{
    DatabaseBackupConfig, DatabaseEngine, DatabaseManifest, DatabaseManifestEntry, DatabaseRuntime,
    DatabaseRuntimes, DiscoveredDatabaseSource, MANIFEST_VERSION,
};

mod discovery;
mod docker;
mod exec;
mod host;
#[cfg(unix)]
mod pipes;
mod postgres;
mod restore;
mod stdin;
mod target;

#[cfg(test)]
pub use self::exec::PodTarget;
pub use self::{
    discovery::{SourceCandidate, find_sources},
    docker::{DockerExec, find_containers},
    exec::{ExecError, KubeExec, read_tail},
    host::{HostExec, host_source},
    restore::restore_dump,
    stdin::Stdin,
    target::{CommandExec, DumpTarget, RuntimeClients},
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

impl<E: CommandExec> PreparedDumps<E> {
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
            let dir = self.dir.display();
            tracing::warn!("could not remove the dump directory {dir}: {e}");
        }
    }
}

/// One database to dump during a backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpJob {
    /// What the manifest says about the dump.
    pub entry: DatabaseManifestEntry,
    /// Where the dump runs.
    pub target: DumpTarget,
    /// The role the dump connects as.
    pub user: String,
}

impl DumpJob {
    /// The command that writes this dump to stdout.
    #[must_use]
    pub fn command(&self) -> Vec<String> {
        match self.entry.engine {
            DatabaseEngine::Postgresql => postgres::dump(
                &self.user,
                &self.entry.database,
                &self.target.connection_args(),
            ),
        }
    }
}

/// The databases `candidate` holds: the ones its annotation or label names,
/// or every database that accepts connections, asked of the server itself.
///
/// # Errors
///
/// Returns an error when the server cannot be asked.
pub async fn list_databases<E: CommandExec>(
    exec: &E,
    candidate: &SourceCandidate,
) -> Result<Vec<String>, ExecError> {
    if let Some(databases) = &candidate.databases {
        return Ok(databases.clone());
    }
    let connection = candidate.target.connection_args();
    let command = match candidate.engine {
        DatabaseEngine::Postgresql => postgres::list_databases(&candidate.user, &connection),
    };
    let mut output = Vec::new();
    exec.run(&candidate.target, command, Stdin::Closed, &mut output)
        .await?;
    Ok(match candidate.engine {
        DatabaseEngine::Postgresql => {
            postgres::parse_database_list(&String::from_utf8_lossy(&output))
        }
    })
}

/// What a scan reports for each source: its databases, or why they could not
/// be listed. One unreachable server does not hide the others.
pub async fn describe_sources<E: CommandExec>(
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
            pod: candidate.target.place(),
            container: candidate.target.detail(),
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
pub async fn plan_dumps<E: CommandExec>(
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
                pod: candidate.target.place(),
                database,
                format: candidate.engine.dump_format(),
            },
            target: candidate.target.clone(),
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

/// What searching a host's runtimes found: a client for each runtime that
/// answered, the sources in them, and why any other could not be searched.
#[derive(Default)]
pub struct Found {
    pub clients: RuntimeClients,
    pub sources: Vec<SourceCandidate>,
    pub failures: Vec<String>,
}

impl Found {
    /// Why the runtimes that could not be searched failed, in one line, or
    /// `None` when every runtime answered.
    #[must_use]
    pub fn failure(&self) -> Option<String> {
        (!self.failures.is_empty()).then(|| self.failures.join("; "))
    }
}

/// Searches every runtime `runtimes` turns on for database sources. A runtime
/// that cannot be searched is reported in [`Found::failures`] without hiding
/// the others.
pub async fn find_all(runtimes: &DatabaseRuntimes) -> Found {
    let mut found = Found::default();
    if runtimes.kubernetes {
        match search_kubernetes(runtimes.kubeconfig_path.as_deref()).await {
            Ok((client, sources)) => {
                found.sources.extend(sources);
                found.clients.kubernetes = Some(client);
            }
            Err(e) => found.failures.push(e),
        }
    }
    if runtimes.docker {
        match search_docker(runtimes.docker_socket.as_deref()).await {
            Ok((client, sources)) => {
                found.sources.extend(sources);
                found.clients.docker = Some(client);
            }
            Err(e) => found.failures.push(e),
        }
    }
    if runtimes.host {
        found.sources.push(host_source(runtimes));
        found.clients.host = Some(HostExec::new(runtimes.host_bin_dir.as_deref()));
    }
    found
}

async fn search_kubernetes(
    kubeconfig_path: Option<&str>,
) -> Result<(KubeExec, Vec<SourceCandidate>), String> {
    let client = KubeExec::connect(kubeconfig_path.map(Path::new))
        .await
        .map_err(|e| format!("could not reach the Kubernetes cluster: {e}"))?;
    let pods = client
        .list_pods()
        .await
        .map_err(|e| format!("could not list the cluster's pods: {e}"))?;
    Ok((client, find_sources(&pods)))
}

async fn search_docker(socket: Option<&str>) -> Result<(DockerExec, Vec<SourceCandidate>), String> {
    let client = DockerExec::connect(socket)
        .map_err(|e| format!("could not reach the container engine: {e}"))?;
    let containers = client
        .list_containers()
        .await
        .map_err(|e| format!("could not list the containers: {e}"))?;
    Ok((client, find_containers(&containers)))
}

/// Where a dump sits, relative to the schedule's dump directory: below its
/// runtime, then its namespace or project when it has one, then its name.
/// Kubernetes names are safe path components already; anything else may hold
/// anything, so every byte outside a conservative set is percent-encoded.
fn dump_path(candidate: &SourceCandidate, database: &str) -> String {
    let id = &candidate.id;
    let mut parts = vec![id.runtime.to_string()];
    let names = [&id.namespace, &id.name];
    parts.extend(
        names
            .into_iter()
            .filter(|part| !part.is_empty())
            .map(|part| match id.runtime {
                DatabaseRuntime::Kubernetes => part.clone(),
                DatabaseRuntime::Docker | DatabaseRuntime::Host => encode_component(part),
            }),
    );
    parts.push(format!(
        "{}.{}",
        encode_component(database),
        candidate.engine.dump_extension()
    ));
    parts.join("/")
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

    use tokio::io::{AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

    use super::{CommandExec, DumpTarget, ExecError, Stdin};

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
        /// Keyed by `<where it runs> <last argument>`.
        pub scripts: Arc<Mutex<HashMap<String, Script>>>,
        pub calls: Arc<Mutex<Vec<String>>>,
        /// Every command run, in full.
        pub commands: Arc<Mutex<Vec<Vec<String>>>>,
        /// What each command that had an input read on stdin.
        pub inputs: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl FakeExec {
        pub fn script(&self, pod: &str, last_arg: &str, script: Script) {
            self.scripts
                .lock()
                .unwrap()
                .insert(format!("{pod} {last_arg}"), script);
        }
    }

    impl CommandExec for FakeExec {
        async fn run<W>(
            &self,
            target: &DumpTarget,
            command: Vec<String>,
            input: Stdin<'_>,
            out: &mut W,
        ) -> Result<(), ExecError>
        where
            W: AsyncWrite + Unpin + Send,
        {
            if let Stdin::From(reader) = input {
                let mut read = Vec::new();
                reader.read_to_end(&mut read).await?;
                self.inputs.lock().unwrap().push(read);
            }
            self.commands.lock().unwrap().push(command.clone());
            let key = format!(
                "{} {}",
                target.place(),
                command.last().cloned().unwrap_or_default()
            );
            self.calls.lock().unwrap().push(key.clone());
            let script = self.scripts.lock().unwrap().get(&key).cloned();
            match script {
                Some(Script::Output(bytes)) => {
                    out.write_all(&bytes).await?;
                    out.flush().await?;
                    Ok(())
                }
                Some(Script::FailAfter(bytes)) => {
                    out.write_all(&bytes).await?;
                    out.flush().await?;
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
                runtime: DatabaseRuntime::Kubernetes,
                namespace: "apps".to_owned(),
                name: name.to_owned(),
            },
            kind: DatabaseSourceKind::CnpgCluster,
            engine: DatabaseEngine::Postgresql,
            target: DumpTarget::Pod(PodTarget {
                namespace: "apps".to_owned(),
                pod: format!("{name}-1"),
                container: "postgres".to_owned(),
            }),
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
        assert_eq!(
            paths,
            [
                "kubernetes/apps/pg/app.pgdump",
                "kubernetes/apps/pg/my%20db%2Fx.pgdump"
            ]
        );
        let job = jobs.first().unwrap();
        assert_eq!(job.entry.format, DumpFormat::PgCustom);
        assert_eq!(job.entry.pod, "pg-1");
        assert_eq!(job.target.detail(), "postgres");
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

        let pipe = dir.join("kubernetes/apps/pg/app.pgdump");
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

    #[cfg(unix)]
    #[tokio::test]
    async fn a_dump_directory_that_is_already_gone_is_only_logged() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("schedule-5");
        let dumps = PreparedDumps::create(FakeExec::default(), dir.clone(), Vec::new())
            .await
            .unwrap();
        tokio::fs::remove_dir_all(&dir).await.unwrap();

        let ((), logs) = crate::tests::logs::logs_while(dumps.remove()).await;

        assert!(
            logs.contains("could not remove the dump directory"),
            "{logs}"
        );
    }

    #[test]
    fn each_runtime_keeps_its_dumps_in_its_own_tree() {
        let in_runtime = |runtime, namespace: &str, name: &str| SourceCandidate {
            id: DatabaseSourceId {
                runtime,
                namespace: namespace.to_owned(),
                name: name.to_owned(),
            },
            ..candidate("pg", None)
        };
        assert_eq!(
            dump_path(&in_runtime(DatabaseRuntime::Docker, "", "wiki.db"), "wiki"),
            "docker/wiki%2Edb/wiki.pgdump"
        );
        assert_eq!(
            dump_path(&in_runtime(DatabaseRuntime::Docker, "shop", "db"), "orders"),
            "docker/shop/db/orders.pgdump"
        );
        assert_eq!(
            dump_path(&in_runtime(DatabaseRuntime::Host, "", "postgresql"), "app"),
            "host/postgresql/app.pgdump"
        );
    }

    #[tokio::test]
    async fn a_host_dump_names_the_hosts_server() {
        let exec = FakeExec::default();
        let host = host_source(&DatabaseRuntimes {
            host: true,
            host_port: Some(5433),
            ..DatabaseRuntimes::default()
        });
        exec.script("", LISTING, Script::Output(b"app\n".to_vec()));
        let jobs = plan_dumps(
            &exec,
            &DatabaseBackupConfig {
                enabled: true,
                ..DatabaseBackupConfig::default()
            },
            &[host],
        )
        .await
        .unwrap();
        let job = jobs.first().unwrap();
        assert_eq!(job.entry.path, "host/postgresql/app.pgdump");
        assert!(job.command().contains(&"--port=5433".to_owned()));
    }

    #[tokio::test]
    async fn runtimes_that_are_off_are_not_searched() {
        let found = find_all(&DatabaseRuntimes::default()).await;
        assert_eq!(found.sources.len(), 0);
        assert_eq!(found.failure(), None);

        let host_only = find_all(&DatabaseRuntimes {
            host: true,
            host_bin_dir: Some("/opt/pg/bin".to_owned()),
            ..DatabaseRuntimes::default()
        })
        .await;
        assert_eq!(host_only.sources.len(), 1);
        assert!(host_only.clients.host.is_some());
        assert!(host_only.clients.kubernetes.is_none());
        assert!(host_only.clients.docker.is_none());
    }

    #[tokio::test]
    async fn a_runtime_that_cannot_be_searched_does_not_hide_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let found = find_all(&DatabaseRuntimes {
            kubernetes: true,
            kubeconfig_path: Some("/nonexistent/kubeconfig".to_owned()),
            docker: true,
            docker_socket: Some(dir.path().join("no.sock").to_string_lossy().into_owned()),
            host: true,
            ..DatabaseRuntimes::default()
        })
        .await;
        assert_eq!(found.sources.len(), 1, "the host is still there");
        let failure = found.failure().unwrap();
        assert!(
            failure.contains("could not reach the Kubernetes cluster"),
            "{failure}"
        );
        assert!(
            failure.contains("could not reach the container engine"),
            "{failure}"
        );

        let engine = crate::tests::fake_docker::FakeDocker::start(Vec::new());
        engine.fail_requests(500);
        let refused = find_all(&DatabaseRuntimes {
            docker: true,
            docker_socket: Some(engine.socket()),
            ..DatabaseRuntimes::default()
        })
        .await;
        let failure = refused.failure().unwrap();
        assert!(
            failure.starts_with("could not list the containers"),
            "{failure}"
        );
    }

    #[tokio::test]
    async fn the_cluster_and_the_engine_are_searched_when_on() {
        use crate::tests::{fake_docker::FakeDocker, fake_kube::FakeKube};

        let dir = tempfile::tempdir().unwrap();
        let kube = FakeKube::start(Vec::new()).await;
        kube.fail_list(403);
        let engine = FakeDocker::start(vec![bollard::models::ContainerSummary {
            id: Some("c1".to_owned()),
            names: Some(vec!["/wiki-db".to_owned()]),
            labels: Some(
                [(
                    shared::database::ANNOTATION_DATABASE.to_owned(),
                    "postgresql".to_owned(),
                )]
                .into(),
            ),
            ..bollard::models::ContainerSummary::default()
        }]);
        let found = find_all(&DatabaseRuntimes {
            kubernetes: true,
            kubeconfig_path: Some(
                kube.kubeconfig(dir.path())
                    .await
                    .to_string_lossy()
                    .into_owned(),
            ),
            docker: true,
            docker_socket: Some(engine.socket()),
            ..DatabaseRuntimes::default()
        })
        .await;
        assert!(found.clients.docker.is_some());
        let names: Vec<String> = found.sources.iter().map(|s| s.id.to_string()).collect();
        assert_eq!(names, ["docker:wiki-db"]);
        let failure = found.failure().unwrap();
        assert!(
            failure.starts_with("could not list the cluster's pods"),
            "{failure}"
        );
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
