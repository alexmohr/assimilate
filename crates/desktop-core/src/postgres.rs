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
}

/// What the embedded instance needs: where its binaries are and how to log in.
#[derive(Debug, Clone)]
pub struct PostgresConfig {
    /// Directory holding `PostgreSQL`'s `bin/` (`initdb`, `pg_ctl`, `postgres`),
    /// shipped with the app. Nothing is ever downloaded at runtime.
    pub install_dir: PathBuf,
    /// The superuser password, kept in the OS keychain.
    pub superuser_password: Secret,
    /// Names the socket file `.s.PGSQL.<port>`. No TCP port is opened.
    pub port: u16,
}

impl PostgresConfig {
    fn settings(&self, paths: &DesktopPaths) -> Settings {
        SettingsBuilder::new()
            .installation_dir(&self.install_dir)
            .trust_installation_dir(true)
            .data_dir(paths.postgres_data())
            .socket_dir(paths.postgres_socket_dir())
            .password_file(paths.postgres_init_password_file())
            .password(self.superuser_password.expose())
            .port(self.port)
            // Without this the library deletes the data directory on drop.
            .temporary(false)
            .timeout(Some(PG_CTL_TIMEOUT))
            // Unix socket only: other local users and processes can't reach
            // the database over TCP.
            .config("listen_addresses", "''")
            .build()
    }
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

        instance.start().await?;
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

/// Writes `contents` to a new file only the current user can read. Creating
/// it ourselves first means the library finds it already there and never
/// writes the password with default permissions.
async fn write_private_file(path: &Path, contents: &str) -> Result<(), std::io::Error> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).await?;
    file.write_all(contents.as_bytes()).await?;
    file.flush().await
}

async fn remove_if_present(path: &Path) -> Result<(), std::io::Error> {
    match tokio::fs::remove_file(path).await {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
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

    #[test]
    fn database_url_points_at_the_socket() {
        let paths = DesktopPaths::at(PathBuf::from("/app"));
        let url = config().settings(&paths).url(DATABASE_NAME);

        assert!(url.contains("/assimilate?host=%2Fapp%2Frun"), "{url}");
        assert!(url.contains(":54321/"), "{url}");
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
