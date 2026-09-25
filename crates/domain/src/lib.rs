// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Pure, I/O-free domain logic shared by the server and the frontend.
//!
//! Everything in this crate must compile for `wasm32-unknown-unknown`: the
//! `wasm` crate exposes it to the frontend so a rule is implemented once in
//! Rust instead of being mirrored by hand in TypeScript.

/// The file-change pattern grammar: one `<glob> [ignore|warn|fatal]` rule per line.
pub mod file_change;
