// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Database servers installed on the host itself.
//!
//! The agent starts the client programs locally, as the OS user the host's
//! settings name, which signs in on its own; Assimilate holds no database
//! credentials:
//!
//! - `PostgreSQL`: over the server's Unix socket, peer authentication lets
//!   that user in as the role of the same name, and a password setup is read
//!   from the user's `~/.pgpass`.
//! - `MySQL` and `MariaDB`: over the socket, `root` is let in as the account
//!   of the same name on Debian and Ubuntu, and a password setup is read from
//!   the user's `~/.my.cnf`.
//! - `MongoDB`: a server with access control on is named in a config file of
//!   the database tools, `uri` and password included.
//! - `SQLite`: no server; `sqlite3` reads the files the settings list, as an
//!   OS user allowed to.
//!
//! Starting the programs is a deliberate exception to the rule against
//! shelling out: no library dumps a database the way the engines' own tools
//! do.

use std::path::{Path, PathBuf};

use shared::database::{
    DEFAULT_HOST_ROOT_USER, DatabaseEngine, DatabaseRuntime, DatabaseRuntimes, DatabaseSourceId,
    DatabaseSourceKind,
};
use tokio::io::AsyncWrite;

use super::{discovery::SourceCandidate, exec::ExecError, stdin::Stdin, target::DumpTarget};

/// Where a server installed on the host listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostServer {
    Postgresql {
        /// Directory of the server's Unix socket, `None` for the client's
        /// default.
        socket_dir: Option<String>,
        /// The server's port, `None` for the client's default.
        port: Option<u16>,
    },
    Mysql {
        /// The server's Unix socket, `None` for the client's default.
        socket: Option<String>,
        /// The server's TCP port on `127.0.0.1`, `None` for the socket.
        port: Option<u16>,
    },
    Mongodb {
        /// The server's port on `127.0.0.1`, `None` for 27017.
        port: Option<u16>,
        /// A database tools config file naming the server's `uri`.
        config_file: Option<String>,
    },
    /// `SQLite` files, which `sqlite3` opens directly.
    Sqlite,
}

/// A server installed on the host, and who the agent runs its clients as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostTarget {
    /// The OS user the client runs as.
    pub os_user: String,
    /// Where the server listens.
    pub server: HostServer,
    /// Directory holding the client programs, `None` to find them on the
    /// agent's `PATH`.
    pub bin_dir: Option<String>,
}

impl HostTarget {
    /// Options telling a client where the server listens.
    #[must_use]
    pub fn connection_args(&self) -> Vec<String> {
        let option = |name: &str, value: &dyn std::fmt::Display| format!("--{name}={value}");
        match &self.server {
            HostServer::Postgresql { socket_dir, port } => socket_dir
                .iter()
                .map(|dir| option("host", dir))
                .chain(port.iter().map(|port| option("port", port)))
                .collect(),
            HostServer::Mysql { socket, port } => socket
                .iter()
                .map(|socket| option("socket", socket))
                .chain(
                    port.iter()
                        .flat_map(|port| [option("host", &"127.0.0.1"), option("port", port)]),
                )
                .collect(),
            // The config file names the server itself; a port beside it
            // would contradict its `uri`.
            HostServer::Mongodb { port, config_file } => config_file.as_ref().map_or_else(
                || port.iter().map(|port| option("port", port)).collect(),
                |file| vec![option("config", file)],
            ),
            HostServer::Sqlite => Vec::new(),
        }
    }

    /// The file or directory that names where the server listens, when the
    /// settings name one: its socket directory, socket or config file.
    #[must_use]
    pub fn place(&self) -> Option<&str> {
        match &self.server {
            HostServer::Postgresql { socket_dir, .. } => socket_dir.as_deref(),
            HostServer::Mysql { socket, .. } => socket.as_deref(),
            HostServer::Mongodb { config_file, .. } => config_file.as_deref(),
            HostServer::Sqlite => None,
        }
    }

    /// Where `name` is started from. An absolute `name`, such as the
    /// `/bin/sh` a script runs in, is started from there.
    pub(super) fn program(&self, name: &str) -> PathBuf {
        self.bin_dir
            .as_deref()
            .map_or_else(|| PathBuf::from(name), |dir| Path::new(dir).join(name))
    }
}

impl std::fmt::Display for HostTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the host (as {})", self.os_user)
    }
}

