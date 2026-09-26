// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use wasm_bindgen::prelude::*;

/// See [`domain::hooks::MAX_HOOK_COMMAND_TIMEOUT_SECONDS`].
#[must_use]
#[wasm_bindgen(js_name = maxHookCommandTimeoutSeconds)]
pub fn max_hook_command_timeout_seconds() -> u32 {
    domain::hooks::MAX_HOOK_COMMAND_TIMEOUT_SECONDS
}

#[cfg(test)]
mod tests {
    #[test]
    fn exposes_the_domain_limit() {
        assert_eq!(super::max_hook_command_timeout_seconds(), 86_400);
    }
}
