// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    io,
    os::windows::process::CommandExt as _,
    path::{Path, PathBuf},
    process::Stdio,
    sync::OnceLock,
};

use windows_sys::Win32::System::{
    Console::{
        AttachConsole, CTRL_C_EVENT, FreeConsole, GenerateConsoleCtrlEvent, SetConsoleCtrlHandler,
    },
    Threading::CREATE_NO_WINDOW,
};

/// The helper's only argument: this prefix, then the process id whose
/// console gets the Ctrl+C.
const HELPER_ARG: &str = "--assimilate-interrupt-console=";

/// `CREATE_NO_WINDOW` gives the child a console of its own that is never
/// shown, instead of sharing the parent's (or opening a window when the
/// parent has none).
///
/// A process that ignores Ctrl+C (`SetConsoleCtrlHandler(NULL, TRUE)`)
/// passes that on to every process it creates, and such a process never sees
/// Ctrl+C, whatever handlers it installs. Whoever started this one may have
/// set it (build tools and shells do, to leave Ctrl+C to their child), so it
/// is cleared here, before the child inherits it. This process keeps
/// receiving its own Ctrl+C, which the agent and server handle.
pub(super) fn isolate(cmd: &mut tokio::process::Command) {
    // SAFETY: a `None` handler routine with FALSE is the documented way to
    // restore normal Ctrl+C processing; it takes no pointers and only
    // changes this process's console state.
    unsafe {
        SetConsoleCtrlHandler(None, 0);
    }
    cmd.creation_flags(CREATE_NO_WINDOW);
}

/// Raises Ctrl+C on the console of `pid`, a child started with
/// [`super::isolate`], using the running executable as the helper. Blocks
/// until the helper has exited, which is quick; it does not wait for `pid`.
///
/// # Errors
///
/// Fails if the helper can't be started, or reports that it couldn't reach
/// the console (for instance because `pid` has already exited).
pub fn request_interrupt(pid: u32) -> io::Result<()> {
    match HELPER.get() {
        Some(helper) => request_interrupt_with(helper, pid),
        None => request_interrupt_with(&std::env::current_exe()?, pid),
    }
}

/// The helper [`set_interrupt_helper`] chose, if any.
static HELPER: OnceLock<PathBuf> = OnceLock::new();

/// Makes [`request_interrupt`] use `helper` instead of the running
/// executable, for a process that can't be the helper itself, such as a test
/// harness. `helper` must call [`super::run_interrupt_helper_if_requested`]
/// at startup. Only the first call takes effect.
pub fn set_interrupt_helper(helper: PathBuf) {
    let _ = HELPER.set(helper);
}

/// [`request_interrupt`] with an explicit helper executable, which must call
/// [`super::run_interrupt_helper_if_requested`] at startup.
///
/// # Errors
///
/// See [`request_interrupt`].
pub fn request_interrupt_with(helper: &Path, pid: u32) -> io::Result<()> {
    let status = std::process::Command::new(helper)
        .arg(format!("{HELPER_ARG}{pid}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "console interrupt helper for process {pid} failed: {status}"
        )))
    }
}

pub(super) fn run_helper_if_requested() {
    let Some(pid) = std::env::args_os()
        .nth(1)
        .and_then(|arg| arg.into_string().ok())
        .and_then(|arg| helper_pid(&arg))
    else {
        return;
    };
    std::process::exit(i32::from(!send_ctrl_c(pid)));
}

fn helper_pid(arg: &str) -> Option<u32> {
    arg.strip_prefix(HELPER_ARG)?.parse().ok()
}

/// Leaves the helper's own console, joins `pid`'s, stops Ctrl+C from ending
/// the helper itself, and raises it for everything on that console.
fn send_ctrl_c(pid: u32) -> bool {
    // SAFETY: these console functions take no pointers (a `None` handler
    // routine is the documented way to ignore Ctrl+C) and only change this
    // short-lived helper process's console state.
    unsafe {
        FreeConsole();
        AttachConsole(pid) != 0
            && SetConsoleCtrlHandler(None, 1) != 0
            && GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_pid_reads_only_its_own_argument() {
        assert_eq!(
            helper_pid("--assimilate-interrupt-console=4242"),
            Some(4242)
        );
        assert_eq!(helper_pid("--assimilate-interrupt-console="), None);
        assert_eq!(helper_pid("--assimilate-interrupt-console=x"), None);
        assert_eq!(helper_pid("--server-url=4242"), None);
    }
}
