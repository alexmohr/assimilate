// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Stdio,
    time::Instant,
};

use shared::{
    borg::{GracefulChild, log_run_result, run_command},
    task_registry::TaskRegistry,
    types::BORG_REPO_ENV_KEY,
};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader},
    process::Command,
    sync::mpsc,
};

/// Wrapper around the borg binary that provides structured logging for every invocation.
pub struct Borg {
    binary: PathBuf,
    /// Extra environment variables injected at run time, used by tests to override behaviour.
    extra_env: Vec<(String, String)>,
    /// Where a [`GracefulChild`]'s SIGKILL-escalation reaper task registers itself, so
    /// shutdown can join it. Shared (not per-`Borg`) so every borg invocation across the
    /// process feeds the same registry that `main` drains on exit.
    task_registry: TaskRegistry,
}

impl Borg {
    pub fn new(task_registry: TaskRegistry) -> Self {
        Self {
            binary: std::env::var("BORG_BINARY")
                .map_or_else(|_| PathBuf::from("borg"), PathBuf::from),
            extra_env: Vec::new(),
            task_registry,
        }
    }

    #[cfg(test)]
    pub fn with_extra_env(binary: PathBuf, extra_env: Vec<(String, String)>) -> Self {
        Self {
            binary,
            extra_env,
            task_registry: TaskRegistry::default(),
        }
    }

    /// Build a full argument list by joining `flags` with `positional` args using a `--`
    /// separator. The separator is omitted when `positional` is empty.
    ///
    /// This prevents argument injection via leading-dash paths by ensuring the `--`
    /// end-of-options marker is structurally guaranteed rather than left to individual
    /// call sites to insert.
    pub fn args_with_positional(
        flags: &[impl AsRef<OsStr>],
        positional: &[impl AsRef<OsStr>],
    ) -> Vec<String> {
        shared::borg::args_with_positional(flags, positional)
    }

    /// Run borg and wait for it to finish, logging subcommand, exit code, and elapsed time.
    ///
    /// `extra_env` entries (used in tests) are appended after `env`.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying process or I/O operation fails.
    pub async fn run<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &[(String, String)],
    ) -> std::io::Result<std::process::Output> {
        run_command(
            &self.binary,
            args,
            &self.combined_env(env),
            None,
            &self.task_registry,
        )
        .await
    }

    /// Like [`Self::run`] but streams stderr lines to `log_tx` as they are emitted.
    ///
    /// Lines are sent with [`mpsc::Sender::try_send`] so the pipe drain is never blocked by a
    /// slow receiver; excess lines are silently dropped. The full stderr is still returned in
    /// the [`std::process::Output`] for post-processing (e.g. warning extraction).
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying process or I/O operation fails.
    pub async fn run_with_log_channel<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &[(String, String)],
        log_tx: mpsc::Sender<String>,
    ) -> std::io::Result<std::process::Output> {
        let subcommand = args.first().map_or_else(
            || "<none>".to_owned(),
            |a| a.as_ref().to_string_lossy().into_owned(),
        );
        tracing::info!(subcommand, "borg: starting");
        let start = Instant::now();
        let combined_env = self.combined_env(env);

        let mut cmd = Command::new(&self.binary);
        cmd.args(args)
            .envs(combined_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.kill_on_drop(false);
        let child = cmd.spawn()?;

        let repo = combined_env
            .iter()
            .find(|(k, _)| k == BORG_REPO_ENV_KEY)
            .map(|(_, v)| v.clone());
        let mut guard = GracefulChild::new(
            child,
            self.binary.clone(),
            repo,
            combined_env,
            self.task_registry.clone(),
        );

        let result = wait_with_stderr_stream(&mut guard, log_tx).await;
        log_run_result(&subcommand, start.elapsed().as_millis(), &result);
        result
    }

    /// Like [`Self::run`] but executes borg with the given working directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying process or I/O operation fails.
    pub async fn run_in_dir<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &[(String, String)],
        dir: &Path,
    ) -> std::io::Result<std::process::Output> {
        run_command(
            &self.binary,
            args,
            &self.combined_env(env),
            Some(dir),
            &self.task_registry,
        )
        .await
    }

    fn combined_env(&self, env: &[(String, String)]) -> Vec<(String, String)> {
        env.iter().chain(self.extra_env.iter()).cloned().collect()
    }
}

/// Drains `guard`'s stdout/stderr concurrently with waiting for it to exit, forwarding each
/// stderr line to `log_tx` as it arrives instead of only returning the full buffer at the end.
async fn wait_with_stderr_stream(
    guard: &mut GracefulChild,
    log_tx: mpsc::Sender<String>,
) -> std::io::Result<std::process::Output> {
    let stdout = guard.take_stdout();
    let stderr = guard.take_stderr();

    let (status, out, err) = {
        let wait = guard.wait();
        let read_out = async move {
            let mut buf = Vec::new();
            if let Some(mut s) = stdout {
                s.read_to_end(&mut buf).await?;
            }
            std::io::Result::Ok(buf)
        };
        let read_err = async move {
            let mut lines_buf = Vec::<u8>::new();
            if let Some(s) = stderr {
                let mut reader = BufReader::new(s);
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line).await {
                        Ok(0) => break,
                        Ok(_) => {
                            lines_buf.extend_from_slice(line.as_bytes());
                            let trimmed = line
                                .trim_end_matches('\n')
                                .trim_end_matches('\r')
                                .to_owned();
                            if !trimmed.is_empty() {
                                // Non-blocking: drop line if channel is full rather than
                                // blocking the pipe drain and causing borg to stall.
                                log_tx.try_send(trimmed).ok();
                            }
                        }
                        Err(e) => return std::io::Result::Err(e),
                    }
                }
            }
            std::io::Result::Ok(lines_buf)
        };
        tokio::join!(wait, read_out, read_err)
    };

    Ok(std::process::Output {
        status: status?,
        stdout: out?,
        stderr: err?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_with_positional_delegates_to_shared_impl() {
        let result =
            Borg::args_with_positional(&["create", "--dry-run"], &["/some/path", "/other"]);
        assert_eq!(
            result,
            vec![
                "create".to_owned(),
                "--dry-run".to_owned(),
                "--".to_owned(),
                "/some/path".to_owned(),
                "/other".to_owned(),
            ]
        );
    }

    #[tokio::test]
    async fn run_with_log_channel_streams_stderr_lines_and_returns_full_output() {
        let (log_tx, mut log_rx) = mpsc::channel(16);
        let borg = Borg::with_extra_env(PathBuf::from("sh"), Vec::new());

        let output = borg
            .run_with_log_channel(
                &["-c", "echo out; echo err-line-1 >&2; echo err-line-2 >&2"],
                &[],
                log_tx,
            )
            .await
            .unwrap();

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "out");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).trim(),
            "err-line-1\nerr-line-2"
        );

        let mut lines = Vec::new();
        while let Ok(line) = log_rx.try_recv() {
            lines.push(line);
        }
        assert_eq!(lines, vec!["err-line-1", "err-line-2"]);
    }
}
