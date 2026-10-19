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
    /// Windows: data lives under `%LOCALAPPDATA%`, which isn't synced to
    /// other machines with a roaming profile.
    Windows,
}

impl Platform {
    /// The environment variable naming this platform's per-user data
    /// directory, which [`DesktopPaths::for_platform`] takes as `data_home`.
    #[must_use]
    pub const fn data_home_var(self) -> Option<&'static str> {
        match self {
            Self::MacOs => None,
            Self::Linux => Some("XDG_DATA_HOME"),
            Self::Windows => Some("LOCALAPPDATA"),
        }
    }

    /// The platform this binary was built for, or `None` where the desktop
    /// app isn't supported.
    #[must_use]
    pub const fn current() -> Option<Self> {
        CURRENT_PLATFORM
    }
}

// Chosen at compile time, so a build contains only its own platform's value.
#[cfg(target_os = "macos")]
const CURRENT_PLATFORM: Option<Platform> = Some(Platform::MacOs);
#[cfg(target_os = "linux")]
const CURRENT_PLATFORM: Option<Platform> = Some(Platform::Linux);
#[cfg(windows)]
const CURRENT_PLATFORM: Option<Platform> = Some(Platform::Windows);
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
const CURRENT_PLATFORM: Option<Platform> = None;

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
    /// Resolves the data root for `platform`. `data_home` is the platform's
    /// own data directory variable, if set: `$XDG_DATA_HOME` on Linux,
    /// `%LOCALAPPDATA%` on Windows, nothing on macOS. It wins when it is an
    /// absolute path, as the XDG spec requires; anything else falls back to
    /// `~/.local/share` or `~\AppData\Local`.
    #[must_use]
    pub fn for_platform(platform: Platform, home: &Path, data_home: Option<&Path>) -> Self {
        // `has_root` is `is_absolute` on Unix; on Windows it also accepts a
        // drive-relative `\path`, which `%LOCALAPPDATA%` never is.
        let absolute = data_home.filter(|dir| dir.has_root());
        let base = match platform {
            Platform::MacOs => home.join("Library").join("Application Support"),
            Platform::Linux => {
                absolute.map_or_else(|| home.join(".local").join("share"), Path::to_path_buf)
            }
            Platform::Windows => {
                absolute.map_or_else(|| home.join("AppData").join("Local"), Path::to_path_buf)
            }
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
        let data_home = platform
            .data_home_var()
            .and_then(std::env::var_os)
            .map(PathBuf::from);
        Ok(Self::for_platform(platform, &home, data_home.as_deref()))
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
    /// Unused on Windows, where the cluster listens on loopback TCP instead.
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
    fn each_platform_names_its_own_data_directory_variable() {
        assert_eq!(Platform::MacOs.data_home_var(), None);
        assert_eq!(Platform::Linux.data_home_var(), Some("XDG_DATA_HOME"));
        assert_eq!(Platform::Windows.data_home_var(), Some("LOCALAPPDATA"));
    }

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
    fn windows_defaults_to_appdata_local() {
        let paths = DesktopPaths::for_platform(Platform::Windows, Path::new("/Users/ada"), None);
        assert_eq!(
            paths.root(),
            Path::new("/Users/ada/AppData/Local/assimilate")
        );
    }

    #[test]
    fn windows_honours_an_absolute_local_app_data() {
        let paths = DesktopPaths::for_platform(
            Platform::Windows,
            Path::new("/Users/ada"),
            Some(Path::new("/profiles/ada/local")),
        );
        assert_eq!(paths.root(), Path::new("/profiles/ada/local/assimilate"));
    }

    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    #[test]
    fn from_env_resolves_on_supported_platforms() {
        assert!(Platform::current().is_some());
        assert!(DesktopPaths::from_env().unwrap().root().ends_with(APP_DIR));
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    #[test]
    fn from_env_refuses_unsupported_platforms() {
        assert_eq!(Platform::current(), None);
        assert!(matches!(
            DesktopPaths::from_env(),
            Err(PathsError::UnsupportedPlatform)
        ));
    }
}
