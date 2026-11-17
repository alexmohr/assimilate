// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Running the agent as a macOS launchd daemon: `service install` writes the
//! daemon's property list to `/Library/LaunchDaemons` and its server URL,
//! token and server key to a file only root can read, then loads it;
//! `service uninstall` unloads it and removes both; `service run` is what
//! launchd starts.
//!
//! launchd starts the agent again after every exit, which is how the
//! dashboard's "Restart agent" works, as under systemd. `launchctl` is the
//! only interface launchd offers for loading a daemon, so this runs it.

use std::{
    os::unix::fs::{MetadataExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
    process::{ExitCode, Stdio},
};

use futures_util::future;
use tokio::{fs, io::AsyncWriteExt as _, process::Command as Process};
use tracing_subscriber::EnvFilter;

use crate::{
    launchd_plist::{self, DaemonPaths, LABEL},
    service_config::{Action, Command, ConfigError, StoredConfig},
    systemd::RestartCapability,
};

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{context}: {source}")]
    Io {
        context: String,
        source: std::io::Error,
    },
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("installing or removing the service needs root: run it with sudo")]
    NotRoot,
    #[error(
        "{} runs as root, so it and every folder above it must be owned by root and writable by \
         no one else; install the agent somewhere like /Library/Assimilate with sudo",
        path.display()
    )]
    UntrustedExecutable { path: PathBuf },
    #[error(
        "{} must be owned by root and closed to everyone else, because it holds the agent's \
         token; remove it and run `service install` again",
        path.display()
    )]
    NotRootOnly { path: PathBuf },
    #[error("`launchctl {args}` failed ({status}): {stderr}")]
    Launchctl {
        args: String,
        status: std::process::ExitStatus,
        stderr: String,
    },
}

impl ServiceError {
    fn io(context: impl Into<String>) -> impl FnOnce(std::io::Error) -> Self {
        let context = context.into();
        move |source| Self::Io { context, source }
    }
}

/// Runs one `service` subcommand to completion. The exit code is the
/// agent's own after `service run`, so launchd sees a fatal stop as one.
///
/// # Errors
///
/// Fails if the daemon can't be installed, removed or run, or its
/// configuration can't be written or read.
pub fn run(command: Command) -> Result<ExitCode, ServiceError> {
    let Command::Service { action } = command;
    let paths = DaemonPaths::system();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_ansi(false)
        .init();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(ServiceError::io("failed to start the async runtime"))?;
    match action {
        Action::Install {
            server_url,
            server_key,
        } => {
            let config = StoredConfig::install(server_url, &server_key, crate::env_token())?;
            runtime
                .block_on(install(&paths, &config))
                .map(|()| ExitCode::SUCCESS)
        }
        Action::Uninstall => runtime
            .block_on(uninstall(&paths))
            .map(|()| ExitCode::SUCCESS),
        Action::Run => runtime.block_on(run_daemon(&paths)),
    }
}

/// The agent as launchd runs it. Everything it logs goes to the log file
/// launchd was told to write standard output and error to.
async fn run_daemon(paths: &DaemonPaths) -> Result<ExitCode, ServiceError> {
    tracing::info!(version = crate::agent_version_string(), "service starting");
    let config_path = paths.config();
    let json = fs::read(&config_path)
        .await
        .map_err(ServiceError::io(format!(
            "reading {}",
            config_path.display()
        )))?;
    let args = StoredConfig::from_json(&json, &config_path)?.endpoint(&config_path)?;
    let restart = RestartCapability {
        supported: true,
        unavailable_reason: None,
    };
    // A fatal stop (say, a rejected token) exits with a failure, as
    // systemd's Restart=on-failure would act on it.
    let fatal = crate::run(args, crate::shutdown_signal(), restart).await;
    Ok(ExitCode::from(u8::from(fatal)))
}

