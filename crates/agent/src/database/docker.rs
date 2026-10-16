// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Databases in Docker or Podman containers, reached through the engine's API
//! socket.
//!
//! A container is a source when it carries the
//! `assimilate.alexmohr.github.io/database` label, the same key a pod is
//! annotated with. A command runs in it through the exec API: its output is
//! streamed back while it runs, and its exit code is asked for once the
//! stream ends.

use std::{collections::HashMap, pin::Pin, time::Duration};

use bollard::{
    API_DEFAULT_VERSION, Docker,
    container::LogOutput,
    exec::{CreateExecOptions, StartExecOptions, StartExecResults},
    models::ContainerSummary,
    query_parameters::ListContainersOptionsBuilder,
};
use futures_util::{Stream, StreamExt as _};
use shared::database::{
    ANNOTATION_DATABASE, ANNOTATION_DATABASES, ANNOTATION_USER, DEFAULT_POSTGRES_USER,
    DatabaseEngine, DatabaseRuntime, DatabaseSourceId, DatabaseSourceKind,
};
use tokio::io::{AsyncWrite, AsyncWriteExt as _};

use super::{
    discovery::{SourceCandidate, database_list},
    exec::{ExecError, ExecOutcome, Tail, outcome_result},
    target::DumpTarget,
};

/// What an attached exec streams back.
type ExecOutput = Pin<Box<dyn Stream<Item = Result<LogOutput, bollard::errors::Error>> + Send>>;

/// Label Docker Compose and podman-compose put on every container of a
/// project, naming the project.
pub const COMPOSE_PROJECT_LABEL: &str = "com.docker.compose.project";

/// Label naming the Compose service a container runs. It outlives the
/// container, which a `compose up` may replace.
pub const COMPOSE_SERVICE_LABEL: &str = "com.docker.compose.service";

/// Seconds the engine has to answer a request. Only the answer is timed: the
/// output of a long dump streams for as long as it takes.
const REQUEST_TIMEOUT_SECS: u64 = 120;

/// Bytes read from an exec stream at a time, so a multi-gigabyte dump does
/// not cost a wake-up per few kilobytes.
const STREAM_BUFFER_BYTES: usize = 1 << 20;

/// How often, and how far apart, a finished exec is asked for its exit code.
/// The engine may still be recording it when the output ends.
const EXIT_CODE_POLLS: u32 = 20;
const EXIT_CODE_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// A container a command runs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerTarget {
    /// The engine's id for the container, which the exec API addresses.
    pub id: String,
    /// The container's name, for people.
    pub name: String,
}

impl std::fmt::Display for ContainerTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "container {}", self.name)
    }
}

/// Runs commands in the containers of a Docker or Podman engine.
#[derive(Clone)]
pub struct DockerExec {
    docker: Docker,
}

impl DockerExec {
    /// Connects to the engine at `socket`, or, without one, at
    /// `$DOCKER_HOST` or `/var/run/docker.sock`. Nothing is sent until the
    /// first request.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket address cannot be used.
    pub fn connect(socket: Option<&str>) -> Result<Self, ExecError> {
        let docker = match socket {
            Some(path) => {
                Docker::connect_with_socket(path, REQUEST_TIMEOUT_SECS, API_DEFAULT_VERSION)
            }
            None => Docker::connect_with_local_defaults(),
        }?;
        Ok(Self { docker })
    }

    /// The running containers that carry the database label.
    ///
    /// # Errors
    ///
    /// Returns an error when the engine refuses the list.
    pub async fn list_containers(&self) -> Result<Vec<ContainerSummary>, ExecError> {
        let filters = HashMap::from([
            ("label", vec![ANNOTATION_DATABASE]),
            ("status", vec!["running"]),
        ]);
        let options = ListContainersOptionsBuilder::new()
            .filters(&filters)
            .build();
        Ok(self.docker.list_containers(Some(options)).await?)
    }

