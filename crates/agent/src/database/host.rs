// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! `PostgreSQL` installed on the host itself.
//!
//! The agent starts the client programs locally, as the OS user the host's
//! settings name: over the server's Unix socket, peer authentication lets
//! that user in as the role of the same name, and a password setup is read
//! from the user's `~/.pgpass`. Assimilate holds no database credentials.
//! Starting the programs is a deliberate exception to the rule against
//! shelling out: no library dumps a database the way `pg_dump` does.

use std::path::{Path, PathBuf};

use shared::database::{
    DatabaseEngine, DatabaseRuntime, DatabaseRuntimes, DatabaseSourceId, DatabaseSourceKind,
    HOST_SOURCE_NAME,
};
use tokio::io::AsyncWrite;

use super::{discovery::SourceCandidate, exec::ExecError, stdin::Stdin, target::DumpTarget};

/// The host's own server, and who the agent runs its clients as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostTarget {
    /// The OS user the client runs as.
    pub os_user: String,
    /// Directory of the server's Unix socket, `None` for the client's default.
    pub socket_dir: Option<String>,
    /// The server's port, `None` for the client's default.
    pub port: Option<u16>,
}

impl HostTarget {
    /// Options telling a client where the server listens.
    #[must_use]
    pub fn connection_args(&self) -> Vec<String> {
        self.socket_dir
            .iter()
            .map(|dir| format!("--host={dir}"))
            .chain(self.port.iter().map(|port| format!("--port={port}")))
            .collect()
    }
}

impl std::fmt::Display for HostTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the host (as {})", self.os_user)
    }
}

/// The one source a host's own server stands for.
#[must_use]
pub fn host_source(runtimes: &DatabaseRuntimes) -> SourceCandidate {
    let os_user = runtimes.host_os_user().to_owned();
    SourceCandidate {
        id: DatabaseSourceId {
            runtime: DatabaseRuntime::Host,
            namespace: String::new(),
            name: HOST_SOURCE_NAME.to_owned(),
        },
        kind: DatabaseSourceKind::Host,
        engine: DatabaseEngine::Postgresql,
        target: DumpTarget::Host(HostTarget {
            os_user: os_user.clone(),
            socket_dir: runtimes.host_socket_dir.clone(),
            port: runtimes.host_port,
        }),
        user: os_user,
        databases: None,
    }
}

/// Runs the database clients on the host.
#[derive(Debug, Clone, Default)]
pub struct HostExec {
    bin_dir: Option<PathBuf>,
}

impl HostExec {
    /// Runs programs from `bin_dir`, or, without one, from the agent's `PATH`.
    #[must_use]
    pub fn new(bin_dir: Option<&str>) -> Self {
        Self {
            bin_dir: bin_dir.map(PathBuf::from),
        }
    }

    fn program(&self, name: &str) -> PathBuf {
        self.bin_dir
            .as_deref()
            .map_or_else(|| PathBuf::from(name), |dir| dir.join(name))
    }

    /// Runs `command` as `target`'s OS user with `input` on its stdin,
    /// copying its stdout into `out` and flushing it. Resolves once the
    /// command has ended, with an error unless it exited with 0. Dropping
    /// the future, or an input that breaks off, kills the command.
    ///
    /// # Errors
    ///
    /// Returns an error when the user does not exist, the program cannot be
    /// started, the input cannot be read, or it does not succeed.
    pub async fn run_on_host<W>(
        &self,
        target: &HostTarget,
        command: Vec<String>,
        input: Stdin<'_>,
        out: &mut W,
    ) -> Result<(), ExecError>
    where
        W: AsyncWrite + Unpin + Send,
    {
        #[cfg(unix)]
        {
            unix::run(
                &self.program(command.first().map_or("", String::as_str)),
                target,
                command,
                input,
                out,
            )
            .await
        }
        #[cfg(not(unix))]
        {
            let _ = (target, command, input, out);
            Err(ExecError::Spawn {
                program: self.program("pg_dump").display().to_string(),
                error: std::io::Error::other("host dumps need a Unix host"),
            })
        }
    }
}