async fn install(paths: &DaemonPaths, config: &StoredConfig) -> Result<(), ServiceError> {
    if !nix::unistd::geteuid().is_root() {
        return Err(ServiceError::NotRoot);
    }
    let exe = fs::canonicalize(
        std::env::current_exe().map_err(ServiceError::io("locating the agent executable"))?,
    )
    .await
    .map_err(ServiceError::io("locating the agent executable"))?;
    require_root_owned_executable(&exe).await?;

    create_private_dir(paths.support_dir()).await?;
    let config_path = paths.config();
    write_private(&config_path, &config.to_json()?, 0o600).await?;
    create_private(paths.log(), 0o600).await?;
    let plist = launchd_plist::render(&exe, paths.log());
    write_private(&paths.plist(), plist.as_bytes(), 0o644).await?;

    // Load the new configuration: unload the daemon first if it is already
    // there, which fails harmlessly when it isn't.
    launchctl_bootout().await;
    let plist_path = paths.plist();
    let plist_path = plist_path.to_string_lossy();
    launchctl(&["bootstrap", "system", plist_path.as_ref()]).await?;
    tracing::info!("installed and started the {LABEL} daemon");
    Ok(())
}

async fn uninstall(paths: &DaemonPaths) -> Result<(), ServiceError> {
    if !nix::unistd::geteuid().is_root() {
        return Err(ServiceError::NotRoot);
    }
    launchctl_bootout().await;
    remove(&paths.plist()).await?;
    remove(&paths.config()).await?;
    tracing::info!("removed the {LABEL} daemon");
    Ok(())
}

/// `launchctl bootout` for the daemon, which fails when it isn't loaded.
async fn launchctl_bootout() {
    let target = format!("system/{LABEL}");
    if let Err(e) = launchctl(&["bootout", &target]).await {
        tracing::debug!(error = %e, "the daemon was not loaded");
    }
}

