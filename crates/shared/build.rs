// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Build script that prepares the output directory used by `ts-rs` to
//! generate TypeScript bindings for the frontend from this crate's types.

use std::{env, io, path::Path};

/// Prepares the output directory for `ts-rs` generated TypeScript bindings
/// and re-exports its path to `rustc` so `#[ts(export)]` types write into it.
#[allow(
    clippy::disallowed_methods,
    reason = "build scripts run at compile time, before any async runtime exists"
)]
fn main() -> Result<(), io::Error> {
    // Ensure output directory exists for generated TS files
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").map_err(io::Error::other)?;
    let out_dir = Path::new(&manifest_dir).join("../../frontend/src/types/generated");

    if !out_dir.exists() {
        std::fs::create_dir_all(&out_dir)?;
    }

    // Pass TS_RS_EXPORT_DIR to rustc so #[ts(export)] test writes .ts files when tests run
    println!(
        "cargo:rustc-env=TS_RS_EXPORT_DIR={}",
        out_dir
            .to_str()
            .ok_or_else(|| io::Error::other("output dir is not valid UTF-8"))?
    );

    // Tell Cargo that this build script depends on files in src/
    println!("cargo:rerun-if-changed=src/");
    Ok(())
}