#[cfg(unix)]
mod unix {
    use std::process::Stdio;

    use nix::unistd::User;
    use tokio::io::{AsyncWrite, AsyncWriteExt as _};

    use super::{super::exec, ExecError, HostTarget, Path, Stdin};

    /// Starts `program` with `command`'s arguments as the target's OS user,
    /// with that user's home so the client finds its `~/.pgpass`, and feeds
    /// it `input`.
    pub(super) async fn run<W>(
        program: &Path,
        target: &HostTarget,
        command: Vec<String>,
        input: Stdin<'_>,
        out: &mut W,
    ) -> Result<(), ExecError>
    where
        W: AsyncWrite + Unpin + Send,
    {
        let name = command.first().cloned().unwrap_or_default();
        let user = User::from_name(&target.os_user)
            .map_err(|e| ExecError::Io(e.into()))?
            .ok_or_else(|| ExecError::UnknownUser(target.os_user.clone()))?;
        // Set even when the agent already runs as this user, which the
        // system allows; as root, the standard library also drops root's
        // supplementary groups.
        let mut child = tokio::process::Command::new(program)
            .args(command.iter().skip(1))
            .uid(user.uid.as_raw())
            .gid(user.gid.as_raw())
            .env("HOME", &user.dir)
            .env("USER", &user.name)
            .env("LOGNAME", &user.name)
            .current_dir("/")
            .stdin(if input.is_open() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| ExecError::Spawn {
                program: program.display().to_string(),
                error,
            })?;
        // Both exist because they were asked for as pipes; stdin is one only
        // when there is an input.
        let missing = |what: &str| ExecError::Stream(format!("the process has no {what}"));
        let mut stdin = child.stdin.take();
        let mut stdout = child.stdout.take().ok_or_else(|| missing("stdout"))?;
        let stderr = child.stderr.take().ok_or_else(|| missing("stderr"))?;
        let output = async {
            Ok::<_, ExecError>(tokio::join!(
                tokio::io::copy(&mut stdout, out),
                exec::read_tail(stderr)
            ))
        };
        // A broken input kills the command before its stdin is closed, so it
        // never sees an end of file.
        let streamed = tokio::try_join!(input.feed(&mut stdin), output);
        let ((), (copied, stderr)) = streamed.inspect_err(|_| {
            let _ = child.start_kill();
        })?;
        out.flush().await?;
        let status = child.wait().await?;
        copied?;
        let outcome = if status.success() {
            exec::ExecOutcome::Succeeded
        } else {
            exec::ExecOutcome::Failed(status.to_string())
        };
        exec::outcome_result(outcome, &name, target, &stderr)
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    /// The user the tests run as, which the agent may always switch to.
    fn me() -> String {
        nix::unistd::User::from_uid(nix::unistd::getuid())
            .unwrap()
            .unwrap()
            .name
    }

    fn target() -> HostTarget {
        HostTarget {
            os_user: me(),
            socket_dir: Some("/run/postgresql".to_owned()),
            port: Some(5433),
        }
    }

    /// A directory holding a `pg_dump` script with `body`.
    async fn bin_with(body: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("pg_dump");
        tokio::fs::write(&script, format!("#!/bin/sh\n{body}\n"))
            .await
            .unwrap();
        tokio::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();
        dir
    }

    async fn run(bin: &Path, target: &HostTarget) -> (Result<(), ExecError>, Vec<u8>) {
        let exec = HostExec::new(bin.to_str());
        let mut out = Vec::new();
        let result = exec
            .run_on_host(
                target,
                vec!["pg_dump".to_owned(), "--dbname=app".to_owned()],
                Stdin::Closed,
                &mut out,
            )
            .await;
        (result, out)
    }

    #[tokio::test]
    async fn a_command_runs_as_the_user_in_its_home_and_streams_its_output() {
        let bin =
            bin_with(r#"printf '%s|%s|%s|%s' "$1" "$USER" "$HOME" "$(pwd)"; echo notice >&2"#)
                .await;
        let (result, out) = run(bin.path(), &target()).await;
        result.unwrap();
        let home = nix::unistd::User::from_name(&me()).unwrap().unwrap().dir;
        assert_eq!(
            String::from_utf8(out).unwrap(),
            format!("--dbname=app|{}|{}|/", me(), home.display())
        );
    }

    #[tokio::test]
    async fn a_command_reads_its_input_to_the_end() {
        let bin = bin_with("cat").await;
        let exec = HostExec::new(bin.path().to_str());
        let mut input: &[u8] = b"dump bytes";
        let mut out = Vec::new();
        exec.run_on_host(
            &target(),
            vec!["pg_dump".to_owned()],
            Stdin::From(&mut input),
            &mut out,
        )
        .await
        .unwrap();
        assert_eq!(out, b"dump bytes");
    }

    #[tokio::test]
    async fn an_input_that_breaks_off_kills_the_command() {
        // Would wait forever for the rest of its input.
        let bin = bin_with("cat > /dev/null; sleep 60").await;
        let exec = HostExec::new(bin.path().to_str());
        let mut input = super::super::stdin::broken::Broken::after(b"half");
        let mut out = Vec::new();
        let result = exec
            .run_on_host(
                &target(),
                vec!["pg_dump".to_owned()],
                Stdin::From(&mut input),
                &mut out,
            )
            .await;
        assert!(matches!(result, Err(ExecError::Input(_))), "{result:?}");
    }

    #[tokio::test]
    async fn a_failing_command_reports_its_status_and_stderr() {
        let bin =
            bin_with("printf half; echo 'pg_dump: error: no such database' >&2; exit 3").await;
        let (result, out) = run(bin.path(), &target()).await;
        let err = result.unwrap_err().to_string();
        assert!(err.starts_with("pg_dump failed in the host (as "), "{err}");
        assert!(err.contains("exit status: 3"), "{err}");
        assert!(err.ends_with("pg_dump: error: no such database"), "{err}");
        assert_eq!(out, b"half");
    }

    #[tokio::test]
    async fn an_unknown_user_or_program_is_refused_before_anything_runs() {
        let bin = bin_with("exit 0").await;
        let nobody = HostTarget {
            os_user: "assimilate-no-such-user".to_owned(),
            ..target()
        };
        let (result, _) = run(bin.path(), &nobody).await;
        assert!(matches!(result, Err(ExecError::UnknownUser(_))));

        let empty = tempfile::tempdir().unwrap();
        let (result, _) = run(empty.path(), &target()).await;
        let err = result.unwrap_err();
        assert!(matches!(err, ExecError::Spawn { .. }));
        assert!(err.to_string().contains("pg_dump"), "{err}");
    }

    #[test]
    fn programs_come_from_the_bin_dir_or_the_path() {
        assert_eq!(HostExec::new(None).program("psql"), Path::new("psql"));
        assert_eq!(
            HostExec::new(Some("/usr/lib/postgresql/16/bin")).program("psql"),
            Path::new("/usr/lib/postgresql/16/bin/psql")
        );
    }

    #[test]
    fn the_host_source_follows_the_settings() {
        let defaults = host_source(&DatabaseRuntimes {
            host: true,
            ..DatabaseRuntimes::default()
        });
        assert_eq!(defaults.id.to_string(), "host:postgresql");
        assert_eq!(defaults.kind, DatabaseSourceKind::Host);
        assert_eq!(defaults.user, "postgres");
        assert_eq!(defaults.target.connection_args().len(), 0);

        let custom = host_source(&DatabaseRuntimes {
            host: true,
            host_os_user: Some("pgadmin".to_owned()),
            host_socket_dir: Some("/var/run/pg".to_owned()),
            host_port: Some(5433),
            ..DatabaseRuntimes::default()
        });
        assert_eq!(custom.user, "pgadmin");
        assert_eq!(custom.target.place(), "/var/run/pg");
        assert_eq!(custom.target.detail(), "pgadmin");
        assert_eq!(
            custom.target.connection_args(),
            ["--host=/var/run/pg", "--port=5433"]
        );
        assert_eq!(target().to_string(), format!("the host (as {})", me()));
    }
}
