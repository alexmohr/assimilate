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

use tokio::{
    io::AsyncReadExt as _,
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command},
};

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

/// Wraps a borg child process. On drop:
/// 1. Sends SIGTERM (graceful shutdown — borg releases its lock)
/// 2. Spawns a reaper thread that waits 30 s, then SIGKILL, then `borg break-lock`
///
/// This replaces `kill_on_drop(true)` which sends immediate SIGKILL with no lock cleanup,
/// leaving a stale `lock.exclusive` on the repository.
pub struct ServerChild {
    child: Option<Child>,
    binary: PathBuf,
    repo: Option<String>,
    env: Vec<(String, String)>,
}

impl ServerChild {
    fn new(
        child: Child,
        binary: PathBuf,
        repo: Option<String>,
        env: Vec<(String, String)>,
    ) -> Self {
        Self {
            child: Some(child),
            binary,
            repo,
            env,
        }
    }

    /// Take the child's stdout for streaming reads.
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.as_mut()?.stdout.take()
    }

    /// Take the child's stderr for streaming reads.
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.as_mut()?.stderr.take()
    }

    /// Take the child's stdin for writing.
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.as_mut()?.stdin.take()
    }

    /// Wait for the child to exit.
    pub async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        match self.child.as_mut() {
            Some(child) => child.wait().await,
            None => Err(std::io::Error::other("child already waited")),
        }
    }

    /// Kill the child (SIGKILL on Unix).
    pub async fn kill(&mut self) -> std::io::Result<()> {
        match self.child.as_mut() {
            Some(child) => child.kill().await,
            None => Err(std::io::Error::other("child already exited")),
        }
    }

    /// Wait for the child to exit, collecting stdout and stderr.
    ///
    /// This drains stdout / stderr concurrently with waiting so the pipe buffer
    /// never fills up and deadlocks the child.
    pub async fn wait_with_output(&mut self) -> std::io::Result<std::process::Output> {
        let stdout = self.child.as_mut().and_then(|c| c.stdout.take());
        let stderr = self.child.as_mut().and_then(|c| c.stderr.take());

        let (status, out, err) = tokio::join!(
            async { self.wait().await },
            async {
                let mut buf = Vec::new();
                if let Some(mut s) = stdout {
                    s.read_to_end(&mut buf).await?;
                }
                std::io::Result::Ok(buf)
            },
            async {
                let mut buf = Vec::new();
                if let Some(mut s) = stderr {
                    s.read_to_end(&mut buf).await?;
                }
                std::io::Result::Ok(buf)
            },
        );

        Ok(std::process::Output {
            status: status?,
            stdout: out?,
            stderr: err?,
        })
    }
}

impl Drop for ServerChild {
    fn drop(&mut self) {
        let Some(ref mut child) = self.child else {
            return;
        };
        let Some(pid) = child.id() else { return };

        // Non-blocking check: if the child has already exited there is nothing to kill.
        match child.try_wait() {
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
    ///
    /// Uses [`ServerChild`] internally so the process is killed gracefully (SIGTERM first,
    /// then SIGKILL + break-lock) if the caller's future is dropped (e.g. a timeout fires).
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

        let mut cmd = Command::new(&self.binary);
        cmd.args(args)
            .envs(env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // kill_on_drop would send SIGKILL, which does not give borg a chance to release
        // its repository lock. ServerChild handles termination with SIGTERM instead.
        cmd.kill_on_drop(false);
        let child = cmd.spawn()?;

        let repo = env.get("BORG_REPO").cloned();
        let env_vec: Vec<_> = env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let mut guard = ServerChild::new(child, self.binary.clone(), repo, env_vec);
        let result = guard.wait_with_output().await;
        log_run_result(&subcommand, start.elapsed().as_millis(), &result);
        result
    }

    /// Spawn borg for streaming output, logging the subcommand at launch.
    ///
    /// Returns a [`ServerChild`] that sends SIGTERM on drop (instead of SIGKILL),
    /// escalating to SIGKILL + break-lock after 30 seconds.
    pub fn spawn<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &HashMap<String, String>,
    ) -> std::io::Result<ServerChild> {
        let subcommand = args
            .first()
            .map(|a| a.as_ref().to_string_lossy().into_owned())
            .unwrap_or_else(|| "<none>".to_owned());
        tracing::info!(subcommand, "borg: spawning");

        let mut cmd = Command::new(&self.binary);
        cmd.args(args)
            .envs(env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.kill_on_drop(false);
        let child = cmd.spawn()?;

        let repo = env.get("BORG_REPO").cloned();
        let env_vec: Vec<_> = env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        Ok(ServerChild::new(child, self.binary.clone(), repo, env_vec))
    }

    /// Like [`spawn`] but also pipes stdin so the caller can write to it.
    pub fn spawn_with_stdin<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &HashMap<String, String>,
    ) -> std::io::Result<ServerChild> {
        let subcommand = args
            .first()
            .map(|a| a.as_ref().to_string_lossy().into_owned())
            .unwrap_or_else(|| "<none>".to_owned());
        tracing::info!(subcommand, "borg: spawning");

        let mut cmd = Command::new(&self.binary);
        cmd.args(args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.kill_on_drop(false);
        let child = cmd.spawn()?;

        let repo = env.get("BORG_REPO").cloned();
        let env_vec: Vec<_> = env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        Ok(ServerChild::new(child, self.binary.clone(), repo, env_vec))
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
