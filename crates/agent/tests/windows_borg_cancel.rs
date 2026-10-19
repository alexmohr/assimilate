// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! On Windows a cancelled borg is stopped with Ctrl+C on its own console
//! (`shared::console`), raised by the agent binary acting as the helper.
//! These run a real Windows borg (`BORG_BINARY`) to show that borg answers it
//! by itself, before any kill, and releases the repository lock on the way
//! out, so no `break-lock` is needed.
#![cfg(windows)]

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use shared::{borg::GracefulChild, task_registry::TaskRegistry};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _},
    process::{Child, Command},
};

/// The file borg backs up is 1 GiB of pseudo-random data, written as
/// `INPUT_CHUNKS` chunks of `CHUNK` bytes. It doesn't compress, so at
/// `lzma,9` borg needs minutes for it, far longer than the test takes to
/// interrupt it.
const INPUT_CHUNKS: usize = 1024;
const CHUNK: usize = 1024 * 1024;

#[cfg(test)]
fn borg_binary() -> PathBuf {
    std::env::var_os("BORG_BINARY").map_or_else(|| panic!("BORG_BINARY must be set"), PathBuf::from)
}

#[cfg(test)]
fn borg(base: &Path) -> Command {
    let mut cmd = Command::new(borg_binary());
    cmd.env("BORG_BASE_DIR", base)
        .env("BORG_UNKNOWN_UNENCRYPTED_REPO_ACCESS_IS_OK", "yes");
    cmd
}

/// Writes 1 GiB of pseudo-random bytes (xorshift64) to `path`.
#[cfg(test)]
async fn write_random(path: &Path) {
    let mut file = tokio::io::BufWriter::new(tokio::fs::File::create(path).await.unwrap());
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut chunk = vec![0u8; CHUNK];
    for _ in 0..INPUT_CHUNKS {
        for word in chunk.as_chunks_mut::<8>().0 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *word = state.to_le_bytes();
        }
        file.write_all(&chunk).await.unwrap();
    }
    file.flush().await.unwrap();
}

/// Waits until borg holds the repository's exclusive lock, failing with
/// borg's stderr if it exits first.
#[cfg(test)]
async fn wait_for_lock(child: &mut Child, lock: &Path) {
    let locked = async {
        while !tokio::fs::try_exists(lock).await.unwrap() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    tokio::select! {
        () = locked => {}
        status = child.wait() => {
            let stderr = stderr_of(child).await;
            panic!("borg create exited with {status:?} before locking the repository:\n{stderr}");
        }
        () = tokio::time::sleep(Duration::from_mins(1)) => {
            panic!("{} never appeared", lock.display());
        }
    }
}

/// What `child`, which has exited, wrote to its piped stderr.
#[cfg(test)]
async fn stderr_of(child: &mut Child) -> String {
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_string(&mut stderr).await.unwrap();
    }
    stderr
}

/// Every lock file or directory borg keeps in the repository
/// (`lock.exclusive`, `lock.roster`).
#[cfg(test)]
async fn locks_in(repo: &Path) -> Vec<String> {
    let mut locks = Vec::new();
    let mut entries = tokio::fs::read_dir(repo).await.unwrap();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("lock") {
            locks.push(name);
        }
    }
    locks
}

// The Windows borg can't archive stdin (`create -` fails with "no such
// user"), so it backs up a file large enough to keep it busy instead.
#[tokio::test]
#[ignore = "requires BORG_BINARY, a Windows borg"]
async fn ctrl_c_stops_borg_create_and_borg_releases_its_lock() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    let repo = dir.path().join("repo");
    let input = dir.path().join("input.bin");
    write_random(&input).await;
    let init = borg(&base)
        .args(["init", "--encryption=none"])
        .arg(&repo)
        .output()
        .await
        .unwrap();
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );

    let mut create = borg(&base);
    create
        .args(["create", "--compression", "lzma,9"])
        .arg(format!("{}::cancelled", repo.display()))
        .arg(&input)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    shared::console::isolate(&mut create);
    let mut child = create.spawn().unwrap();
    wait_for_lock(&mut child, &repo.join("lock.exclusive")).await;

    // Cancelling is dropping the guard, as the agent does. With no repository
    // to break the lock of, a forced kill would leave it behind.
    let stderr = child.stderr.take().unwrap();
    let stderr = tokio::spawn(async move {
        let mut text = String::new();
        let _ = tokio::io::BufReader::new(stderr)
            .read_to_string(&mut text)
            .await;
        text
    });
    // GracefulChild raises Ctrl+C through its own executable, which here is
    // the test harness: let the agent do it, as it does for itself.
    shared::console::set_interrupt_helper(PathBuf::from(env!("CARGO_BIN_EXE_agent")));
    let registry = TaskRegistry::default();
    let started = Instant::now();
    drop(GracefulChild::new(
        child,
        borg_binary(),
        None,
        Vec::new(),
        registry.clone(),
    ));
    assert_eq!(registry.shutdown(Duration::from_mins(2)).await, 0);
    let stderr = stderr.await.unwrap();

    assert!(
        started.elapsed() < shared::borg::kill_escalation_delay(),
        "borg had to be killed:\n{stderr}"
    );
    assert_eq!(
        locks_in(&repo).await,
        Vec::<String>::new(),
        "borg left its lock behind:\n{stderr}"
    );
    let list = borg(&base)
        .args(["list", "--lock-wait", "1"])
        .arg(&repo)
        .output()
        .await
        .unwrap();
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
}

/// Set in the environment of [`child_waits_for_ctrl_c`] when
/// [`the_interrupt_reaches_an_isolated_child`] starts it.
const CHILD_ENV: &str = "ASSIMILATE_CTRL_C_CHILD";
/// What the child prints once its Ctrl+C handler is in place.
const CHILD_READY: &str = "ctrl-c handler ready";

