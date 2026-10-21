// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The `SQLite` commands an agent runs where a database file lives: in its
//! container, or on the host. `sqlite3` reads the file directly; there is no
//! server to ask for its databases and no account to sign in as, so a
//! source's "databases" are the absolute paths of its files.
//!
//! A dump is the SQL `sqlite3 .dump` writes from one read transaction, so it
//! is consistent while the application keeps writing. A restore over the
//! original loads the dump into a scratch file next to it first, then copies
//! that into the live database with `.restore`, which takes `SQLite`'s own
//! locks: an application holding the file open sees the restored content,
//! and a dump that does not load leaves the database as it was.

use shared::database::{DatabaseRestore, DatabaseRestoreTarget};

/// The shell the scripts run in. Absolute, so a host's programs directory is
/// never searched for it.
const SHELL: &str = "/bin/sh";

/// Creates a new database file, refusing one that exists already.
///
/// Arguments: the file.
const CREATE_SCRIPT: &str = r#"set -C
: > "$1" || { echo "assimilate: $1 exists already" >&2; exit 1; }
"#;

/// A shell function `remove` that removes a database file with every file
/// `SQLite` keeps beside it: its rollback journal, or its write-ahead log
/// and shared-memory index.
macro_rules! remove_function {
    () => {
        r#"remove() { rm -f -- "$1" "$1-journal" "$1-wal" "$1-shm"; }
"#
    };
}

/// Removes a database file a failed restore created, with its journal.
///
/// Arguments: the file.
const DROP_SCRIPT: &str = concat!(remove_function!(), r#"remove "$1""#);

/// Loads the dump on stdin into a scratch file next to the database, then
/// copies it into the database with `.restore`. The scratch file's name is
/// digits and letters only, so `.restore` reads it as one argument.
///
/// Arguments: the database file, then `sqlite3`.
const RESTORE_SCRIPT: &str = concat!(
    "set -e\n",
    remove_function!(),
    r#"file=$1 program=$2
cd -- "$(dirname -- "$file")"
scratch=.assimilate-restore-$$
trap 'remove "$scratch"' EXIT
remove "$scratch"
"$program" -bail "$scratch"
"$program" -bail "$file" ".restore $scratch"
"#
);

/// The commands for one source's database files.
#[derive(Debug, Clone, Copy)]
pub struct Sqlite<'a> {
    /// Where `sqlite3` is started from: its name, or its path in a host's
    /// programs directory.
    pub program: &'a str,
}

impl Sqlite<'_> {
    fn script(script: &str, args: &[&str]) -> Vec<String> {
        [SHELL, "-c", script, "sh"]
            .into_iter()
            .chain(args.iter().copied())
            .map(str::to_owned)
            .collect()
    }

    /// Dumps the database file `file` to stdout as SQL. `-readonly` keeps
    /// the dump from ever writing to the file.
    #[cfg(unix)]
    pub fn dump(&self, file: &str) -> Vec<String> {
        [self.program, "-readonly", file, ".dump"]
            .map(str::to_owned)
            .to_vec()
    }

    /// Creates the empty database file `file` a restore writes into.
    pub fn create_database(file: &str) -> Vec<String> {
        Self::script(CREATE_SCRIPT, &[file])
    }

    /// Removes the database file `file` again after a restore into it
    /// failed.
    pub fn drop_database(file: &str) -> Vec<String> {
        Self::script(DROP_SCRIPT, &[file])
    }

    /// Restores an SQL dump read from stdin into `restore`'s target: a new
    /// file [`new_database`] created, or the original through a scratch
    /// file.
    pub fn restore(&self, restore: &DatabaseRestore) -> Vec<String> {
        match &restore.target {
            DatabaseRestoreTarget::NewDatabase { name } => {
                let file = new_database(&restore.database, name);
                [self.program, "-bail", &file].map(str::to_owned).to_vec()
            }
            DatabaseRestoreTarget::Original => {
                Self::script(RESTORE_SCRIPT, &[&restore.database, self.program])
            }
        }
    }
}

