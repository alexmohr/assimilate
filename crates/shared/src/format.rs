// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Human-readable formatting shared by the agent's VM staging output and the
//! server's notification content.

pub use domain::format::format_bytes;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_counts_render_in_the_expected_units() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KiB");
        assert_eq!(format_bytes(3 << 30), "3.0 GiB");
    }
}
