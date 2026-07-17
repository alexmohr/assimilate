// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

#[cfg(test)]
use std::sync::{Mutex, OnceLock};
use std::{
    collections::HashMap,
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Stdio,
};

use shared::{
    borg::{self, GracefulChild},
    task_registry::TaskRegistry,
    types::BORG_REPO_ENV_KEY,
};
use tokio::process::Command;

#[cfg(test)]
static TEST_BINARY_OVERRIDE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

/// Wrapper around the borg binary that provides structured logging for every invocation.
pub struct Borg {
    binary: PathBuf,
    /// Where a [`GracefulChild`]'s SIGKILL-escalation reaper task registers itself. Each
    /// `Borg` gets its own registry: unlike the agent (which joins a single process-wide
    /// registry on shutdown, see `agent::main`), the server doesn't yet coordinate a
    /// graceful-shutdown wait across its many independent borg invocations, so there's no
    /// shared instance for this to feed into today.
    task_registry: TaskRegistry,
}

impl Default for Borg {
    fn default() -> Self {
        Self::new()
    }
}

impl Borg {
    /// Create a new Borg wrapper, resolving the binary path from `BORG_BINARY` env var.
    pub fn new() -> Self {
        #[cfg(test)]
        if let Some(binary) = test_binary_override() {
            return Self {
                binary,
                task_registry: TaskRegistry::default(),
            };
        }

        Self {
            binary: std::env::var("BORG_BINARY")
                .map_or_else(|_| PathBuf::from("borg"), PathBuf::from),
            task_registry: TaskRegistry::default(),
        }
    }

    /// Return the path to the borg binary being used.
    #[must_use]
    pub fn binary(&self) -> &Path {
        &self.binary
    }

    /// Run borg and wait for it to finish, logging subcommand, exit code, and elapsed time.
    ///
    /// Uses [`GracefulChild`] internally so the process is killed gracefully (SIGTERM first,
    /// then SIGKILL + break-lock) if the caller's future is dropped (e.g. a timeout fires).
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying process or I/O operation fails.
    pub async fn run<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &HashMap<String, String>,
    ) -> std::io::Result<std::process::Output> {
        borg::run_command(&self.binary, args, &env_vec(env), None, &self.task_registry).await
    }

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
        shared::borg::args_with_positional(flags, positional)
    }

    /// Spawn borg for streaming output, logging the subcommand at launch.
    ///
    /// Returns a [`GracefulChild`] that sends SIGTERM on drop (instead of SIGKILL),
    /// escalating to SIGKILL + break-lock if it doesn't exit in time.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying process or I/O operation fails.
    pub fn spawn<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &HashMap<String, String>,
    ) -> std::io::Result<GracefulChild> {
        self.spawn_inner(args, env, false)
    }

    /// Like [`Self::spawn`] but also pipes stdin so the caller can write to it.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying process or I/O operation fails.
    pub fn spawn_with_stdin<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &HashMap<String, String>,
    ) -> std::io::Result<GracefulChild> {
        self.spawn_inner(args, env, true)
    }

    fn spawn_inner<A: AsRef<OsStr>>(
        &self,
        args: &[A],
        env: &HashMap<String, String>,
        with_stdin: bool,
    ) -> std::io::Result<GracefulChild> {
        let subcommand = args.first().map_or_else(
            || "<none>".to_owned(),
            |a| a.as_ref().to_string_lossy().into_owned(),
        );
        tracing::info!(subcommand, "borg: spawning");

        let mut cmd = Command::new(&self.binary);
        cmd.args(args)
            .envs(env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if with_stdin {
            cmd.stdin(Stdio::piped());
        }
        cmd.kill_on_drop(false);
        let child = cmd.spawn()?;

        let repo = env.get(BORG_REPO_ENV_KEY).cloned();
        Ok(GracefulChild::new(
            child,
            self.binary.clone(),
            repo,
            env_vec(env),
            self.task_registry.clone(),
        ))
    }
}

fn env_vec(env: &HashMap<String, String>) -> Vec<(String, String)> {
    env.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
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

    #[test]
    fn new_picks_up_test_binary_override() {
        let _guard = override_binary_for_tests(PathBuf::from("/custom/borg"));
        assert_eq!(Borg::new().binary(), Path::new("/custom/borg"));
    }

    #[test]
    fn new_falls_back_to_borg_binary_env_var() {
        assert!(test_binary_override().is_none());
        // SAFETY: single-threaded test, no concurrent env access.
        unsafe { std::env::set_var("BORG_BINARY", "/env/borg") };
        assert_eq!(Borg::new().binary(), Path::new("/env/borg"));
        // SAFETY: single-threaded test, no concurrent env access.
        unsafe { std::env::remove_var("BORG_BINARY") };
    }

    #[tokio::test]
    async fn spawn_returns_a_child_whose_stdout_and_stderr_are_streamable() {
        let _guard = override_binary_for_tests(PathBuf::from("sh"));
        let borg = Borg::new();
        let mut child = borg
            .spawn(&["-c", "echo out; echo err >&2"], &HashMap::new())
            .unwrap();

        assert!(child.take_stdout().is_some());
        assert!(child.take_stderr().is_some());
        assert!(child.take_stdin().is_none());
        child.wait().await.unwrap();
    }

    #[tokio::test]
    async fn spawn_with_stdin_pipes_stdin_for_writing() {
        use tokio::io::AsyncWriteExt;

        let _guard = override_binary_for_tests(PathBuf::from("sh"));
        let borg = Borg::new();
        let mut child = borg
            .spawn_with_stdin(&["-c", "cat"], &HashMap::new())
            .unwrap();

        let mut stdin = child.take_stdin().unwrap();
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
    async fn run_uses_graceful_child_and_returns_its_output() {
        let _guard = override_binary_for_tests(PathBuf::from("echo"));
        let borg = Borg::new();
        let output = borg.run(&["hello"], &HashMap::new()).await.unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
    }

    #[test]
    fn args_with_positional_delegates_to_shared_impl() {
        let result = Borg::args_with_positional(&["list", "--json"], &[] as &[&str]);
        assert_eq!(result, vec!["list".to_owned(), "--json".to_owned()]);
    }
}
