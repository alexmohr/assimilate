// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The `PostgreSQL` commands an agent runs where a database lives. Each takes
//! the options naming the server's socket or port first, which only a host
//! server needs.

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

    #[test]
    fn the_listing_output_is_one_name_per_line() {
        assert_eq!(
            parse_database_list("app\n\n  shop  \npostgres\n"),
            ["app", "shop", "postgres"]
        );
        assert_eq!(parse_database_list("").len(), 0);
    }
}
