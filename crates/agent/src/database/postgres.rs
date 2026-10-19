// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The `PostgreSQL` commands an agent runs where a database lives. Each takes
//! the options naming the server's socket or port first, which only a host
//! server needs.

use shared::database::{DatabaseRestore, DatabaseRestoreTarget};

/// Lists the databases that accept connections, one name per line.
///
/// `-X` keeps a `.psqlrc` in the image from changing the output, and `-At`
/// prints bare values without headers or alignment.
pub fn list_databases(user: &str, connection: &[String]) -> Vec<String> {
    let mut command: Vec<String> = ["psql", "-X", "-A", "-t", "--no-password"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    command.extend_from_slice(connection);
    command.extend(
        [
            "--username",
            user,
            "--dbname",
            "postgres",
            "--command",
            "SELECT datname FROM pg_database WHERE datallowconn AND NOT datistemplate ORDER BY \
             datname",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    command
}

/// Dumps one database to stdout in custom format.
///
/// The dump is left uncompressed so borg can deduplicate it against the
/// previous night's and compress it itself. The database is passed as
/// `--dbname=<name>` so a name starting with `-` is never read as an option.
#[cfg(unix)]
pub fn dump(user: &str, database: &str, connection: &[String]) -> Vec<String> {
    let mut command = vec![
        "pg_dump".to_owned(),
        "--format=custom".to_owned(),
        "--compress=0".to_owned(),
        "--no-password".to_owned(),
    ];
    command.extend_from_slice(connection);
    command.push(format!("--username={user}"));
    command.push(format!("--dbname={database}"));
    command
}

/// The options every command that changes a server's databases starts with:
/// the server's address, the role, and `postgres` as the database to connect
/// to, since the one being created or dropped cannot be.
fn maintenance(program: &str, user: &str, connection: &[String], options: &[&str]) -> Vec<String> {
    let mut command = vec![program.to_owned(), "--no-password".to_owned()];
    command.extend(options.iter().map(|option| (*option).to_owned()));
    command.extend_from_slice(connection);
    command.push(format!("--username={user}"));
    command.push("--maintenance-db=postgres".to_owned());
    command
}

/// Creates the empty database `name` a restore writes into. The name is a
/// validated new database name, which never starts with `-`.
pub fn create_database(user: &str, name: &str, connection: &[String]) -> Vec<String> {
    let mut command = maintenance("createdb", user, connection, &[]);
    command.push(name.to_owned());
    command
}

/// Drops the database `name` again after a restore into it failed.
pub fn drop_database(user: &str, name: &str, connection: &[String]) -> Vec<String> {
    let mut command = maintenance("dropdb", user, connection, &["--if-exists"]);
    command.push(name.to_owned());
    command
}

/// Restores a custom-format dump read from stdin into `restore`'s target.
///
/// `--single-transaction` with `--exit-on-error` makes a restore that fails
/// change nothing.
///
/// Over the original database, `--clean --if-exists` first drops what the
/// dump recreates, and the objects get their owners back, so the roles the
/// application connects as keep their access. A new database is a copy to
/// look at: `--no-owner` leaves it owned by the restoring role, which works
/// for a role that may not assign other owners.
pub fn restore(user: &str, restore: &DatabaseRestore, connection: &[String]) -> Vec<String> {
    let mut command: Vec<String> = ["pg_restore", "--no-password"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let target: &[&str] = match restore.target {
        DatabaseRestoreTarget::NewDatabase { .. } => &["--no-owner"],
        DatabaseRestoreTarget::Original => &["--clean", "--if-exists"],
    };
    command.extend(
        ["--single-transaction", "--exit-on-error"]
            .iter()
            .chain(target)
            .map(|option| (*option).to_owned()),
    );
    command.extend_from_slice(connection);
    command.push(format!("--username={user}"));
    command.push(format!("--dbname={}", restore.target_database()));
    command
}

/// The database names in `psql -At` output.
pub fn parse_database_list(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn dump_streams_an_uncompressed_custom_format_dump() {
        assert_eq!(
            dump("postgres", "shop", &[]),
            [
                "pg_dump",
                "--format=custom",
                "--compress=0",
                "--no-password",
                "--username=postgres",
                "--dbname=shop",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_host_server_is_named_before_the_database() {
        let connection = ["--host=/run/pg".to_owned(), "--port=5433".to_owned()];
        let command = dump("postgres", "shop", &connection);
        assert_eq!(
            command.get(4..6).unwrap(),
            ["--host=/run/pg", "--port=5433"]
        );
        assert_eq!(command.last().map(String::as_str), Some("--dbname=shop"));
        let listing = list_databases("postgres", &connection);
        assert!(listing.contains(&"--port=5433".to_owned()));
        assert!(listing.last().unwrap().starts_with("SELECT datname"));
    }

    #[cfg(unix)]
    #[test]
    fn a_database_named_like_an_option_stays_a_value() {
        assert!(dump("postgres", "--help", &[]).contains(&"--dbname=--help".to_owned()));
    }

    #[test]
    fn the_listing_skips_templates_and_closed_databases() {
        let command = list_databases("app", &[]);
        assert_eq!(command.first().map(String::as_str), Some("psql"));
        assert!(command.windows(2).any(|pair| pair == ["--username", "app"]));
        let query = command.last().map_or("", String::as_str);
        assert!(query.contains("datallowconn"));
        assert!(query.contains("NOT datistemplate"));
    }

    fn restoring(target: DatabaseRestoreTarget) -> DatabaseRestore {
        DatabaseRestore {
            source: shared::database::DatabaseSourceId {
                runtime: shared::database::DatabaseRuntime::Host,
                namespace: String::new(),
                name: "postgresql".to_owned(),
            },
            database: "shop".to_owned(),
            archive_path: "var/lib/assimilate/database-dumps/manual/host/postgresql/shop.pgdump"
                .to_owned(),
            format: shared::database::DumpFormat::PgCustom,
            target,
        }
    }

    #[test]
    fn a_new_database_is_created_and_dropped_through_the_maintenance_database() {
        let connection = ["--port=5433".to_owned()];
        assert_eq!(
            create_database("postgres", "shop_restored", &connection),
            [
                "createdb",
                "--no-password",
                "--port=5433",
                "--username=postgres",
                "--maintenance-db=postgres",
                "shop_restored",
            ]
        );
        assert_eq!(
            drop_database("postgres", "shop_restored", &[]),
            [
                "dropdb",
                "--no-password",
                "--if-exists",
                "--username=postgres",
                "--maintenance-db=postgres",
                "shop_restored",
            ]
        );
    }

    #[test]
    fn a_restore_into_a_new_database_runs_in_one_transaction() {
        let target = DatabaseRestoreTarget::NewDatabase {
            name: "shop_restored".to_owned(),
        };
        assert_eq!(
            restore("postgres", &restoring(target), &[]),
            [
                "pg_restore",
                "--no-password",
                "--single-transaction",
                "--exit-on-error",
                "--no-owner",
                "--username=postgres",
                "--dbname=shop_restored",
            ]
        );
    }

    #[test]
    fn a_restore_over_the_original_drops_what_it_recreates() {
        let connection = ["--host=/run/pg".to_owned()];
        assert_eq!(
            restore(
                "app",
                &restoring(DatabaseRestoreTarget::Original),
                &connection
            ),
            [
                "pg_restore",
                "--no-password",
                "--single-transaction",
                "--exit-on-error",
                "--clean",
                "--if-exists",
                "--host=/run/pg",
                "--username=app",
                "--dbname=shop",
            ]
        );
    }

    #[test]
    fn the_listing_output_is_one_name_per_line() {
        assert_eq!(
            parse_database_list("app\n\n  shop  \npostgres\n"),
            ["app", "shop", "postgres"]
        );
        assert_eq!(parse_database_list("").len(), 0);
    }
}
