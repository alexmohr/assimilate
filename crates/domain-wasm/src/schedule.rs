// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use wasm_bindgen::prelude::*;

/// Validates a cron expression exactly as the server does when a schedule is
/// saved; see [`domain::schedule::validate_cron`].
///
/// Returns the server's error message, or `undefined` when the expression is valid.
#[must_use]
#[wasm_bindgen(js_name = validateCron)]
pub fn validate_cron(expression: &str) -> Option<String> {
    domain::schedule::validate_cron(expression).err()
}
