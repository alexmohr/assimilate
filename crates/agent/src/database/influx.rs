// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The `InfluxDB` commands an agent runs in a database's container or pod.
//!
//! Both generations back up into a directory rather than to stdout, so a
//! dump runs the backup into a scratch directory of the container and
//! streams it out as a tar, which is removed again whatever happens. A
//! restore does the reverse: it unpacks the tar from stdin into a scratch
//! directory and restores from there. The scratch space is the size of the
//! backup, which is the one place a dump of these engines touches disk.
//!
//! `InfluxDB` 2 backs up the whole instance and restores it with `--full`,
//! which replaces everything including users and tokens. `InfluxDB` 1 is
//! dumped one database at a time, so a database can be restored next to the
//! original under a new name.

use shared::database::{DatabaseRestore, DatabaseRestoreTarget};

use super::credentials;

/// Variables the `InfluxDB` 2 image and its chart hold the operator token in.
/// The `influx` client reads `INFLUX_TOKEN` itself.
const V2_TOKEN_VARS: &[&str] = &[
    "DOCKER_INFLUXDB_INIT_ADMIN_TOKEN",
    "INFLUXDB_TOKEN",
    "INFLUX_TOKEN",
];

/// Variables the `InfluxDB` 1 image holds the admin password in. The
/// `influx` client reads `INFLUX_PASSWORD` itself.
const V1_PASSWORD_VARS: &[&str] = &["INFLUXDB_ADMIN_PASSWORD", "INFLUX_PASSWORD"];

/// Reads the token and hands it to `influx` in its environment, so it is on
/// no command line. A server without one is reached unauthenticated.
///
/// Arguments: the token variables, then the `influx` client.
const V2_PRELUDE: &str = r#"read_var "$1"; found=$?
[ "$found" -eq 2 ] && exit 1
[ "$found" -ne 0 ] || export INFLUX_TOKEN=$value
program=$2
dir=$(mktemp -d) || exit 1
trap 'rm -rf -- "$dir"' EXIT
"#;

/// Backs the instance up into a scratch directory and writes it as a tar.
/// The client's own output goes to stderr, which keeps stdout the tar.
#[cfg(unix)]
const V2_DUMP: &str = r#""$program" backup "$dir" >&2 || exit 1
tar -C "$dir" -cf - .
"#;

/// Unpacks the tar on stdin and replaces the whole instance with it.
const V2_RESTORE: &str = r#"tar -C "$dir" -xf - || exit 1
"$program" restore --full "$dir" >&2
"#;

/// Signs `influx` in as the account and password the environment names.
///
/// Arguments: the account (empty to read it from the environment), the
/// password variables, the `influx` client, the `influxd` server program,
/// then the script's own.
const V1_PRELUDE: &str = r#"user=$1 program=$3 influxd=$4
if [ -z "$user" ]; then read_var "INFLUXDB_ADMIN_USER INFLUX_USERNAME" && user=$value; fi
read_var "$2"; found=$?
[ "$found" -eq 2 ] && exit 1
[ "$found" -ne 0 ] || export INFLUX_PASSWORD=$value
[ -z "$user" ] || export INFLUX_USERNAME=$user
shift 4
"#;

/// Prints the databases an application keeps, one name per line: every one
/// but the server's own `_internal`.
const V1_LIST: &str = r#"found=$("$program" -execute 'SHOW DATABASES' -format csv) || exit 1
printf '%s\n' "$found" | sed -n -e '/^databases,/!d' -e 's/^databases,//' -e '/^_internal$/!p'
"#;

/// Backs up the database in `$1` into a scratch directory and writes it as a
/// tar.
#[cfg(unix)]
const V1_DUMP: &str = r#"dir=$(mktemp -d) || exit 1
trap 'rm -rf -- "$dir"' EXIT
"$influxd" backup -portable -database "$1" "$dir" >&2 || exit 1
tar -C "$dir" -cf - .
"#;

/// Fails when the database `$1` exists already, so a restore under that name
/// never merges into it.
const V1_CREATE: &str = r#"found=$("$program" -execute 'SHOW DATABASES' -format csv) || exit 1
if printf '%s\n' "$found" | grep -qx "databases,$1"; then
  echo "assimilate: the database $1 exists already" >&2
  exit 1
