// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Stdio,
    time::Instant,
};

use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader},
    process::Command,
    sync::mpsc,
};

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

/// Wraps a borg child process and sends SIGTERM (not SIGKILL) on drop, giving borg time
/// to release its repository lock. Escalates to SIGKILL after 30 s if borg does not exit,
/// then runs `borg break-lock` to remove the stale lock that SIGKILL leaves behind.
struct GracefulChild {
    child: tokio::process::Child,
    binary: PathBuf,
    /// `BORG_REPO` value, used to run break-lock after a forced SIGKILL.
    repo: Option<String>,
    env: Vec<(String, String)>,
}

impl GracefulChild {
    fn new(
        child: tokio::process::Child,
        binary: PathBuf,
        repo: Option<String>,
        env: Vec<(String, String)>,
    ) -> Self {
        Self {
            child,
            binary,
            repo,
            env,
        }
    }

    async fn wait_with_output(&mut self) -> std::io::Result<std::process::Output> {
        let stdout = self.child.stdout.take();
        let stderr = self.child.stderr.take();

        // Drain stdout/stderr concurrently with wait to avoid pipe-buffer deadlocks.
        let (status, out, err) = {
            let wait = self.child.wait();
            let read_out = async move {
                let mut buf = Vec::new();
                if let Some(mut s) = stdout {
                    s.read_to_end(&mut buf).await?;
                }
                std::io::Result::Ok(buf)
            };
            let read_err = async move {
                let mut buf = Vec::new();
                if let Some(mut s) = stderr {
                    s.read_to_end(&mut buf).await?;
                }
                std::io::Result::Ok(buf)
            };
            tokio::join!(wait, read_out, read_err)
        };

        Ok(std::process::Output {
            status: status?,
            stdout: out?,
            stderr: err?,
        })
    }

    async fn wait_with_stderr_stream(
        &mut self,
        log_tx: mpsc::Sender<String>,
    ) -> std::io::Result<std::process::Output> {
        let stdout = self.child.stdout.take();
        let stderr = self.child.stderr.take();

        let (status, out, err) = {
            let wait = self.child.wait();
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
}

impl Drop for GracefulChild {
    fn drop(&mut self) {
        // id() returns None once the process has been successfully waited on, meaning it has
        // already exited and released any locks. Some(pid) means it may still be running.
        let Some(pid) = self.child.id() else { return };

        // Non-blocking check: if the child has already exited there is nothing to kill and
        // no lock to break. Skipping the reaper avoids sending SIGKILL to a recycled PID.
        match self.child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error = %e, "borg: try_wait failed; proceeding with SIGTERM");
            }
        }

        #[cfg(unix)]
        if let Ok(pid) = i32::try_from(pid) {
            let nix_pid = nix::unistd::Pid::from_raw(pid);
            let _ = nix::sys::signal::kill(nix_pid, nix::sys::signal::Signal::SIGTERM);

            let binary = self.binary.clone();
            let repo = self.repo.clone();
            let env = self.env.clone();

            // Escalate to SIGKILL if borg has not responded to SIGTERM after 30 s.
            // SIGKILL leaves lock.exclusive on the repository, so break-lock is run
            // immediately after to remove the stale lock.
            let _ = std::thread::Builder::new()
                .name("borg-reaper".to_owned())
                .spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(30));

                    // Send signal 0 (existence check) before escalating to SIGKILL.
                    // If the process is already gone, skip SIGKILL to avoid hitting a
                    // recycled PID.
                    if nix::sys::signal::kill(nix_pid, None).is_err() {
                        return;
                    }

                    let _ = nix::sys::signal::kill(nix_pid, nix::sys::signal::Signal::SIGKILL);

                    if let Some(repo) = repo
                        && let Err(e) = std::process::Command::new(&binary)
                            .arg("break-lock")
                            .arg(&repo)
                            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                            .output()
                    {
                        tracing::warn!(error = %e, "borg: break-lock failed after SIGKILL");
                    }
                });
        }
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

    /// Like [`run`] but streams stderr lines to `log_tx` as they are emitted.
    ///
    /// Lines are sent with [`mpsc::Sender::try_send`] so the pipe drain is never blocked by a
    /// slow receiver; excess lines are silently dropped.  The full stderr is still returned in the
    /// [`std::process::Output`] for post-processing (e.g. warning extraction).
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
        let mut cmd = Command::new(&self.binary);
        cmd.args(args)
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .envs(self.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.kill_on_drop(false);
        let child = cmd.spawn()?;

        let repo = env
            .iter()
            .find(|(k, _)| k == "BORG_REPO")
            .map(|(_, v)| v.clone());
        let combined_env: Vec<_> = env.iter().chain(self.extra_env.iter()).cloned().collect();

        let mut guard = GracefulChild::new(child, self.binary.clone(), repo, combined_env);
        let result = guard.wait_with_stderr_stream(log_tx).await;
        log_run_result(&subcommand, start.elapsed().as_millis(), &result);
        result
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
        // kill_on_drop would send SIGKILL, which does not give borg a chance to release
        // its repository lock. GracefulChild handles termination with SIGTERM instead.
        cmd.kill_on_drop(false);
        let child = cmd.spawn()?;

        let repo = env
            .iter()
            .find(|(k, _)| k == "BORG_REPO")
            .map(|(_, v)| v.clone());
        let combined_env: Vec<_> = env.iter().chain(self.extra_env.iter()).cloned().collect();

        let mut guard = GracefulChild::new(child, self.binary.clone(), repo, combined_env);
        let result = guard.wait_with_output().await;
        log_run_result(&subcommand, start.elapsed().as_millis(), &result);
        result
    }
}