/// The file a restore into the new database `name` creates: next to the
/// `original` file, with its extension. `name` is a validated new database
/// name, so it is one path component.
///
/// The path is a POSIX one whatever the agent runs on: it names a file in a
/// pod, a container or a Unix host, so it is split at `/` by hand rather
/// than by the agent's own [`std::path::Path`] rules.
#[must_use]
pub fn new_database(original: &str, name: &str) -> String {
    let start = original
        .rfind('/')
        .map_or(0, |slash| slash.saturating_add(1));
    let (dir, file) = original.split_at(start);
    // An extension as `Path::extension` reads one: after the last dot, and
    // never a leading dot alone.
    let extension = file
        .rfind('.')
        .filter(|dot| *dot > 0)
        .and_then(|dot| file.get(dot..));
    format!("{dir}{name}{}", extension.unwrap_or_default())
}

/// Whether `file` can stand for a `SQLite` database: an absolute POSIX path
/// holding no `..`, so it names the same file wherever `sqlite3` starts.
#[must_use]
pub fn is_database_file(file: &str) -> bool {
    shared::database::is_absolute_posix_path(file)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use shared::database::{DatabaseRuntime, DatabaseSourceId, DumpFormat};

    use super::*;

    fn restoring(target: DatabaseRestoreTarget) -> DatabaseRestore {
        DatabaseRestore {
            source: DatabaseSourceId {
                runtime: DatabaseRuntime::Docker,
                namespace: String::new(),
                name: "vaultwarden".to_owned(),
            },
            database: "/data/db.sqlite3".to_owned(),
            archive_path: "docker/vaultwarden/%2Fdata%2Fdb%2Esqlite3.sqlite.sql".to_owned(),
            format: DumpFormat::SqliteSql,
            target,
        }
    }

    #[test]
    fn a_new_database_sits_next_to_the_original() {
        assert_eq!(
            new_database("/data/db.sqlite3", "copy"),
            "/data/copy.sqlite3"
        );
        assert_eq!(new_database("/srv/app/state", "copy"), "/srv/app/copy");
        assert_eq!(new_database("/a.b/app.tar.db", "x"), "/a.b/x.db");
        assert_eq!(new_database("/data/.state", "copy"), "/data/copy");
    }

    #[test]
    fn only_absolute_paths_without_parents_are_database_files() {
        assert!(is_database_file("/data/db.sqlite3"));
        for refused in ["data/db.sqlite3", "-x.db", "/data/../etc/shadow", ""] {
            assert!(!is_database_file(refused), "{refused}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_dump_reads_the_file_read_only() {
        let sqlite = Sqlite {
            program: "/opt/sqlite/bin/sqlite3",
        };
        assert_eq!(
            sqlite.dump("/data/db.sqlite3"),
            [
                "/opt/sqlite/bin/sqlite3",
                "-readonly",
                "/data/db.sqlite3",
                ".dump"
            ]
        );
    }

    #[test]
    fn a_restore_writes_a_new_file_or_restores_over_the_original() {
        let sqlite = Sqlite { program: "sqlite3" };
        assert_eq!(
            sqlite.restore(&restoring(DatabaseRestoreTarget::NewDatabase {
                name: "copy".to_owned()
            })),
            ["sqlite3", "-bail", "/data/copy.sqlite3"]
        );
        let original = sqlite.restore(&restoring(DatabaseRestoreTarget::Original));
        assert_eq!(original.first().map(String::as_str), Some(SHELL));
        assert_eq!(
            original.get(3..).unwrap(),
            ["sh", "/data/db.sqlite3", "sqlite3"]
        );
    }

    #[cfg(unix)]
    mod against_sqlite3 {
        //! Runs the scripts with the real `sqlite3`, which CI installs.

        use super::*;

        async fn sh(command: &[String], stdin: &str) -> std::process::Output {
            use tokio::io::AsyncWriteExt;
            let (program, args) = command.split_first().unwrap();
            let mut child = tokio::process::Command::new(program)
                .args(args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let mut input = child.stdin.take().unwrap();
            input.write_all(stdin.as_bytes()).await.unwrap();
            drop(input);
            child.wait_with_output().await.unwrap()
        }

        async fn files_in(dir: &Path) -> Vec<String> {
            let mut entries = tokio::fs::read_dir(dir).await.unwrap();
            let mut names = Vec::new();
            while let Some(entry) = entries.next_entry().await.unwrap() {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
            names
        }

        async fn query(file: &Path, sql: &str) -> String {
            let output = tokio::process::Command::new("sqlite3")
                .arg(file)
                .arg(sql)
                .output()
                .await
                .unwrap();
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }

        #[tokio::test]
        async fn a_dump_restores_over_the_original_and_into_a_new_file() {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("app.db");
            let file_str = file.to_string_lossy().into_owned();
            query(
                &file,
                "CREATE TABLE t(v TEXT); INSERT INTO t VALUES ('kept');",
            )
            .await;
            let sqlite = Sqlite { program: "sqlite3" };

            let dump = sh(&sqlite.dump(&file_str), "").await;
            assert!(dump.status.success());
            let dump = String::from_utf8(dump.stdout).unwrap();
            assert!(dump.contains("INSERT INTO t VALUES('kept');"), "{dump}");

            query(&file, "UPDATE t SET v = 'changed';").await;
            let mut restore = restoring(DatabaseRestoreTarget::Original);
            restore.database.clone_from(&file_str);
            let restored = sh(&sqlite.restore(&restore), &dump).await;
            assert!(restored.status.success(), "{restored:?}");
            assert_eq!(query(&file, "SELECT v FROM t;").await, "kept");
            assert_eq!(
                files_in(dir.path()).await,
                ["app.db"],
                "the scratch file is removed"
            );

            let new = DatabaseRestoreTarget::NewDatabase {
                name: "copy".to_owned(),
            };
            let copy = dir.path().join("copy.db");
            let copy_str = copy.to_string_lossy().into_owned();
            assert!(
                sh(&Sqlite::create_database(&copy_str), "")
                    .await
                    .status
                    .success()
            );
            assert!(
                !sh(&Sqlite::create_database(&copy_str), "")
                    .await
                    .status
                    .success(),
                "an existing file is never written over"
            );
            restore.target = new;
            assert!(sh(&sqlite.restore(&restore), &dump).await.status.success());
            assert_eq!(query(&copy, "SELECT v FROM t;").await, "kept");
            assert!(
                sh(&Sqlite::drop_database(&copy_str), "")
                    .await
                    .status
                    .success()
            );
            assert!(!tokio::fs::try_exists(&copy).await.unwrap());
        }

        #[tokio::test]
        async fn dropping_a_database_removes_its_journal_and_write_ahead_log() {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("copy.db");
            for suffix in ["", "-journal", "-wal", "-shm"] {
                let mut name = file.clone().into_os_string();
                name.push(suffix);
                tokio::fs::write(name, b"").await.unwrap();
            }
            tokio::fs::write(dir.path().join("app.db"), b"")
                .await
                .unwrap();

            let drop = Sqlite::drop_database(&file.to_string_lossy());
            assert!(sh(&drop, "").await.status.success());
            assert_eq!(files_in(dir.path()).await, ["app.db"]);
        }

        #[tokio::test]
        async fn a_dump_that_does_not_load_leaves_the_original_alone() {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("app.db");
            query(
                &file,
                "CREATE TABLE t(v TEXT); INSERT INTO t VALUES ('kept');",
            )
            .await;
            let mut restore = restoring(DatabaseRestoreTarget::Original);
            restore.database = file.to_string_lossy().into_owned();
            let sqlite = Sqlite { program: "sqlite3" };

            let broken = sh(&sqlite.restore(&restore), "CREATE TABLE t(;\n").await;
            assert!(!broken.status.success());
            assert_eq!(query(&file, "SELECT v FROM t;").await, "kept");
            assert_eq!(files_in(dir.path()).await, ["app.db"]);
        }
    }
}