fi
"#;

/// Drops the database `$1`, quoted as an `InfluxQL` identifier.
const V1_DROP: &str = r#"quoted=$(printf '%s' "$1" | sed 's/[\\"]/\\&/g')
"$program" -execute "DROP DATABASE \"$quoted\"" >&2
"#;

/// Unpacks the tar on stdin, then restores the database `$1` from it: under
/// the name `$2` when there is one, else over the original, which is dropped
/// first because `InfluxDB` 1 refuses to restore into a database that
/// exists.
const V1_RESTORE: &str = r#"dir=$(mktemp -d) || exit 1
trap 'rm -rf -- "$dir"' EXIT
tar -C "$dir" -xf - || exit 1
if [ -n "$2" ]; then
  "$influxd" restore -portable -db "$1" -newdb "$2" "$dir" >&2
else
  quoted=$(printf '%s' "$1" | sed 's/[\\"]/\\&/g')
  "$program" -execute "DROP DATABASE \"$quoted\"" >&2 || exit 1
  "$influxd" restore -portable -db "$1" "$dir" >&2
fi
"#;

/// The commands for one `InfluxDB` container.
#[derive(Debug, Clone, Copy)]
pub enum Influx<'a> {
    /// `InfluxDB` 2, signed in with a token.
    V2 { password_env: Option<&'a str> },
    /// `InfluxDB` 1, signed in as `user` when it names one, or as the
    /// account the environment names.
    V1 {
        user: &'a str,
        password_env: Option<&'a str>,
    },
}

impl Influx<'_> {
    fn v2(password_env: Option<&str>, tail: &str, args: &[&str]) -> Vec<String> {
        let mut arguments = vec![
            credentials::password_vars(password_env, V2_TOKEN_VARS),
            "influx".to_owned(),
        ];
        arguments.extend(args.iter().copied().map(str::to_owned));
        super::shell_command(
            "sh",
            credentials::script(&format!("{V2_PRELUDE}{tail}")),
            arguments,
        )
    }

    fn v1(user: &str, password_env: Option<&str>, tail: &str, args: &[&str]) -> Vec<String> {
        let mut arguments = vec![
            user.to_owned(),
            credentials::password_vars(password_env, V1_PASSWORD_VARS),
            "influx".to_owned(),
            "influxd".to_owned(),
        ];
        arguments.extend(args.iter().copied().map(str::to_owned));
        super::shell_command(
            "sh",
            credentials::script(&format!("{V1_PRELUDE}{tail}")),
            arguments,
        )
    }

    /// Prints the databases, one name per line. `None` for `InfluxDB` 2,
    /// whose instance is dumped as one.
    pub fn list_databases(&self) -> Option<Vec<String>> {
        match *self {
            Self::V2 { .. } => None,
            Self::V1 { user, password_env } => Some(Self::v1(user, password_env, V1_LIST, &[])),
        }
    }

    /// Writes the dump to stdout as a tar. `database` is the one of an
    /// `InfluxDB` 1 server, and ignored for the instance of an `InfluxDB` 2.
    #[cfg(unix)]
    pub fn dump(&self, database: &str) -> Vec<String> {
        match *self {
            Self::V2 { password_env } => Self::v2(password_env, V2_DUMP, &[]),
            Self::V1 { user, password_env } => Self::v1(user, password_env, V1_DUMP, &[database]),
        }
    }

    /// Makes sure the database `name` a restore writes into does not exist.
    pub fn create_database(&self, name: &str) -> Vec<String> {
        match *self {
            Self::V2 { .. } => {
                super::unsupported_command("an InfluxDB 2 instance is restored as a whole")
            }
            Self::V1 { user, password_env } => Self::v1(user, password_env, V1_CREATE, &[name]),
        }
    }

    /// Drops the database `name` again after a restore into it failed.
    pub fn drop_database(&self, name: &str) -> Vec<String> {
        match *self {
            Self::V2 { .. } => {
                super::unsupported_command("an InfluxDB 2 instance is restored as a whole")
            }
            Self::V1 { user, password_env } => Self::v1(user, password_env, V1_DROP, &[name]),
        }
    }

    /// Restores the tar on stdin into `restore`'s target.
    pub fn restore(&self, restore: &DatabaseRestore) -> Vec<String> {
        match *self {
            Self::V2 { password_env } => Self::v2(password_env, V2_RESTORE, &[]),
            Self::V1 { user, password_env } => {
                let new = match &restore.target {
                    DatabaseRestoreTarget::NewDatabase { name } => name.as_str(),
                    DatabaseRestoreTarget::Original => "",
                };
                Self::v1(user, password_env, V1_RESTORE, &[&restore.database, new])
            }
        }
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    //! The scripts run in a real `sh` here, with stand-ins for `influx`,
    //! `influxd` and `tar`'s input that record what they were given.

    use std::path::Path;

    use shared::database::{DatabaseRuntime, DatabaseSourceId, DumpFormat};

    use super::*;
    use crate::database::script_tests::{install, run};

    /// A stand-in `influx`: logs its arguments and the credentials that
    /// reached it. A backup writes one file, a restore logs what it finds.
    const INFLUX: &str = r#"#!/bin/sh
auth="token=${INFLUX_TOKEN-unset}:user=${INFLUX_USERNAME-unset}"
printf 'influx:%s:%s:pw=%s\n' "$*" "$auth" "${INFLUX_PASSWORD-unset}" >> "$ASSIMILATE_LOG"
case "$1" in
backup) echo "data" > "$2/backup.file" ;;
restore) echo "restoring $(ls "$3")" >> "$ASSIMILATE_LOG" ;;
-execute)
  case "$2" in
  "SHOW DATABASES")
    printf 'name,name\ndatabases,_internal\ndatabases,telegraf\ndatabases,my db\n' ;;
  esac ;;
