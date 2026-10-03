// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Generates the Tauri context (config, icons, capabilities) at compile time.

fn main() {
    tauri_build::build();
}
