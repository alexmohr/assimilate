// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use tauri::async_runtime::block_on;

/// A fresh directory under the system temp dir, removed again on drop.
pub struct ScratchDir(PathBuf);

impl ScratchDir {
    pub fn new(name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "assimilate-desktop-{name}-{}-{nanos}",
            std::process::id()
        ));
        block_on(tokio::fs::create_dir_all(&dir)).unwrap();
        Self(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        // Best effort: a leftover temp dir must not fail the test.
        drop(block_on(tokio::fs::remove_dir_all(&self.0)));
    }
}
