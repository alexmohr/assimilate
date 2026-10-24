// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Instant,
};

use shared::{
    borg::{GracefulChild, log_run_result, run_command},
    task_registry::TaskRegistry,
};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader},
    process::{ChildStderr, ChildStdout},
    sync::mpsc,
};

/// Where the agent's release archive puts borg, next to the agent's own
/// executable. The Windows archive ships one (see `docs/agents.md`); the
/// Linux binary doesn't, so there this is normally absent.
const BUNDLED_BORG: &str = "borg/borg.exe";

/// The bundled borg, if [`locate_bundled_borg`] found one.
static BUNDLED: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Looks for borg in the agent's release archive layout once, at startup.
/// `BORG_BINARY` still wins over it, and `borg` on `PATH` is the fallback.
pub async fn locate_bundled_borg() {
    // An executable path that can't be found is empty: no directory, so no
    // bundled borg.
    let exe = std::env::current_exe().unwrap_or_default();
    let _ = BUNDLED.set(bundled_borg_next_to(&exe).await);
}

/// The borg the release archive puts next to `exe`, if it is there.
async fn bundled_borg_next_to(exe: &Path) -> Option<PathBuf> {
    let path = exe.parent()?.join(BUNDLED_BORG);
    if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return None;
    }
    tracing::info!(path = %path.display(), "using the borg bundled with the agent");
    Some(path)
}

/// `BORG_BINARY` if set, then the bundled borg, then `borg` on `PATH`.
fn default_binary() -> PathBuf {
    std::env::var_os("BORG_BINARY")
        .map(PathBuf::from)
        .or_else(|| BUNDLED.get().cloned().flatten())
        .unwrap_or_else(|| PathBuf::from("borg"))
}

/// Wrapper around the borg binary that provides structured logging for every invocation.
#[derive(Clone)]
pub struct Borg {
    binary: PathBuf,
    /// Extra environment variables injected at run time, used by tests to override behaviour.
    extra_env: Vec<(String, String)>,
    /// Where a [`GracefulChild`]'s SIGKILL-escalation reaper task registers itself, so
    /// shutdown can join it. Shared (not per-`Borg`) so every borg invocation across the
    /// process feeds the same registry that `main` drains on exit.
    task_registry: TaskRegistry,
    /// The mount namespace borg runs in, where filesystem snapshots stand in
    /// for the backup sources; `None` for the agent's own.
    #[cfg(target_os = "linux")]
    namespace: Option<std::sync::Arc<crate::snapshot::MountPlan>>,
}

impl Borg {
    pub fn new(task_registry: TaskRegistry) -> Self {
        Self {
            binary: default_binary(),
            extra_env: Vec::new(),
            task_registry,
            #[cfg(target_os = "linux")]
            namespace: None,
        }
    }

    /// This borg, run in the mount namespace `plan` describes when there is
    /// one.
    #[cfg(target_os = "linux")]
    #[must_use]
    pub fn in_namespace(&self, plan: Option<std::sync::Arc<crate::snapshot::MountPlan>>) -> Self {
        Self {
            namespace: plan,
            ..self.clone()
        }
    }

    #[cfg(test)]
    pub fn with_extra_env(binary: PathBuf, extra_env: Vec<(String, String)>) -> Self {
        Self {
            binary,
            extra_env,
            task_registry: TaskRegistry::default(),
            #[cfg(target_os = "linux")]
            namespace: None,
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
        let env = self.combined_env(env);
        let cmd = shared::borg::command(&self.binary, args, &env, None);
        #[cfg(target_os = "linux")]
        let cmd = self.namespaced(cmd);
        shared::borg::run_prepared(cmd, &self.binary, args, &env, &self.task_registry).await
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
        let subcommand = shared::borg::subcommand_of(args);
        tracing::info!(subcommand, "borg: starting");
        let start = Instant::now();
        let mut guard = self.spawn_child(args, env)?;
        let result = wait_with_stderr_stream(&mut guard, log_tx).await;
        log_run_result(&subcommand, start.elapsed().as_millis(), &result);
        result
    }

    /// Like [`Self::run`] but executes borg with the given working directory.
    /// It never enters the snapshot namespace set with `in_namespace`: it
    /// serves restores and extracts, which read no backup sources.
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

    /// Starts borg with its stdout and stderr piped for the caller to
    /// stream, without waiting for it. Dropping the returned child stops borg
    /// gracefully.
    ///
    /// # Errors
    ///
    /// Returns an error if borg cannot be started.
    pub fn spawn<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &[(String, String)],
    ) -> std::io::Result<StreamingBorg> {
        let subcommand = args
            .first()
            .map_or_else(String::new, |a| a.as_ref().to_string_lossy().into_owned());
        tracing::info!(subcommand, "borg: spawning");
        let mut child = self.spawn_child(args, env)?;
        // Both exist because they were asked for as pipes.
        let missing = |what: &str| std::io::Error::other(format!("borg has no {what} pipe"));
        let stdout = child.take_stdout().ok_or_else(|| missing("stdout"))?;
        let stderr = child.take_stderr().ok_or_else(|| missing("stderr"))?;
        Ok(StreamingBorg {
            child,
            stdout,
            stderr,
        })
    }

