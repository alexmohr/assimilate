// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Assimilate as a single-machine desktop app: runs the server, the local
//! agent and an embedded `PostgreSQL` in the background and shows the web UI
//! in a native window, signed in automatically.

// No console window behind the app on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use desktop_core::paths::DesktopPaths;

mod app;
mod layout;
mod lifecycle;
mod logging;
mod session;
#[cfg(test)]
mod test_support;

fn main() -> Result<(), app::AppError> {
    let paths = app::data_paths();
    logging::init(paths.as_ref().ok().map(DesktopPaths::logs).as_deref());
    app::run(paths)
}
