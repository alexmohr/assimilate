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
//! SIGKILL.

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

/// Runs `command` through `sh -c`, collecting its output. If it is still
/// running after `timeout`, its process group is terminated (SIGTERM, then
/// SIGKILL after `grace`) and this returns `Ok(None)`.
pub(super) async fn run(
    command: &str,
    timeout: Duration,
    grace: Duration,
    task_registry: &TaskRegistry,
) -> io::Result<Option<Output>> {
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // The guard below terminates the group itself, gracefully first;
        // kill_on_drop would SIGKILL only `sh`.
        .kill_on_drop(false);
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("no stdout pipe"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("no stderr pipe"))?;
    let mut guard = HookProcess {
        group: ProcessGroup::of(&child)?,
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
        let (group, grace) = (self.group, self.grace);
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
    group.signal(&mut child, GroupSignal::Terminate);
    if tokio::time::timeout(grace, child.wait()).await.is_err() {
        tracing::warn!("hook command ignored SIGTERM; sending SIGKILL");
    }
    group.signal(&mut child, GroupSignal::Kill);
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
        let group = self.group;
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                let handle = runtime.spawn(terminate_group(group, child, self.grace));
                self.task_registry.register(handle);
            }
            // No runtime left to wait out the grace period in.
            Err(_) => group.signal(&mut child, GroupSignal::Kill),
        }
    }
}

#[derive(Clone, Copy)]
enum GroupSignal {
    Terminate,
    Kill,
}

/// The hook's process group, led by its `sh`. Captured at spawn: once `sh`
/// has been reaped its PID is gone from [`Child`], while processes it started
/// may still be running in the group.
#[cfg(unix)]
#[derive(Clone, Copy)]
struct ProcessGroup(nix::unistd::Pid);

#[cfg(unix)]
impl ProcessGroup {
    fn of(child: &Child) -> io::Result<Self> {
        let id = child
            .id()
            .ok_or_else(|| io::Error::other("hook has no PID"))?;
        let id = i32::try_from(id).map_err(io::Error::other)?;
        Ok(Self(nix::unistd::Pid::from_raw(id)))
    }

    /// Sends `signal` to every process in the group. A group that no longer
    /// exists (ESRCH) is expected: everything in it has already exited.
    fn signal(self, _child: &mut Child, signal: GroupSignal) {
        use nix::sys::signal::{Signal, killpg};

        let signal = match signal {
            GroupSignal::Terminate => Signal::SIGTERM,
            GroupSignal::Kill => Signal::SIGKILL,
        };
        let result = killpg(self.0, signal);
        tracing::debug!(
            ?signal,
            ?result,
            "signalled the hook command's process group"
        );
    }
}

/// Without Unix process groups or SIGTERM, the only thing to stop is `sh`.
#[cfg(not(unix))]
#[derive(Clone, Copy)]
struct ProcessGroup;

#[cfg(not(unix))]
impl ProcessGroup {
    fn of(_child: &Child) -> io::Result<Self> {
        Ok(Self)
    }

    fn signal(self, child: &mut Child, signal: GroupSignal) {
        match signal {
            GroupSignal::Terminate | GroupSignal::Kill => {
                let _ = child.start_kill();
            }
        }
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