    /// Runs `command` in `target`, copying its stdout into `out` and flushing
    /// it. Resolves once the command has ended, with an error unless it
    /// exited with 0.
    ///
    /// # Errors
    ///
    /// Returns an error when the exec cannot start, its stream fails, or the
    /// command does not succeed.
    pub async fn run_in_container<W>(
        &self,
        target: &ContainerTarget,
        command: Vec<String>,
        out: &mut W,
    ) -> Result<(), ExecError>
    where
        W: AsyncWrite + Unpin + Send,
    {
        let name = command.first().cloned().unwrap_or_default();
        let options = CreateExecOptions {
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            cmd: Some(command),
            ..CreateExecOptions::default()
        };
        let exec = self.docker.create_exec(&target.id, options).await?;
        let start = StartExecOptions {
            output_capacity: Some(STREAM_BUFFER_BYTES),
            ..StartExecOptions::default()
        };
        let mut stderr = Tail::default();
        let mut output = exec_output(self.docker.start_exec(&exec.id, Some(start)).await?);
        while let Some(chunk) = output.next().await {
            match chunk? {
                LogOutput::StdErr { message } => stderr.push(&message),
                other => out.write_all(&other.into_bytes()).await?,
            }
        }
        out.flush().await?;
        let outcome = self.exit_outcome(&exec.id).await?;
        outcome_result(outcome, &name, target, &stderr.text())
    }

    /// How the exec ended, once the engine has recorded it.
    async fn exit_outcome(&self, exec_id: &str) -> Result<ExecOutcome, ExecError> {
        for _ in 0..EXIT_CODE_POLLS {
            let inspected = self.docker.inspect_exec(exec_id).await?;
            if inspected.running != Some(true) {
                return Ok(match inspected.exit_code {
                    Some(0) => ExecOutcome::Succeeded,
                    Some(code) => ExecOutcome::Failed(format!("exit code {code}")),
                    None => ExecOutcome::Failed("the engine reported no exit code".to_owned()),
                });
            }
            tokio::time::sleep(EXIT_CODE_POLL_INTERVAL).await;
        }
        Ok(ExecOutcome::Failed(
            "the command's output ended, but the engine still reports it running".to_owned(),
        ))
    }
}

/// The database containers among `containers`, one per Compose service or
/// standalone container, ordered by project and name.
pub fn find_containers(containers: &[ContainerSummary]) -> Vec<SourceCandidate> {
    let mut named: Vec<(String, &ContainerSummary)> = containers
        .iter()
        .filter_map(|container| Some((container_name(container)?, container)))
        .collect();
    named.sort_by(|a, b| a.0.cmp(&b.0));

    let mut sources: Vec<SourceCandidate> = Vec::new();
    for (name, container) in named {
        let Some(candidate) = container_source(name, container) else {
            continue;
        };
        if !sources.iter().any(|source| source.id == candidate.id) {
            sources.push(candidate);
        }
    }
    sources.sort_by(|a, b| a.id.cmp(&b.id));
    sources
}

/// The container's name without the leading `/` the engine reports.
fn container_name(container: &ContainerSummary) -> Option<String> {
    let name = container.names.as_ref()?.first()?;
    Some(name.trim_start_matches('/').to_owned())
}

fn container_source(name: String, container: &ContainerSummary) -> Option<SourceCandidate> {
    let labels = container.labels.as_ref()?;
    let label = |key: &str| {
        labels
            .get(key)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
    };
    let engine: DatabaseEngine = label(ANNOTATION_DATABASE)?.parse().ok()?;
    let id = DatabaseSourceId {
        runtime: DatabaseRuntime::Docker,
        namespace: label(COMPOSE_PROJECT_LABEL).unwrap_or_default().to_owned(),
        name: label(COMPOSE_SERVICE_LABEL).map_or_else(|| name.clone(), str::to_owned),
    };
    Some(SourceCandidate {
        id,
        kind: DatabaseSourceKind::Container,
        engine,
        target: DumpTarget::Container(ContainerTarget {
            id: container.id.clone()?,
            name,
        }),
        user: label(ANNOTATION_USER)
            .unwrap_or(DEFAULT_POSTGRES_USER)
            .to_owned(),
        databases: database_list(label(ANNOTATION_DATABASES)),
    })
}

