// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::os::unix::fs::PermissionsExt;

use sqlx::PgPool;

pub(super) async fn install_fake_script(
    script: &str,
) -> (tempfile::TempDir, crate::borg::TestBinaryOverrideGuard) {
    let tempdir = tempfile::tempdir().unwrap();
    let borg_path = tempdir.path().join("borg");
    tokio::fs::write(&borg_path, script).await.unwrap();
    let mut permissions = tokio::fs::metadata(&borg_path).await.unwrap().permissions();
    permissions.set_mode(0o755);
    tokio::fs::set_permissions(&borg_path, permissions)
        .await
        .unwrap();
    let guard = crate::borg::override_binary_for_tests(borg_path);
    (tempdir, guard)
}

/// A pool that is already closed, so every query on it fails at once.
pub(super) async fn closed_pool() -> PgPool {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy_with(sqlx::postgres::PgConnectOptions::new());
    pool.close().await;
    pool
}

/// Points borg at a binary that does not exist, so running it fails.
pub(super) fn missing_borg() -> crate::borg::TestBinaryOverrideGuard {
    crate::borg::override_binary_for_tests("/nonexistent/borg".into())
}

/// A fake borg that fails on a held repository lock the first time it runs
/// and prints `output` every time after.
pub(super) fn locked_once_then(output: &str) -> String {
    format!(
        "#!/bin/sh\nif [ ! -e \"$0.locked\" ]; then\n  : > \"$0.locked\"\n  echo 'Failed to \
         create/acquire the lock /repo/lock.exclusive (timeout).' >&2\n  exit 2\nfi\nprintf '%s' \
         '{output}'\n"
    )
}

pub(super) const REPO_URL: &str = "ssh://user@host/repo";

pub(super) const LOCKED: &str = "#!/bin/sh\necho 'Failed to create/acquire the lock \
                                 /repo/lock.exclusive (timeout).' >&2\nexit 2\n";
