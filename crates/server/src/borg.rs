// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

#[cfg(test)]
use std::sync::{Mutex, OnceLock};
use std::{
    collections::HashMap,
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Stdio,
    time::Instant,
};

use tokio::process::{Child, Command};

#[cfg(test)]
static TEST_BINARY_OVERRIDE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn log_run_result(
    subcommand: &str,
    elapsed_ms: u128,
    result: &std::io::Result<std::process::Output>,
) {
    match result {
        Ok(out) => {
            let exit_code = out.status.code().unwrap_or(-1);
            if exit_code >= 2 {
                let stderr = String::from_utf8_lossy(&out.stderr);
                tracing::warn!(subcommand, exit_code, elapsed_ms, %stderr, "borg: non-zero exit");
            } else {
                tracing::info!(subcommand, exit_code, elapsed_ms, "borg: done");
            }
        }
        Err(e) => tracing::error!(subcommand, error = %e, "borg: failed to spawn"),
    }
}

/// Wrapper around the borg binary that provides structured logging for every invocation.
pub struct Borg {
    binary: PathBuf,
}

impl Default for Borg {
    fn default() -> Self {
        Self::new()
    }
}

impl Borg {
    pub fn new() -> Self {
        #[cfg(test)]
        if let Some(binary) = test_binary_override() {
            return Self { binary };
        }

        Self {
            binary: std::env::var("BORG_BINARY")
                .map_or_else(|_| PathBuf::from("borg"), PathBuf::from),
        }
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }

    /// Run borg and wait for it to finish, logging subcommand, exit code, and elapsed time.
    pub async fn run<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &HashMap<String, String>,
    ) -> std::io::Result<std::process::Output> {
        let subcommand = args
            .first()
            .map(|a| a.as_ref().to_string_lossy().into_owned())
            .unwrap_or_else(|| "<none>".to_owned());
        tracing::info!(subcommand, "borg: starting");
        let start = Instant::now();
        let result = Command::new(&self.binary)
            .args(args)
            .envs(env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Kill borg if the caller's future is dropped (e.g. a timeout fires),
            // so a hung process is not left running and holding the repo lock.
            .kill_on_drop(true)
            .output()
            .await;
        log_run_result(&subcommand, start.elapsed().as_millis(), &result);
        result
    }

    /// Spawn borg for streaming output, logging the subcommand at launch.
    /// The child has `kill_on_drop(true)` set automatically.
    pub fn spawn<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &HashMap<String, String>,
    ) -> std::io::Result<Child> {
        let subcommand = args
            .first()
            .map(|a| a.as_ref().to_string_lossy().into_owned())
            .unwrap_or_else(|| "<none>".to_owned());
        tracing::info!(subcommand, "borg: spawning");
        Command::new(&self.binary)
            .args(args)
            .envs(env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
    }

    /// Like [`spawn`] but also pipes stdin so the caller can write to it.
    pub fn spawn_with_stdin<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &HashMap<String, String>,
    ) -> std::io::Result<Child> {
        let subcommand = args
            .first()
            .map(|a| a.as_ref().to_string_lossy().into_owned())
            .unwrap_or_else(|| "<none>".to_owned());
        tracing::info!(subcommand, "borg: spawning");
        Command::new(&self.binary)
            .args(args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
    }
}

#[cfg(test)]
fn test_binary_override() -> Option<PathBuf> {
    TEST_BINARY_OVERRIDE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned())
}

#[cfg(test)]
pub(crate) struct TestBinaryOverrideGuard {
    previous: Option<PathBuf>,
}

#[cfg(test)]
impl Drop for TestBinaryOverrideGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = TEST_BINARY_OVERRIDE.get_or_init(|| Mutex::new(None)).lock() {
            *guard = self.previous.take();
        }
    }
}

#[cfg(test)]
pub(crate) fn override_binary_for_tests(binary: PathBuf) -> TestBinaryOverrideGuard {
    let previous = TEST_BINARY_OVERRIDE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_or(None, |mut guard| guard.replace(binary));

    TestBinaryOverrideGuard { previous }
}
