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
    process::{Output, Stdio},
    time::Duration,
};

use shared::task_registry::TaskRegistry;
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
};

#[derive(Debug, thiserror::Error)]
pub(super) enum HookRunError {
    #[error("timed out")]
    TimedOut,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Runs `command` through `sh -c`, collecting its output. If it is still
/// running after `timeout`, its process group is terminated (SIGTERM, then
/// SIGKILL after `grace`) before this returns [`HookRunError::TimedOut`].
pub(super) async fn run(
    command: &str,
    timeout: Duration,
    grace: Duration,
    task_registry: &TaskRegistry,
) -> Result<Output, HookRunError> {
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

    let child = cmd.spawn()?;
    #[cfg(unix)]
    let group = ProcessGroup::of(&child);
    #[cfg(not(unix))]
    let group = ProcessGroup;
    let mut guard = HookProcess {
        child: Some(child),
        group,
        grace,
        task_registry: task_registry.clone(),
    };
    let stdout = guard.child.as_mut().and_then(|c| c.stdout.take());
    let stderr = guard.child.as_mut().and_then(|c| c.stderr.take());

    let finished = tokio::time::timeout(timeout, async {
        let (status, stdout, stderr) =
            tokio::join!(guard.wait(), read_all(stdout), read_all(stderr));
        Ok::<_, std::io::Error>(Output {
            status: status?,
            stdout: stdout?,
            stderr: stderr?,
        })
    })
    .await;

    if let Ok(output) = finished {
        return Ok(output?);
    }
    guard.terminate().await;
    Err(HookRunError::TimedOut)
}

async fn read_all<R: AsyncReadExt + Unpin>(reader: Option<R>) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    if let Some(mut reader) = reader {
        reader.read_to_end(&mut buf).await?;
    }
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
    async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        match self.child.as_mut() {
            Some(child) => child.wait().await,
            None => Err(std::io::Error::other("hook process already terminated")),
        }
    }

    /// Terminates the whole process group and waits until `sh` has exited.
    ///
    /// Called on timeout, so something in the group is known to be alive:
    /// either `sh` itself, or a process it started that still holds the
    /// output pipes after `sh` exited. A process group ID is not reused while
    /// any member of the group is alive, so signalling it here cannot reach
    /// an unrelated process.
    async fn terminate(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        self.group.signal(&mut child, GroupSignal::Terminate);
        if tokio::time::timeout(self.grace, child.wait())
            .await
            .is_err()
        {
            tracing::warn!("hook command ignored SIGTERM; sending SIGKILL");
        }
        // Its descendants may outlive `sh` or ignore SIGTERM; SIGKILL the
        // rest of the group either way.
        self.group.signal(&mut child, GroupSignal::Kill);
        if let Err(e) = child.wait().await {
            tracing::warn!(error = %e, "failed to reap the hook command");
        }
    }
}

impl Drop for HookProcess {
    /// Reached when the backup run is cancelled while the hook is still
    /// running: terminate its group in the background, registered with the
    /// task registry so agent shutdown waits for it.
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        // Already exited: nothing to stop, and its PID may since have been
        // reused, so it must not be signalled.
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        let Some(mut child) = self.child.take() else {
            return;
        };
        let group = self.group;
        group.signal(&mut child, GroupSignal::Terminate);
        let grace = self.grace;
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            group.signal(&mut child, GroupSignal::Kill);
            return;
        };
        let handle = runtime.spawn(async move {
            let _ = tokio::time::timeout(grace, child.wait()).await;
            group.signal(&mut child, GroupSignal::Kill);
            let _ = child.wait().await;
        });
        self.task_registry.register(handle);
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
struct ProcessGroup(Option<nix::unistd::Pid>);

#[cfg(unix)]
impl ProcessGroup {
    fn of(child: &Child) -> Self {
        Self(
            child
                .id()
                .and_then(|id| i32::try_from(id).ok())
                .map(nix::unistd::Pid::from_raw),
        )
    }

    /// Sends `signal` to every process in the group. A group that no longer
    /// exists is not an error. Without a group ID, falls back to killing `sh`.
    fn signal(self, child: &mut Child, signal: GroupSignal) {
        use nix::sys::signal::{Signal, killpg};

        let Some(pgid) = self.0 else {
            let _ = child.start_kill();
            return;
        };
        let signal = match signal {
            GroupSignal::Terminate => Signal::SIGTERM,
            GroupSignal::Kill => Signal::SIGKILL,
        };
        if let Err(e) = killpg(pgid, signal)
            && e != nix::errno::Errno::ESRCH
        {
            tracing::warn!(error = %e, "failed to signal the hook command's process group");
        }
    }
}

/// Without Unix process groups or SIGTERM, the only thing to stop is `sh`.
#[cfg(not(unix))]
#[derive(Clone, Copy)]
struct ProcessGroup;

#[cfg(not(unix))]
impl ProcessGroup {
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

    async fn wait_until_exited(pid: i32) -> bool {
        for _ in 0..100 {
            if exited(pid).await {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }

    /// The PID the hook wrote to `path`, once it has.
    async fn read_pid(path: &Path) -> Option<i32> {
        for _ in 0..100 {
            let written = tokio::fs::read_to_string(path).await.unwrap_or_default();
            if let Ok(pid) = written.trim().parse() {
                return Some(pid);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        None
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
        .unwrap();
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

        assert!(matches!(result, Err(HookRunError::TimedOut)));
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

        assert!(matches!(result, Err(HookRunError::TimedOut)));
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
}
