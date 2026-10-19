// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Asking a console program to stop the way Ctrl+C would, so it can clean up
//! first. On Unix that is a signal sent straight to the child, so everything
//! here is a no-op there.
//!
//! Windows has no signal to send one process: a Ctrl+C event goes to every
//! process attached to a console. Each child is therefore started on a
//! console of its own ([`isolate`]), and [`request_interrupt`] starts a short
//! helper - this same executable, see [`run_interrupt_helper_if_requested`] -
//! that attaches to that console alone and raises the event there, so neither
//! the caller nor any other child sees it.

use tokio::process::Command;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{request_interrupt, request_interrupt_with, set_interrupt_helper};

/// Starts `cmd` on a console of its own, without a window, so
/// [`request_interrupt`] can reach it and only it. Also clears an inherited
/// "ignore Ctrl+C" from this process, which the child would otherwise
/// inherit in turn and so never see the interrupt.
#[cfg(windows)]
pub fn isolate(cmd: &mut Command) {
    windows::isolate(cmd);
}

/// Outside Windows a signal reaches the child directly: nothing to set up.
#[cfg(not(windows))]
pub fn isolate(_: &mut Command) {}

/// Every binary that stops children through [`request_interrupt`] calls this
/// first thing in `main`: when the process was started as the interrupt
/// helper, it raises Ctrl+C on the requested console and exits. Otherwise,
/// and always outside Windows, it returns at once.
pub fn run_interrupt_helper_if_requested() {
    #[cfg(windows)]
    windows::run_helper_if_requested();
}
