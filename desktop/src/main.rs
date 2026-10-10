// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Assimilate as a single-machine desktop app: runs the server, the local
//! agent and an embedded `PostgreSQL` in the background and shows the web UI
//! in a native window, signed in automatically.

// No console window behind the app on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod layout;
mod session;

fn main() -> Result<(), app::AppError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    app::run()
}
