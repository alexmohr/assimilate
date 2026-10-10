// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    time::Duration,
};

use tokio::process::{Child, Command};

use crate::secrets::Secret;

/// One environment variable for a child process. Secrets are only ever
/// passed this way - never as arguments, which other local users can read
/// from the process table.
#[derive(Debug, Clone)]
pub enum EnvValue {
    /// A value safe to log.
    Plain(String),
    /// A value that must never be logged.
    Secret(Secret),
}

impl EnvValue {
    pub(crate) fn expose(&self) -> &str {
        match self {
            Self::Plain(value) => value,
            Self::Secret(secret) => secret.expose(),
        }
    }
}

/// What to run, with which environment, logging where.
#[derive(Debug, Clone)]
pub struct ChildSpec {
    /// A short name for logs and errors, e.g. `server`.
    pub name: &'static str,
    /// The executable.
    pub program: PathBuf,
    /// Variables set on top of the inherited environment.
    pub env: Vec<(&'static str, EnvValue)>,
    /// File the child's stdout and stderr are appended to.
    pub log_file: PathBuf,
}

/// Why a child couldn't be started or stopped.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    /// Spawning, signalling or waiting on the child failed.
    #[error("{name}: {source}")]
    Io {
        /// Which child.
        name: &'static str,
        /// What went wrong.
        source: std::io::Error,
    },
    /// The child's process id can't be signalled.
    #[error("{name}: process id out of range")]
    Pid {
        /// Which child.
        name: &'static str,
    },
}

impl ProcessError {
    /// Wraps an I/O error from the child called `name`.
    fn io(name: &'static str) -> impl FnOnce(std::io::Error) -> Self {
        move |source| Self::Io { name, source }
    }
}

/// A running child process the app owns.
#[derive(Debug)]
pub struct Supervised {
    name: &'static str,
    child: Child,
}

impl Supervised {
    /// Starts the child. It is killed if this handle is dropped without
    /// [`Supervised::terminate`], so a crashing app never leaves it behind.
    ///
    /// # Errors
    ///
    /// Fails if the log file can't be opened or the program can't be started.
    pub async fn spawn(spec: &ChildSpec) -> Result<Self, ProcessError> {
        let io = || ProcessError::io(spec.name);
        let log = open_log(&spec.log_file).await.map_err(io())?;
        let log_err = log.try_clone().map_err(io())?;
        let child = Command::new(&spec.program)
            .envs(spec.env.iter().map(|(key, value)| (key, value.expose())))
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(log_err)
            .kill_on_drop(true)
            .spawn()
            .map_err(io())?;
        let program = spec.program.display();
        tracing::info!(name = spec.name, %program, "started child process");
        Ok(Self {
            name: spec.name,
            child,
        })
    }

    /// The child's name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Whether the child has exited, and how.
    ///
    /// # Errors
    ///
    /// Fails if the child's status can't be read.
    pub fn exit_status(&mut self) -> Result<Option<ExitStatus>, ProcessError> {
        self.child.try_wait().map_err(ProcessError::io(self.name))
    }

    /// Asks the child to stop (SIGTERM), so it can finish what it's doing,
    /// and kills it if it hasn't exited after `grace`.
    ///
    /// # Errors
    ///
    /// Fails if the child can't be signalled or waited on.
    pub async fn terminate(mut self, grace: Duration) -> Result<ExitStatus, ProcessError> {
        let io = || ProcessError::io(self.name);
        if let Some(status) = self.child.try_wait().map_err(io())? {
            return Ok(status);
        }
        self.request_stop()?;
        if let Ok(status) = tokio::time::timeout(grace, self.child.wait()).await {
            return status.map_err(io());
        }
        tracing::warn!(
            name = self.name,
            ?grace,
            "child ignored SIGTERM, killing it"
        );
        self.child.kill().await.map_err(io())?;
        self.child.wait().await.map_err(io())
    }

