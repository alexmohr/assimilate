// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The commands that list, dump and restore a source's databases, whichever
//! engine it runs.

use shared::database::{DatabaseEngine, DatabaseRestore, DumpFormat};

use super::{
    credentials::MysqlFlavor,
    host::{HostServer, HostTarget},
    mongodb::Mongodb,
    mysql::{Mysql, MysqlLogin},
    postgres,
    sqlite::{self, Sqlite},
    target::DumpTarget,
};

/// A dump the source's engine cannot read back.
#[derive(Debug, thiserror::Error)]
#[error("a {engine} server cannot restore a {format} dump")]
pub struct FormatMismatch {
    pub engine: DatabaseEngine,
    pub format: DumpFormat,
}

/// The commands for one source's server.
pub struct Client<'a> {
    engine: DatabaseEngine,
    user: &'a str,
    password_env: Option<&'a str>,
    /// The server installed on the host, `None` for a pod or a container.
    host: Option<&'a HostTarget>,
    /// Where a client finds a host's server.
    connection: Vec<String>,
    /// Where `sqlite3` is started from: its name, or its path in a host's
    /// programs directory.
    sqlite3: String,
}

/// One engine's command builders.
enum Server<'a> {
    Postgresql {
        user: &'a str,
        connection: &'a [String],
    },
    Mysql(Mysql<'a>),
    Mongodb(Mongodb<'a>),
    Sqlite(Sqlite<'a>),
}

impl<'a> Client<'a> {
    /// The commands `engine` takes at `target`, signed in as `user` with the
    /// password in the container's `password_env`, or its image's own
    /// variables.
    #[must_use]
    pub fn new(
        engine: DatabaseEngine,
        user: &'a str,
        password_env: Option<&'a str>,
        target: &'a DumpTarget,
    ) -> Self {
        let host = match target {
            DumpTarget::Host(host) => Some(host),
            DumpTarget::Pod(_) | DumpTarget::Container(_) => None,
        };
        Self {
            engine,
            user,
            password_env,
            host,
            connection: target.connection_args(),
            sqlite3: host.map_or_else(
                || "sqlite3".to_owned(),
                |host| host.program("sqlite3").to_string_lossy().into_owned(),
            ),
        }
    }

    fn server(&self) -> Server<'_> {
        let mysql = |flavor| {
            let login = match self.host {
                Some(_) => MysqlLogin::Host {
                    connection: &self.connection,
                },
                None => MysqlLogin::Container {
                    user: self.user,
                    password_env: self.password_env,
                },
            };
            Server::Mysql(Mysql { flavor, login })
        };
        let mongodb = match self.host.map(|host| &host.server) {
            Some(HostServer::Mongodb { port, config_file }) => Mongodb::Host {
                connection: &self.connection,
                port: *port,
                config_file: config_file.as_deref(),
            },
            Some(HostServer::Postgresql { .. } | HostServer::Mysql { .. } | HostServer::Sqlite) => {
                Mongodb::Host {
                    connection: &self.connection,
                    port: None,
                    config_file: None,
                }
            }
            None => Mongodb::Container {
                user: self.user,
                password_env: self.password_env,
            },
        };
        match self.engine {
            DatabaseEngine::Postgresql => Server::Postgresql {
                user: self.user,
                connection: &self.connection,
            },
            DatabaseEngine::Mysql => mysql(MysqlFlavor::Mysql),
            DatabaseEngine::Mariadb => mysql(MysqlFlavor::Mariadb),
            DatabaseEngine::Mongodb => Server::Mongodb(mongodb),
            DatabaseEngine::Sqlite => Server::Sqlite(Sqlite {
                program: &self.sqlite3,
            }),
        }
    }

    /// Prints the databases the server holds, one name per line. `None` for
    /// `SQLite`, which has no server to ask: its source names its files.
    #[must_use]
    pub fn list_databases(&self) -> Option<Vec<String>> {
        match self.server() {
            Server::Postgresql { user, connection } => {
                Some(postgres::list_databases(user, connection))
            }
            Server::Mysql(mysql) => Some(mysql.list_databases()),
            Server::Mongodb(mongodb) => Some(mongodb.list_databases()),
            Server::Sqlite(_) => None,
        }
    }

    /// Writes one database's dump to stdout.
    #[cfg(unix)]
    #[must_use]
    pub fn dump(&self, database: &str) -> Vec<String> {
        match self.server() {
            Server::Postgresql { user, connection } => postgres::dump(user, database, connection),
            Server::Mysql(mysql) => mysql.dump(database),
            Server::Mongodb(mongodb) => mongodb.dump(database),
            Server::Sqlite(sqlite) => sqlite.dump(database),
        }
    }

    /// The database a restore into the new database `name` creates: that
    /// name, or for `SQLite` the file next to the one the dump came from.
    #[must_use]
    pub fn new_database(&self, restore: &DatabaseRestore, name: &str) -> String {
        match self.engine {
            DatabaseEngine::Sqlite => sqlite::new_database(&restore.database, name),
            DatabaseEngine::Postgresql
            | DatabaseEngine::Mysql
            | DatabaseEngine::Mariadb
            | DatabaseEngine::Mongodb => name.to_owned(),
        }
    }

    /// Creates the database `name` a restore writes into, or, where the
    /// first write creates it, makes sure it does not exist yet.
    #[must_use]
    pub fn create_database(&self, name: &str) -> Vec<String> {
        match self.server() {
            Server::Postgresql { user, connection } => {
                postgres::create_database(user, name, connection)
            }
            Server::Mysql(mysql) => mysql.create_database(name),
            Server::Mongodb(mongodb) => mongodb.create_database(name),
            Server::Sqlite(_) => Sqlite::create_database(name),
        }
    }

    /// Drops the database `name` a failed restore created.
    #[must_use]
    pub fn drop_database(&self, name: &str) -> Vec<String> {
        match self.server() {
            Server::Postgresql { user, connection } => {
                postgres::drop_database(user, name, connection)
            }
            Server::Mysql(mysql) => mysql.drop_database(name),
            Server::Mongodb(mongodb) => mongodb.drop_database(name),
            Server::Sqlite(_) => Sqlite::drop_database(name),
        }
    }

    /// Restores a dump read from stdin into `restore`'s target.
    ///
    /// # Errors
    ///
    /// Returns an error when this engine cannot read the dump's format.
    pub fn restore(&self, restore: &DatabaseRestore) -> Result<Vec<String>, FormatMismatch> {
        match (self.server(), restore.format) {
            (Server::Postgresql { user, connection }, DumpFormat::PgCustom) => {
                Ok(postgres::restore(user, restore, connection))
            }
            (Server::Mysql(mysql), DumpFormat::MysqlSql) => Ok(mysql.restore(restore)),
            (Server::Mongodb(mongodb), DumpFormat::MongoArchive) => Ok(mongodb.restore(restore)),
            (Server::Sqlite(sqlite), DumpFormat::SqliteSql) => Ok(sqlite.restore(restore)),
            (
                Server::Postgresql { .. }
                | Server::Mysql(_)
                | Server::Mongodb(_)
                | Server::Sqlite(_),
                DumpFormat::PgCustom
                | DumpFormat::MysqlSql
                | DumpFormat::MongoArchive
                | DumpFormat::SqliteSql,
            ) => Err(FormatMismatch {
                engine: self.engine,
                format: restore.format,
            }),
        }
    }
}

/// The database names in a listing's output, one per line.
#[must_use]
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
    use shared::database::{DatabaseRestoreTarget, DatabaseRuntime, DatabaseSourceId};

    use super::*;
    use crate::database::docker::ContainerTarget;

    fn container() -> DumpTarget {
        DumpTarget::Container(ContainerTarget {
            id: "c1".to_owned(),
            name: "db".to_owned(),
        })
    }

    fn restoring(format: DumpFormat) -> DatabaseRestore {
        DatabaseRestore {
            source: DatabaseSourceId {
                runtime: DatabaseRuntime::Docker,
                namespace: String::new(),
                name: "db".to_owned(),
            },
            database: "shop".to_owned(),
            archive_path: "dumps/docker/db/shop.sql".to_owned(),
            format,
            target: DatabaseRestoreTarget::Original,
        }
    }

    #[test]
    fn each_engine_runs_its_own_programs() {
        let program = |engine, command: Vec<String>| {
            let program = match engine {
                DatabaseEngine::Postgresql | DatabaseEngine::Sqlite => command.first(),
                DatabaseEngine::Mysql | DatabaseEngine::Mariadb => command.get(6),
                DatabaseEngine::Mongodb => command.get(8),
            };
            program.cloned().unwrap()
        };
        let engines = [
            (DatabaseEngine::Postgresql, "psql", "createdb", "dropdb"),
            (DatabaseEngine::Mysql, "mysql", "mysql", "mysql"),
            (DatabaseEngine::Mariadb, "mariadb", "mariadb", "mariadb"),
            (DatabaseEngine::Mongodb, "mongosh", "mongosh", "mongosh"),
        ];
        let target = container();
        for (engine, list, create, drop) in engines {
            let client = Client::new(engine, "u", None, &target);
            assert_eq!(program(engine, client.list_databases().unwrap()), list);
            assert_eq!(program(engine, client.create_database("x")), create);
            assert_eq!(program(engine, client.drop_database("x")), drop);
        }

        // SQLite has no server to list its files: its source names them.
        let sqlite = Client::new(DatabaseEngine::Sqlite, "", None, &target);
        assert_eq!(sqlite.list_databases(), None);
        let create = sqlite.create_database("/data/copy.db");
        assert_eq!(program(DatabaseEngine::Sqlite, create), "/bin/sh");
        let drop = sqlite.drop_database("/data/copy.db");
        assert_eq!(drop.last().map(String::as_str), Some("/data/copy.db"));
    }

    #[cfg(unix)]
    #[test]
    fn each_engine_dumps_with_its_own_program() {
        let dumps = [
            (DatabaseEngine::Postgresql, "pg_dump"),
            (DatabaseEngine::Mysql, "mysqldump"),
            (DatabaseEngine::Mariadb, "mariadb-dump"),
            (DatabaseEngine::Mongodb, "mongodump"),
            (DatabaseEngine::Sqlite, "sqlite3"),
        ];
        let target = container();
        for (engine, program) in dumps {
            let dump = Client::new(engine, "u", None, &target).dump("shop");
            assert!(dump.contains(&program.to_owned()), "{engine}: {dump:?}");
        }
    }

    #[test]
    fn only_a_host_server_is_given_its_address() {
        let host = DumpTarget::Host(HostTarget {
            os_user: "postgres".to_owned(),
            server: HostServer::Postgresql {
                socket_dir: None,
                port: Some(5433),
            },
            bin_dir: None,
        });
        let listing =
            Client::new(DatabaseEngine::Postgresql, "postgres", None, &host).list_databases();
        let listing = listing.unwrap();
        assert!(listing.contains(&"--port=5433".to_owned()), "{listing:?}");
    }

    #[test]
    fn a_host_server_takes_its_connection_and_no_sign_in_script() {
        let host = |server| {
            DumpTarget::Host(HostTarget {
                os_user: "root".to_owned(),
                server,
                bin_dir: None,
            })
        };
        let mysql = host(HostServer::Mysql {
            socket: Some("/run/mysqld/mysqld.sock".to_owned()),
            port: None,
        });
        let listing = Client::new(DatabaseEngine::Mariadb, "root", None, &mysql)
            .list_databases()
            .unwrap();
        assert_eq!(
            listing.get(..2).unwrap(),
            ["mariadb", "--socket=/run/mysqld/mysqld.sock"]
        );

        let mongodb = host(HostServer::Mongodb {
            port: Some(27018),
            config_file: None,
        });
        let listing = Client::new(DatabaseEngine::Mongodb, "root", None, &mongodb)
            .list_databases()
            .unwrap();
        assert_eq!(listing.first().unwrap(), "mongosh");
        assert!(
            listing.last().unwrap().contains("127.0.0.1:27018"),
            "{listing:?}"
        );

        // A server the settings describe for another engine still runs no
        // sign-in script.
        let mismatched = host(HostServer::Postgresql {
            socket_dir: None,
            port: None,
        });
        let listing = Client::new(DatabaseEngine::Mongodb, "root", None, &mismatched)
            .list_databases()
            .unwrap();
        assert_eq!(listing.first().unwrap(), "mongosh");
    }

    #[cfg(unix)]
    #[test]
    fn sqlite_on_the_host_starts_sqlite3_from_the_programs_directory() {
        let sqlite = DumpTarget::Host(HostTarget {
            os_user: "root".to_owned(),
            server: HostServer::Sqlite,
            bin_dir: Some("/opt/sqlite/bin".to_owned()),
        });
        let dump = Client::new(DatabaseEngine::Sqlite, "root", None, &sqlite).dump("/srv/app.db");
        assert_eq!(dump.first().unwrap(), "/opt/sqlite/bin/sqlite3");
    }

    #[test]
    fn a_dump_is_restored_only_by_the_engine_that_reads_its_format() {
        let pairs = [
            (
                DatabaseEngine::Postgresql,
                DumpFormat::PgCustom,
                "pg_restore",
            ),
            (DatabaseEngine::Mysql, DumpFormat::MysqlSql, "mysql"),
            (DatabaseEngine::Mariadb, DumpFormat::MysqlSql, "mariadb"),
            (
                DatabaseEngine::Mongodb,
                DumpFormat::MongoArchive,
                "mongorestore",
            ),
            (DatabaseEngine::Sqlite, DumpFormat::SqliteSql, "sqlite3"),
        ];
        let target = container();
        for (engine, format, program) in pairs {
            let client = Client::new(engine, "u", None, &target);
            let command = client.restore(&restoring(format)).unwrap();
            assert!(
                command.contains(&program.to_owned()),
                "{engine}: {command:?}"
            );
        }

        let mismatched = [
            (DatabaseEngine::Postgresql, DumpFormat::MysqlSql),
            (DatabaseEngine::Mariadb, DumpFormat::PgCustom),
            (DatabaseEngine::Mysql, DumpFormat::MongoArchive),
            (DatabaseEngine::Mongodb, DumpFormat::MysqlSql),
            (DatabaseEngine::Sqlite, DumpFormat::MysqlSql),
            (DatabaseEngine::Mysql, DumpFormat::SqliteSql),
        ];
        for (engine, format) in mismatched {
            let client = Client::new(engine, "u", None, &target);
            let err = client.restore(&restoring(format)).unwrap_err();
            assert_eq!(
                err.to_string(),
                format!("a {engine} server cannot restore a {format} dump")
            );
        }
    }

    #[test]
    fn only_a_sqlite_database_is_restored_as_a_file_next_to_the_original() {
        let target = container();
        let mut restore = restoring(DumpFormat::SqliteSql);
        restore.database = "/data/app.db".to_owned();
        let sqlite = Client::new(DatabaseEngine::Sqlite, "", None, &target);
        assert_eq!(sqlite.new_database(&restore, "copy"), "/data/copy.db");
        let mysql = Client::new(DatabaseEngine::Mysql, "root", None, &target);
        assert_eq!(mysql.new_database(&restore, "copy"), "copy");
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