    /// Starts borg with stdout and stderr piped, guarded so that dropping it
    /// stops borg gracefully.
    fn spawn_child<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &[(String, String)],
    ) -> std::io::Result<GracefulChild> {
        let env = self.combined_env(env);
        let cmd = shared::borg::command(&self.binary, args, &env, None);
        #[cfg(target_os = "linux")]
        let cmd = self.namespaced(cmd);
        shared::borg::spawn_guarded(cmd, &self.binary, &env, &self.task_registry)
    }

    /// `cmd`, set to enter the mount namespace set with `in_namespace` if
    /// there is one.
    #[cfg(target_os = "linux")]
    fn namespaced(&self, mut cmd: tokio::process::Command) -> tokio::process::Command {
        if let Some(plan) = self.namespace.clone() {
            let mut fds = Vec::with_capacity(plan.descriptor_count());
            // SAFETY: the closure runs in the forked child just before exec,
            // where only async-signal-safe calls are allowed; `enter` makes
            // only system calls, on paths prepared in the plan, and `fds`
            // already has room for every descriptor it opens.
            unsafe {
                cmd.pre_exec(move || plan.enter(nix::sched::CloneFlags::CLONE_NEWNS, &mut fds));
            }
        }
        cmd
    }

    fn combined_env(&self, env: &[(String, String)]) -> Vec<(String, String)> {
        env.iter().chain(self.extra_env.iter()).cloned().collect()
    }
}

/// A running borg whose output the caller streams.
pub struct StreamingBorg {
    /// The process; dropping it stops borg gracefully.
    pub child: GracefulChild,
    pub stdout: ChildStdout,
    pub stderr: ChildStderr,
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

    #[tokio::test]
    async fn the_bundled_borg_is_found_next_to_the_executable() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("assimilate-agent.exe");
        assert_eq!(bundled_borg_next_to(&exe).await, None);

        let borg = dir.path().join(BUNDLED_BORG);
        tokio::fs::create_dir_all(borg.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&borg, b"").await.unwrap();
        assert_eq!(bundled_borg_next_to(&exe).await, Some(borg));
    }

    #[tokio::test]
    async fn an_executable_without_a_directory_has_no_bundled_borg() {
        assert_eq!(bundled_borg_next_to(Path::new("")).await, None);
    }

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

    #[cfg(unix)]
    #[tokio::test]
    async fn a_spawned_borg_streams_its_output_to_the_caller() {
        let borg = Borg::with_extra_env(PathBuf::from("sh"), Vec::new());
        let StreamingBorg {
            mut child,
            mut stdout,
            mut stderr,
        } = borg
            .spawn(&["-c", "printf dump; echo warn >&2; exit 1"], &[])
            .unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        stdout.read_to_end(&mut out).await.unwrap();
        stderr.read_to_end(&mut err).await.unwrap();
        assert_eq!(out, b"dump");
        assert_eq!(err, b"warn\n");
        assert_eq!(child.wait().await.unwrap().code(), Some(1));

        let missing = Borg::with_extra_env(PathBuf::from("/nonexistent/borg"), Vec::new());
        assert!(missing.spawn(&["extract"], &[]).is_err());
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
