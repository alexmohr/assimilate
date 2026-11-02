// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The `Redis` and `Valkey` commands an agent runs in a database's container
//! or pod. The two speak the same protocol and ship the same tools under
//! different names, so one set of commands serves both.
//!
//! A dump is the RDB file `--rdb -` makes the client download from the
//! server and write to stdout, so it is streamed like any other dump and
//! costs the server a fork, not a lock.
//!
//! An RDB cannot be loaded into a running server through its protocol, so a
//! restore starts a throwaway server of the container's own image on a
//! loopback port, loads the dump into it, and makes the real server its
//! replica until the full sync has replaced its dataset. It then promotes
//! itself back to a master, so nothing restarts and the append-only file, if
//! any, is rewritten from the restored data.

use super::credentials;

/// Which of the two servers a container runs, which decides the program
/// names and the variables the password is read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Redis,
    Valkey,
}

impl Flavor {
    const fn cli(self) -> &'static str {
        match self {
            Self::Redis => "redis-cli",
            Self::Valkey => "valkey-cli",
        }
    }

    const fn server(self) -> &'static str {
        match self {
            Self::Redis => "redis-server",
            Self::Valkey => "valkey-server",
        }
    }

    /// Variables the images and charts read the password from, newest name
    /// first. Neither official image sets one, but the Bitnami charts do.
    const fn password_vars(self) -> &'static [&'static str] {
        match self {
            Self::Redis => &["REDIS_PASSWORD"],
            Self::Valkey => &["VALKEY_PASSWORD", "REDIS_PASSWORD"],
        }
    }
}

/// Starts the client signed in as the account, when one is named, and with
/// the password from the environment, when it holds one. The password goes
/// in the client's own environment variable, so it is on no command line.
///
/// Arguments: the account (empty for the default one), the password
/// variables, the client, then its own arguments.
#[cfg(unix)]
const CLI_SCRIPT: &str = r#"user=$1 program=$3
read_var "$2"; found=$?
[ "$found" -eq 2 ] && exit 1
shift 3
[ -z "$user" ] || set -- --user "$user" "$@"
[ "$found" -eq 0 ] || exec "$program" "$@"
REDISCLI_AUTH=$value exec "$program" "$@"
"#;

/// Replaces the dataset of the running server with the RDB on stdin.
///
/// Arguments: the account, the password variables, the client, the server.
const RESTORE_SCRIPT: &str = r#"user=$1 program=$3 server=$4
read_var "$2"; found=$?
[ "$found" -eq 2 ] && exit 1
cli() {
  [ -z "$user" ] || set -- --user "$user" "$@"
  if [ "$found" -eq 0 ]; then REDISCLI_AUTH=$value "$program" "$@"; else "$program" "$@"; fi
}
role=$(cli --raw ROLE | head -n 1) || exit 1
if [ "$role" != master ]; then
  echo "assimilate: the server is a $role, which its master would overwrite again" >&2
  exit 1
fi
dir=$(mktemp -d) || exit 1
pid= replicating=0
cleanup() {
  [ "$replicating" -eq 0 ] || cli REPLICAOF NO ONE >/dev/null 2>&1
  [ -z "$pid" ] || kill "$pid" 2>/dev/null
  rm -rf -- "$dir"
}
trap cleanup EXIT
cat > "$dir/dump.rdb" || exit 1
if [ ! -s "$dir/dump.rdb" ]; then
  echo "assimilate: the dump is empty" >&2
  exit 1
fi
port=$((20000 + $$ % 20000))
"$server" --port "$port" --bind 127.0.0.1 --dir "$dir" --dbfilename dump.rdb --save "" \
  --appendonly no >"$dir/server.log" 2>&1 &
pid=$!
tries=0
until [ "$("$program" -h 127.0.0.1 -p "$port" PING 2>/dev/null)" = PONG ]; do
  tries=$((tries + 1))
  if [ "$tries" -gt 3600 ] || ! kill -0 "$pid" 2>/dev/null; then
    echo "assimilate: could not start a server to load the dump:" >&2
    tail -n 5 "$dir/server.log" >&2
    exit 1
  fi
  sleep 1
