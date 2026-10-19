// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Runs a pre/post-backup hook command and makes sure it is gone once its
//! timeout passes.
//!
//! A hook is `sh -c <command>`, and the command usually starts processes of
//! its own (`pg_dump | gzip > ...`, `vzdump ...`). Killing only `sh` would leave
//! those running, still holding locks or writing the dump the next run
//! reads. Each hook therefore runs in its own process group, and on timeout,
//! or when the backup run is cancelled while a hook is running, the whole
//! group is sent SIGTERM and, if anything is still alive after `grace`,
//! SIGKILL. On Windows the hook is `cmd /C <command>` on a console of its
//! own and in a job object of its own: the stop request is Ctrl+C on that
//! console, and the kill ends the whole job.

use std::{
    io,
    process::{Output, Stdio},
    time::Duration,
};

use futures_util::future::OptionFuture;
use shared::task_registry::TaskRegistry;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
};

/// Runs `command` through `sh -c` (`cmd /C` on Windows), collecting its
/// output. If it is still running after `timeout`, its process group is
/// terminated (SIGTERM, then SIGKILL after `grace`; on Windows Ctrl+C on its
/// console, then a kill) and this returns `Ok(None)`.
pub(super) async fn run(
    command: &str,
    timeout: Duration,
    grace: Duration,
    task_registry: &TaskRegistry,
) -> io::Result<Option<Output>> {
    let mut cmd = shell(command);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // The guard below terminates the group itself, gracefully first;
        // kill_on_drop would SIGKILL only `sh`.
        .kill_on_drop(false);
    let (mut child, group) = ProcessGroup::spawn(&mut cmd)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("no stdout pipe"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("no stderr pipe"))?;
    let mut guard = HookProcess {
        group,
        child: Some(child),
        grace,
        task_registry: task_registry.clone(),
    };

    let finished = tokio::time::timeout(timeout, async {
        let child = guard
            .child
            .as_mut()
            .ok_or_else(|| io::Error::other("hook gone"))?;
        let (status, stdout, stderr) =
            tokio::join!(child.wait(), read_all(stdout), read_all(stderr));
        Ok::<_, io::Error>(Output {
            status: status?,
            stdout: stdout?,
            stderr: stderr?,
        })
    })
    .await;

    if let Ok(output) = finished {
        return output.map(Some);
    }
    guard.terminate().await;
    Ok(None)
}

#[cfg(unix)]
fn shell(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd
}

/// `cmd` parses its own command line, so the hook is passed verbatim rather
/// than quoted the way a C program would expect.
#[cfg(windows)]
fn shell(command: &str) -> Command {
    let mut cmd = Command::new("cmd.exe");
    cmd.arg("/C").raw_arg(command);
    cmd
}

async fn read_all(mut reader: impl AsyncRead + Unpin) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).await?;
    Ok(buf)
}

/// Owns a hook's `sh` process, the leader of the hook's process group.
struct HookProcess {
    /// `None` once the group has been terminated and `sh` reaped.
    child: Option<Child>,
    group: ProcessGroup,
    grace: Duration,
    task_registry: TaskRegistry,
}

impl HookProcess {
    /// Terminates the whole process group and waits until `sh` has exited.
    ///
    /// Called on timeout, so something in the group is known to be alive:
    /// either `sh` itself, or a process it started that still holds the
    /// output pipes after `sh` exited. A process group ID is not reused while
    /// any member of the group is alive, so signalling it here cannot reach
    /// an unrelated process.
    async fn terminate(&mut self) {
        let (group, grace) = (self.group.clone(), self.grace);
        let terminate = self
            .child
            .take()
            .map(|child| terminate_group(group, child, grace));
        OptionFuture::from(terminate).await;
    }
}

