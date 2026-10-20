// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The `MongoDB` commands an agent runs in a database's container or on the
//! host.
//!
//! Dumps and restores use the database tools. Listing, claiming and dropping
//! a database run a short `mongosh` script. In a container it signs in with
//! the account the sign-in script puts in its environment; on the host it
//! connects to the `uri` the tools' config file names.

use shared::database::{DatabaseRestore, DatabaseRestoreTarget};

use super::credentials::{self, MongoProgram};

/// Where a script connects when nothing else names the server.
const DEFAULT_PORT: u16 = 27017;

/// Reads the `uri` a database tools config file names, the way the tools
/// read it: one `uri:` line, its value bare or quoted.
const URI_FROM: &str = "const uriFrom = path => { const found = \
                        /^\\s*uri\\s*:\\s*['\"]?([^'\"\\s]+)['\"]?\\s*$/m.exec(require('fs').\
                        readFileSync(path, 'utf8')); if (!found) throw new Error(path + ' names \
                        no uri'); return found[1]; };";

/// Signs in when the environment names an account, and lists the server's
/// databases. Every script starts with it, after `m` connected.
const SIGN_IN: &str = "const admin = m.getDB('admin'); if (process.env.ASSIMILATE_MONGO_USER) \
                       admin.auth(process.env.ASSIMILATE_MONGO_USER, \
                       process.env.ASSIMILATE_MONGO_PASSWORD); const names = () => \
                       admin.adminCommand({ listDatabases: 1, nameOnly: true }).databases.map(d \
                       => d.name);";

/// Prints the databases an application keeps, one name per line: every one
/// but the server's own.
const LIST: &str =
    "names().filter(n => !['admin', 'config', 'local'].includes(n)).sort().forEach(n => print(n));";

/// How a command reaches its server and signs in.
#[derive(Debug, Clone, Copy)]
pub enum Mongodb<'a> {
    /// Inside the database's container, as `user` (read from the
    /// container's environment when empty) with the password in its
    /// `password_env`, or its image's own variables.
    Container {
        user: &'a str,
        password_env: Option<&'a str>,
    },
    /// On the host: the tools take `connection`, and a script connects to
    /// the `uri` in `config_file`, or to `port` on `127.0.0.1`.
    Host {
        connection: &'a [String],
        port: Option<u16>,
        config_file: Option<&'a str>,
    },
}

/// `name` as a script's string literal.
fn literal(name: &str) -> String {
    serde_json::Value::from(name).to_string()
}

/// `database` as the start of a namespace pattern: `mongorestore` reads `*`
/// as a wildcard and `\` as its escape.
fn namespace(database: &str) -> String {
    let escaped = database.replace('\\', "\\\\").replace('*', "\\*");
    format!("{escaped}.*")
}

impl Mongodb<'_> {
    fn tool(&self, program: &str, args: Vec<String>) -> Vec<String> {
        match *self {
            Self::Container { user, password_env } => {
                credentials::mongodb(MongoProgram::Tool, user, password_env, program, args)
            }
            Self::Host { connection, .. } => std::iter::once(program.to_owned())
                .chain(connection.iter().cloned())
                .chain(args)
                .collect(),
        }
    }

    /// The statement that connects `m` to the server.
    fn connect(&self) -> String {
        let local = |port: Option<u16>| {
            let port = port.unwrap_or(DEFAULT_PORT);
            format!("const m = new Mongo('mongodb://127.0.0.1:{port}/');")
        };
        match *self {
            Self::Container { .. } => local(None),
            Self::Host {
                config_file: Some(file),
                ..
            } => format!(
                "{URI_FROM} const m = new Mongo(uriFrom({}));",
                literal(file)
            ),
            Self::Host {
                port,
                config_file: None,
                ..
            } => local(port),
        }
    }

    fn script(&self, script: &str) -> Vec<String> {
        let args: Vec<String> = ["--nodb", "--quiet", "--eval"]
            .into_iter()
            .map(str::to_owned)
            .chain([format!("{} {SIGN_IN} {script}", self.connect())])
            .collect();
        match *self {
            Self::Container { user, password_env } => {
                credentials::mongodb(MongoProgram::Shell, user, password_env, "mongosh", args)
            }
            Self::Host { .. } => std::iter::once("mongosh".to_owned()).chain(args).collect(),
        }
    }

    /// Lists the databases, one name per line.
    pub fn list_databases(&self) -> Vec<String> {
        self.script(LIST)
    }

    /// Dumps one database to stdout as an archive. It stays uncompressed so
    /// borg can deduplicate it against the previous one.
    #[cfg(unix)]
    pub fn dump(&self, database: &str) -> Vec<String> {
        self.tool(
            "mongodump",
            vec!["--archive".to_owned(), format!("--db={database}")],
        )
    }

    /// Makes sure the database `name` a restore writes into does not exist
    /// yet. `MongoDB` creates a database with its first write, so a restore
    /// into one that already exists would merge into it instead.
    pub fn create_database(&self, name: &str) -> Vec<String> {
        let name = literal(name);
        self.script(&format!(
            "if (names().includes({name})) throw new Error('the database ' + {name} + ' already \
             exists');"
        ))
    }

    /// Drops the database `name` again after a restore into it failed.
    pub fn drop_database(&self, name: &str) -> Vec<String> {
        self.script(&format!("m.getDB({}).dropDatabase();", literal(name)))
    }

    /// Restores an archive read from stdin into `restore`'s target: renamed
    /// into the new database, or over the original, where `--drop` replaces
    /// each collection the archive holds.
    pub fn restore(&self, restore: &DatabaseRestore) -> Vec<String> {
        let from = namespace(&restore.database);
        let mut args = vec!["--archive".to_owned(), format!("--nsInclude={from}")];
        match &restore.target {
            DatabaseRestoreTarget::NewDatabase { name } => {
                args.push(format!("--nsFrom={from}"));
                args.push(format!("--nsTo={}", namespace(name)));
            }
            DatabaseRestoreTarget::Original => args.push("--drop".to_owned()),
        }
        self.tool("mongorestore", args)
    }
}