/// The child half of [`the_interrupt_reaches_an_isolated_child`]; on its own
/// it does nothing.
#[tokio::test]
#[ignore = "run by the_interrupt_reaches_an_isolated_child"]
async fn child_waits_for_ctrl_c() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    let mut ctrl_c = tokio::signal::windows::ctrl_c().unwrap();
    println!("{CHILD_READY}");
    let received = tokio::time::timeout(Duration::from_mins(1), ctrl_c.recv()).await;
    assert!(
        matches!(received, Ok(Some(()))),
        "no Ctrl+C within a minute"
    );
}

/// The interrupt alone, on a child that only waits for it: the helper's
/// Ctrl+C reaches a process started with `console::isolate`, and that
/// process's own handler sees it.
#[tokio::test]
async fn the_interrupt_reaches_an_isolated_child() {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([
        "--exact",
        "child_waits_for_ctrl_c",
        "--ignored",
        "--nocapture",
    ])
    .env(CHILD_ENV, "1")
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    shared::console::isolate(&mut cmd);
    let mut child = cmd.spawn().unwrap();
    let mut lines = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
    let ready = async {
        while let Some(line) = lines.next_line().await.unwrap() {
            if line.contains(CHILD_READY) {
                return true;
            }
        }
        false
    };
    let ready = tokio::time::timeout(Duration::from_mins(1), ready).await;
    assert!(
        matches!(ready, Ok(true)),
        "the child never got ready:\n{}",
        stderr_of(&mut child).await
    );

    shared::console::request_interrupt_with(
        Path::new(env!("CARGO_BIN_EXE_agent")),
        child.id().unwrap(),
    )
    .unwrap();
    let Ok(status) = tokio::time::timeout(Duration::from_mins(1), child.wait()).await else {
        child.kill().await.unwrap();
        panic!("the child never exited:\n{}", stderr_of(&mut child).await);
    };
    let status = status.unwrap();

    // 0xC000013A (STATUS_CONTROL_C_EXIT) would mean Ctrl+C reached the
    // console but not the child's handler; 101 that it never arrived.
    assert!(
        status.success(),
        "the child exited with {status:?}:\n{}",
        stderr_of(&mut child).await
    );
}

/// A borg call dropped before its child exits, as on a timeout, interrupts
/// that child. Its Ctrl+C reaches the whole console the child is on, so the
/// child must have one of its own: on this process's console it would stop
/// this process too, as it would the agent or the server.
#[tokio::test]
async fn cancelling_a_command_does_not_interrupt_its_caller() {
    shared::console::set_interrupt_helper(PathBuf::from(env!("CARGO_BIN_EXE_agent")));
    let mut ctrl_c = tokio::signal::windows::ctrl_c().unwrap();
    let registry = TaskRegistry::default();

    let run = shared::borg::run_command(
        Path::new("cmd"),
        &["/C", "ping", "-n", "60", "127.0.0.1"],
        &[],
        None,
        &registry,
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(2), run)
            .await
            .is_err(),
        "the command should still be running"
    );
    // Dropping the call interrupts the child (twice, five seconds apart);
    // the registry holds the stop until it is done. Only a child on a console
    // of its own receives the interrupt: otherwise it runs until the kill.
    let stopping = Instant::now();
    assert_eq!(registry.shutdown(Duration::from_mins(2)).await, 0);
    assert!(
        stopping.elapsed() < shared::borg::kill_escalation_delay(),
        "the child had to be killed: the Ctrl+C never reached it"
    );

    let reached_us = tokio::time::timeout(Duration::from_secs(2), ctrl_c.recv()).await;
    assert!(
        reached_us.is_err(),
        "the child's Ctrl+C reached this process"
    );
}

/// Whether the process `pid` is still running.
fn running(pid: u32) -> std::io::Result<bool> {
    let listed = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()?;
    Ok(String::from_utf8_lossy(&listed.stdout).contains(&pid.to_string()))
}

/// A borg call dropped where no runtime is left, as while the agent's
/// runtime shuts down, still interrupts its child, on a runtime of its own.
#[test]
fn a_child_dropped_without_a_runtime_is_still_interrupted() {
    shared::console::set_interrupt_helper(PathBuf::from(env!("CARGO_BIN_EXE_agent")));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (guard, pid) = runtime.block_on(async {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "ping", "-n", "60", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(false);
        shared::console::isolate(&mut cmd);
        let child = cmd.spawn().unwrap();
        let pid = child.id().unwrap();
        let guard = GracefulChild::new(
            child,
            PathBuf::from("borg"),
            None,
            Vec::new(),
            TaskRegistry::default(),
        );
        (guard, pid)
    });
    drop(runtime);
    assert!(running(pid).unwrap());

    let stopping = Instant::now();
    drop(guard);
    while running(pid).unwrap() && stopping.elapsed() < Duration::from_mins(1) {
        std::thread::sleep(Duration::from_millis(200));
    }

    assert!(!running(pid).unwrap(), "the child outlived its stop");
    assert!(
        stopping.elapsed() < shared::borg::kill_escalation_delay(),
        "the child had to be killed: the Ctrl+C never reached it"
    );
}

#[tokio::test]
async fn the_interrupt_helper_reports_a_process_it_cannot_reach() {
    // No such process: the helper can't attach to its console and says so,
    // instead of raising Ctrl+C anywhere else.
    let result = shared::console::request_interrupt_with(
        Path::new(env!("CARGO_BIN_EXE_agent")),
        u32::MAX - 1,
    );

    assert!(result.is_err());
}
