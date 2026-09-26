// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

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
