// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::path::Path;

fn resolve_git_sha() -> Option<String> {
    let git_dir = Path::new("../.git");
    let head_content = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head_content.trim();

    let full_sha = if let Some(ref_path) = head.strip_prefix("ref: ") {
        std::fs::read_to_string(git_dir.join(ref_path))
            .ok()?
            .trim()
            .to_owned()
    } else {
        head.to_owned()
    };

    Some(full_sha.get(..7)?.to_owned())
}

fn main() {
    let sha = resolve_git_sha().unwrap_or_default();

    println!("cargo::rustc-env=GIT_SHA={sha}");
    println!("cargo::rerun-if-changed=../.git/HEAD");
    println!("cargo::rerun-if-changed=../.git/refs/");
}
