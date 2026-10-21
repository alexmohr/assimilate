// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! How a `MySQL`, `MariaDB` or `MongoDB` client inside the database's own
//! container signs in.
//!
//! Unlike `PostgreSQL`, whose local socket lets the `postgres` role in
//! without a password, these servers want one. The container already holds
//! it in its environment: the official images and the Bitnami charts are set
//! up with it there, or with a `_FILE` variable naming a file that holds it.
//! So the client is started through a short `sh` script that reads it from
//! that environment and hands it to the client through a file descriptor.
//! Assimilate never holds the password, and it never appears on a command
//! line, where every process in the container could read it.

/// Variables the `MySQL` image reads its root password from.
const MYSQL_PASSWORD_VARS: &[&str] = &["MYSQL_ROOT_PASSWORD"];

/// Variables the `MariaDB` image reads its root password from, newest name
/// first. It still accepts the `MySQL` image's names.
const MARIADB_PASSWORD_VARS: &[&str] = &["MARIADB_ROOT_PASSWORD", "MYSQL_ROOT_PASSWORD"];

/// Variables the official `MongoDB` image and the Bitnami chart read the
/// root account's name from.
const MONGODB_USER_VARS: &[&str] = &["MONGO_INITDB_ROOT_USERNAME", "MONGODB_ROOT_USER"];

/// Variables they read the root account's password from.
const MONGODB_PASSWORD_VARS: &[&str] = &["MONGO_INITDB_ROOT_PASSWORD", "MONGODB_ROOT_PASSWORD"];

/// Shell functions every script starts with.
///
/// `set -f` keeps a variable name from being expanded as a file pattern.
/// `read_var` sets `value` from the first of the space-separated variables
/// in `$1` that is set, or from the file its `_FILE` twin names. It returns
/// 1 when none is set and 2 when the file cannot be read. `single_line`
/// refuses a value with a line break, which would end the line it is
/// written on and start an option of its own.
const PRELUDE: &str = r#"set -f
read_var() {
  for name in $1; do
    if value=$(printenv "$name"); then return 0; fi
    if path=$(printenv "${name}_FILE"); then value=$(cat -- "$path") && return 0; return 2; fi
  done
  return 1
}
single_line() {
  case $1 in *"
"*)
    echo "assimilate: the $2 holds a line break, which the client cannot be given" >&2
    exit 1 ;;
  esac
}
"#;

/// Starts a `MySQL` or `MariaDB` client with an options file on descriptor
/// 3 naming the account and, when the environment holds one, its password.
///
/// Arguments: the account, the password variables, the client, then its own
/// arguments. The options file must be the client's first argument.
const MYSQL_SCRIPT: &str = r#"user=$1 program=$3
read_var "$2"; found=$?
[ "$found" -eq 2 ] && exit 1
shift 3
single_line "$user" "account name"
quote() { printf '%s' "$1" | sed 's/[\\"]/\\&/g'; }
options="[client]
user=\"$(quote "$user")\""
if [ "$found" -eq 0 ]; then
  single_line "$value" password
  options="$options
password=\"$(quote "$value")\""
fi
exec 3<<END
$options
END
exec "$program" --defaults-extra-file=/dev/fd/3 "$@"
"#;

/// Starts a `MongoDB` program signed in as the account the environment
/// names, or unauthenticated when the environment holds no password.
///
/// Arguments: the account (empty to read it from the environment), the
/// account variables, the password variables, the [`MongoProgram`] kind,
/// the program, then its own arguments. The database tools get the password
/// in a config file on descriptor 3; `mongosh`, which reads none, gets the
/// account in its environment for the script it runs.
const MONGODB_SCRIPT: &str = r#"user=$1 kind=$4 program=$5
if [ -z "$user" ]; then
  read_var "$2"; case $? in 0) user=$value ;; 2) exit 1 ;; esac
fi
read_var "$3"; found=$?
[ "$found" -eq 2 ] && exit 1
shift 5
[ "$found" -eq 0 ] || exec "$program" "$@"
[ -n "$user" ] || user=root
single_line "$user" "account name"
single_line "$value" password
case $kind in
shell)
  ASSIMILATE_MONGO_USER=$user ASSIMILATE_MONGO_PASSWORD=$value exec "$program" "$@" ;;
tool)
  quoted=$(printf '%s' "$value" | sed "s/'/''/g")
  exec 3<<END