    #[cfg(unix)]
    fn request_stop(&self) -> Result<(), ProcessError> {
        use nix::{
            sys::signal::{Signal, kill},
            unistd::Pid,
        };
        // No id means the child was already reaped: nothing left to stop.
        self.child.id().map_or(Ok(()), |pid| {
            let pid = i32::try_from(pid).map_err(|_| ProcessError::Pid { name: self.name })?;
            let signalled = kill(Pid::from_raw(pid), Signal::SIGTERM);
            signalled.map_err(|errno| ProcessError::io(self.name)(errno.into()))
        })
    }
}

async fn open_log(path: &Path) -> std::io::Result<std::fs::File> {
    tokio::fs::create_dir_all(path.parent().unwrap_or_else(|| Path::new("."))).await?;
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    Ok(file.into_std().await)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes an executable shell script and a spec that runs it with a
    /// secret in its environment.
    async fn script(dir: &Path, name: &'static str, body: &str) -> ChildSpec {
        use std::os::unix::fs::PermissionsExt;
        let program = dir.join(format!("{name}.sh"));
        tokio::fs::write(&program, format!("#!/bin/sh\n{body}\n"))
            .await
            .unwrap();
        tokio::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();
        ChildSpec {
            name,
            program,
            env: vec![(
                "SECRET",
                EnvValue::Secret(Secret::from_stored("s3cret".to_owned())),
            )],
            log_file: dir.join("logs").join(format!("{name}.log")),
        }
    }

    #[tokio::test]
    async fn passes_secrets_through_the_environment_and_logs_output() {
        let dir = tempfile::tempdir().unwrap();
        let spec = script(dir.path(), "printer", "echo \"got $SECRET\"").await;

        let mut child = Supervised::spawn(&spec).await.unwrap();
        let status = child.child.wait().await.unwrap();

        assert!(status.success());
        assert_eq!(
            tokio::fs::read_to_string(&spec.log_file).await.unwrap(),
            "got s3cret\n"
        );
    }

    #[tokio::test]
    async fn spawning_a_missing_program_names_the_child() {
        let dir = tempfile::tempdir().unwrap();
        let spec = ChildSpec {
            program: dir.path().join("no-such-program"),
            ..script(dir.path(), "ghost", "true").await
        };

        let error = Supervised::spawn(&spec).await.unwrap_err();

        assert!(matches!(error, ProcessError::Io { name: "ghost", .. }));
        assert!(error.to_string().starts_with("ghost: "));
    }

    #[tokio::test]
    async fn a_supervised_child_reports_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let spec = script(dir.path(), "named", "exit 0").await;

        let child = Supervised::spawn(&spec).await.unwrap();

        assert_eq!(child.name(), "named");
        child.terminate(Duration::from_secs(5)).await.unwrap();
    }

    #[tokio::test]
    async fn terminate_stops_a_child_that_honours_sigterm() {
        let dir = tempfile::tempdir().unwrap();
        let spec = script(dir.path(), "sleeper", "exec sleep 30").await;

        let child = Supervised::spawn(&spec).await.unwrap();
        let started = std::time::Instant::now();
        let status = child.terminate(Duration::from_secs(10)).await.unwrap();

        assert!(!status.success());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn terminate_kills_a_child_that_ignores_sigterm() {
        let dir = tempfile::tempdir().unwrap();
        let spec = script(
            dir.path(),
            "stubborn",
            "trap '' TERM\nwhile :; do sleep 1; done",
        )
        .await;

        let mut child = Supervised::spawn(&spec).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(child.exit_status().unwrap().is_none());
        let status = child.terminate(Duration::from_millis(300)).await.unwrap();

        assert!(!status.success());
    }

    #[tokio::test]
    async fn terminate_returns_the_status_of_a_child_that_already_exited() {
        let dir = tempfile::tempdir().unwrap();
        let spec = script(dir.path(), "quick", "exit 3").await;

        let mut child = Supervised::spawn(&spec).await.unwrap();
        while child.exit_status().unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let status = child.terminate(Duration::from_secs(1)).await.unwrap();

        assert_eq!(status.code(), Some(3));
    }
}