#[cfg(test)]
mod tests {
    use shared::database::{DatabaseRuntime, DatabaseSourceId, DumpFormat};

    use super::*;

    const ROOT: Mongodb<'static> = Mongodb::Container {
        user: "",
        password_env: None,
    };

    /// The program and its arguments, after the sign-in script's own.
    fn program_args(command: &[String]) -> &[String] {
        command.get(8..).unwrap()
    }

    fn restoring(database: &str, target: DatabaseRestoreTarget) -> DatabaseRestore {
        DatabaseRestore {
            source: DatabaseSourceId {
                runtime: DatabaseRuntime::Kubernetes,
                namespace: "chat".to_owned(),
                name: "mongo".to_owned(),
            },
            database: database.to_owned(),
            archive_path: "dumps/kubernetes/chat/mongo/rocket.mongoarchive".to_owned(),
            format: DumpFormat::MongoArchive,
            target,
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_dump_is_an_uncompressed_archive_of_one_database() {
        assert_eq!(
            program_args(&ROOT.dump("rocket")),
            ["mongodump", "--archive", "--db=rocket"]
        );
    }

    #[test]
    fn the_scripts_run_in_mongosh_and_sign_in_first() {
        let list = ROOT.list_databases();
        assert_eq!(
            program_args(&list).get(..4).unwrap(),
            ["mongosh", "--nodb", "--quiet", "--eval"]
        );
        let script = list.last().unwrap();
        assert!(
            script.starts_with("const m = new Mongo('mongodb://127.0.0.1:27017/'); const admin"),
            "{script}"
        );
        assert!(script.contains("'admin', 'config', 'local'"), "{script}");

        let claim = ROOT.create_database("rocket_restored");
        assert!(
            claim
                .last()
                .unwrap()
                .contains("names().includes(\"rocket_restored\")"),
            "{claim:?}"
        );
        let drop = ROOT.drop_database("rocket_restored");
        assert!(
            drop.last()
                .unwrap()
                .ends_with("m.getDB(\"rocket_restored\").dropDatabase();"),
            "{drop:?}"
        );
    }

    #[test]
    fn a_restore_into_a_new_database_renames_its_namespaces() {
        let target = DatabaseRestoreTarget::NewDatabase {
            name: "rocket_restored".to_owned(),
        };
        assert_eq!(
            program_args(&ROOT.restore(&restoring("rocket", target))),
            [
                "mongorestore",
                "--archive",
                "--nsInclude=rocket.*",
                "--nsFrom=rocket.*",
                "--nsTo=rocket_restored.*",
            ]
        );
    }

    #[test]
    fn a_restore_over_the_original_replaces_its_collections() {
        assert_eq!(
            program_args(&ROOT.restore(&restoring("odd*db\\", DatabaseRestoreTarget::Original))),
            [
                "mongorestore",
                "--archive",
                "--nsInclude=odd\\*db\\\\.*",
                "--drop"
            ]
        );
    }

    #[test]
    fn a_host_server_is_reached_without_the_sign_in_script() {
        let connection = ["--port=27018".to_owned()];
        let host = Mongodb::Host {
            connection: &connection,
            port: Some(27018),
            config_file: None,
        };
        let restore = host.restore(&restoring("rocket", DatabaseRestoreTarget::Original));
        assert_eq!(restore.get(..2).unwrap(), ["mongorestore", "--port=27018"]);
        let list = host.list_databases();
        assert_eq!(list.first().unwrap(), "mongosh");
        assert!(
            list.last()
                .unwrap()
                .starts_with("const m = new Mongo('mongodb://127.0.0.1:27018/');"),
            "{list:?}"
        );
    }

    #[test]
    fn a_host_config_file_names_the_server_for_the_scripts_too() {
        let connection = ["--config=/etc/assimilate/mongodump.yaml".to_owned()];
        let host = Mongodb::Host {
            connection: &connection,
            port: Some(27018),
            config_file: Some("/etc/assimilate/mongodump.yaml"),
        };
        let script = host.drop_database("rocket_restored");
        let script = script.last().unwrap();
        assert!(script.starts_with(URI_FROM), "{script}");
        assert!(
            script.contains("new Mongo(uriFrom(\"/etc/assimilate/mongodump.yaml\"))"),
            "{script}"
        );
        assert!(!script.contains("27018"), "{script}");
    }
}
