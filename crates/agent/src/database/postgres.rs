// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The `PostgreSQL` commands an agent runs inside a database pod.

/// Lists the databases that accept connections, one name per line.
///
/// `-X` keeps a `.psqlrc` in the image from changing the output, and `-At`
/// prints bare values without headers or alignment.
pub fn list_databases(user: &str) -> Vec<String> {
    [
        "psql",
        "-X",
        "-A",
        "-t",
        "--no-password",
        "--username",
        user,
        "--dbname",
        "postgres",
        "--command",
        "SELECT datname FROM pg_database WHERE datallowconn AND NOT datistemplate ORDER BY datname",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// Dumps one database to stdout in custom format.
///
/// The dump is left uncompressed so borg can deduplicate it against the
/// previous night's and compress it itself. The database is passed as
/// `--dbname=<name>` so a name starting with `-` is never read as an option.
pub fn dump(user: &str, database: &str) -> Vec<String> {
    vec![
        "pg_dump".to_owned(),
        "--format=custom".to_owned(),
        "--compress=0".to_owned(),
        "--no-password".to_owned(),
        format!("--username={user}"),
        format!("--dbname={database}"),
    ]
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
            dump("postgres", "shop"),
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
    fn a_database_named_like_an_option_stays_a_value() {
        assert!(dump("postgres", "--help").contains(&"--dbname=--help".to_owned()));
    }

    #[test]
    fn the_listing_skips_templates_and_closed_databases() {
        let command = list_databases("app");
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
