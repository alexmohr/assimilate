// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Where a database command runs, and what runs it there.
//!
//! A pod is reached through the Kubernetes exec API, a container through its
//! engine's API socket, and the host's own server by starting the client
//! locally. The backup flow only sees [`CommandExec`], so it treats them all
//! alike.

use std::future::Future;

use shared::database::DatabaseRuntime;
use tokio::io::AsyncWrite;

use super::{
    docker::{ContainerTarget, DockerExec},
    exec::{ExecError, KubeExec, PodTarget},
    host::{HostExec, HostTarget},
};

/// Where one command runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DumpTarget {
    /// A container of a pod in the host's Kubernetes cluster.
    Pod(PodTarget),
    /// A Docker or Podman container.
    Container(ContainerTarget),
    /// The host itself, as a local OS user.
    Host(HostTarget),
}

impl DumpTarget {
    /// Where the command runs, as a scan reports it: the pod, the container's
    /// name, or the host's socket directory (empty for the default one).
    #[must_use]
    pub fn place(&self) -> String {
        match self {
            Self::Pod(pod) => pod.pod.clone(),
            Self::Container(container) => container.name.clone(),
            Self::Host(host) => host.socket_dir.clone().unwrap_or_default(),
        }
    }

    /// What narrows [`Self::place`] down: the container inside a pod, or the
    /// OS user a host command runs as. Empty for a container.
    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            Self::Pod(pod) => pod.container.clone(),
            Self::Container(_) => String::new(),
            Self::Host(host) => host.os_user.clone(),
        }
    }

    /// Options telling a client where its server listens. Only the host needs
    /// them: inside a pod or a container, the client's default is the server.
    #[must_use]
    pub fn connection_args(&self) -> Vec<String> {
        match self {
            Self::Pod(_) | Self::Container(_) => Vec::new(),
            Self::Host(host) => host.connection_args(),
        }
    }
}

impl std::fmt::Display for DumpTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pod(pod) => pod.fmt(f),
            Self::Container(container) => container.fmt(f),
            Self::Host(host) => host.fmt(f),
        }
    }
}

/// Runs commands where databases live. A trait so the backup flow can be
/// tested without a cluster, a container engine or a database server.
pub trait CommandExec: Send + Sync {
    /// Runs `command` at `target`, copying its stdout into `out` and flushing
    /// it. Resolves once the command has ended, with an error unless it
    /// succeeded.
    fn run<W>(
        &self,
        target: &DumpTarget,
        command: Vec<String>,
        out: &mut W,
    ) -> impl Future<Output = Result<(), ExecError>> + Send
    where
        W: AsyncWrite + Unpin + Send;
}

/// One client per runtime a backup or scan reached. A target in a runtime
/// that was not reached is refused rather than guessed at.
#[derive(Default)]
pub struct RuntimeClients {
    pub kubernetes: Option<KubeExec>,
    pub docker: Option<DockerExec>,
    pub host: Option<HostExec>,
}

impl CommandExec for RuntimeClients {
    async fn run<W>(
        &self,
        target: &DumpTarget,
        command: Vec<String>,
        out: &mut W,
    ) -> Result<(), ExecError>
    where
        W: AsyncWrite + Unpin + Send,
    {
        let not_connected = |runtime| ExecError::NotConnected(runtime);
        match target {
            DumpTarget::Pod(pod) => {
                let kube = self.kubernetes.as_ref();
                let kube = kube.ok_or_else(|| not_connected(DatabaseRuntime::Kubernetes))?;
                kube.run_in_pod(pod, command, out).await
            }
            DumpTarget::Container(container) => {
                let docker = self.docker.as_ref();
                let docker = docker.ok_or_else(|| not_connected(DatabaseRuntime::Docker))?;
                docker.run_in_container(container, command, out).await
            }
            DumpTarget::Host(host) => {
                let local = self.host.as_ref();
                let local = local.ok_or_else(|| not_connected(DatabaseRuntime::Host))?;
                local.run_on_host(host, command, out).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod() -> DumpTarget {
        DumpTarget::Pod(PodTarget {
            namespace: "apps".to_owned(),
            pod: "pg-1".to_owned(),
            container: "postgres".to_owned(),
        })
    }

    fn container() -> DumpTarget {
        DumpTarget::Container(ContainerTarget {
            id: "0123abcd".to_owned(),
            name: "wiki-db".to_owned(),
        })
    }

    fn host(socket_dir: Option<&str>, port: Option<u16>) -> DumpTarget {
        DumpTarget::Host(HostTarget {
            os_user: "postgres".to_owned(),
            socket_dir: socket_dir.map(str::to_owned),
            port,
        })
    }

    #[test]
    fn each_target_says_where_it_runs() {
        assert_eq!(
            (pod().place(), pod().detail()),
            ("pg-1".into(), "postgres".into())
        );
        assert_eq!(
            (container().place(), container().detail()),
            ("wiki-db".into(), String::new())
        );
        assert_eq!(
            (host(None, None).place(), host(None, None).detail()),
            (String::new(), "postgres".into())
        );
        assert_eq!(host(Some("/run/pg"), None).place(), "/run/pg");

        assert_eq!(pod().to_string(), "apps/pg-1 (postgres)");
        assert_eq!(container().to_string(), "container wiki-db");
        assert_eq!(host(None, None).to_string(), "the host (as postgres)");
    }

    #[test]
    fn only_the_host_names_its_server() {
        assert_eq!(pod().connection_args().len(), 0);
        assert_eq!(container().connection_args().len(), 0);
        assert_eq!(host(None, None).connection_args().len(), 0);
        assert_eq!(
            host(Some("/run/pg"), Some(5433)).connection_args(),
            ["--host=/run/pg", "--port=5433"]
        );
    }

    #[tokio::test]
    async fn a_target_in_a_runtime_that_was_not_reached_is_refused() {
        let clients = RuntimeClients::default();
        for (target, runtime) in [
            (pod(), "kubernetes"),
            (container(), "docker"),
            (host(None, None), "host"),
        ] {
            let mut out = Vec::new();
            let err = clients
                .run(&target, vec!["true".to_owned()], &mut out)
                .await
                .unwrap_err();
            assert!(matches!(err, ExecError::NotConnected(_)));
            assert!(err.to_string().contains(runtime), "{err}");
        }
    }
}
