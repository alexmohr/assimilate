// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Building blocks of the Assimilate desktop app that don't depend on the
//! window toolkit, so each can be tested on its own: where the app keeps its
//! files, how it keeps its secrets, and the `PostgreSQL` instance it runs.

/// Where the desktop app keeps its data on each platform.
pub mod paths;
/// A free loopback TCP port for the local server.
pub mod ports;
/// The embedded `PostgreSQL` instance backing the local server.
pub mod postgres;
/// Generated secrets and the OS keychain they live in.
pub mod secrets;