fn host_candidate(
    engine: DatabaseEngine,
    os_user: &str,
    server: HostServer,
    bin_dir: Option<&String>,
    databases: Option<Vec<String>>,
) -> SourceCandidate {
    SourceCandidate {
        id: DatabaseSourceId {
            runtime: DatabaseRuntime::Host,
            namespace: String::new(),
            name: engine.to_string(),
        },
        kind: DatabaseSourceKind::Host,
        engine,
        target: DumpTarget::Host(HostTarget {
            os_user: os_user.to_owned(),
            server,
            bin_dir: bin_dir.cloned(),
        }),
        user: os_user.to_owned(),
        password_env: None,
        databases,
    }
}

/// The source the host's own `PostgreSQL` server stands for.
#[must_use]
pub fn host_source(runtimes: &DatabaseRuntimes) -> SourceCandidate {
    host_candidate(
        DatabaseEngine::Postgresql,
        runtimes.host_os_user(),
        HostServer::Postgresql {
            socket_dir: runtimes.host_socket_dir.clone(),
            port: runtimes.host_port,
        },
        runtimes.host_bin_dir.as_ref(),
        None,
    )
}

/// One source for every server installed on the host that `runtimes`
/// dumps.
#[must_use]
pub fn host_sources(runtimes: &DatabaseRuntimes) -> Vec<SourceCandidate> {
    let mysql = &runtimes.host_mysql;
    let mongodb = &runtimes.host_mongodb;
    let sqlite = &runtimes.host_sqlite;
    let root = |user: &Option<String>| {
        user.clone()
            .unwrap_or_else(|| DEFAULT_HOST_ROOT_USER.to_owned())
    };
    [
        runtimes.host.then(|| host_source(runtimes)),
        mysql.enabled.then(|| {
            host_candidate(
                mysql.engine.into(),
                &root(&mysql.os_user),
                HostServer::Mysql {
                    socket: mysql.socket.clone(),
                    port: mysql.port,
                },
                mysql.bin_dir.as_ref(),
                None,
            )
        }),
        mongodb.enabled.then(|| {
            host_candidate(
                DatabaseEngine::Mongodb,
                &root(&mongodb.os_user),
                HostServer::Mongodb {
                    port: mongodb.port,
                    config_file: mongodb.config_file.clone(),
                },
                mongodb.bin_dir.as_ref(),
                None,
            )
        }),
        sqlite.enabled.then(|| {
            host_candidate(
                DatabaseEngine::Sqlite,
                &root(&sqlite.os_user),
                HostServer::Sqlite,
                sqlite.bin_dir.as_ref(),
                Some(sqlite.files.clone()),
            )
        }),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Runs the database clients on the host.
#[derive(Debug, Clone, Default)]
pub struct HostExec;

impl HostExec {
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
            &target.program(command.first().map_or("", String::as_str)),
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
        let program = target.program(command.first().map_or("", String::as_str));
        let _ = (self, command, input, out);
        std::future::ready(Err(ExecError::Spawn {
            program: program.display().to_string(),
            error: std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "host dumps need a Unix host",
            ),
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
#[cfg(not(unix))]
mod windows_tests {
    use super::*;

    #[tokio::test]
    async fn a_host_dump_is_refused_without_unix_users() {
        let target = HostTarget {
            os_user: "postgres".to_owned(),
            server: HostServer::Postgresql {
                socket_dir: None,
                port: None,
            },
            bin_dir: None,
        };
        let mut out = Vec::new();

        let result = HostExec
            .run_on_host(&target, vec!["pg_dump".to_owned()], Stdin::Closed, &mut out)
            .await;

        assert!(
            matches!(&result, Err(ExecError::Spawn { error, .. })
                if error.kind() == std::io::ErrorKind::Unsupported
                    && error.to_string().contains("need a Unix host")),
            "{result:?}"
        );
        assert_eq!(out, b"");
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
            server: HostServer::Postgresql {
                socket_dir: Some("/run/postgresql".to_owned()),
                port: Some(5433),
            },
            bin_dir: None,
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

    /// `target` with its programs in `bin`.
    fn from_bin(bin: &Path, target: &HostTarget) -> HostTarget {
        HostTarget {
            bin_dir: Some(bin.to_string_lossy().into_owned()),
            ..target.clone()
        }
    }

    async fn run(bin: &Path, target: &HostTarget) -> (Result<(), ExecError>, Vec<u8>) {
        let mut out = Vec::new();
        let result = HostExec
            .run_on_host(
                &from_bin(bin, target),
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
        let mut input: &[u8] = b"dump bytes";
        let mut out = Vec::new();
        HostExec
            .run_on_host(
                &from_bin(bin.path(), &target()),
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
        let mut input = super::super::stdin::broken::Broken::after(b"half");
        let mut out = Vec::new();
        let result = HostExec
            .run_on_host(
                &from_bin(bin.path(), &target()),
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
        assert_eq!(target().program("psql"), Path::new("psql"));
        assert_eq!(
            from_bin(Path::new("/usr/lib/postgresql/16/bin"), &target()).program("psql"),
            Path::new("/usr/lib/postgresql/16/bin/psql")
        );
        assert_eq!(
            from_bin(Path::new("/opt/sqlite/bin"), &target()).program("/bin/sh"),
            Path::new("/bin/sh"),
            "a script's shell is never looked for in the programs directory"
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

    #[test]
    fn every_server_installed_on_the_host_is_a_source_of_its_own() {
        let mut runtimes = DatabaseRuntimes::default();
        assert_eq!(host_sources(&runtimes).len(), 0);
        runtimes.host = true;
        runtimes.host_mysql = shared::database::HostMysql {
            enabled: true,
            engine: shared::database::MysqlEngine::Mariadb,
            socket: Some("/run/mysqld/mysqld.sock".to_owned()),
            bin_dir: Some("/opt/mariadb/bin".to_owned()),
            ..shared::database::HostMysql::default()
        };
        runtimes.host_mongodb = shared::database::HostMongodb {
            enabled: true,
            os_user: Some("mongodb".to_owned()),
            port: Some(27018),
            ..shared::database::HostMongodb::default()
        };
        runtimes.host_sqlite = shared::database::HostSqlite {
            enabled: true,
            os_user: Some("app".to_owned()),
            files: vec!["/srv/app/app.db".to_owned()],
            bin_dir: Some("/opt/sqlite/bin".to_owned()),
        };

        let sources = host_sources(&runtimes);
        let names: Vec<String> = sources.iter().map(|s| s.id.to_string()).collect();
        assert_eq!(
            names,
            [
                "host:postgresql",
                "host:mariadb",
                "host:mongodb",
                "host:sqlite"
            ]
        );
        let mariadb = sources.get(1).unwrap();
        assert_eq!(mariadb.engine, DatabaseEngine::Mariadb);
        assert_eq!(mariadb.user, "root");
        assert_eq!(mariadb.target.place(), "/run/mysqld/mysqld.sock");
        assert_eq!(
            mariadb.target.connection_args(),
            ["--socket=/run/mysqld/mysqld.sock"]
        );
        assert!(
            matches!(&mariadb.target, DumpTarget::Host(host)
                if host.program("mariadb-dump") == Path::new("/opt/mariadb/bin/mariadb-dump")),
            "a host source runs the programs of its bin dir on the host"
        );
        let mongodb = sources.get(2).unwrap();
        assert_eq!(mongodb.target.detail(), "mongodb");
        assert_eq!(mongodb.target.connection_args(), ["--port=27018"]);
        let sqlite = sources.get(3).unwrap();
        assert_eq!(sqlite.engine, DatabaseEngine::Sqlite);
        assert_eq!(sqlite.target.detail(), "app");
        assert_eq!(sqlite.target.place(), "");
        assert_eq!(sqlite.target.connection_args().len(), 0);
        assert_eq!(
            sqlite.databases.as_deref(),
            Some(["/srv/app/app.db".to_owned()].as_slice())
        );
    }

    #[test]
    fn a_host_server_is_reached_the_way_its_settings_say() {
        let reached = |server| HostTarget { server, ..target() }.connection_args();
        assert_eq!(
            reached(HostServer::Mysql {
                socket: None,
                port: Some(3307)
            }),
            ["--host=127.0.0.1", "--port=3307"]
        );
        assert_eq!(
            reached(HostServer::Mysql {
                socket: None,
                port: None
            })
            .len(),
            0
        );
        assert_eq!(
            reached(HostServer::Mongodb {
                port: Some(27018),
                config_file: Some("/etc/assimilate/mongodump.yaml".to_owned()),
            }),
            ["--config=/etc/assimilate/mongodump.yaml"],
            "the config file's uri names the server"
        );
        let configured = HostTarget {
            server: HostServer::Mongodb {
                port: None,
                config_file: Some("/etc/assimilate/mongodump.yaml".to_owned()),
            },
            ..target()
        };
        assert_eq!(configured.place(), Some("/etc/assimilate/mongodump.yaml"));
    }
}
