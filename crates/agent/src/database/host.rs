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

#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;

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
    #[cfg(unix)]
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
        unix::run(
            &self.program(command.first().map_or("", String::as_str)),
            target,
            command,
            input,
            out,
        )
        .await
    }

    /// Without Unix users to run it as, a host command is refused.
    ///
    /// # Errors
    ///
    /// Always.
    #[cfg(not(unix))]
    pub fn run_on_host<W>(
        &self,
        target: &HostTarget,
        command: Vec<String>,
        input: Stdin<'_>,
        out: &mut W,
    ) -> std::future::Ready<Result<(), ExecError>>
    where
        W: AsyncWrite + Unpin + Send,
    {
        let _ = (target, command, input, out);
        std::future::ready(Err(ExecError::Spawn {
            program: self.program("pg_dump").display().to_string(),
            error: std::io::Error::other("host dumps need a Unix host"),
        }))
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
        let mut client = tokio::process::Command::new(program);
        // A clean environment: the agent's own can hold its token and other
        // secrets that are none of the OS user's business. The agent's PATH
        // stays, so the client finds the programs the agent finds.
        client.env_clear();
        if let Some(path) = std::env::var_os("PATH") {
            client.env("PATH", path);
        }
        // Set even when the agent already runs as this user, which the
        // system allows.
        #[cfg(not(target_vendor = "apple"))]
        run_as(&mut client, &user)?;
        // macOS offers no way here to look up and set a user's other groups,
        // so the client runs in the user's primary group only.
        #[cfg(target_vendor = "apple")]
        client.uid(user.uid.as_raw()).gid(user.gid.as_raw());
        let mut child = client
            .args(command.iter().skip(1))
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

    /// Makes `command` run as `user` with all of the user's groups, the
    /// supplementary ones too, as a login would: a group can be what lets the
    /// user reach the server's socket directory or read its `~/.pgpass`.
    #[cfg(not(target_vendor = "apple"))]
    fn run_as(command: &mut tokio::process::Command, user: &User) -> Result<(), ExecError> {
        use std::ffi::CString;

        use nix::unistd::{geteuid, getgrouplist};

        let name = CString::new(user.name.as_str()).map_err(|e| ExecError::Io(e.into()))?;
        let groups = getgrouplist(&name, user.gid).map_err(|e| ExecError::Io(e.into()))?;
        // Only root may change its groups. An agent that is not root can only
        // run the client as itself, which keeps the groups it already has.
        let groups = geteuid().is_root().then_some(groups);
        let (uid, gid) = (user.uid, user.gid);
        // SAFETY: the closure runs in the forked child just before exec, where
        // only async-signal-safe calls are allowed; `become_user` makes only
        // system calls, on values prepared above, and allocates nothing.
        unsafe {
            command.pre_exec(move || become_user(groups.as_deref(), gid, uid));
        }
        Ok(())
    }

    /// Switches the calling process to `uid` and `gid`, and to `groups` when
    /// given. The groups go first, while the process still has the rights to
    /// change them. It runs in a forked child just before exec, so it only
    /// makes system calls and allocates nothing: a failure is a raw OS error.
    #[cfg(not(target_vendor = "apple"))]
    pub(super) fn become_user(
        groups: Option<&[nix::unistd::Gid]>,
        gid: nix::unistd::Gid,
        uid: nix::unistd::Uid,
    ) -> std::io::Result<()> {
        groups.map(nix::unistd::setgroups).transpose()?;
        nix::unistd::setgid(gid)?;
        nix::unistd::setuid(uid)?;
        Ok(())
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
    async fn a_command_sees_none_of_the_agents_environment() {
        let bin = bin_with("env").await;
        let (result, out) = run(bin.path(), &target()).await;
        result.unwrap();
        let names: std::collections::BTreeSet<String> = String::from_utf8(out)
            .unwrap()
            .lines()
            .filter_map(|line| line.split_once('=').map(|(name, _)| name.to_owned()))
            .collect();
        // Left out: what the shell running the script sets on its own.
        let shell: std::collections::BTreeSet<String> =
            ["PWD", "OLDPWD", "SHLVL", "_"].map(str::to_owned).into();
        let passed: Vec<&String> = names.difference(&shell).collect();
        assert_eq!(passed, ["HOME", "LOGNAME", "PATH", "USER"], "{names:?}");
    }

    /// A user listed as a member of some group, so that a login as it has a
    /// supplementary group, or `None` when the system has no such user.
    #[cfg(not(target_vendor = "apple"))]
    async fn a_user_with_supplementary_groups() -> Option<String> {
        tokio::fs::read_to_string("/etc/group")
            .await
            .ok()?
            .lines()
            .filter_map(|line| line.split(':').nth(3))
            .flat_map(|members| members.split(','))
            .find(|name| {
                !name.is_empty() && matches!(nix::unistd::User::from_name(name), Ok(Some(_)))
            })
            .map(str::to_owned)
    }

    #[cfg(not(target_vendor = "apple"))]
    #[tokio::test]
    async fn a_command_runs_with_all_of_the_users_groups() {
        // Only root can run the command as someone else and set its groups;
        // as anyone else the agent runs it as itself, with the groups the
        // agent already has.
        let as_root = nix::unistd::geteuid().is_root();
        let member = a_user_with_supplementary_groups().await;
        let os_user = member.filter(|_| as_root).unwrap_or_else(me);
        let bin = bin_with("id -G").await;
        let (result, out) = run(
            bin.path(),
            &HostTarget {
                os_user: os_user.clone(),
                ..target()
            },
        )
        .await;
        result.unwrap();

        let user = nix::unistd::User::from_name(&os_user).unwrap().unwrap();
        let name = std::ffi::CString::new(os_user).unwrap();
        let login_groups = nix::unistd::getgrouplist(&name, user.gid).unwrap();
        let mut own_groups = nix::unistd::getgroups().unwrap();
        own_groups.push(nix::unistd::getegid());
        let expected_groups = if as_root { login_groups } else { own_groups };
        let expected: std::collections::BTreeSet<u32> = expected_groups
            .into_iter()
            .map(nix::unistd::Gid::as_raw)
            .collect();
        let got: std::collections::BTreeSet<u32> = String::from_utf8(out)
            .unwrap()
            .split_whitespace()
            .map(|gid| gid.parse().unwrap())
            .collect();
        assert_eq!(got, expected);
    }

    #[cfg(not(target_vendor = "apple"))]
    #[test]
    fn becoming_the_user_the_process_already_is_changes_nothing() {
        let (uid, gid) = (nix::unistd::geteuid(), nix::unistd::getegid());
        let groups = nix::unistd::getgroups().unwrap();
        // Only root may set its groups, even to the ones it already has.
        let settable = uid.is_root().then_some(groups.as_slice());
        unix::become_user(settable, gid, uid).unwrap();
        assert_eq!(nix::unistd::geteuid(), uid);
        assert_eq!(nix::unistd::getegid(), gid);
        assert_eq!(nix::unistd::getgroups().unwrap(), groups);
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
