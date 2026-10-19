// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use postgresql_embedded::{PostgreSQL, Settings, SettingsBuilder};
use tokio::io::AsyncWriteExt;

use crate::{paths::DesktopPaths, secrets::Secret};

/// The database the Assimilate server uses inside the embedded cluster.
pub const DATABASE_NAME: &str = "assimilate";

/// How long `pg_ctl` may take to start or stop the cluster. The library's
/// 5-second default is too tight for a first `initdb` on a slow disk.
const PG_CTL_TIMEOUT: Duration = Duration::from_mins(1);

/// Why the embedded `PostgreSQL` instance couldn't be prepared, started or stopped.
#[derive(Debug, thiserror::Error)]
pub enum PostgresError {
    /// Reading or writing the app's files failed.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// The cluster couldn't be initialised, started, stopped or queried.
    #[error("embedded postgres error: {0}")]
    Embedded(#[from] postgresql_embedded::Error),
    /// The configured installation has no `bin/initdb`, so the app was
    /// bundled or configured without `PostgreSQL`.
    #[error("no PostgreSQL installation found at {}: bin/initdb is missing", .0.display())]
    NotInstalled(PathBuf),
    /// `pg_ctl start` failed; its own error says little, the server's log
    /// says why.
    #[error("PostgreSQL did not start ({error}); its log ends with:\n{log}")]
    NotStarted {
        /// What `pg_ctl` reported.
        error: postgresql_embedded::Error,
        /// The end of the server's start log.
        log: String,
    },
}

/// What the embedded instance needs: where its binaries are and how to log in.
#[derive(Debug, Clone)]
pub struct PostgresConfig {
    /// Directory holding `PostgreSQL`'s `bin/` (`initdb`, `pg_ctl`, `postgres`),
    /// shipped with the app. Nothing is ever downloaded at runtime.
    pub install_dir: PathBuf,
    /// The superuser password, kept in the OS keychain.
    pub superuser_password: Secret,
    /// Names the socket file `.s.PGSQL.<port>`; no TCP port is opened. On
    /// Windows, the loopback TCP port the cluster listens on instead.
    pub port: u16,
}

impl PostgresConfig {
    fn settings(&self, paths: &DesktopPaths) -> Settings {
        let builder = SettingsBuilder::new()
            .installation_dir(&self.install_dir)
            .trust_installation_dir(true)
            .data_dir(paths.postgres_data())
            .password_file(paths.postgres_init_password_file())
            .password(self.superuser_password.expose())
            .port(self.port)
            // Without this the library deletes the data directory on drop.
            .temporary(false)
            .timeout(Some(PG_CTL_TIMEOUT));
        #[cfg(unix)]
        let builder = listen_privately(builder, paths);
        #[cfg(windows)]
        let builder = listen_privately(builder);
        builder.build()
    }
}

/// Unix socket only: other local users and processes can't reach the
/// database over TCP, and the socket sits in the app's own data folder.
#[cfg(unix)]
fn listen_privately(builder: SettingsBuilder, paths: &DesktopPaths) -> SettingsBuilder {
    builder
        .socket_dir(paths.postgres_socket_dir())
        .config("listen_addresses", "''")
}

/// The server's database driver has no unix sockets on Windows, so the
/// cluster listens on loopback TCP, on a port picked fresh at each start.
/// Other local programs can connect to it, so every connection needs the
/// superuser password: the library runs `initdb --auth=password`, and the
/// password only exists in the keychain and the server's environment.
#[cfg(windows)]
fn listen_privately(builder: SettingsBuilder) -> SettingsBuilder {
    // An explicit address, not `localhost`, which may resolve to `::1` first.
    // Unquoted: pg_ctl starts the server through cmd.exe, which, unlike sh,
    // passes single quotes on as part of the value.
    builder
        .host("127.0.0.1")
        .config("listen_addresses", "127.0.0.1")
}

/// A running embedded `PostgreSQL` cluster holding the server's database.
pub struct EmbeddedPostgres {
    instance: PostgreSQL,
}

impl EmbeddedPostgres {
    /// Initialises the cluster on first run, starts it, and creates the
    /// server's database if it doesn't exist yet.
    ///
    /// # Errors
    ///
    /// Fails if the cluster can't be initialised or started, or the database
    /// can't be created.
    pub async fn start(
        paths: &DesktopPaths,
        config: &PostgresConfig,
    ) -> Result<Self, PostgresError> {
        let initdb = config
            .install_dir
            .join("bin")
            .join(format!("initdb{}", std::env::consts::EXE_SUFFIX));
        // An error here (say, no permission to read the directory) is
        // reported as such, not as a missing installation.
        if !tokio::fs::try_exists(&initdb).await? {
            not_a_directory(&config.install_dir).await?;
            not_a_directory(&config.install_dir.join("bin")).await?;
            return Err(PostgresError::NotInstalled(config.install_dir.clone()));
        }
        tokio::fs::create_dir_all(paths.root()).await?;
        let settings = config.settings(paths);
        let password_file = settings.password_file.clone();
        write_private_file(&password_file, config.superuser_password.expose()).await?;

        let mut instance = PostgreSQL::new(settings);
        let setup = instance.setup().await;
        // The password lives in the keychain; never leave a copy on disk,
        // whether or not initdb succeeded.
        remove_if_present(&password_file).await?;
        setup?;

        if let Err(error) = instance.start().await {
            let log = start_log_tail(&paths.postgres_data().join(START_LOG)).await;
            return Err(PostgresError::NotStarted { error, log });
        }
        if !instance.database_exists(DATABASE_NAME).await? {
            instance.create_database(DATABASE_NAME).await?;
        }
        Ok(Self { instance })
    }

