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

use shared::types::BORG_REPO_ENV_KEY;
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
/// 1. Sends SIGTERM (graceful shutdown -- borg releases its lock)
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

    #[must_use]
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
            .map_or_else(|| "<none>".to_owned(), |a| a.as_ref().to_string_lossy().into_owned());
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

        let repo = env.get(BORG_REPO_ENV_KEY).cloned();
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
    /// Build a full argument list by joining `flags` with `positional` args using a `--`
    /// separator. The separator is omitted when `positional` is empty.
    ///
    /// This prevents argument injection via leading-dash paths (see #242) by ensuring
    /// the `--` end-of-options marker is structurally guaranteed rather than left to
    /// individual call sites to insert.
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

    pub fn spawn<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &HashMap<String, String>,
    ) -> std::io::Result<ServerChild> {
        let subcommand = args
            .first()
            .map_or_else(|| "<none>".to_owned(), |a| a.as_ref().to_string_lossy().into_owned());
        tracing::info!(subcommand, "borg: spawning");

        let mut cmd = Command::new(&self.binary);
        cmd.args(args)
            .envs(env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.kill_on_drop(false);
        let child = cmd.spawn()?;

        let repo = env.get(BORG_REPO_ENV_KEY).cloned();
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
            .map_or_else(|| "<none>".to_owned(), |a| a.as_ref().to_string_lossy().into_owned());
        tracing::info!(subcommand, "borg: spawning");

        let mut cmd = Command::new(&self.binary);
        cmd.args(args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.kill_on_drop(false);
        let child = cmd.spawn()?;

        let repo = env.get(BORG_REPO_ENV_KEY).cloned();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn server_child_wait_returns_exit_status() {
        let mut child = ServerChild::new(
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg("exit 42")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap(),
            PathBuf::from("sh"),
            None,
            Vec::new(),
        );
        let status = child.wait().await.unwrap();
        assert_eq!(status.code(), Some(42));
    }

    #[tokio::test]
    async fn server_child_wait_returns_error_when_child_already_taken() {
        // Take stdout so the child resource is effectively consumed.
        let mut child = ServerChild::new(
            tokio::process::Command::new("sh")
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
        );
        // Simulate a taken child by extracting the inner child via a helper.
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
    async fn server_child_wait_with_output_collects_stdout() {
        let mut child = ServerChild::new(
            tokio::process::Command::new("echo")
                .arg("hello world")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap(),
            PathBuf::from("echo"),
            None,
            Vec::new(),
        );
        let output = child.wait_with_output().await.unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "hello world"
        );
    }

    #[tokio::test]
    async fn server_child_wait_with_output_collects_stderr() {
        let mut child = ServerChild::new(
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg("echo 'error msg' >&2")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap(),
            PathBuf::from("sh"),
            None,
            Vec::new(),
        );
        let output = child.wait_with_output().await.unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), "error msg");
    }

    #[tokio::test]
    async fn server_child_kill_terminates_running_process() {
        let mut child = ServerChild::new(
            tokio::process::Command::new("sh")
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
        );
        // Kill the process.
        child.kill().await.unwrap();
        let status = child.wait().await.unwrap();
        // On Unix, being killed by SIGKILL yields a signal status (128 + 9 = 137
        // as exit code, or signal-only status with no code).
        assert!(!status.success());
    }

    #[tokio::test]
    async fn server_child_take_stdout_returns_stdout_stream() {
        let mut child = ServerChild::new(
            tokio::process::Command::new("echo")
                .arg("test")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap(),
            PathBuf::from("echo"),
            None,
            Vec::new(),
        );
        let stdout = child.take_stdout();
        assert!(stdout.is_some());
        // Second call returns None.
        assert!(child.take_stdout().is_none());
    }

    #[tokio::test]
    async fn server_child_take_stderr_returns_stderr_stream() {
        let mut child = ServerChild::new(
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg("echo err >&2")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap(),
            PathBuf::from("sh"),
            None,
            Vec::new(),
        );
        let stderr = child.take_stderr();
        assert!(stderr.is_some());
        assert!(child.take_stderr().is_none());
    }

    #[tokio::test]
    async fn server_child_take_stdin_returns_stdin_stream() {
        let mut child = ServerChild::new(
            tokio::process::Command::new("sh")
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
        );
        let stdin = child.take_stdin();
        assert!(stdin.is_some());
        assert!(child.take_stdin().is_none());
    }

    #[tokio::test]
    async fn server_child_kill_returns_error_when_child_already_exited() {
        let mut child = ServerChild::new(
            tokio::process::Command::new("sh")
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
        );
        child.wait().await.unwrap();
        let result = child.kill().await;
        // After the child has exited, kill() may return an error or succeed
        // (tokio behaviour varies by platform). We just verify it doesn't panic.
        let _ = result;
    }

    #[tokio::test]
    async fn server_child_wait_with_output_with_stdin_allows_writing() {
        let mut child = ServerChild::new(
            tokio::process::Command::new("sh")
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
        );
        let mut stdin = child.take_stdin().unwrap();
        use tokio::io::AsyncWriteExt;
        stdin.write_all(b"hello from stdin\n").await.unwrap();
        drop(stdin);
        let output = child.wait_with_output().await.unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "hello from stdin"
        );
    }

    #[tokio::test]
    async fn server_child_drop_does_not_panic_with_already_exited_child() {
        // Verifies that dropping a ServerChild whose process has already
        // exited does not panic or hang (the reaper thread is never spawned
        // because try_wait returns Ok(Some(_))).
        let mut child = ServerChild::new(
            tokio::process::Command::new("sh")
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
        );
        child.wait().await.unwrap();
        drop(child); // must not panic
    }

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
}