password: '$quoted'
END
  exec "$program" --config=/dev/fd/3 "--username=$user" --authenticationDatabase=admin "$@" ;;
esac
"#;

/// Which of the two ways a `MongoDB` program takes a password.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum_macros::Display)]
#[strum(serialize_all = "lowercase")]
pub enum MongoProgram {
    /// `mongosh`, whose script signs in with the account from its
    /// environment.
    Shell,
    /// `mongodump` or `mongorestore`, which read a config file.
    Tool,
}

/// The variable `password_env` names, or the engine's own when it names
/// none.
fn password_vars(password_env: Option<&str>, defaults: &[&str]) -> String {
    password_env.map_or_else(|| defaults.join(" "), str::to_owned)
}

fn script(body: &str) -> String {
    format!("{PRELUDE}{body}")
}

/// Which `MySQL`-compatible server a client talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MysqlFlavor {
    Mysql,
    Mariadb,
}

impl MysqlFlavor {
    const fn password_vars(self) -> &'static [&'static str] {
        match self {
            Self::Mysql => MYSQL_PASSWORD_VARS,
            Self::Mariadb => MARIADB_PASSWORD_VARS,
        }
    }
}

/// `program` with `args`, signed in as `user` with the password from the
/// container's environment.
pub fn mysql(
    flavor: MysqlFlavor,
    user: &str,
    password_env: Option<&str>,
    program: &str,
    args: Vec<String>,
) -> Vec<String> {
    let mut command = vec![
        "sh".to_owned(),
        "-c".to_owned(),
        script(MYSQL_SCRIPT),
        "sh".to_owned(),
        user.to_owned(),
        password_vars(password_env, flavor.password_vars()),
        program.to_owned(),
    ];
    command.extend(args);
    command
}