    /// The `DATABASE_URL` for the server. It carries the superuser password,
    /// so it is a [`Secret`]: pass it through the environment, never argv.
    #[must_use]
    pub fn database_url(&self) -> Secret {
        Secret::from_stored(self.instance.settings().url(DATABASE_NAME))
    }

    /// Stops the cluster, waiting for it to shut down.
    ///
    /// # Errors
    ///
    /// Fails if `pg_ctl stop` fails.
    pub async fn stop(self) -> Result<(), PostgresError> {
        self.instance.stop().await?;
        Ok(())
    }
}

/// Writes `contents` to a new file only the current user can read (and, on
/// Windows, Administrators and `LocalSystem`). Creating
/// it ourselves first means the library finds it already there and never
/// writes the password with default permissions.
async fn write_private_file(path: &Path, contents: &str) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    let mut file = {
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true).mode(0o600);
        options.open(path).await?
    };
    // A protected DACL rather than whatever the folder passes on: the
    // current user, Administrators and LocalSystem.
    #[cfg(windows)]
    let mut file = tokio::fs::File::from_std(
        shared::windows_acl::create_private_file(
            path,
            shared::windows_acl::Access::SystemAdministratorsAndOwner,
        )
        .await?,
    );
    file.write_all(contents.as_bytes()).await?;
    file.flush().await
}

/// Where `pg_ctl start` sends the server's log, in the data directory.
const START_LOG: &str = "start.log";
/// How much of the end of [`START_LOG`] a start failure carries.
const START_LOG_TAIL_BYTES: usize = 4096;

/// The end of the server's start log, or why it can't be read.
async fn start_log_tail(path: &Path) -> String {
    match tokio::fs::read(path).await {
        Ok(log) => {
            let start = log.len().saturating_sub(START_LOG_TAIL_BYTES);
            String::from_utf8_lossy(log.get(start..).unwrap_or_default())
                .trim()
                .to_owned()
        }
        Err(error) => format!("({} could not be read: {error})", path.display()),
    }
}

