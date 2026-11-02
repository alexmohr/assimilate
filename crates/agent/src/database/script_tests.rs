// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Helpers for the tests that run an engine's shell scripts in a real `sh`,
//! with stand-in programs that print what they were given.

#![cfg(unix)]

use std::{os::unix::fs::PermissionsExt as _, path::Path};

/// What one run of a script left behind.
pub struct Run {
    pub status: std::process::ExitStatus,
    pub stdout: String,
    /// The same output, for a dump that is not text.
    pub bytes: Vec<u8>,
    pub stderr: String,
    /// What the stand-in programs appended to the file `ASSIMILATE_LOG`
    /// names.
    pub log: String,
}

/// Writes the executable `body` as `name` in `dir`, returning its path.
pub async fn install(dir: &Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    tokio::fs::write(&path, body).await.unwrap();
    tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .await
        .unwrap();
    path.to_string_lossy().into_owned()
}

/// Runs `command` with only the `PATH` and `env`, feeding it `stdin`.
pub async fn run(dir: &Path, command: &[String], env: &[(&str, &str)], stdin: &[u8]) -> Run {
    use tokio::io::AsyncWriteExt as _;
    let log = dir.join("log");
    let (program, args) = command.split_first().unwrap();
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("ASSIMILATE_LOG", &log)
        .envs(env.iter().copied())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(stdin).await.unwrap();
    drop(input);
    let output = child.wait_with_output().await.unwrap();
    Run {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        bytes: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        log: tokio::fs::read_to_string(&log).await.unwrap_or_default(),
    }
}