/// `program` with `args`, signed in as `user`, or the account the
/// container's environment names when `user` is empty.
pub fn mongodb(
    kind: MongoProgram,
    user: &str,
    password_env: Option<&str>,
    program: &str,
    args: Vec<String>,
) -> Vec<String> {
    let mut command = vec![
        "sh".to_owned(),
        "-c".to_owned(),
        script(MONGODB_SCRIPT),
        "sh".to_owned(),
        user.to_owned(),
        MONGODB_USER_VARS.join(" "),
        password_vars(password_env, MONGODB_PASSWORD_VARS),
        kind.to_string(),
        program.to_owned(),
    ];
    command.extend(args);
    command
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    //! The scripts run in a real `sh` here, with stand-in clients that print
    //! what they were given.

    use std::{os::unix::fs::PermissionsExt as _, path::Path};

    use super::*;

    /// A stand-in client: prints its arguments, one per line, then the
    /// file on descriptor 3 if there is one, then the account variables a
    /// `mongosh` script would read.
    const CLIENT: &str = r#"#!/bin/sh
for arg in "$@"; do echo "arg:$arg"; done
[ -e /dev/fd/3 ] && sed 's/^/fd3:/' /dev/fd/3
echo "env:${ASSIMILATE_MONGO_USER-unset}:${ASSIMILATE_MONGO_PASSWORD-unset}"
"#;

    struct Run {
        status: std::process::ExitStatus,
        stdout: String,
        stderr: String,
    }

    async fn install(dir: &Path) -> String {
        let client = dir.join("client");
        tokio::fs::write(&client, CLIENT).await.unwrap();
        tokio::fs::set_permissions(&client, std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();
        client.to_string_lossy().into_owned()
    }

    async fn run(command: &[String], env: &[(&str, &str)]) -> Run {
        let (program, args) = command.split_first().unwrap();
        let output = tokio::process::Command::new(program)
            .args(args)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .envs(env.iter().copied())
            .output()
            .await
            .unwrap();
        Run {
            status: output.status,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    fn mysql_with(client: &str, password_env: Option<&str>) -> Vec<String> {
        mysql(
            MysqlFlavor::Mariadb,
            "root",
            password_env,
            client,
            vec!["--".to_owned(), "shop".to_owned()],
        )
    }

    #[tokio::test]
    async fn a_mysql_client_gets_the_password_in_an_options_file_first() {
        let dir = tempfile::tempdir().unwrap();
        let client = install(dir.path()).await;

        let run = run(
            &mysql_with(&client, None),
            &[("MYSQL_ROOT_PASSWORD", r#"p"a\s$x'y * #c ;d"#)],
        )
        .await;

        assert!(run.status.success(), "{}", run.stderr);
        assert_eq!(
            run.stdout.lines().collect::<Vec<_>>(),
            [
                "arg:--defaults-extra-file=/dev/fd/3",
                "arg:--",
                "arg:shop",
                "fd3:[client]",
                r#"fd3:user="root""#,
                r#"fd3:password="p\"a\\s$x'y * #c ;d""#,
                "env:unset:unset",
            ]
        );
    }

    #[tokio::test]
    async fn the_newest_variable_wins_and_a_file_is_read_too() {
        let dir = tempfile::tempdir().unwrap();
        let client = install(dir.path()).await;
        let secret = dir.path().join("secret");
        tokio::fs::write(&secret, "from-file\n").await.unwrap();
        let secret = secret.to_string_lossy().into_owned();

        let newest = run(
            &mysql_with(&client, None),
            &[
                ("MARIADB_ROOT_PASSWORD", "new"),
                ("MYSQL_ROOT_PASSWORD", "old"),
            ],
        )
        .await;
        assert!(
            newest.stdout.contains("fd3:password=\"new\""),
            "{}",
            newest.stdout
        );

        let from_file = run(
            &mysql_with(&client, None),
            &[("MARIADB_ROOT_PASSWORD_FILE", secret.as_str())],
        )
        .await;
        assert!(
            from_file.stdout.contains("fd3:password=\"from-file\""),
            "{}",
            from_file.stdout
        );
    }

    #[tokio::test]
    async fn a_named_variable_replaces_the_images_own() {
        let dir = tempfile::tempdir().unwrap();
        let client = install(dir.path()).await;

        let run = run(
            &mysql_with(&client, Some("APP_DB_PASSWORD")),
            &[
                ("APP_DB_PASSWORD", "app"),
                ("MARIADB_ROOT_PASSWORD", "root"),
            ],
        )
        .await;

        assert!(
            run.stdout.contains("fd3:password=\"app\""),
            "{}",
            run.stdout
        );
    }

    #[tokio::test]
    async fn without_a_password_the_client_signs_in_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let client = install(dir.path()).await;

        let run = run(&mysql_with(&client, Some("*")), &[]).await;

        assert!(run.status.success(), "{}", run.stderr);
        assert!(run.stdout.contains("fd3:user=\"root\"\n"), "{}", run.stdout);
        assert!(!run.stdout.contains("password"), "{}", run.stdout);
    }

    #[tokio::test]
    async fn an_unreadable_password_file_or_a_line_break_stops_the_client() {
        let dir = tempfile::tempdir().unwrap();
        let client = install(dir.path()).await;

        let unreadable = run(
            &mysql_with(&client, None),
            &[("MYSQL_ROOT_PASSWORD_FILE", "/nonexistent/secret")],
        )
        .await;
        assert!(!unreadable.status.success());
        assert_eq!(unreadable.stdout, "");
        assert!(
            unreadable.stderr.contains("/nonexistent/secret"),
            "{}",
            unreadable.stderr
        );

        let broken = run(
            &mysql_with(&client, None),
            &[("MYSQL_ROOT_PASSWORD", "a\nb")],
        )
        .await;
        assert!(!broken.status.success());
        assert_eq!(broken.stdout, "");
        assert!(
            broken.stderr.contains("the password holds a line break"),
            "{}",
            broken.stderr
        );

        let account = mysql(
            MysqlFlavor::Mysql,
            "root\nplugin-dir=/tmp",
            None,
            &client,
            Vec::new(),
        );
        let account = run(&account, &[]).await;
        assert!(!account.status.success());
        assert!(
            account.stderr.contains("account name holds a line break"),
            "{}",
            account.stderr
        );
    }

    fn mongodb_with(client: &str, kind: MongoProgram, user: &str) -> Vec<String> {
        mongodb(kind, user, None, client, vec!["--archive".to_owned()])
    }

    #[tokio::test]
    async fn a_mongodb_tool_signs_in_with_the_images_root_account() {
        let dir = tempfile::tempdir().unwrap();
        let client = install(dir.path()).await;

        let run = run(
            &mongodb_with(&client, MongoProgram::Tool, ""),
            &[
                ("MONGO_INITDB_ROOT_USERNAME", "admin"),
                ("MONGO_INITDB_ROOT_PASSWORD", "it's \"x\""),
            ],
        )
        .await;

        assert!(run.status.success(), "{}", run.stderr);
        assert_eq!(
            run.stdout.lines().collect::<Vec<_>>(),
            [
                "arg:--config=/dev/fd/3",
                "arg:--username=admin",
                "arg:--authenticationDatabase=admin",
                "arg:--archive",
                r#"fd3:password: 'it''s "x"'"#,
                "env:unset:unset",
            ]
        );
    }

    #[tokio::test]
    async fn the_mongodb_shell_gets_the_account_in_its_environment() {
        let dir = tempfile::tempdir().unwrap();
        let client = install(dir.path()).await;

        let bitnami = run(
            &mongodb_with(&client, MongoProgram::Shell, ""),
            &[("MONGODB_ROOT_PASSWORD", "pw")],
        )
        .await;
        assert_eq!(bitnami.stdout, "arg:--archive\nenv:root:pw\n");

        let named = run(
            &mongodb_with(&client, MongoProgram::Shell, "backup"),
            &[
                ("MONGO_INITDB_ROOT_USERNAME", "admin"),
                ("MONGO_INITDB_ROOT_PASSWORD", "pw"),
            ],
        )
        .await;
        assert_eq!(named.stdout, "arg:--archive\nenv:backup:pw\n");
    }

    #[tokio::test]
    async fn a_mongodb_without_a_password_is_reached_without_signing_in() {
        let dir = tempfile::tempdir().unwrap();
        let client = install(dir.path()).await;

        let run = run(
            &mongodb_with(&client, MongoProgram::Tool, ""),
            &[("MONGO_INITDB_ROOT_USERNAME", "admin")],
        )
        .await;

        assert!(run.status.success(), "{}", run.stderr);
        assert_eq!(run.stdout, "arg:--archive\nenv:unset:unset\n");
    }

    #[tokio::test]
    async fn an_unreadable_mongodb_secret_stops_the_program() {
        let dir = tempfile::tempdir().unwrap();
        let client = install(dir.path()).await;

        for env in [
            [("MONGO_INITDB_ROOT_USERNAME_FILE", "/nonexistent/user")],
            [("MONGO_INITDB_ROOT_PASSWORD_FILE", "/nonexistent/password")],
            [("MONGO_INITDB_ROOT_PASSWORD", "a\nb")],
        ] {
            let run = run(&mongodb_with(&client, MongoProgram::Tool, ""), &env).await;
            assert!(!run.status.success(), "{env:?}");
            assert_eq!(run.stdout, "", "{env:?}");
        }
    }

    #[tokio::test]
    async fn a_line_break_in_the_mongodb_account_stops_the_program() {
        let dir = tempfile::tempdir().unwrap();
        let client = install(dir.path()).await;
        let password = ("MONGO_INITDB_ROOT_PASSWORD", "secret");

        for kind in [MongoProgram::Tool, MongoProgram::Shell] {
            let given = run(&mongodb_with(&client, kind, "admin\nx"), &[password]).await;
            let from_env = run(
                &mongodb_with(&client, kind, ""),
                &[("MONGO_INITDB_ROOT_USERNAME", "admin\nx"), password],
            )
            .await;
            for broken in [given, from_env] {
                assert!(!broken.status.success(), "{kind}");
                assert_eq!(broken.stdout, "", "{kind}");
                assert!(
                    broken
                        .stderr
                        .contains("the account name holds a line break"),
                    "{}",
                    broken.stderr
                );
            }
        }
    }

    #[test]
    fn the_password_never_reaches_the_command_line() {
        let command = mysql(MysqlFlavor::Mysql, "root", None, "mysqldump", Vec::new());
        assert_eq!(command.get(..2).unwrap(), ["sh", "-c"]);
        assert_eq!(
            command.get(3..).unwrap(),
            ["sh", "root", "MYSQL_ROOT_PASSWORD", "mysqldump"]
        );
        let command = mongodb(MongoProgram::Shell, "", Some("PW"), "mongosh", Vec::new());
        assert_eq!(
            command.get(3..).unwrap(),
            [
                "sh",
                "",
                "MONGO_INITDB_ROOT_USERNAME MONGODB_ROOT_USER",
                "PW",
                "shell",
                "mongosh"
            ]
        );
    }
}
