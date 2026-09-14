// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Human-readable formatting shared by the agent's VM staging output and the
//! server's notification content.

/// Renders a byte count the way the operator sees it (e.g. `12.4 GiB`).
#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [(&str, u64); 4] = [
        ("TiB", 1 << 40),
        ("GiB", 1 << 30),
        ("MiB", 1 << 20),
        ("KiB", 1 << 10),
    ];
    for (unit, size) in UNITS {
        if bytes >= size {
            let whole = bytes.checked_div(size).unwrap_or(0);
            let tenths = bytes
                .checked_rem(size)
                .unwrap_or(0)
                .saturating_mul(10)
                .checked_div(size)
                .unwrap_or(0);
            return format!("{whole}.{tenths} {unit}");
        }
    }
    format!("{bytes} B")
}

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
