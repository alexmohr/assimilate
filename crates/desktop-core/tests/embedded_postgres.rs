// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Starts a real `PostgreSQL` from the binaries in `ASSIMILATE_TEST_PG_INSTALL_DIR`
//! (a directory containing `bin/initdb`, e.g. `/usr/lib/postgresql/16`).
//!
//! Run with: `ASSIMILATE_TEST_PG_INSTALL_DIR=... cargo test -p desktop-core -- --ignored`

use std::path::PathBuf;

use desktop_core::{
    paths::DesktopPaths,
    ports::free_loopback_port,
    postgres::{EmbeddedPostgres, PostgresConfig},
    secrets::Secret,
};

#[cfg(test)]
fn config() -> PostgresConfig {
    let install_dir = std::env::var_os("ASSIMILATE_TEST_PG_INSTALL_DIR")
        .expect("ASSIMILATE_TEST_PG_INSTALL_DIR must point at a PostgreSQL install");
    PostgresConfig {
        install_dir: PathBuf::from(install_dir),
        superuser_password: Secret::generate(),
        port: free_loopback_port().unwrap(),
    }
}

#[tokio::test]
#[ignore = "requires ASSIMILATE_TEST_PG_INSTALL_DIR"]
async fn starts_restarts_and_keeps_data_with_no_password_left_on_disk() {
    let root = tempfile::tempdir().unwrap();
    let paths = DesktopPaths::at(root.path().to_path_buf());
    let config = config();

    let first = EmbeddedPostgres::start(&paths, &config).await.unwrap();
    let url = first.database_url();
    assert!(
        !tokio::fs::try_exists(paths.postgres_init_password_file())
            .await
            .unwrap(),
        "the initdb password file must be deleted"
    );
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", config.port))
            .await
            .is_err(),
        "postgres must not listen on TCP"
    );
    first.stop().await.unwrap();

    // PostgreSQL ignores unknown files in its data directory, so this marker
    // only survives if the second start reuses the cluster rather than
    // deleting the directory or initialising a new one.
    let marker = paths.postgres_data().join("assimilate-test-marker");
    tokio::fs::write(&marker, b"kept").await.unwrap();

    let second = EmbeddedPostgres::start(&paths, &config).await.unwrap();
    assert_eq!(second.database_url(), url);
    second.stop().await.unwrap();
    assert_eq!(
        tokio::fs::read(&marker).await.unwrap(),
        b"kept",
        "the cluster's data directory must survive a restart"
    );
}
