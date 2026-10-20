// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The `MySQL` and `MariaDB` commands an agent runs in a database's
//! container. `MariaDB` names its programs after itself; its newer images no
//! longer ship the `mysql` names.

use shared::database::DatabaseRestore;

use super::credentials::{self, MysqlFlavor};

/// Lists the databases an application keeps, one name per line: every
/// schema but the server's own.
const LIST_QUERY: &str = "SELECT schema_name FROM information_schema.schemata WHERE schema_name \
                          NOT IN ('information_schema', 'mysql', 'performance_schema', 'sys') \
                          ORDER BY schema_name";

/// Who a command signs in as, and on which server.
#[derive(Debug, Clone, Copy)]
pub struct Mysql<'a> {
    pub flavor: MysqlFlavor,
    pub user: &'a str,
    pub password_env: Option<&'a str>,
}

impl Mysql<'_> {
    const fn client(&self) -> &'static str {
        match self.flavor {
            MysqlFlavor::Mysql => "mysql",
            MysqlFlavor::Mariadb => "mariadb",
        }
    }

    fn run(&self, program: &str, args: &[&str]) -> Vec<String> {
        credentials::mysql(
            self.flavor,
            self.user,
            self.password_env,
            program,
            args.iter().map(|arg| (*arg).to_owned()).collect(),
        )
    }

    /// Runs one statement through the client. `-N -B` prints bare values,
    /// one row per line.
    fn statement(&self, statement: &str) -> Vec<String> {
        self.run(self.client(), &["-N", "-B", "-e", statement])
    }

    /// Lists the databases, one name per line.
    pub fn list_databases(&self) -> Vec<String> {
        self.statement(LIST_QUERY)
    }

    /// Dumps one database to stdout as SQL.
    ///
    /// `--single-transaction` reads a consistent snapshot without locking
    /// `InnoDB` tables, and routines, events and triggers come along. The
    /// dump names no database, so it restores into whichever one the client
    /// selects. `--no-tablespaces` spares the account the `PROCESS`
    /// privilege. `MySQL` would also write the server's GTIDs, which a
    /// server with GTIDs of its own refuses to restore. The database comes
    /// after `--`, so a name starting with `-` is never read as an option.
    #[cfg(unix)]
    pub fn dump(&self, database: &str) -> Vec<String> {
        let (program, gtids): (&str, &[&str]) = match self.flavor {
            MysqlFlavor::Mysql => ("mysqldump", &["--set-gtid-purged=OFF"]),
            MysqlFlavor::Mariadb => ("mariadb-dump", &[]),
        };
        let options = [
            "--single-transaction",
            "--routines",
            "--events",
            "--triggers",
            "--hex-blob",
            "--no-tablespaces",
        ];
        let args: Vec<&str> = options
            .iter()
            .chain(gtids)
            .copied()
            .chain(["--", database])
            .collect();
        self.run(program, &args)
    }

    /// Creates the empty database `name` a restore writes into. The name is
    /// a validated new database name, which holds no backtick.
    pub fn create_database(&self, name: &str) -> Vec<String> {
        self.statement(&format!("CREATE DATABASE `{name}`"))
    }

    /// Drops the database `name` again after a restore into it failed.
    pub fn drop_database(&self, name: &str) -> Vec<String> {
        self.statement(&format!("DROP DATABASE IF EXISTS `{name}`"))
    }

    /// Restores an SQL dump read from stdin into `restore`'s target. The dump
    /// drops each table before creating it again, so over the original it
    /// replaces what it holds.
    pub fn restore(&self, restore: &DatabaseRestore) -> Vec<String> {
        let database = restore.target_database();
        self.run(self.client(), &[&format!("--database={database}")])
    }
}

#[cfg(test)]
mod tests {
    use shared::database::{DatabaseRestoreTarget, DatabaseRuntime, DatabaseSourceId, DumpFormat};

    use super::*;

    const fn server(flavor: MysqlFlavor) -> Mysql<'static> {
        Mysql {
            flavor,
            user: "root",
            password_env: None,
        }
    }

    /// The client and its arguments, after the sign-in script's own.
    fn client_args(command: &[String]) -> &[String] {
        command.get(6..).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn each_flavor_dumps_with_its_own_program() {
        let mysql = server(MysqlFlavor::Mysql).dump("shop");
        assert_eq!(
            client_args(&mysql),
            [
                "mysqldump",
                "--single-transaction",
                "--routines",
                "--events",
                "--triggers",
                "--hex-blob",
                "--no-tablespaces",
                "--set-gtid-purged=OFF",
                "--",
                "shop",
            ]
        );
        let mariadb = server(MysqlFlavor::Mariadb).dump("-odd");
        assert_eq!(client_args(&mariadb).first().unwrap(), "mariadb-dump");
        assert!(!mariadb.contains(&"--set-gtid-purged=OFF".to_owned()));
        assert_eq!(mariadb.get(mariadb.len() - 2..).unwrap(), ["--", "-odd"]);
    }

    #[test]
    fn the_listing_leaves_out_the_servers_own_schemas() {
        let command = server(MysqlFlavor::Mariadb).list_databases();
        assert_eq!(
            client_args(&command).get(..4).unwrap(),
            ["mariadb", "-N", "-B", "-e"]
        );
        let query = command.last().unwrap();
        assert!(query.starts_with("SELECT schema_name"), "{query}");
        assert!(query.contains("'performance_schema'"), "{query}");
    }

    #[test]
    fn a_new_database_is_created_and_dropped_by_statement() {
        let mysql = server(MysqlFlavor::Mysql);
        assert_eq!(
            client_args(&mysql.create_database("shop_restored")),
            ["mysql", "-N", "-B", "-e", "CREATE DATABASE `shop_restored`"]
        );
        assert_eq!(
            mysql.drop_database("shop_restored").last().unwrap(),
            "DROP DATABASE IF EXISTS `shop_restored`"
        );
    }

    #[test]
    fn a_restore_selects_the_database_it_writes_into() {
        let mut restore = DatabaseRestore {
            source: DatabaseSourceId {
                runtime: DatabaseRuntime::Docker,
                namespace: String::new(),
                name: "wiki-db".to_owned(),
            },
            database: "wiki".to_owned(),
            archive_path: "dumps/docker/wiki-db/wiki.sql".to_owned(),
            format: DumpFormat::MysqlSql,
            target: DatabaseRestoreTarget::Original,
        };
        let mariadb = server(MysqlFlavor::Mariadb);
        assert_eq!(
            client_args(&mariadb.restore(&restore)),
            ["mariadb", "--database=wiki"]
        );
        restore.target = DatabaseRestoreTarget::NewDatabase {
            name: "wiki_restored".to_owned(),
        };
        assert_eq!(
            client_args(&mariadb.restore(&restore)),
            ["mariadb", "--database=wiki_restored"]
        );
    }
}
