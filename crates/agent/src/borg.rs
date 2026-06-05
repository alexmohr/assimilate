// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Stdio,
    time::Instant,
};

use tokio::process::Command;

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
    /// Extra environment variables injected at run time, used by tests to override behaviour.
    extra_env: Vec<(String, String)>,
}

impl Default for Borg {
    fn default() -> Self {
        Self::new()
    }
}

impl Borg {
    pub fn new() -> Self {
        Self {
            binary: std::env::var("BORG_BINARY")
                .map_or_else(|_| PathBuf::from("borg"), PathBuf::from),
            extra_env: Vec::new(),
        }
    }

    #[cfg(test)]
    pub fn with_extra_env(binary: PathBuf, extra_env: Vec<(String, String)>) -> Self {
        Self { binary, extra_env }
    }

    /// Run borg and wait for it to finish, logging subcommand, exit code, and elapsed time.
    ///
    /// `extra_env` entries (used in tests) are appended after `env`.
    pub async fn run<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &[(String, String)],
    ) -> std::io::Result<std::process::Output> {
        self.run_impl(args, env, None).await
    }

    /// Like [`run`] but executes borg with the given working directory.
    pub async fn run_in_dir<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &[(String, String)],
        dir: &Path,
    ) -> std::io::Result<std::process::Output> {
        self.run_impl(args, env, Some(dir)).await
    }

    async fn run_impl<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &[(String, String)],
        dir: Option<&Path>,
    ) -> std::io::Result<std::process::Output> {
        let subcommand = args.first().map_or_else(
            || "<none>".to_owned(),
            |a| a.as_ref().to_string_lossy().into_owned(),
        );
        tracing::info!(subcommand, "borg: starting");
        let start = Instant::now();
        let mut cmd = Command::new(&self.binary);
        cmd.args(args)
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(d) = dir {
            cmd.current_dir(d);
        }
        cmd.kill_on_drop(true);
        let child = cmd.spawn()?;
        let result = child.wait_with_output().await;
        log_run_result(&subcommand, start.elapsed().as_millis(), &result);
        result
    }
}
