// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Mutex,
};

use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// The app's own log, next to the server's and the agent's.
const LOG_FILE_NAME: &str = "desktop.log";

/// Where the app writes its own log inside `logs_dir`.
pub fn log_file(logs_dir: &Path) -> PathBuf {
    logs_dir.join(LOG_FILE_NAME)
}

/// Opens the app's log for appending, creating `logs_dir` first.
///
/// # Errors
///
/// Fails if the directory can't be created or the file can't be opened.
pub async fn open_log_file(logs_dir: &Path) -> std::io::Result<File> {
    tokio::fs::create_dir_all(logs_dir).await?;
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file(logs_dir))
        .await?;
    Ok(file.into_std().await)
}

/// Whether the log also goes to stdout: always in debug builds, and in
/// release builds only when there is no log file to write to, so nothing is
/// lost silently.
const fn log_to_stdout(debug_build: bool, has_file: bool) -> bool {
    debug_build || !has_file
}

/// Sends the app's `tracing` output to `desktop.log` in `logs_dir` and, see
/// [`log_to_stdout`], to stdout. A packaged build has no console, so the
/// file is the only place its log survives.
pub fn init(logs_dir: Option<&Path>) {
    let opened = logs_dir.map(|dir| tauri::async_runtime::block_on(open_log_file(dir)));
    let (file, open_error) = match opened {
        Some(Ok(file)) => (Some(file), None),
        Some(Err(e)) => (None, Some(e)),
        None => (None, None),
    };
    let stdout = log_to_stdout(cfg!(debug_assertions), file.is_some());
    let file_layer = file.map(|file| fmt::layer().with_ansi(false).with_writer(Mutex::new(file)));
    let stdout_layer = stdout.then(fmt::layer);
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(file_layer)
        .with(stdout_layer)
        .init();
    if let (Some(error), Some(dir)) = (open_error, logs_dir) {
        tracing::warn!(
            %error,
            path = %log_file(dir).display(),
            "could not open the log file; logging to stdout only",
        );
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tauri::async_runtime::block_on;

    use super::*;
    use crate::test_support::ScratchDir;

    #[test]
    fn the_log_file_sits_in_the_logs_directory() {
        assert_eq!(
            log_file(Path::new("/data/assimilate/logs")),
            PathBuf::from("/data/assimilate/logs/desktop.log")
        );
    }

    #[test]
    fn opening_creates_the_logs_directory_and_appends() {
        let root = ScratchDir::new("logging");
        let logs = root.path().join("nested").join("logs");

        writeln!(block_on(open_log_file(&logs)).unwrap(), "first").unwrap();
        writeln!(block_on(open_log_file(&logs)).unwrap(), "second").unwrap();

        let written = block_on(tokio::fs::read_to_string(log_file(&logs))).unwrap();
        assert_eq!(written, "first\nsecond\n");
    }

    #[test]
    fn opening_fails_when_the_logs_directory_cannot_be_created() {
        let root = ScratchDir::new("logging-blocked");
        let blocker = root.path().join("not-a-dir");
        block_on(tokio::fs::write(&blocker, b"")).unwrap();

        assert!(block_on(open_log_file(&blocker.join("logs"))).is_err());
    }

    #[test]
    fn stdout_is_kept_in_debug_builds_and_when_there_is_no_file() {
        assert!(log_to_stdout(true, true));
        assert!(log_to_stdout(true, false));
        assert!(log_to_stdout(false, false));
        assert!(!log_to_stdout(false, true));
    }
}