/// SIGTERMs the group, waits up to `grace` for `sh` to exit, then SIGKILLs
/// whatever is left (its descendants may outlive `sh` or ignore SIGTERM) and
/// reaps `sh`.
async fn terminate_group(group: ProcessGroup, mut child: Child, grace: Duration) {
    // The grace period starts once the request has gone out, however long
    // sending it took.
    group.request_stop().await;
    if tokio::time::timeout(grace, child.wait()).await.is_err() {
        tracing::warn!("hook command ignored the stop request; killing it");
    }
    group.kill(&mut child);
    let _ = child.wait().await;
}

impl Drop for HookProcess {
    /// Reached when the backup run is cancelled while the hook is still
    /// running: terminate its group in the background, registered with the
    /// task registry so agent shutdown waits for it.
    fn drop(&mut self) {
        // Not taken if it already exited: nothing to stop, and its PID may
        // since have been reused, so it must not be signalled.
        let Some(mut child) = self.child.take_if(|c| matches!(c.try_wait(), Ok(None))) else {
            return;
        };
        let group = self.group.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                let handle = runtime.spawn(terminate_group(group, child, self.grace));
                self.task_registry.register(handle);
            }
            // No runtime left to wait out the grace period in.
            Err(_) => group.kill(&mut child),
        }
    }
}

/// The hook's process group, led by its `sh`. Captured at spawn: once `sh`
/// has been reaped its PID is gone from [`Child`], while processes it started
/// may still be running in the group.
#[cfg(unix)]
#[derive(Clone)]
struct ProcessGroup(nix::unistd::Pid);

#[cfg(unix)]
impl ProcessGroup {
    /// Spawns `cmd` as the leader of a process group of its own.
    fn spawn(cmd: &mut Command) -> io::Result<(Child, Self)> {
        cmd.process_group(0);
        let child = cmd.spawn()?;
        let id = child
            .id()
            .ok_or_else(|| io::Error::other("hook has no PID"))?;
        let id = i32::try_from(id).map_err(io::Error::other)?;
        Ok((child, Self(nix::unistd::Pid::from_raw(id))))
    }

    /// SIGTERMs every process in the group; the signal is sent by the time
    /// this returns.
    fn request_stop(&self) -> std::future::Ready<()> {
        self.signal(nix::sys::signal::Signal::SIGTERM);
        std::future::ready(())
    }

    /// SIGKILLs every process in the group.
    fn kill(&self, _child: &mut Child) {
        self.signal(nix::sys::signal::Signal::SIGKILL);
    }

    /// Sends `signal` to every process in the group. A group that no longer
    /// exists (ESRCH) is expected: everything in it has already exited.
    fn signal(&self, signal: nix::sys::signal::Signal) {
        let result = nix::sys::signal::killpg(self.0, signal);
        tracing::debug!(
            ?signal,
            ?result,
            "signalled the hook command's process group"
        );
    }
}

/// The hook's `cmd`, alone on its console (see [`shared::console`]) and in
/// a job object with everything it starts (see [`shared::job`]).
#[cfg(windows)]
#[derive(Clone)]
struct ProcessGroup {
    pid: u32,
    job: std::sync::Arc<shared::job::Job>,
}

#[cfg(windows)]
impl ProcessGroup {
    fn spawn(cmd: &mut Command) -> io::Result<(Child, Self)> {
        let (child, job) = shared::job::spawn_in_job(cmd)?;
        let pid = child
            .id()
            .ok_or_else(|| io::Error::other("hook has no PID"))?;
        Ok((
            child,
            Self {
                pid,
                job: std::sync::Arc::new(job),
            },
        ))
    }

    /// Ctrl+C on the hook's console, which everything `cmd` started shares.
    /// Resolves once its helper has sent it; starting the helper blocks, so
    /// it runs off the runtime's worker threads. The helper finds the
    /// console through `cmd`'s PID, so once `cmd` itself has exited the
    /// programs it started can no longer be reached this way; the kill
    /// after the grace period still ends them, through the job.
    async fn request_stop(&self) {
        let pid = self.pid;
        let sent = tokio::task::spawn_blocking(move || shared::console::request_interrupt(pid))
            .await
            .map_err(std::io::Error::other)
            .and_then(|sent| sent);
        if let Err(e) = sent {
            tracing::warn!(error = %e, "could not send Ctrl+C to the hook command");
        }
    }

