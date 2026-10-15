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
    /// Runs `command` in `target`, copying its stdout into `out`. Resolves
    /// once the command has ended, with an error unless it succeeded.
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
        let status = process.take_status();
        let stdout = process.stdout();
        let stderr = process.stderr();
        let (copied, stderr) = tokio::join!(
            async {
                match stdout {
                    Some(mut stdout) => tokio::io::copy(&mut stdout, out).await.map(|_| ()),
                    None => Ok(()),
                }
            },
            async {
                match stderr {
                    Some(stderr) => read_tail(stderr).await,
                    None => String::new(),
                }
            },
        );
        out.flush().await?;
        let status = match status {
            Some(status) => status.await,
            None => None,
        };
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

    #[tokio::test]
    async fn only_the_end_of_stderr_is_kept() {
        let long = "x".repeat(MAX_STDERR_BYTES * 3) + "the reason";
        let tail = read_tail(long.as_bytes()).await;
        assert_eq!(tail.len(), MAX_STDERR_BYTES);
        assert!(tail.ends_with("the reason"));
        assert_eq!(read_tail(&b"short"[..]).await, "short");
    }
}
