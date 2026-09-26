// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/// Upper bound for a hook command's own `timeout_seconds`, in seconds (24h).
///
/// Deliberately more generous than the schedule-level default bound: that
/// value applies blanket to every hook, whereas this one is a per-script
/// statement by the operator that *this* command legitimately runs long.
pub const MAX_HOOK_COMMAND_TIMEOUT_SECONDS: u32 = 86_400;
