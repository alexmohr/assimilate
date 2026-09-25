// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! WebAssembly bindings exposing the `domain` crate to the frontend.
//!
//! Built by `scripts/build-wasm.sh` into `frontend/src/wasm/generated/`. The
//! JavaScript-facing shapes live here rather than in `domain` so the agent
//! wire format (serde's default variant names) stays untouched.

/// JavaScript bindings for the file-change pattern grammar.
pub mod file_change;
/// JavaScript bindings for hook command limits.
pub mod hooks;
/// JavaScript bindings for notification template rendering.
pub mod notification;
/// JavaScript bindings for cron validation.
pub mod schedule;