    /// Kills `cmd` and everything it started, through its job. Should the
    /// job refuse, `cmd` itself is still killed.
    fn kill(&self, child: &mut Child) {
        match self.job.terminate() {
            Ok(()) => tracing::debug!(pid = self.pid, "killed the hook command's job"),
            Err(e) => {
                let result = child.start_kill();
                tracing::warn!(
                    pid = self.pid,
                    error = %e,
                    ?result,
                    "could not kill the hook command's job; killed the command alone"
                );
            }
        }
    }
}

/// The Windows path through [`run`]. The interrupt helper is normally the
/// running executable; the test harness can't be one, so these use the agent
/// binary that `cargo test` builds next to it.
#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    /// Ctrl+C reaches what the hook started: `ping` stops on it, so the hook
    /// ends well before the grace period would have it killed.
    #[tokio::test]
    async fn a_stop_request_reaches_what_the_hook_started() {
        // The agent binary, `target/<profile>/agent.exe`: the nearest folder
        // above this test binary that holds one, wherever cargo put the test.
        let test_binary = std::env::current_exe().unwrap();
        let mut agent = None;
        for folder in test_binary.ancestors().skip(1) {
            let candidate = folder.join("agent.exe");
            if tokio::fs::try_exists(&candidate).await.unwrap() {
                agent = Some(candidate);
                break;
            }
        }
        let agent = agent.unwrap_or_else(|| {
            panic!(
                "no agent.exe above {}; `cargo test -p agent` builds it",
                test_binary.display()
            )
        });
        shared::console::set_interrupt_helper(agent);
        let registry = TaskRegistry::default();
        let started = std::time::Instant::now();

        let output = run(
            "ping -n 60 127.0.0.1",
            Duration::from_secs(1),
            Duration::from_secs(40),
            &registry,
        )
        .await
        .unwrap();

        assert!(output.is_none(), "a timed-out hook has no output");
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the hook was killed after its grace period rather than stopped by Ctrl+C: {:?}",
            started.elapsed()
        );
        assert_eq!(registry.shutdown(Duration::from_secs(30)).await, 0);
    }

    #[tokio::test]
    async fn returns_the_output_of_a_command_that_finishes() {
        let output = run(
            "echo out & echo err 1>&2",
            Duration::from_secs(10),
            Duration::from_secs(1),
            &TaskRegistry::default(),
        )
        .await
        .unwrap()
        .unwrap();

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "out");
        assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), "err");
    }

    #[tokio::test]
    async fn a_command_past_its_timeout_is_stopped() {
        let registry = TaskRegistry::default();
        let started = std::time::Instant::now();

        let output = run(
            "ping -n 60 127.0.0.1",
            Duration::from_secs(1),
            Duration::from_secs(1),
            &registry,
        )
        .await
        .unwrap();

        assert!(output.is_none(), "a timed-out hook has no output");
        assert_eq!(registry.shutdown(Duration::from_secs(30)).await, 0);
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the hook outlived its timeout and grace: {:?}",
            started.elapsed()
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::path::Path;

    use super::*;

    /// True once `pid` has exited. A zombie counts as exited: in a container
    /// without an init process, nobody reaps an orphan the hook left behind.
    async fn exited(pid: i32) -> bool {
        tokio::fs::read_to_string(format!("/proc/{pid}/stat"))
            .await
            .map_or(true, |stat| {
                stat.rsplit_once(')')
                    .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
            })
    }

    /// Polls until `pid` has exited; false if it is still running after 10 s.
    async fn wait_until_exited(pid: i32) -> bool {
        let poll = async {
            let mut done = false;
            while !done {
                tokio::time::sleep(Duration::from_millis(50)).await;
                done = exited(pid).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), poll)
            .await
            .is_ok()
    }

    /// The PID the hook wrote to `path`, once it has.
    async fn read_pid(path: &Path) -> Option<i32> {
        let poll = async {
            let mut pid = None;
            while pid.is_none() {
                tokio::time::sleep(Duration::from_millis(50)).await;
                let written = tokio::fs::read_to_string(path).await.unwrap_or_default();
                pid = written.trim().parse().ok();
            }
            pid
        };
        tokio::time::timeout(Duration::from_secs(10), poll)
            .await
            .ok()
            .flatten()
    }

    /// A hook that starts a long-running process and records its PID.
    fn hook_with_child(pid_file: &Path) -> String {
        format!("sleep 300 & echo $! > '{}'; wait", pid_file.display())
    }

    #[tokio::test]
    async fn returns_the_output_of_a_command_that_finishes() {
        let output = run(
            "echo out; echo err >&2",
            Duration::from_secs(10),
            Duration::from_secs(1),
            &TaskRegistry::default(),
        )
        .await
        .unwrap()
        .expect("the command finished before its timeout");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"out\n");
        assert_eq!(output.stderr, b"err\n");
    }

    #[tokio::test]
    async fn timeout_stops_the_processes_the_hook_started() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");

        let result = run(
            &hook_with_child(&pid_file),
            Duration::from_secs(1),
            Duration::from_secs(5),
            &TaskRegistry::default(),
        )
        .await;

        assert!(matches!(result, Ok(None)), "the hook should have timed out");
        let pid = read_pid(&pid_file).await.unwrap();
        assert!(
            wait_until_exited(pid).await,
            "the hook's child process {pid} is still running after the timeout"
        );
    }

    #[tokio::test]
    async fn timeout_kills_a_hook_that_ignores_sigterm() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let command = format!(
            "trap '' TERM; echo $$ > '{}'; sleep 300",
            pid_file.display()
        );

        let result = run(
            &command,
            Duration::from_secs(1),
            Duration::from_secs(1),
            &TaskRegistry::default(),
        )
        .await;

        assert!(matches!(result, Ok(None)), "the hook should have timed out");
        let pid = read_pid(&pid_file).await.unwrap();
        assert!(
            wait_until_exited(pid).await,
            "the hook {pid} ignored SIGTERM and was not killed after the grace period"
        );
    }

    #[tokio::test]
    async fn cancelling_the_run_stops_the_hook() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let registry = TaskRegistry::default();

        // The backup run being cancelled drops the hook's future mid-run.
        let cancelled = tokio::time::timeout(
            Duration::from_millis(500),
            run(
                &hook_with_child(&pid_file),
                Duration::from_mins(1),
                Duration::from_secs(5),
                &registry,
            ),
        )
        .await;

        assert!(
            cancelled.is_err(),
            "the hook should still have been running"
        );
        let pid = read_pid(&pid_file).await.unwrap();
        assert!(
            wait_until_exited(pid).await,
            "the hook's child process {pid} is still running after the run was cancelled"
        );
    }

    /// Dropped where no runtime is left to wait out the grace period in (the
    /// agent's runtime shutting down), the hook's group is killed at once.
    #[test]
    fn dropping_the_run_outside_a_runtime_kills_the_hook() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let command = hook_with_child(&pid_file);
        let registry = TaskRegistry::default();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let mut hook = Box::pin(run(
            &command,
            Duration::from_mins(1),
            Duration::from_mins(1),
            &registry,
        ));
        let pid = runtime
            .block_on(async {
                let pid = read_pid(&pid_file);
                tokio::select! {
                    _ = &mut hook => panic!("the hook should still have been running"),
                    pid = pid => pid,
                }
            })
            .unwrap();
        drop(hook);

        assert!(
            runtime.block_on(wait_until_exited(pid)),
            "the hook's child process {pid} is still running after the run was dropped"
        );
    }
}
