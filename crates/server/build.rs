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

fn resolve_git_commit_count() -> Option<u32> {
    let output = std::process::Command::new("git")
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

fn main() {
    let app_version = std::env::var("APP_VERSION_OVERRIDE").unwrap_or_else(|_| {
        std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION must be set")
    });

    let sha = resolve_git_sha().unwrap_or_default();
    let commit_count = resolve_git_commit_count().unwrap_or(0);
    let build_timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    println!("cargo::rustc-env=APP_VERSION={app_version}");
    println!("cargo::rustc-env=GIT_SHA={sha}");
    println!("cargo::rustc-env=GIT_COMMIT_COUNT={commit_count}");
    println!("cargo::rustc-env=BUILD_TIMESTAMP={build_timestamp}");
    println!("cargo::rerun-if-changed=../.git/HEAD");
    println!("cargo::rerun-if-changed=../.git/refs/");
}
