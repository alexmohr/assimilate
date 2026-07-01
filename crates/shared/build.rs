// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{env, path::Path};

fn main() {
    // Ensure output directory exists for generated TS files
    let out_dir =
        Path::new(&env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"))
            .join("../../frontend/src/types/generated");

    if !out_dir.exists() {
        std::fs::create_dir_all(&out_dir).expect("Failed to create output directory");
    }

    // Pass TS_RS_EXPORT_DIR to rustc so #[ts(export)] test writes .ts files when tests run
    println!(
        "cargo:rustc-env=TS_RS_EXPORT_DIR={}",
        out_dir.to_str().expect("out_dir must be valid UTF-8")
    );

    // Tell Cargo that this build script depends on files in src/
    println!("cargo:rerun-if-changed=src/");
}
