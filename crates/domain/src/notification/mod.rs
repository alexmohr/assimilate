// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/// Notification event types and their human-readable labels.
mod event;
/// The per-channel `{{placeholder}}` content template and its built-in defaults.
pub mod template;

pub use event::{EventType, event_label};