async fn launchctl(args: &[&str]) -> Result<(), ServiceError> {
    let output = Process::new("launchctl")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(ServiceError::io("running launchctl"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(ServiceError::Launchctl {
        args: args.join(" "),
        status: output.status,
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}

/// Refuses an executable that anyone but root could replace, or that sits
/// below a folder anyone but root could replace, since launchd runs it as
/// root.
async fn require_root_owned_executable(exe: &Path) -> Result<(), ServiceError> {
    let chain = future::try_join_all(exe.ancestors().map(|path| async move {
        let metadata = fs::metadata(path).await.map_err(ServiceError::io(format!(
            "reading the permissions of {}",
            path.display()
        )))?;
        Ok::<_, ServiceError>((path.to_path_buf(), metadata.uid(), metadata.mode()))
    }))
    .await?;
    launchd_plist::first_untrusted(&chain).map_or(Ok(()), |path| {
        Err(ServiceError::UntrustedExecutable {
            path: path.to_path_buf(),
        })
    })
}

/// Creates `dir` and its missing parents, with `dir` itself only root can
/// open.
async fn create_private_dir(dir: &Path) -> Result<(), ServiceError> {
    let context = format!("creating {}", dir.display());
    fs::create_dir_all(dir)
        .await
        .map_err(ServiceError::io(context.clone()))?;
    fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .await
        .map_err(ServiceError::io(context.clone()))?;
    // A folder someone else made first stays theirs whatever its mode is
    // set to, and a link would send the token elsewhere.
    let metadata = fs::symlink_metadata(dir)
        .await
        .map_err(ServiceError::io(context))?;
    if !metadata.is_dir() || !launchd_plist::only_root_can_open(metadata.uid(), metadata.mode()) {
        return Err(ServiceError::NotRootOnly {
            path: dir.to_path_buf(),
        });
    }
    Ok(())
}

/// What opening a private file does to one that is already there.
#[derive(Clone, Copy)]
enum Existing {
    /// Empty it: its contents are being replaced.
    Replace,
    /// Leave its contents alone.
    Keep,
}

/// Opens `path` for writing with permission bits `mode`, which also reset
/// those of a file already there. A symbolic link at `path` is refused rather
/// than followed.
async fn open_private(
    path: &Path,
    mode: u32,
    existing: Existing,
) -> Result<fs::File, ServiceError> {
    let context = format!("writing {}", path.display());
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .await
            .map_err(ServiceError::io(context.clone()))?;
    }
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .mode(mode)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(path)
        .await
        .map_err(ServiceError::io(context.clone()))?;
    // A file someone else made first stays theirs, whatever its mode is set
    // to: refuse it before anything secret is written, and before emptying
    // it, which is why the open doesn't truncate.
    let metadata = file
        .metadata()
        .await
        .map_err(ServiceError::io(context.clone()))?;
    if metadata.uid() != 0 {
        return Err(ServiceError::NotRootOnly {
            path: path.to_path_buf(),
        });
    }
    // `mode` only applies to a file that is new.
    file.set_permissions(std::fs::Permissions::from_mode(mode))
        .await
        .map_err(ServiceError::io(context.clone()))?;
    if matches!(existing, Existing::Replace) {
        file.set_len(0).await.map_err(ServiceError::io(context))?;
    }
    Ok(file)
}

/// Writes `contents` to `path`, replacing what is there.
async fn write_private(path: &Path, contents: &[u8], mode: u32) -> Result<(), ServiceError> {
    let context = format!("writing {}", path.display());
    let mut file = open_private(path, mode, Existing::Replace).await?;
    file.write_all(contents)
        .await
        .map_err(ServiceError::io(context.clone()))?;
    file.flush().await.map_err(ServiceError::io(context))
}

/// Creates `path` if it isn't there, and keeps what is in it if it is: the
/// log outlives every `service install`.
async fn create_private(path: &Path, mode: u32) -> Result<(), ServiceError> {
    open_private(path, mode, Existing::Keep).await.map(drop)
}

async fn remove(path: &Path) -> Result<(), ServiceError> {
    match fs::remove_file(path).await {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(ServiceError::io(format!("removing {}", path.display()))(e))
        }
        Ok(()) | Err(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    fn is_root() -> bool {
        nix::unistd::geteuid().is_root()
    }

    #[tokio::test]
    async fn removing_a_file_that_is_not_there_is_fine() {
        let dir = tempfile::tempdir().unwrap();

        assert!(remove(&dir.path().join("missing")).await.is_ok());
    }

    #[tokio::test]
    async fn a_symbolic_link_is_refused_instead_of_followed() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        tokio::fs::write(&target, b"keep").await.unwrap();
        let link = dir.path().join("link");
        symlink(&target, &link).unwrap();

        let result = write_private(&link, b"secret", 0o600).await;

        assert!(matches!(result, Err(ServiceError::Io { .. })), "{result:?}");
        assert_eq!(tokio::fs::read(&target).await.unwrap(), b"keep");
    }

    /// Whoever else made the file keeps it, so it is refused before anything
    /// is written to it or taken out of it.
    #[tokio::test]
    async fn a_file_someone_else_owns_is_refused_and_left_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.json");
        tokio::fs::write(&path, b"keep").await.unwrap();

        let result = write_private(&path, b"secret", 0o600).await;

        // A test run as root owns the file it just made, which is fine.
        if is_root() {
            assert!(result.is_ok(), "{result:?}");
        } else {
            assert!(
                matches!(result, Err(ServiceError::NotRootOnly { path: ref refused }) if *refused == path),
                "{result:?}"
            );
            assert_eq!(tokio::fs::read(&path).await.unwrap(), b"keep");
        }
    }

    #[tokio::test]
    async fn a_folder_someone_else_owns_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("agent");

        let result = create_private_dir(&folder).await;

        if is_root() {
            assert!(result.is_ok(), "{result:?}");
        } else {
            assert!(
                matches!(result, Err(ServiceError::NotRootOnly { .. })),
                "{result:?}"
            );
        }
    }

    #[tokio::test]
    async fn an_executable_below_a_folder_anyone_can_write_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("assimilate-agent");
        tokio::fs::write(&exe, b"").await.unwrap();

        let result = require_root_owned_executable(&exe).await;

        // The test's folder is under the system's temp folder, which anyone
        // can write, and is the test user's own unless that is root.
        assert!(
            matches!(result, Err(ServiceError::UntrustedExecutable { .. })),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn the_log_is_created_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("agent.log");

        let result = create_private(&log, 0o600).await;

        // Made by the test user, so only root's own test run accepts it.
        assert_eq!(result.is_ok(), is_root(), "{result:?}");
        assert!(log.exists());
    }
}
