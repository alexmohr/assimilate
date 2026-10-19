// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Stand-in `PostgreSQL` client programs on the host for tests: `createdb`,
//! `pg_restore` and `dropdb` scripts that log how they were called, so a
//! restore can run end to end as the user the tests run as.

use std::os::unix::fs::PermissionsExt as _;

use shared::database::DatabaseRuntimes;

/// A directory of client scripts. Each appends its name and arguments to a
/// log; `pg_restore` first copies its stdin to a file.
pub struct HostPostgres {
    dir: tempfile::TempDir,
}

impl HostPostgres {
    /// Installs the scripts; the ones `fail` names exit 1 with an error on
    /// stderr once they have logged their call.
    pub async fn install(fail: &[&str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("log");
        let restored = format!("cat > {}\n", dir.path().join("restored").display());
        for (program, read) in [
            ("createdb", ""),
            ("pg_restore", restored.as_str()),
            ("dropdb", ""),
        ] {
            let exit = if fail.contains(&program) {
                format!("echo '{program}: error: it broke' >&2; exit 1")
            } else {
                "exit 0".to_owned()
            };
            let body = format!(
                "#!/bin/sh\n{read}echo \"{program} $*\" >> {}\n{exit}\n",
                log.display()
            );
            let script = dir.path().join(program);
            tokio::fs::write(&script, body).await.unwrap();
            tokio::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .await
                .unwrap();
        }
        Self { dir }
    }

    /// Settings that reach the host's server through these scripts, as the
    /// user the tests run as, on port 5433.
    pub fn runtimes(&self) -> DatabaseRuntimes {
        let me = nix::unistd::User::from_uid(nix::unistd::getuid())
            .unwrap()
            .unwrap()
            .name;
        DatabaseRuntimes {
            host: true,
            host_os_user: Some(me),
            host_port: Some(5433),
            host_bin_dir: Some(self.dir.path().to_string_lossy().into_owned()),
            ..DatabaseRuntimes::default()
        }
    }

    /// Every call, one `<program> <arguments>` line each.
    pub async fn log(&self) -> String {
        tokio::fs::read_to_string(self.dir.path().join("log"))
            .await
            .unwrap_or_default()
    }

    /// The programs called, in order.
    pub async fn programs(&self) -> Vec<String> {
        self.log()
            .await
            .lines()
            .filter_map(|line| line.split(' ').next())
            .map(str::to_owned)
            .collect()
    }

    /// What `pg_restore` read on stdin.
    pub async fn restored(&self) -> Vec<u8> {
        tokio::fs::read(self.dir.path().join("restored"))
            .await
            .unwrap()
    }
}
