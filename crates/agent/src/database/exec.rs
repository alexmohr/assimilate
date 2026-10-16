// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Running a command inside a pod through the Kubernetes exec API.
//!
//! The exec stream carries the command's exit as a `Status` object on its own
//! channel rather than as an exit code, so success is only known once that
//! status arrives; a stream that ends without one was cut, not finished.

use std::{future::Future, path::Path};

use k8s_openapi::{api::core::v1::Pod, apimachinery::pkg::apis::meta::v1::Status};
use kube::{
    Api, Client, Config,
    api::{AttachParams, ListParams},
    config::{KubeConfigOptions, Kubeconfig},
};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

/// Bytes buffered per exec stream. kube's default of 1 KiB would cost a
/// wake-up per kilobyte of a multi-gigabyte dump.
const STREAM_BUFFER_BYTES: usize = 1 << 20;

/// stderr kept from one command for its error message. A dump that fails says
/// why in its last lines, never in megabytes of output.
const MAX_STDERR_BYTES: usize = 8 * 1024;

/// Where a command runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodTarget {
    pub namespace: String,
    pub pod: String,
    pub container: String,
}

impl std::fmt::Display for PodTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{} ({})", self.namespace, self.pod, self.container)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("could not load the Kubernetes configuration: {0}")]
    Config(String),
    #[error("the Kubernetes API refused the request: {0}")]
    Api(#[from] kube::Error),
    #[error("the exec stream failed: {0}")]
    Stream(String),
    #[error("could not copy the command's output: {0}")]
    Io(#[from] std::io::Error),
    #[error("{command} failed in {target}: {message}")]
    Failed {
        command: String,
        target: String,
        message: String,
    },
    #[error(
        "the connection to {target} ended before {command} finished; a stalled command is cut off \
         by the kubelet's stream idle timeout (4 hours by default)"
    )]
    Cut { command: String, target: String },
}

/// How a remote command ended, read from its exec status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecOutcome {
    Succeeded,
    Failed(String),
    /// The stream closed without a status: the connection was cut.
    Missing,
}

impl From<Option<&Status>> for ExecOutcome {
    fn from(status: Option<&Status>) -> Self {
        let Some(status) = status else {
            return Self::Missing;
        };
        if status.status.as_deref() == Some("Success") {
            return Self::Succeeded;
        }
        Self::Failed(
            status
                .message
                .clone()
                .or_else(|| status.reason.clone())
                .unwrap_or_else(|| "the command failed without saying why".to_owned()),
        )
    }
}

/// Runs commands in pods. A trait so the backup flow can be tested without a
/// cluster.
pub trait PodExec: Send + Sync {
    /// Runs `command` in `target`, copying its stdout into `out` and
    /// flushing it. Resolves once the command has ended, with an error unless
    /// it succeeded.
    fn run<W>(
        &self,
        target: &PodTarget,
        command: Vec<String>,
        out: &mut W,
    ) -> impl Future<Output = Result<(), ExecError>> + Send
    where
        W: AsyncWrite + Unpin + Send;
}

/// [`PodExec`] against a real cluster.
#[derive(Clone)]
pub struct KubeExec {
    client: Client,
}

impl KubeExec {
    /// Connects with the kubeconfig at `path`, or, without one, the
    /// in-cluster service account or `$KUBECONFIG` and `~/.kube/config`.
    ///
    /// # Errors
    ///
    /// Returns an error when no usable configuration is found.
    pub async fn connect(path: Option<&Path>) -> Result<Self, ExecError> {
        let config = match path {
            Some(path) => {
                let kubeconfig = Kubeconfig::read_from(path)
                    .map_err(|e| ExecError::Config(format!("{}: {e}", path.display())))?;
                Config::from_custom_kubeconfig(kubeconfig, &KubeConfigOptions::default())
                    .await
                    .map_err(|e| ExecError::Config(e.to_string()))?
            }
            None => Config::infer()
                .await
                .map_err(|e| ExecError::Config(e.to_string()))?,
        };
        Ok(Self {
            client: Client::try_from(config)?,
        })
    }