async fn remove_if_present(path: &Path) -> Result<(), std::io::Error> {
    match tokio::fs::remove_file(path).await {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Fails when `dir` exists as something other than a directory. Unix already
/// reports looking below such a file as "not a directory", but Windows calls
/// it "not found", which would pass for a missing installation.
async fn not_a_directory(dir: &Path) -> std::io::Result<()> {
    match tokio::fs::metadata(dir).await {
        Ok(metadata) if !metadata.is_dir() => Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            format!("{} is not a directory", dir.display()),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> PostgresConfig {
        PostgresConfig {
            install_dir: PathBuf::from("/opt/pg"),
            superuser_password: Secret::from_stored("pw".to_string()),
            port: 54_321,
        }
    }

    #[cfg(unix)]
    #[test]
    fn settings_keep_data_and_listen_on_the_socket_only() {
        let paths = DesktopPaths::at(PathBuf::from("/app"));
        let settings = config().settings(&paths);

        assert!(!settings.temporary);
        assert!(settings.trust_installation_dir);
        assert_eq!(settings.data_dir, paths.postgres_data());
        assert_eq!(settings.socket_dir, Some(paths.postgres_socket_dir()));
        assert_eq!(settings.password_file, paths.postgres_init_password_file());
        assert_eq!(
            settings
                .configuration
                .get("listen_addresses")
                .map(String::as_str),
            Some("''")
        );
    }

    #[cfg(unix)]
    #[test]
    fn database_url_points_at_the_socket() {
        let paths = DesktopPaths::at(PathBuf::from("/app"));
        let url = config().settings(&paths).url(DATABASE_NAME);

        assert!(url.contains("/assimilate?host=%2Fapp%2Frun"), "{url}");
        assert!(url.contains(":54321/"), "{url}");
    }

    #[cfg(windows)]
    #[test]
    fn settings_keep_data_and_listen_on_loopback_only() {
        let paths = DesktopPaths::at(PathBuf::from(r"C:\app"));
        let settings = config().settings(&paths);

        assert!(!settings.temporary);
        assert_eq!(settings.data_dir, paths.postgres_data());
        assert_eq!(settings.socket_dir, None);
        assert_eq!(
            settings
                .configuration
                .get("listen_addresses")
                .map(String::as_str),
            // Unquoted, as cmd.exe passes quotes on (see listen_privately).
            Some("127.0.0.1")
        );
    }

    #[cfg(windows)]
    #[test]
    fn database_url_points_at_the_loopback_port_with_the_password() {
        let paths = DesktopPaths::at(PathBuf::from(r"C:\app"));
        let url = config().settings(&paths).url(DATABASE_NAME);

        assert!(url.contains(":pw@"), "{url}");
        assert!(url.contains("@127.0.0.1:54321/assimilate"), "{url}");
        assert!(!url.contains("host="), "{url}");
    }

    #[tokio::test]
    async fn start_names_a_missing_installation() {
        let root = tempfile::tempdir().unwrap();
        let paths = DesktopPaths::at(root.path().to_path_buf());
        let config = PostgresConfig {
            install_dir: root.path().join("no-postgres-here"),
            ..config()
        };

        let result = EmbeddedPostgres::start(&paths, &config).await;

        assert_eq!(
            result.err().map(|e| e.to_string()),
            Some(format!(
                "no PostgreSQL installation found at {}: bin/initdb is missing",
                config.install_dir.display()
            ))
        );
        assert!(
            !tokio::fs::try_exists(paths.postgres_init_password_file())
                .await
                .unwrap(),
            "nothing may be written before the installation is found"
        );
    }

    #[tokio::test]
    async fn a_start_failure_carries_the_end_of_the_server_log() {
        let root = tempfile::tempdir().unwrap();
        let log = root.path().join(START_LOG);
        let mut text = "x".repeat(START_LOG_TAIL_BYTES);
        text.push_str("\nFATAL: the reason\n");
        tokio::fs::write(&log, text).await.unwrap();

        let tail = start_log_tail(&log).await;

        assert!(tail.ends_with("FATAL: the reason"), "{tail}");
        assert_eq!(tail.len(), START_LOG_TAIL_BYTES - 1);
        let missing = start_log_tail(&root.path().join("missing.log")).await;
        assert!(missing.contains("could not be read"), "{missing}");
    }

    #[tokio::test]
    async fn only_something_other_than_a_directory_is_not_a_directory() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("file");
        tokio::fs::write(&file, b"").await.unwrap();

        let error = not_a_directory(&file).await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotADirectory);
        assert!(not_a_directory(root.path()).await.is_ok());
        assert!(not_a_directory(&root.path().join("missing")).await.is_ok());
        // A path the OS can't even look up is its own error.
        let error = not_a_directory(Path::new("nul\0byte")).await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[tokio::test]
    async fn an_unreadable_installation_is_not_reported_as_missing() {
        let root = tempfile::tempdir().unwrap();
        let paths = DesktopPaths::at(root.path().join("data"));
        // A file where the installation directory should be: looking for
        // bin/initdb inside it fails with "not a directory", not "not found".
        let install_dir = root.path().join("postgres");
        tokio::fs::write(&install_dir, b"").await.unwrap();
        let config = PostgresConfig {
            install_dir,
            ..config()
        };

        let result = EmbeddedPostgres::start(&paths, &config).await;

        assert!(
            matches!(result, Err(PostgresError::Io(_))),
            "{:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn the_private_file_is_owner_only_and_removal_tolerates_absence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pw");

        write_private_file(&path, "secret").await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = tokio::fs::metadata(&path)
                .await
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "secret");

        remove_if_present(&path).await.unwrap();
        remove_if_present(&path).await.unwrap();
        assert!(!tokio::fs::try_exists(&path).await.unwrap());
    }
}