done
replicating=1
cli REPLICAOF 127.0.0.1 "$port" >/dev/null || exit 1
tries=0
while :; do
  info=$(cli --raw INFO replication) || info=
  case $info in *master_link_status:up*) break ;; esac
  tries=$((tries + 1))
  if [ "$tries" -gt 3600 ]; then
    echo "assimilate: the server did not finish loading the dump" >&2
    exit 1
  fi
  sleep 1
done
cli REPLICAOF NO ONE >/dev/null || exit 1
replicating=0
"#;

/// The commands for one `Redis` or `Valkey` container.
#[derive(Debug, Clone, Copy)]
pub struct Redis<'a> {
    pub flavor: Flavor,
    /// The account to sign in as, empty for the default one.
    pub user: &'a str,
    /// The variable in the container's environment holding the password,
    /// `None` for the ones the flavor's chart is set up with.
    pub password_env: Option<&'a str>,
}

impl Redis<'_> {
    fn arguments(&self, script: &str, rest: &[&str]) -> Vec<String> {
        let mut arguments = vec![
            self.user.to_owned(),
            credentials::password_vars(self.password_env, self.flavor.password_vars()),
            self.flavor.cli().to_owned(),
        ];
        arguments.extend(rest.iter().copied().map(str::to_owned));
        super::shell_command("sh", credentials::script(script), arguments)
    }

    /// Writes the whole dataset to stdout as an RDB file.
    #[cfg(unix)]
    pub fn dump(&self) -> Vec<String> {
        self.arguments(CLI_SCRIPT, &["--rdb", "-"])
    }

    /// Replaces the dataset with the RDB read from stdin.
    pub fn restore(&self) -> Vec<String> {
        self.arguments(RESTORE_SCRIPT, &[self.flavor.server()])
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    //! The scripts run in a real `sh` here, with stand-ins for the client
    //! and the server that print what they were given.

    use std::path::Path;

    use super::*;
    use crate::database::script_tests::{install, run};

    /// A stand-in client: prints its arguments and whether the password
    /// reached it, and answers like a master that syncs at once.
    const CLIENT: &str = r#"#!/bin/sh
echo "cli:$*:auth=${REDISCLI_AUTH-unset}" >> "$ASSIMILATE_LOG"
case "$*" in
*ROLE*) echo "${ASSIMILATE_ROLE-master}" ;;
*PING*) echo PONG ;;
*"INFO replication"*) echo "master_link_status:up" ;;
*--rdb*) printf 'REDIS0011 dump' ;;
esac
"#;

    /// A stand-in server that records how it was started and stays up.
    const SERVER: &str = r#"#!/bin/sh