    /// Every pod in every namespace the credentials may list.
    ///
    /// # Errors
    ///
    /// Returns an error when the API refuses the list.
    pub async fn list_pods(&self) -> Result<Vec<Pod>, ExecError> {
        Ok(Api::<Pod>::all(self.client.clone())
            .list(&ListParams::default())
            .await?
            .items)
    }
}

impl PodExec for KubeExec {
    async fn run<W>(
        &self,
        target: &PodTarget,
        command: Vec<String>,
        out: &mut W,
    ) -> Result<(), ExecError>
    where
        W: AsyncWrite + Unpin + Send,
    {
        let name = command.first().cloned().unwrap_or_default();
        let params = AttachParams::default()
            .container(target.container.clone())
            .stdin(false)
            .stdout(true)
            .stderr(true)
            .max_stdout_buf_size(STREAM_BUFFER_BYTES)
            .max_stderr_buf_size(STREAM_BUFFER_BYTES);
        let mut process = Api::<Pod>::namespaced(self.client.clone(), &target.namespace)
            .exec(&target.pod, command, &params)
            .await?;
        // All three exist because the params ask for them; a missing one is
        // a kube bug, reported rather than unwrapped.
        let missing = |what: &str| ExecError::Stream(format!("the exec stream has no {what}"));
        let status = process.take_status().ok_or_else(|| missing("status"))?;
        let mut stdout = process.stdout().ok_or_else(|| missing("stdout"))?;
        let stderr = process.stderr().ok_or_else(|| missing("stderr"))?;
        let (copied, stderr) = tokio::join!(tokio::io::copy(&mut stdout, out), read_tail(stderr));
        out.flush().await?;
        let status = status.await;
        process
            .join()
            .await
            .map_err(|e| ExecError::Stream(e.to_string()))?;
        copied?;
        let outcome = ExecOutcome::from(status.as_ref());
        outcome_result(outcome, &name, target, &stderr)
    }
}

/// Turns how a command ended into the result its caller sees, naming the
/// command and adding what it wrote to stderr.
pub fn outcome_result(
    outcome: ExecOutcome,
    command: &str,
    target: &PodTarget,
    stderr: &str,
) -> Result<(), ExecError> {
    let stderr = stderr.trim();
    match outcome {
        ExecOutcome::Succeeded => {
            if !stderr.is_empty() {
                tracing::info!(%target, command, "{stderr}");
            }
            Ok(())
        }
        ExecOutcome::Failed(message) => Err(ExecError::Failed {
            command: command.to_owned(),
            target: target.to_string(),
            message: if stderr.is_empty() {
                message
            } else {
                format!("{message}: {stderr}")
            },
        }),
        ExecOutcome::Missing => Err(ExecError::Cut {
            command: command.to_owned(),
            target: target.to_string(),
        }),
    }
}

