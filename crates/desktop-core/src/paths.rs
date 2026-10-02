// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::path::{Path, PathBuf};

const APP_DIR: &str = "assimilate";

/// The operating systems the desktop app supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// macOS: data lives under `~/Library/Application Support`.
    MacOs,
    /// Linux: data follows the XDG base directory spec.
    Linux,
}

impl Platform {
    /// The platform this binary was built for, or `None` where the desktop
    /// app isn't supported.
    #[must_use]
    pub const fn current() -> Option<Self> {
        if cfg!(target_os = "macos") {
            Some(Self::MacOs)
        } else if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else {
            None
        }
    }
}

/// Why the app's data directory couldn't be determined.
#[derive(Debug, thiserror::Error)]
pub enum PathsError {
    /// The desktop app doesn't run on this operating system.
    #[error("the desktop app is not supported on this operating system")]
    UnsupportedPlatform,
    /// The user's home directory couldn't be determined.
    #[error("could not determine the home directory")]
    NoHomeDirectory,
}

/// Every location the desktop app reads or writes, derived from one root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopPaths {
    root: PathBuf,
}

impl DesktopPaths {
    /// Resolves the data root for `platform`. On Linux, `$XDG_DATA_HOME` wins
    /// when it is an absolute path, as the XDG spec requires; anything else
    /// falls back to `~/.local/share`.
    #[must_use]
    pub fn for_platform(platform: Platform, home: &Path, xdg_data_home: Option<&Path>) -> Self {
        let base = match platform {
            Platform::MacOs => home.join("Library").join("Application Support"),
            Platform::Linux => xdg_data_home
                .filter(|dir| dir.is_absolute())
                .map_or_else(|| home.join(".local").join("share"), Path::to_path_buf),
        };
        Self {
            root: base.join(APP_DIR),
        }
    }

    /// Resolves the data root for the running user and platform.
    ///
    /// # Errors
    ///
    /// Fails on an unsupported platform or when the home directory is unknown.
    pub fn from_env() -> Result<Self, PathsError> {
        let platform = Platform::current().ok_or(PathsError::UnsupportedPlatform)?;
        let home = std::env::home_dir().ok_or(PathsError::NoHomeDirectory)?;
        let xdg_data_home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from);
        Ok(Self::for_platform(
            platform,
            &home,
            xdg_data_home.as_deref(),
        ))
    }

    /// Uses `root` directly, for tests and for a user-chosen location.
    #[must_use]
    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }

    /// The directory everything else lives under.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The `PostgreSQL` cluster's data directory.
    #[must_use]
    pub fn postgres_data(&self) -> PathBuf {
        self.root.join("postgres")
    }

    /// Where `PostgreSQL` puts its unix socket. Kept short on purpose: socket
    /// paths are limited to about 100 bytes, and the socket file name adds to it.
    #[must_use]
    pub fn postgres_socket_dir(&self) -> PathBuf {
        self.root.join("run")
    }

    /// The short-lived file `initdb` reads the superuser password from. It
    /// sits outside the data directory, which `initdb` requires to be empty,
    /// and is deleted as soon as the cluster exists.
    #[must_use]
    pub fn postgres_init_password_file(&self) -> PathBuf {
        self.root.join("initdb.pw")
    }

    /// The server's SSH key directory (`SSH_KEY_DIR`).
    #[must_use]
    pub fn ssh_keys(&self) -> PathBuf {
        self.root.join("ssh")
    }

    /// Where the app and its child processes write logs.
    #[must_use]
    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_uses_application_support() {
        let paths = DesktopPaths::for_platform(Platform::MacOs, Path::new("/Users/ada"), None);
        assert_eq!(
            paths.root(),
            Path::new("/Users/ada/Library/Application Support/assimilate")
        );
    }

    #[test]
    fn macos_ignores_xdg_data_home() {
        let paths = DesktopPaths::for_platform(
            Platform::MacOs,
            Path::new("/Users/ada"),
            Some(Path::new("/elsewhere")),
        );
        assert_eq!(
            paths.root(),
            Path::new("/Users/ada/Library/Application Support/assimilate")
        );
    }

    #[test]
    fn linux_defaults_to_local_share() {
        let paths = DesktopPaths::for_platform(Platform::Linux, Path::new("/home/ada"), None);
        assert_eq!(paths.root(), Path::new("/home/ada/.local/share/assimilate"));
    }

    #[test]
    fn linux_honours_an_absolute_xdg_data_home() {
        let paths = DesktopPaths::for_platform(
            Platform::Linux,
            Path::new("/home/ada"),
            Some(Path::new("/data/xdg")),
        );
        assert_eq!(paths.root(), Path::new("/data/xdg/assimilate"));
    }

    #[test]
    fn linux_ignores_a_relative_xdg_data_home() {
        let paths = DesktopPaths::for_platform(
            Platform::Linux,
            Path::new("/home/ada"),
            Some(Path::new("relative/xdg")),
        );
        assert_eq!(paths.root(), Path::new("/home/ada/.local/share/assimilate"));
    }

    #[test]
    fn every_location_sits_under_the_root_and_the_init_file_outside_the_data_dir() {
        let paths = DesktopPaths::at(PathBuf::from("/r"));
        let all = [
            paths.postgres_data(),
            paths.postgres_socket_dir(),
            paths.postgres_init_password_file(),
            paths.ssh_keys(),
            paths.logs(),
        ];
        assert!(all.iter().all(|path| path.starts_with("/r")));
        assert!(
            !paths
                .postgres_init_password_file()
                .starts_with(paths.postgres_data())
        );
    }

    #[test]
    fn from_env_resolves_on_supported_platforms() {
        let resolved = DesktopPaths::from_env();
        match Platform::current() {
            Some(_) => assert!(resolved.unwrap().root().ends_with(APP_DIR)),
            None => assert!(matches!(resolved, Err(PathsError::UnsupportedPlatform))),
        }
    }
}