echo "server:$*" >> "$ASSIMILATE_LOG"
sleep 30
"#;

    /// The command with the stand-in programs in place of the flavor's.
    fn with_programs(mut command: Vec<String>, cli: &str, server: Option<&str>) -> Vec<String> {
        let cli_at = 6;
        *command.get_mut(cli_at).unwrap() = cli.to_owned();
        if let Some(server) = server {
            *command.get_mut(cli_at.saturating_add(1)).unwrap() = server.to_owned();
        }
        command
    }

    fn redis<'a>(user: &'a str, password_env: Option<&'a str>) -> Redis<'a> {
        Redis {
            flavor: Flavor::Redis,
            user,
            password_env,
        }
    }

    #[test]
    fn each_flavor_runs_its_own_programs() {
        let redis = redis("", None).dump();
        assert_eq!(redis.get(5).map(String::as_str), Some("REDIS_PASSWORD"));
        assert_eq!(redis.get(6).map(String::as_str), Some("redis-cli"));
        assert_eq!(redis.get(7..).unwrap(), ["--rdb", "-"]);

        let valkey = Redis {
            flavor: Flavor::Valkey,
            user: "backup",
            password_env: None,
        };
        let dump = valkey.dump();
        assert_eq!(dump.get(4).map(String::as_str), Some("backup"));
        assert_eq!(
            dump.get(5).map(String::as_str),
            Some("VALKEY_PASSWORD REDIS_PASSWORD")
        );
        assert_eq!(dump.get(6).map(String::as_str), Some("valkey-cli"));
        assert_eq!(
            valkey.restore().last().map(String::as_str),
            Some("valkey-server")
        );
    }

    #[tokio::test]
    async fn a_dump_signs_in_with_the_password_from_the_environment() {
        let dir = tempfile::tempdir().unwrap();
        let cli = install(dir.path(), "cli", CLIENT).await;
        let command = with_programs(redis("backup", None).dump(), &cli, None);

        let run = run(dir.path(), &command, &[("REDIS_PASSWORD", "s3cret")], b"").await;

        assert!(run.status.success(), "{}", run.stderr);
        assert_eq!(run.stdout, "REDIS0011 dump");
        assert_eq!(run.log, "cli:--user backup --rdb -:auth=s3cret\n");
    }

    #[tokio::test]
    async fn a_dump_without_a_password_signs_in_as_nobody() {
        let dir = tempfile::tempdir().unwrap();
        let cli = install(dir.path(), "cli", CLIENT).await;
        let command = with_programs(redis("", None).dump(), &cli, None);

        let run = run(dir.path(), &command, &[], b"").await;

        assert!(run.status.success(), "{}", run.stderr);
        assert_eq!(run.log, "cli:--rdb -:auth=unset\n");
    }

    #[tokio::test]
    async fn a_named_variable_replaces_the_charts_own() {
        let dir = tempfile::tempdir().unwrap();
        let cli = install(dir.path(), "cli", CLIENT).await;
        let command = with_programs(redis("", Some("APP_REDIS_PW")).dump(), &cli, None);

        let run = run(
            dir.path(),
            &command,
            &[("APP_REDIS_PW", "app"), ("REDIS_PASSWORD", "chart")],
            b"",
        )
        .await;

        assert_eq!(run.log, "cli:--rdb -:auth=app\n");
    }

    #[tokio::test]
    async fn a_password_file_that_cannot_be_read_fails_the_dump() {
        let dir = tempfile::tempdir().unwrap();
        let cli = install(dir.path(), "cli", CLIENT).await;
        let command = with_programs(redis("", None).dump(), &cli, None);

        let run = run(
            dir.path(),
            &command,
            &[("REDIS_PASSWORD_FILE", "/nonexistent/secret")],
            b"",
        )
        .await;

        assert!(!run.status.success());
        assert_eq!(run.log, "");
    }

    #[tokio::test]
    async fn a_restore_replicates_from_a_throwaway_server_and_promotes_back() {
        let dir = tempfile::tempdir().unwrap();
        let cli = install(dir.path(), "cli", CLIENT).await;
        let server = install(dir.path(), "server", SERVER).await;
        let command = with_programs(redis("", None).restore(), &cli, Some(&server));

        let run = run(
            dir.path(),
            &command,
            &[("REDIS_PASSWORD", "s3cret")],
            b"REDIS0011 dump",
        )
        .await;

        assert!(run.status.success(), "{}", run.stderr);
        let log: Vec<&str> = run.log.lines().collect();
        let position = |needle: &str| log.iter().position(|line| line.contains(needle)).unwrap();
        let started = log.get(position("server:")).unwrap();
        assert!(started.contains("--bind 127.0.0.1 --dir "));
        assert!(started.contains("--appendonly no"));
        assert!(position("ROLE") < position("server:"));
        assert!(position("server:") < position("REPLICAOF 127.0.0.1"));
        assert!(position("REPLICAOF 127.0.0.1") < position("INFO replication"));
        assert!(position("INFO replication") < position("REPLICAOF NO ONE"));
        assert!(
            log.iter()
                .filter(|line| line.starts_with("cli:") && !line.contains("PING"))
                .all(|line| line.ends_with("auth=s3cret")),
            "{log:?}"
        );
        assert!(
            log.iter()
                .filter(|line| line.contains("PING"))
                .all(|line| line.ends_with("auth=unset")),
            "the throwaway server must not be sent the password: {log:?}"
        );
    }

    #[tokio::test]
    async fn a_restore_into_a_replica_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let cli = install(dir.path(), "cli", CLIENT).await;
        let server = install(dir.path(), "server", SERVER).await;
        let command = with_programs(redis("", None).restore(), &cli, Some(&server));

        let run = run(dir.path(), &command, &[("ASSIMILATE_ROLE", "slave")], b"x").await;

        assert!(!run.status.success());
        assert!(
            run.stderr.contains("the server is a slave"),
            "{}",
            run.stderr
        );
        assert!(!run.log.contains("server:") && !run.log.contains("REPLICAOF"));
    }

    #[tokio::test]
    async fn an_empty_dump_is_refused_before_anything_starts() {
        let dir = tempfile::tempdir().unwrap();
        let cli = install(dir.path(), "cli", CLIENT).await;
        let server = install(dir.path(), "server", SERVER).await;
        let command = with_programs(redis("", None).restore(), &cli, Some(&server));

        let run = run(dir.path(), &command, &[], b"").await;

        assert!(!run.status.success());
        assert!(run.stderr.contains("the dump is empty"), "{}", run.stderr);
        assert!(!run.log.contains("server:"));
    }

    mod against_redis {
        //! Runs the scripts against a real `redis-server`, which CI installs.

        use super::*;

        /// A `redis-cli` that reaches the server under test, ahead of the real
        /// one on the `PATH`. Its own `-p` comes first, so the scripts' own
        /// `-p` for the throwaway server wins.
        fn shim(port: u16) -> String {
            format!("#!/bin/sh\nexec /usr/bin/redis-cli -p {port} \"$@\"\n")
        }

        async fn free_port() -> u16 {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().port()
        }

        async fn cli(port: u16, args: &[&str]) -> String {
            let output = tokio::process::Command::new("/usr/bin/redis-cli")
                .args(["-p", &port.to_string()])
                .args(args)
                .output()
                .await
                .unwrap();
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }

        /// A real server that stays up for the test.
        async fn server(dir: &Path) -> (tokio::process::Child, u16) {
            let port = free_port().await;
            let child = tokio::process::Command::new("redis-server")
                .args([
                    "--port",
                    &port.to_string(),
                    "--bind",
                    "127.0.0.1",
                    "--save",
                    "",
                ])
                .args(["--appendonly", "no", "--dir"])
                .arg(dir)
                .kill_on_drop(true)
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap();
            for _ in 0..100 {
                if cli(port, &["PING"]).await == "PONG" {
                    return (child, port);
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            panic!("redis-server did not come up");
        }

        fn path_with(shim_dir: &Path) -> String {
            format!(
                "{}:{}",
                shim_dir.display(),
                std::env::var("PATH").unwrap_or_default()
            )
        }

        #[tokio::test]
        async fn a_dump_restores_over_a_running_server_without_restarting_it() {
            let dir = tempfile::tempdir().unwrap();
            let (server, port) = server(dir.path()).await;
            install(dir.path(), "redis-cli", &shim(port)).await;
            let path = path_with(dir.path());
            cli(port, &["SET", "kept", "yes"]).await;
            cli(port, &["RPUSH", "queue", "a", "b"]).await;

            let dump = run(
                dir.path(),
                &redis("", None).dump(),
                &[("PATH", path.as_str())],
                b"",
            )
            .await;
            assert!(dump.status.success(), "{}", dump.stderr);
            assert!(dump.bytes.starts_with(b"REDIS"), "{:?}", dump.stdout);

            cli(port, &["FLUSHALL"]).await;
            cli(port, &["SET", "added-later", "no"]).await;
            let restore = run(
                dir.path(),
                &redis("", None).restore(),
                &[("PATH", path.as_str())],
                &dump.bytes,
            )
            .await;

            assert!(restore.status.success(), "{}", restore.stderr);
            assert_eq!(cli(port, &["GET", "kept"]).await, "yes");
            assert_eq!(cli(port, &["LLEN", "queue"]).await, "2");
            assert_eq!(cli(port, &["EXISTS", "added-later"]).await, "0");
            assert!(
                cli(port, &["ROLE"]).await.starts_with("master"),
                "the server is promoted back"
            );
            drop(server);
        }
    }
}