/// Drains `reader`, keeping only its last [`MAX_STDERR_BYTES`]. Draining all
/// of it matters: an unread stream would stall the remote command.
async fn read_tail<R: AsyncRead + Unpin>(mut reader: R) -> String {
    let mut kept: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 8 * 1024];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                kept.extend(chunk.iter().take(read));
                let excess = kept.len().saturating_sub(MAX_STDERR_BYTES);
                kept.drain(..excess);
            }
        }
    }
    String::from_utf8_lossy(&kept).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> PodTarget {
        PodTarget {
            namespace: "apps".to_owned(),
            pod: "pg-2".to_owned(),
            container: "postgres".to_owned(),
        }
    }

    fn status(state: &str, message: Option<&str>, reason: Option<&str>) -> Status {
        Status {
            status: Some(state.to_owned()),
            message: message.map(str::to_owned),
            reason: reason.map(str::to_owned),
            ..Status::default()
        }
    }

    #[test]
    fn only_a_success_status_is_success() {
        assert_eq!(
            ExecOutcome::from(Some(&status("Success", None, None))),
            ExecOutcome::Succeeded
        );
        assert_eq!(
            ExecOutcome::from(Some(&status(
                "Failure",
                Some("command terminated with non-zero exit code: exit status 1"),
                Some("NonZeroExitCode"),
            ))),
            ExecOutcome::Failed(
                "command terminated with non-zero exit code: exit status 1".to_owned()
            )
        );
        assert_eq!(
            ExecOutcome::from(Some(&status("Failure", None, Some("NonZeroExitCode")))),
            ExecOutcome::Failed("NonZeroExitCode".to_owned())
        );
        assert!(matches!(
            ExecOutcome::from(Some(&status("Failure", None, None))),
            ExecOutcome::Failed(_)
        ));
        assert_eq!(ExecOutcome::from(None), ExecOutcome::Missing);
    }

    #[test]
    fn a_failure_carries_the_commands_stderr() {
        let err = outcome_result(
            ExecOutcome::Failed("exit status 1".to_owned()),
            "pg_dump",
            &target(),
            "pg_dump: error: connection failed\n",
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            err,
            "pg_dump failed in apps/pg-2 (postgres): exit status 1: pg_dump: error: connection \
             failed"
        );

        let err = outcome_result(
            ExecOutcome::Failed("exit status 2".to_owned()),
            "pg_dump",
            &target(),
            "",
        )
        .unwrap_err()
        .to_string();
        assert!(err.ends_with(": exit status 2"));
    }

    #[test]
    fn a_cut_stream_is_reported_as_cut() {
        let err = outcome_result(ExecOutcome::Missing, "pg_dump", &target(), "")
            .unwrap_err()
            .to_string();
        assert!(err.contains("ended before pg_dump finished"));
        assert!(err.contains("idle timeout"));
        assert!(outcome_result(ExecOutcome::Succeeded, "psql", &target(), "notice").is_ok());
    }

    mod against_a_fake_cluster {
        use super::*;
        use crate::tests::fake_kube::{ConnectProxy, FakeKube, Reply};

        async fn connect(kube: &FakeKube, dir: &Path) -> KubeExec {
            KubeExec::connect(Some(&kube.kubeconfig(dir).await))
                .await
                .unwrap()
        }

        /// Runs `pg_dump --dbname=app` in `apps/pg-2` against a fake cluster
        /// that answers it with `reply`, returning the outcome, the output
        /// and the calls the cluster saw.
        async fn dump_app(reply: Reply) -> (Result<(), ExecError>, Vec<u8>, Vec<String>) {
            let dir = tempfile::tempdir().unwrap();
            let kube = FakeKube::start(Vec::new()).await;
            kube.reply("apps", "pg-2", "postgres", "--dbname=app", reply);
            let exec = connect(&kube, dir.path()).await;
            let mut out = Vec::new();
            let result = exec
                .run(
                    &target(),
                    vec!["pg_dump".to_owned(), "--dbname=app".to_owned()],
                    &mut out,
                )
                .await;
            (result, out, kube.calls())
        }

        #[tokio::test]
        async fn the_api_is_reached_through_the_configured_http_proxy() {
            let dir = tempfile::tempdir().unwrap();
            let kube = FakeKube::start(Vec::new()).await;
            kube.reply(
                "apps",
                "pg-2",
                "postgres",
                "--dbname=app",
                Reply::ok(b"through the proxy"),
            );
            let proxy = ConnectProxy::start().await;
            let exec = KubeExec::connect(Some(&kube.kubeconfig_via(dir.path(), &proxy.url).await))
                .await
                .unwrap();

            assert_eq!(exec.list_pods().await.unwrap(), Vec::<Pod>::new());
            let mut out = Vec::new();
            exec.run(
                &target(),
                vec!["pg_dump".to_owned(), "--dbname=app".to_owned()],
                &mut out,
            )
            .await
            .unwrap();

            assert_eq!(out, b"through the proxy");
            assert_eq!(kube.calls(), ["apps/pg-2 postgres --dbname=app"]);
            // One tunnel for the pod list, one for the exec stream.
            let targets = proxy.targets();
            assert_eq!(targets.len(), 2);
            assert!(targets.iter().all(|t| t.starts_with("127.0.0.1:")));
        }

        #[tokio::test]
        async fn a_command_streams_its_output_and_succeeds() {
            let (result, out, calls) = dump_app(Reply::Success {
                stdout: b"dump bytes".to_vec(),
                stderr: "pg_dump: notice\n".to_owned(),
            })
            .await;
            result.unwrap();
            assert_eq!(out, b"dump bytes");
            assert_eq!(calls, ["apps/pg-2 postgres --dbname=app"]);
        }

        #[tokio::test]
        async fn a_failing_command_reports_its_stderr() {
            let (result, out, _) = dump_app(Reply::Failure {
                stdout: b"half".to_vec(),
                stderr: "pg_dump: error: connection lost".to_owned(),
            })
            .await;
            let err = result.unwrap_err().to_string();
            assert!(err.starts_with("pg_dump failed in apps/pg-2 (postgres)"));
            assert!(err.contains("non-zero exit code"));
            assert!(err.ends_with("pg_dump: error: connection lost"));
            assert_eq!(out, b"half");
        }

        #[tokio::test]
        async fn a_stream_cut_before_its_status_is_not_success() {
            let (result, _, _) = dump_app(Reply::Cut {
                stdout: b"partial".to_vec(),
            })
            .await;
            assert!(matches!(result, Err(ExecError::Cut { .. })));
        }

        #[tokio::test]
        async fn pods_are_listed_and_a_refused_list_is_an_error() {
            let dir = tempfile::tempdir().unwrap();
            let pod = Pod {
                metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                    name: Some("pg-1".to_owned()),
                    namespace: Some("apps".to_owned()),
                    ..Default::default()
                },
                ..Pod::default()
            };
            let kube = FakeKube::start(vec![pod]).await;
            let exec = connect(&kube, dir.path()).await;
            let pods = exec.list_pods().await.unwrap();
            assert_eq!(pods.len(), 1);
            assert_eq!(pods.first().unwrap().metadata.name.as_deref(), Some("pg-1"));

            kube.fail_list(403);
            assert!(matches!(exec.list_pods().await, Err(ExecError::Api(_))));
        }

        #[tokio::test]
        async fn a_kubeconfig_that_is_missing_or_unusable_is_a_config_error() {
            let dir = tempfile::tempdir().unwrap();
            let missing = KubeExec::connect(Some(&dir.path().join("absent"))).await;
            assert!(matches!(
                missing,
                Err(ExecError::Config(message)) if message.contains("absent")
            ));

            // Parses, but names no context to use.
            let empty = dir.path().join("empty");
            tokio::fs::write(&empty, r#"{"apiVersion":"v1","kind":"Config"}"#)
                .await
                .unwrap();
            assert!(matches!(
                KubeExec::connect(Some(&empty)).await,
                Err(ExecError::Config(_))
            ));
        }

        /// Without a path the agent's own environment decides. Whatever
        /// this machine has, a failure must surface as a config error.
        #[tokio::test]
        async fn without_a_path_a_failure_is_a_config_error() {
            if let Err(e) = KubeExec::connect(None).await {
                assert!(matches!(e, ExecError::Config(_)), "{e}");
            }
        }
    }

    #[tokio::test]
    async fn only_the_end_of_stderr_is_kept() {
        let long = "x".repeat(MAX_STDERR_BYTES * 3) + "the reason";
        let tail = read_tail(long.as_bytes()).await;
        assert_eq!(tail.len(), MAX_STDERR_BYTES);
        assert!(tail.ends_with("the reason"));
        assert_eq!(read_tail(&b"short"[..]).await, "short");
    }
}
