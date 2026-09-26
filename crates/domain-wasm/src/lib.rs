// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! WebAssembly bindings exposing the `domain` crate to the frontend.
//!
//! Built by `scripts/build-wasm.sh` into `frontend/src/wasm/generated/`. Values
//! cross the boundary as strings, string arrays and `js_sys::Array` tuples,
//! never through a fallible JavaScript-object (de)serializer, so every path
//! through the generated glue is one the frontend can take.

/// JavaScript bindings for the file-change pattern grammar.
pub mod file_change;
/// JavaScript bindings for hook command limits.
pub mod hooks;
/// JavaScript bindings for notification template rendering.
pub mod notification;
/// JavaScript bindings for cron validation and next-run calculation.
pub mod schedule;
