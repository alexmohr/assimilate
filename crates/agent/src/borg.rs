// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use shared::types::BORG_REPO_ENV_KEY;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader},
    process::Command,
    sync::mpsc,
};

use crate::task_registry::TaskRegistry;

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

const DEFAULT_KILL_ESCALATION_SECS: u64 = 30;

/// Delay before escalating a SIGTERM'd borg child to SIGKILL. Overridable via
/// `BORG_KILL_ESCALATION_SECS` so CI/test runs that need the escalation path to complete
/// quickly don't have to wait out the full 30s default; shutdown now joins the reaper task
/// via `TaskRegistry` (see `GracefulChild::drop`) instead of racing it against teardown.
pub(crate) fn kill_escalation_delay() -> Duration {
    let secs = std::env::var("BORG_KILL_ESCALATION_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_KILL_ESCALATION_SECS);
    Duration::from_secs(secs)
}

/// Wraps a borg child process and sends SIGTERM (not SIGKILL) on drop, giving borg time
/// to release its repository lock. Escalates to SIGKILL after
/// [`kill_escalation_delay`] if borg does not exit, then runs `borg break-lock` to
/// remove the stale lock that SIGKILL leaves behind.
struct GracefulChild {
    child: tokio::process::Child,
    binary: PathBuf,
    /// `BORG_REPO` value, used to run break-lock after a forced SIGKILL.
    repo: Option<String>,
    env: Vec<(String, String)>,
    task_registry: TaskRegistry,
}

impl GracefulChild {
    fn new(
        child: tokio::process::Child,
        binary: PathBuf,
        repo: Option<String>,
        env: Vec<(String, String)>,
        task_registry: TaskRegistry,
    ) -> Self {
        Self {
            child,
            binary,
            repo,
            env,
            task_registry,
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

            // Escalate to SIGKILL if borg has not responded to SIGTERM within
            // kill_escalation_delay(). SIGKILL leaves lock.exclusive on the repository,
            // so break-lock is run immediately after to remove the stale lock.
            //
            // Registered with task_registry (rather than a detached std::thread) so shutdown
            // can join this task instead of racing it: a detached thread sleeping through
            // kill_escalation_delay gets killed along with the rest of the process on exit,
            // meaning the SIGKILL+break-lock cleanup it promised would never run at all.
            let escalation_delay = kill_escalation_delay();
            let handle = tokio::spawn(async move {
                tokio::time::sleep(escalation_delay).await;

                // Send signal 0 (existence check) before escalating to SIGKILL.
                // If the process is already gone, skip SIGKILL to avoid hitting a
                // recycled PID.
                if nix::sys::signal::kill(nix_pid, None).is_err() {
                    return;
                }

                let _ = nix::sys::signal::kill(nix_pid, nix::sys::signal::Signal::SIGKILL);

                if let Some(repo) = repo
                    && let Err(e) = tokio::process::Command::new(&binary)
                        .arg("break-lock")
                        .arg(&repo)
                        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                        .output()
                        .await
                {
                    tracing::warn!(error = %e, "borg: break-lock failed after SIGKILL");
                }
            });
            self.task_registry.register(handle);
        }
    }
}

/// Wrapper around the borg binary that provides structured logging for every invocation.
pub struct Borg {
    binary: PathBuf,
    /// Extra environment variables injected at run time, used by tests to override behaviour.
    extra_env: Vec<(String, String)>,
    /// Where a `GracefulChild`'s SIGKILL-escalation reaper task registers itself, so shutdown
    /// can join it. Shared (not per-`Borg`) so every borg invocation across the process feeds
    /// the same registry that `main` drains on exit.
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
        let mut args: Vec<String> = flags
            .iter()
            .map(|a| a.as_ref().to_string_lossy().into_owned())
            .collect();
        if !positional.is_empty() {
            args.push("--".to_owned());
            args.extend(
                positional
                    .iter()
                    .map(|a| a.as_ref().to_string_lossy().into_owned()),
            );
        }
        args
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
            .find(|(k, _)| k == BORG_REPO_ENV_KEY)
            .map(|(_, v)| v.clone());
        let combined_env: Vec<_> = env.iter().chain(self.extra_env.iter()).cloned().collect();

        let mut guard = GracefulChild::new(
            child,
            self.binary.clone(),
            repo,
            combined_env,
            self.task_registry.clone(),
        );
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
            .find(|(k, _)| k == BORG_REPO_ENV_KEY)
            .map(|(_, v)| v.clone());
        let combined_env: Vec<_> = env.iter().chain(self.extra_env.iter()).cloned().collect();

        let mut guard = GracefulChild::new(
            child,
            self.binary.clone(),
            repo,
            combined_env,
            self.task_registry.clone(),
        );
        let result = guard.wait_with_output().await;
        log_run_result(&subcommand, start.elapsed().as_millis(), &result);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_with_positional_includes_separator_when_positional_nonempty() {
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

    #[test]
    fn args_with_positional_omits_separator_when_positional_empty() {
        let result = Borg::args_with_positional(&["list", "--json"], &[] as &[&str]);
        assert_eq!(result, vec!["list".to_owned(), "--json".to_owned(),]);
    }

    #[test]
    fn args_with_positional_works_with_empty_flags() {
        let result = Borg::args_with_positional(&[] as &[&str], &["/path"]);
        assert_eq!(result, vec!["--".to_owned(), "/path".to_owned(),]);
    }

    #[test]
    fn args_with_positional_works_with_osstr_impls() {
        let flag = OsStr::new("create");
        let path = OsStr::new("/data");
        let result = Borg::args_with_positional(&[flag], &[path]);
        assert_eq!(
            result,
            vec!["create".to_owned(), "--".to_owned(), "/data".to_owned(),]
        );
    }

    // Combined into one test: both cases mutate BORG_KILL_ESCALATION_SECS, causing races
    // when parallel.
    #[test]
    fn kill_escalation_delay_reads_env_override_and_falls_back_to_default() {
        unsafe { std::env::set_var("BORG_KILL_ESCALATION_SECS", "2") };
        assert_eq!(kill_escalation_delay(), Duration::from_secs(2));

        unsafe { std::env::remove_var("BORG_KILL_ESCALATION_SECS") };
        assert_eq!(
            kill_escalation_delay(),
            Duration::from_secs(DEFAULT_KILL_ESCALATION_SECS)
        );

        unsafe { std::env::set_var("BORG_KILL_ESCALATION_SECS", "not-a-number") };
        assert_eq!(
            kill_escalation_delay(),
            Duration::from_secs(DEFAULT_KILL_ESCALATION_SECS)
        );
        unsafe { std::env::remove_var("BORG_KILL_ESCALATION_SECS") };
    }

    // Registration happens synchronously inside Drop, before the reaper's escalation delay
    // even starts sleeping - this is what makes it trackable at all: a test (or shutdown)
    // that checks the registry right after drop() returns must never race the reaper's own
    // scheduling to see it, unlike the detached std::thread this replaced.
    #[tokio::test]
    async fn graceful_child_drop_registers_its_reaper_task_synchronously() {
        let registry = TaskRegistry::default();
        let child = Command::new("sleep")
            .arg("5")
            .kill_on_drop(false)
            .spawn()
            .expect("failed to spawn sleep for test");

        let guard = GracefulChild::new(
            child,
            PathBuf::from("borg"),
            None,
            Vec::new(),
            registry.clone(),
        );
        assert_eq!(registry.pending_count(), 0);

        drop(guard);

        assert_eq!(
            registry.pending_count(),
            1,
            "reaper task must be registered before drop() returns"
        );
        // drop(guard) already sent the child SIGTERM; nothing left to clean up here.
    }
}