/// The output an exec streams, empty for one started detached.
fn exec_output(results: StartExecResults) -> ExecOutput {
    match results {
        StartExecResults::Attached { output, .. } => output,
        StartExecResults::Detached => Box::pin(futures_util::stream::empty()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn container(id: &str, name: &str, labels: &[(&str, &str)]) -> ContainerSummary {
        ContainerSummary {
            id: Some(id.to_owned()),
            names: Some(vec![format!("/{name}")]),
            labels: Some(
                labels
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
            ),
            ..ContainerSummary::default()
        }
    }

    #[tokio::test]
    async fn a_detached_exec_streams_nothing() {
        assert!(
            exec_output(StartExecResults::Detached)
                .next()
                .await
                .is_none()
        );
    }

    #[test]
    fn a_labelled_container_reads_its_settings_from_its_labels() {
        let sources = find_containers(&[container(
            "c1",
            "wiki-db",
            &[
                (ANNOTATION_DATABASE, "postgresql"),
                (ANNOTATION_USER, "wiki"),
                (ANNOTATION_DATABASES, "wiki, , media"),
            ],
        )]);
        let source = sources.first().unwrap();
        assert_eq!(source.id.to_string(), "docker:wiki-db");
        assert_eq!(source.kind, DatabaseSourceKind::Container);
        assert_eq!(
            source.target,
            DumpTarget::Container(ContainerTarget {
                id: "c1".to_owned(),
                name: "wiki-db".to_owned(),
            })
        );
        assert_eq!(source.user, "wiki");
        assert_eq!(
            source.databases,
            Some(vec!["wiki".to_owned(), "media".to_owned()])
        );
    }

    #[test]
    fn a_compose_service_is_one_source_named_after_it() {
        let replica = |id: &str, name: &str| {
            container(
                id,
                name,
                &[
                    (ANNOTATION_DATABASE, "postgresql"),
                    (COMPOSE_PROJECT_LABEL, "shop"),
                    (COMPOSE_SERVICE_LABEL, "db"),
                ],
            )
        };
        let sources = find_containers(&[replica("c2", "shop-db-2"), replica("c1", "shop-db-1")]);
        assert_eq!(sources.len(), 1);
        let source = sources.first().unwrap();
        assert_eq!(source.id.to_string(), "docker:shop/db");
        assert_eq!(source.target.place(), "shop-db-1");
        assert_eq!(source.user, DEFAULT_POSTGRES_USER);
        assert_eq!(source.databases, None);
    }

    #[test]
    fn unknown_engines_and_unnamed_or_unlabelled_containers_are_not_sources() {
        let mut nameless = container("c3", "x", &[(ANNOTATION_DATABASE, "postgresql")]);
        nameless.names = None;
        let mut idless = container("c4", "y", &[(ANNOTATION_DATABASE, "postgresql")]);
        idless.id = None;
        let mut unlabelled = container("c5", "z", &[]);
        unlabelled.labels = None;
        let sources = find_containers(&[
            container("c1", "oracle", &[(ANNOTATION_DATABASE, "oracle")]),
            container("c2", "web", &[(ANNOTATION_DATABASE, " ")]),
            nameless,
            idless,
            unlabelled,
        ]);
        assert_eq!(sources.len(), 0);
    }

    #[test]
    fn sources_are_ordered_by_project_and_name() {
        let labelled = |id: &str, name: &str, project: Option<&str>| {
            let mut labels = vec![(ANNOTATION_DATABASE, "postgresql")];
            labels.extend(project.map(|project| (COMPOSE_PROJECT_LABEL, project)));
            container(id, name, &labels)
        };
        let names: Vec<String> = find_containers(&[
            labelled("c1", "zeta", None),
            labelled("c2", "beta", Some("app")),
            labelled("c3", "alpha", None),
        ])
        .into_iter()
        .map(|source| source.id.to_string())
        .collect();
        assert_eq!(names, ["docker:alpha", "docker:zeta", "docker:app/beta"]);
    }

    #[test]
    fn without_a_socket_the_engines_defaults_are_used() {
        assert!(DockerExec::connect(None).is_ok());
    }

    mod against_a_fake_engine {
        use super::*;
        use crate::tests::fake_docker::{ExecReply, FakeDocker};

        fn target() -> ContainerTarget {
            ContainerTarget {
                id: "c1".to_owned(),
                name: "wiki-db".to_owned(),
            }
        }

        async fn dump(reply: ExecReply) -> (Result<(), ExecError>, Vec<u8>, FakeDocker) {
            let engine = FakeDocker::start(Vec::new());
            engine.reply("c1", "--dbname=wiki", reply);
            let exec = DockerExec::connect(Some(&engine.socket())).unwrap();
            let mut out = Vec::new();
            let result = exec
                .run_in_container(
                    &target(),
                    vec!["pg_dump".to_owned(), "--dbname=wiki".to_owned()],
                    &mut out,
                )
                .await;
            (result, out, engine)
        }

        #[tokio::test]
        async fn labelled_running_containers_are_listed() {
            let engine = FakeDocker::start(vec![container(
                "c1",
                "wiki-db",
                &[(ANNOTATION_DATABASE, "postgresql")],
            )]);
            let exec = DockerExec::connect(Some(&engine.socket())).unwrap();
            let listed = exec.list_containers().await.unwrap();
            assert_eq!(listed.len(), 1);
            let query = engine.list_query();
            assert!(query.contains(ANNOTATION_DATABASE), "{query}");
            assert!(query.contains("running"), "{query}");
        }

        #[tokio::test]
        async fn a_command_streams_its_output_and_succeeds() {
            let (result, out, engine) = dump(ExecReply::exit(0, b"dump bytes", "notice")).await;
            result.unwrap();
            assert_eq!(out, b"dump bytes");
            assert_eq!(engine.calls(), ["c1 pg_dump --dbname=wiki"]);
        }

        #[tokio::test]
        async fn a_failing_command_reports_its_exit_code_and_stderr() {
            let (result, out, _) = dump(ExecReply::exit(1, b"half", "pg_dump: error: gone")).await;
            let err = result.unwrap_err().to_string();
            assert!(
                err.starts_with("pg_dump failed in container wiki-db"),
                "{err}"
            );
            assert!(err.contains("exit code 1"), "{err}");
            assert!(err.ends_with("pg_dump: error: gone"), "{err}");
            assert_eq!(out, b"half");
        }

        #[tokio::test]
        async fn an_exec_without_an_exit_code_is_not_success() {
            let (result, _, _) = dump(ExecReply {
                exit_code: None,
                ..ExecReply::exit(0, b"", "")
            })
            .await;
            let err = result.unwrap_err().to_string();
            assert!(err.contains("no exit code"), "{err}");
        }

        #[tokio::test]
        async fn an_exec_still_running_after_its_output_ends_is_waited_for_then_failed() {
            let (result, _, engine) = dump(ExecReply {
                running_polls: 2,
                ..ExecReply::exit(0, b"done", "")
            })
            .await;
            result.unwrap();
            assert_eq!(engine.inspections(), 3);

            let (result, _, _) = dump(ExecReply {
                running_polls: u32::MAX,
                ..ExecReply::exit(0, b"", "")
            })
            .await;
            let err = result.unwrap_err().to_string();
            assert!(err.contains("still reports it running"), "{err}");
        }

        #[tokio::test]
        async fn an_engine_that_refuses_is_a_docker_error() {
            let engine = FakeDocker::start(Vec::new());
            engine.fail_requests(500);
            let exec = DockerExec::connect(Some(&engine.socket())).unwrap();
            assert!(matches!(
                exec.list_containers().await,
                Err(ExecError::Docker(_))
            ));
            let mut out = Vec::new();
            assert!(matches!(
                exec.run_in_container(&target(), vec!["true".to_owned()], &mut out)
                    .await,
                Err(ExecError::Docker(_))
            ));
        }
    }
}
