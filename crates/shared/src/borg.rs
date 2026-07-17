// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use tokio::{
    io::AsyncReadExt as _,
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command},
};

use crate::{task_registry::TaskRegistry, types::BORG_REPO_ENV_KEY};

/// Logs a completed borg invocation's exit code and elapsed time at `warn` (non-zero exit)
/// or `info` (success), or the spawn error itself if the process never started.
pub fn log_run_result(
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
/// quickly don't have to wait out the full 30s default; shutdown can join the reaper task
/// via `TaskRegistry` (see `GracefulChild::drop`) instead of racing it against teardown.
#[must_use]
pub fn kill_escalation_delay() -> Duration {
    let secs = std::env::var("BORG_KILL_ESCALATION_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_KILL_ESCALATION_SECS);
    Duration::from_secs(secs)
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

/// Wraps a borg child process and sends SIGTERM (not SIGKILL) on drop, giving borg time
/// to release its repository lock. Escalates to SIGKILL after [`kill_escalation_delay`]
/// if borg does not exit, then runs `borg break-lock` to remove the stale lock that
/// SIGKILL leaves behind. The escalation/break-lock step is a `tokio::spawn` task
/// registered with `TaskRegistry` (rather than a detached `std::thread`) so shutdown can
/// join it instead of racing it: a detached thread sleeping through the escalation delay
/// gets killed along with the rest of the process on exit, meaning the cleanup it
/// promised would never run at all.
///
/// `child` is `Option` so a caller that takes ownership of the underlying pipes (via
/// [`Self::take_stdout`] etc.) or has already awaited [`Self::wait`] can still safely
/// hold the guard for its SIGTERM-on-drop behaviour without a second `wait()` panicking.
pub struct GracefulChild {
    child: Option<Child>,
    binary: PathBuf,
    /// `BORG_REPO` value, used to run break-lock after a forced SIGKILL.
    repo: Option<String>,
    env: Vec<(String, String)>,
    task_registry: TaskRegistry,
}

impl GracefulChild {
    /// Wraps an already-spawned `child`, ready to send it SIGTERM (then SIGKILL +
    /// break-lock, if needed) on drop.
    #[must_use]
    pub fn new(
        child: Child,
        binary: PathBuf,
        repo: Option<String>,
        env: Vec<(String, String)>,
        task_registry: TaskRegistry,
    ) -> Self {
        Self {
            child: Some(child),
            binary,
            repo,
            env,
            task_registry,
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
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying process or I/O operation fails.
    pub async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        match self.child.as_mut() {
            Some(child) => child.wait().await,
            None => Err(std::io::Error::other("child already waited")),
        }
    }

    /// Kill the child (SIGKILL on Unix).
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying process or I/O operation fails.
    pub async fn kill(&mut self) -> std::io::Result<()> {
        match self.child.as_mut() {
            Some(child) => child.kill().await,
            None => Err(std::io::Error::other("child already exited")),
        }
    }

    /// Wait for the child to exit, collecting stdout and stderr.
    ///
    /// Drains stdout/stderr concurrently with waiting so the pipe buffer never fills up
    /// and deadlocks the child.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying process or I/O operation fails.
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

impl Drop for GracefulChild {
    fn drop(&mut self) {
        let Some(ref mut child) = self.child else {
            return;
        };
        // id() returns None once the process has been successfully waited on, meaning it has
        // already exited and released any locks.
        if child.id().is_none() {
            return;
        }

        // Non-blocking check: if the child has already exited there is nothing to kill and
        // no lock to break. Skipping the reaper avoids sending a signal to a recycled PID.
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error = %e, "borg: try_wait failed; proceeding with termination");
            }
        }

        #[cfg(unix)]
        {
            // child.id() was Some above and try_wait() just confirmed it hasn't exited, so
            // this can't fail.
            let Some(pid) = child.id() else { return };
            if let Ok(pid) = i32::try_from(pid) {
                let nix_pid = nix::unistd::Pid::from_raw(pid);
                let _ = nix::sys::signal::kill(nix_pid, nix::sys::signal::Signal::SIGTERM);

                let binary = self.binary.clone();
                let repo = self.repo.clone();
                let env = self.env.clone();

                // Escalate to SIGKILL if borg has not responded to SIGTERM within
                // kill_escalation_delay(). SIGKILL leaves lock.exclusive on the repository,
                // so break-lock is run immediately after to remove the stale lock.
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

        // Windows has no SIGTERM equivalent for an arbitrary child process, so there's no
        // graceful stage to attempt here the way there is on Unix - request termination
        // immediately (start_kill() only submits the request, it doesn't wait for exit)
        // rather than leaving the child, and its repository lock, running forever. Still
        // tracked the same way so shutdown can join it: waits for the actual exit before
        // running break-lock, since running it while the process might still hold the lock
        // would be racing the very thing it's meant to clean up after.
        #[cfg(not(unix))]
        {
            let _ = child.start_kill();
            let Some(mut owned_child) = self.child.take() else {
                return;
            };

            let binary = self.binary.clone();
            let repo = self.repo.clone();
            let env = self.env.clone();
            let handle = tokio::spawn(async move {
                let _ = owned_child.wait().await;

                if let Some(repo) = repo
                    && let Err(e) = tokio::process::Command::new(&binary)
                        .arg("break-lock")
                        .arg(&repo)
                        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                        .output()
                        .await
                {
                    tracing::warn!(error = %e, "borg: break-lock failed after termination");
                }
            });
            self.task_registry.register(handle);
        }
    }
}

/// Spawns `binary` with `args`/`env` (optionally in `dir`), waits for it to finish via a
/// [`GracefulChild`] (SIGTERM then SIGKILL+break-lock if it doesn't exit within
/// [`kill_escalation_delay`]), and logs the outcome. This is the spawn/wait/log pattern
/// common to every simple (non-streaming) `Borg::run`-style method.
///
/// # Errors
///
/// Returns an error if the underlying process or I/O operation fails.
pub async fn run_command<A: AsRef<OsStr>>(
    binary: &Path,
    args: &[A],
    env: &[(String, String)],
    dir: Option<&Path>,
    task_registry: &TaskRegistry,
) -> std::io::Result<std::process::Output> {
    let subcommand = args.first().map_or_else(
        || "<none>".to_owned(),
        |a| a.as_ref().to_string_lossy().into_owned(),
    );
    tracing::info!(subcommand, "borg: starting");
    let start = Instant::now();

    let mut cmd = Command::new(binary);
    cmd.args(args)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
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

    let mut guard = GracefulChild::new(
        child,
        binary.to_path_buf(),
        repo,
        env.to_vec(),
        task_registry.clone(),
    );
    let result = guard.wait_with_output().await;
    log_run_result(&subcommand, start.elapsed().as_millis(), &result);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_with_positional_includes_separator_when_positional_nonempty() {
        let result = args_with_positional(&["create", "--dry-run"], &["/some/path", "/other"]);
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
        let result = args_with_positional(&["list", "--json"], &[] as &[&str]);
        assert_eq!(result, vec!["list".to_owned(), "--json".to_owned(),]);
    }

    #[test]
    fn args_with_positional_works_with_empty_flags() {
        let result = args_with_positional(&[] as &[&str], &["/path"]);
        assert_eq!(result, vec!["--".to_owned(), "/path".to_owned(),]);
    }

    #[test]
    fn args_with_positional_works_with_osstr_impls() {
        let flag = OsStr::new("create");
        let path = OsStr::new("/data");
        let result = args_with_positional(&[flag], &[path]);
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

    /// Spawns a trivial `sh -c "exit 0"` child wrapped in a fresh `GracefulChild`, for tests
    /// that only care about post-exit behaviour and don't need a real long-running process.
    fn spawn_exited_child() -> GracefulChild {
        GracefulChild::new(
            Command::new("sh")
                .arg("-c")
                .arg("exit 0")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap(),
            PathBuf::from("sh"),
            None,
            Vec::new(),
            TaskRegistry::default(),
        )
    }

    #[tokio::test]
    async fn graceful_child_drop_does_not_panic_with_already_exited_child() {
        let mut child = spawn_exited_child();
        child.wait().await.unwrap();
        drop(child); // must not panic
    }

    #[tokio::test]
    async fn graceful_child_wait_returns_error_when_child_already_taken() {
        let mut child = spawn_exited_child();
        let _taken = child.child.take();
        let result = child.wait().await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("child already waited")
        );
    }

    #[tokio::test]
    async fn graceful_child_wait_with_output_collects_stdout_and_stderr() {
        let mut child = GracefulChild::new(
            Command::new("sh")
                .arg("-c")
                .arg("echo hello; echo err >&2")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap(),
            PathBuf::from("sh"),
            None,
            Vec::new(),
            TaskRegistry::default(),
        );
        let output = child.wait_with_output().await.unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
        assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), "err");
    }

    #[tokio::test]
    async fn graceful_child_kill_terminates_running_process() {
        let mut child = GracefulChild::new(
            Command::new("sh")
                .arg("-c")
                .arg("sleep 60")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap(),
            PathBuf::from("sh"),
            None,
            Vec::new(),
            TaskRegistry::default(),
        );
        child.kill().await.unwrap();
        let status = child.wait().await.unwrap();
        assert!(!status.success());
    }

    #[tokio::test]
    async fn graceful_child_take_stdout_stderr_stdin_return_once() {
        let mut child = GracefulChild::new(
            Command::new("sh")
                .arg("-c")
                .arg("cat")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap(),
            PathBuf::from("sh"),
            None,
            Vec::new(),
            TaskRegistry::default(),
        );
        assert!(child.take_stdout().is_some());
        assert!(child.take_stdout().is_none());
        assert!(child.take_stderr().is_some());
        assert!(child.take_stderr().is_none());
        assert!(child.take_stdin().is_some());
        assert!(child.take_stdin().is_none());
    }

    #[tokio::test]
    async fn run_command_returns_output_of_successful_command() {
        let output = run_command(
            Path::new("sh"),
            &["-c", "echo hi"],
            &[],
            None,
            &TaskRegistry::default(),
        )
        .await
        .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hi");
    }

    #[tokio::test]
    async fn run_command_runs_in_the_given_directory() {
        let dir = std::env::temp_dir();
        let output = run_command(
            Path::new("pwd"),
            &[] as &[&str],
            &[],
            Some(dir.as_path()),
            &TaskRegistry::default(),
        )
        .await
        .unwrap();
        assert!(output.status.success());
        let printed = String::from_utf8_lossy(&output.stdout);
        assert_eq!(printed.trim(), dir.to_string_lossy().trim_end_matches('/'));
    }
}
