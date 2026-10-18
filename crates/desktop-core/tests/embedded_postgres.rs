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
    postgres::{EmbeddedPostgres, PostgresConfig, PostgresError},
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

/// What the server on `port` answers a client that only says who it is: the
/// code of its authentication request, where 0 lets the client in with no
/// password at all (`trust`).
#[cfg(windows)]
async fn authentication_asked_for(port: u16) -> std::io::Result<u32> {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    // A protocol 3.0 startup message for user `postgres` and the app's
    // database, after its length (which counts itself).
    let mut body = 196_608u32.to_be_bytes().to_vec();
    for (key, value) in [("user", "postgres"), ("database", "assimilate")] {
        body.extend_from_slice(key.as_bytes());
        body.push(0);
        body.extend_from_slice(value.as_bytes());
        body.push(0);
    }
    body.push(0);
    let length = u32::try_from(body.len().saturating_add(4)).map_err(std::io::Error::other)?;
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
    stream.write_all(&length.to_be_bytes()).await?;
    stream.write_all(&body).await?;

    // `R`, a 4-byte length, then the 4-byte authentication code.
    let mut reply = [0u8; 9];
    stream.read_exact(&mut reply).await?;
    let malformed = || std::io::Error::other(format!("not an authentication request: {reply:?}"));
    let (&kind, rest) = reply.split_first().ok_or_else(malformed)?;
    if kind != b'R' {
        return Err(malformed());
    }
    let code: [u8; 4] = rest
        .get(4..)
        .and_then(|code| code.try_into().ok())
        .ok_or_else(malformed)?;
    Ok(u32::from_be_bytes(code))
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
    // A unix socket only on Unix; loopback TCP, behind the password, on
    // Windows.
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", config.port)).await;
    #[cfg(unix)]
    assert!(tcp.is_err(), "postgres must not listen on TCP");
    #[cfg(windows)]
    {
        assert!(tcp.is_ok(), "postgres must listen on 127.0.0.1: {tcp:?}");
        assert_ne!(
            authentication_asked_for(config.port).await.unwrap(),
            0,
            "a client that gives no password must be asked for one, not let in"
        );
    }
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

#[tokio::test]
#[ignore = "requires ASSIMILATE_TEST_PG_INSTALL_DIR"]
async fn a_cluster_that_does_not_start_says_why() {
    let root = tempfile::tempdir().unwrap();
    let paths = DesktopPaths::at(root.path().to_path_buf());
    let config = config();
    EmbeddedPostgres::start(&paths, &config)
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
    // The server itself refuses this, after pg_ctl has started it, so only
    // its log names the reason.
    let conf = paths.postgres_data().join("postgresql.conf");
    let mut text = tokio::fs::read_to_string(&conf).await.unwrap();
    text.push_str("\nassimilate_no_such_setting = 1\n");
    tokio::fs::write(&conf, text).await.unwrap();

    let Err(error) = EmbeddedPostgres::start(&paths, &config).await else {
        panic!("a cluster with an unknown setting must not start");
    };

    assert!(
        matches!(error, PostgresError::NotStarted { .. }),
        "{error:?}"
    );
    let message = error.to_string();
    let log = message
        .split_once("its log ends with:")
        .map_or("", |(_, log)| log);
    assert!(log.contains("assimilate_no_such_setting"), "{message}");
}
