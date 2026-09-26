// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Pure, I/O-free domain logic shared by the server and the frontend.
//!
//! Everything in this crate must compile for `wasm32-unknown-unknown`: the
//! `wasm` crate exposes it to the frontend so a rule is implemented once in
//! Rust instead of being mirrored by hand in TypeScript.

/// The file-change pattern grammar: one `<glob> [ignore|warn|fatal]` rule per line.
pub mod file_change;
/// Human-readable formatting shared by the agent, the server and the frontend.
pub mod format;
/// Limits for pre- and post-backup hook commands.
pub mod hooks;
/// Notification event types and the `{{placeholder}}` content template.
pub mod notification;
/// Cron validation and next-run calculation in a schedule's timezone.
pub mod schedule;