esac
"#;

    /// A stand-in `influxd`.
    const INFLUXD: &str = r#"#!/bin/sh
printf 'influxd:%s\n' "$*" >> "$ASSIMILATE_LOG"
case "$1" in
backup) for last; do :; done; echo "data" > "$last/backup.file" ;;
restore) for last; do :; done; echo "restoring $(ls "$last")" >> "$ASSIMILATE_LOG" ;;
esac
"#;

    /// Puts the stand-ins where the real programs are named: the `influx`
    /// client, and for `InfluxDB` 1 the `influxd` after it.
    fn standing_in(mut command: Vec<String>, influx: &str, influxd: Option<&str>) -> Vec<String> {
        let at = command.iter().position(|arg| arg == "influx").unwrap();
        *command.get_mut(at).unwrap() = influx.to_owned();
        if let Some(influxd) = influxd {
            *command.get_mut(at.saturating_add(1)).unwrap() = influxd.to_owned();
        }
        command
    }

    fn restoring(database: &str, target: DatabaseRestoreTarget) -> DatabaseRestore {
        DatabaseRestore {
            source: DatabaseSourceId {
                runtime: DatabaseRuntime::Docker,
                namespace: String::new(),
                name: "influx".to_owned(),
            },
            database: database.to_owned(),
            archive_path: "docker/influx/telegraf.influxbackup.tar".to_owned(),
            format: DumpFormat::InfluxBackup,
            target,
        }
    }

    const V2: Influx<'static> = Influx::V2 { password_env: None };
    const V1: Influx<'static> = Influx::V1 {
        user: "",
        password_env: None,
    };

    async fn tar_of_one_file(dir: &Path) -> Vec<u8> {
        let source = dir.join("source");
        tokio::fs::create_dir(&source).await.unwrap();
        tokio::fs::write(source.join("backup.file"), "data")
            .await
            .unwrap();
        tokio::process::Command::new("tar")
            .arg("-C")
            .arg(&source)
            .args(["-cf", "-", "."])
            .output()
            .await
            .unwrap()
            .stdout
    }

    #[tokio::test]
    async fn a_v2_dump_is_a_tar_of_the_backup_with_the_token_in_the_environment() {
        let dir = tempfile::tempdir().unwrap();
        let influx = install(dir.path(), "influx", INFLUX).await;
        let command = standing_in(V2.dump("instance"), &influx, None);

        let run = run(dir.path(), &command, &[("INFLUXDB_TOKEN", "t0ken")], b"").await;

        assert!(run.status.success(), "{}", run.stderr);
        assert!(run.log.starts_with("influx:backup /"), "{}", run.log);
        assert!(run.log.contains(":token=t0ken:"), "{}", run.log);
        let listing = {
            use tokio::io::AsyncWriteExt as _;
            let mut tar = tokio::process::Command::new("tar")
                .args(["-tf", "-"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let mut stdin = tar.stdin.take().unwrap();
            stdin.write_all(&run.bytes).await.unwrap();
            drop(stdin);
            String::from_utf8(tar.wait_with_output().await.unwrap().stdout).unwrap()
        };
        assert!(listing.contains("backup.file"), "{listing}");
    }

    #[tokio::test]
    async fn the_scratch_directory_is_removed_even_when_the_backup_fails() {
        let dir = tempfile::tempdir().unwrap();
        let failing = install(
            dir.path(),
            "influx",
            "#!/bin/sh\nfor last; do :; done\necho \"$last\" > \"$ASSIMILATE_LOG\"\nexit 3\n",
        )
        .await;
        let command = standing_in(V2.dump("instance"), &failing, None);

        let run = run(dir.path(), &command, &[], b"").await;

        assert!(!run.status.success());
        let scratch = run.log.trim();
        assert!(scratch.starts_with('/'), "{scratch}");
        assert!(!tokio::fs::try_exists(scratch).await.unwrap());
    }

    #[tokio::test]
    async fn a_v2_restore_unpacks_the_tar_and_replaces_everything() {
        let dir = tempfile::tempdir().unwrap();
        let influx = install(dir.path(), "influx", INFLUX).await;
        let tar = tar_of_one_file(dir.path()).await;
        let restore = restoring("instance", DatabaseRestoreTarget::Original);
        let command = standing_in(V2.restore(&restore), &influx, None);

        let run = run(dir.path(), &command, &[], &tar).await;

        assert!(run.status.success(), "{}", run.stderr);
        assert!(run.log.contains("influx:restore --full /"), "{}", run.log);
        assert!(run.log.contains("restoring backup.file"), "{}", run.log);
    }

    #[tokio::test]
    async fn a_v1_server_lists_every_database_but_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let influx = install(dir.path(), "influx", INFLUX).await;
        let command = standing_in(V1.list_databases().unwrap(), &influx, Some("influxd"));

        let run = run(
            dir.path(),
            &command,
            &[
                ("INFLUXDB_ADMIN_USER", "admin"),
                ("INFLUXDB_ADMIN_PASSWORD", "pw"),
            ],
            b"",
        )
        .await;

        assert!(run.status.success(), "{}", run.stderr);
        assert_eq!(run.stdout, "telegraf\nmy db\n");
        assert!(run.log.contains(":user=admin:pw=pw"), "{}", run.log);
        assert!(
            !run.stdout.contains("_internal"),
            "the server's own database is not dumped"
        );
    }

    #[tokio::test]
    async fn a_named_account_replaces_the_one_from_the_environment() {
        let dir = tempfile::tempdir().unwrap();
        let influx = install(dir.path(), "influx", INFLUX).await;
        let influx_as = Influx::V1 {
            user: "backup",
            password_env: Some("BACKUP_PW"),
        };
        let command = standing_in(
            influx_as.list_databases().unwrap(),
            &influx,
            Some("influxd"),
        );

        let run = run(
            dir.path(),
            &command,
            &[
                ("INFLUXDB_ADMIN_USER", "admin"),
                ("INFLUXDB_ADMIN_PASSWORD", "root"),
                ("BACKUP_PW", "mine"),
            ],
            b"",
        )
        .await;

        assert!(run.log.contains(":user=backup:pw=mine"), "{}", run.log);
    }

    #[tokio::test]
    async fn a_v1_dump_backs_up_the_one_database_portably() {
        let dir = tempfile::tempdir().unwrap();
        let influx = install(dir.path(), "influx", INFLUX).await;
        let influxd = install(dir.path(), "influxd", INFLUXD).await;
        let command = standing_in(V1.dump("telegraf"), &influx, Some(&influxd));

        let run = run(dir.path(), &command, &[], b"").await;

        assert!(run.status.success(), "{}", run.stderr);
        assert!(
            run.log
                .starts_with("influxd:backup -portable -database telegraf /"),
            "{}",
            run.log
        );
        assert_ne!(run.bytes.len(), 0);
    }

    #[tokio::test]
    async fn a_v1_restore_into_a_new_database_uses_newdb_and_drops_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let influx = install(dir.path(), "influx", INFLUX).await;
        let influxd = install(dir.path(), "influxd", INFLUXD).await;
        let tar = tar_of_one_file(dir.path()).await;
        let restore = restoring(
            "telegraf",
            DatabaseRestoreTarget::NewDatabase {
                name: "telegraf_restored".to_owned(),
            },
        );
        let command = standing_in(V1.restore(&restore), &influx, Some(&influxd));

        let run = run(dir.path(), &command, &[], &tar).await;

        assert!(run.status.success(), "{}", run.stderr);
        assert!(
            run.log
                .contains("influxd:restore -portable -db telegraf -newdb telegraf_restored /"),
            "{}",
            run.log
        );
        assert!(!run.log.contains("DROP"), "{}", run.log);
    }

    #[tokio::test]
    async fn a_v1_restore_over_the_original_drops_it_first_with_the_name_quoted() {
        let dir = tempfile::tempdir().unwrap();
        let influx = install(dir.path(), "influx", INFLUX).await;
        let influxd = install(dir.path(), "influxd", INFLUXD).await;
        let tar = tar_of_one_file(dir.path()).await;
        let restore = restoring(r#"we"ird\db"#, DatabaseRestoreTarget::Original);
        let command = standing_in(V1.restore(&restore), &influx, Some(&influxd));

        let run = run(dir.path(), &command, &[], &tar).await;

        assert!(run.status.success(), "{}", run.stderr);
        let lines: Vec<&str> = run.log.lines().collect();
        assert_eq!(
            lines.first().copied(),
            Some(r#"influx:-execute DROP DATABASE "we\"ird\\db":token=unset:user=unset:pw=unset"#)
        );
        assert!(
            lines
                .get(1)
                .unwrap()
                .starts_with(r#"influxd:restore -portable -db we"ird\db /"#)
        );
    }

    #[tokio::test]
    async fn creating_a_v1_database_refuses_one_that_exists() {
        let dir = tempfile::tempdir().unwrap();
        let influx = install(dir.path(), "influx", INFLUX).await;
        let create = |name: &str| standing_in(V1.create_database(name), &influx, Some("influxd"));

        let taken = run(dir.path(), &create("telegraf"), &[], b"").await;
        assert!(!taken.status.success());
        assert!(
            taken
                .stderr
                .contains("the database telegraf exists already"),
            "{}",
            taken.stderr
        );
        let free = run(dir.path(), &create("fresh"), &[], b"").await;
        assert!(free.status.success(), "{}", free.stderr);
    }

    #[tokio::test]
    async fn dropping_a_v1_database_drops_it_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let influx = install(dir.path(), "influx", INFLUX).await;
        let command = standing_in(
            V1.drop_database("telegraf_restored"),
            &influx,
            Some("influxd"),
        );

        let run = run(dir.path(), &command, &[], b"").await;

        assert!(run.status.success(), "{}", run.stderr);
        assert!(
            run.log
                .contains(r#"influx:-execute DROP DATABASE "telegraf_restored""#),
            "{}",
            run.log
        );
    }

    #[tokio::test]
    async fn a_v2_instance_cannot_be_restored_next_to_itself() {
        let dir = tempfile::tempdir().unwrap();
        let run = run(dir.path(), &V2.create_database("copy"), &[], b"").await;
        assert!(!run.status.success());
        assert!(run.stderr.contains("restored as a whole"), "{}", run.stderr);
        assert_eq!(V2.list_databases(), None);
    }
}
