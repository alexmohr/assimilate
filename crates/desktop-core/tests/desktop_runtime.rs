// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Brings the whole desktop stack up from real binaries: an embedded
//! `PostgreSQL` from `ASSIMILATE_TEST_PG_INSTALL_DIR`, and the workspace's own
//! `server` and `agent` from `ASSIMILATE_TEST_SERVER_BIN` /
//! `ASSIMILATE_TEST_AGENT_BIN`.

use std::{path::PathBuf, sync::Arc, time::Duration};

use desktop_core::{
    paths::DesktopPaths,
    runtime::{DesktopRuntime, RuntimeConfig, RuntimeError},
    secrets::{InMemoryStore, SecretStore},
};
use serde_json::{Value, json};

const HOSTNAME: &str = "desktop-test";

#[cfg(test)]
fn env_path(key: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(key).unwrap_or_else(|| panic!("{key} must be set")))
}

#[cfg(test)]
fn config(root: PathBuf) -> RuntimeConfig {
    RuntimeConfig {
        paths: DesktopPaths::at(root),
        postgres_install_dir: env_path("ASSIMILATE_TEST_PG_INSTALL_DIR"),
        server_binary: env_path("ASSIMILATE_TEST_SERVER_BIN"),
        agent_binary: env_path("ASSIMILATE_TEST_AGENT_BIN"),
        static_dir: None,
        docs_dir: None,
        borg_binary: None,
        hostname: HOSTNAME.to_owned(),
        startup_timeout: Duration::from_mins(1),
    }
}

#[cfg(test)]
async fn get_with_session(runtime: &DesktopRuntime, path: &str) -> reqwest::Response {
    reqwest::Client::new()
        .get(format!("{}{path}", runtime.server_url()))
        .header(
            reqwest::header::COOKIE,
            format!("session={}", runtime.session().cookie_value().expose()),
        )
        .send()
        .await
        .unwrap()
}

#[cfg(test)]
async fn wait_for_agent_connection(runtime: &DesktopRuntime) {
    let connected = async {
        loop {
            let agents: Vec<Value> = get_with_session(runtime, "/api/agents")
                .await
                .json()
                .await
                .unwrap();
            let is_connected = agents.iter().any(|agent| {
                agent["hostname"].as_str() == Some(HOSTNAME) && agent["is_connected"] == json!(true)
            });
            if is_connected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(30), connected)
        .await
        .expect("the local agent should connect");
}

#[tokio::test]
#[ignore = "requires ASSIMILATE_TEST_PG_INSTALL_DIR, ASSIMILATE_TEST_SERVER_BIN and \
            ASSIMILATE_TEST_AGENT_BIN"]
async fn brings_the_stack_up_signs_in_and_restarts_with_the_same_secrets() {
    let root = tempfile::tempdir().unwrap();
    let config = config(root.path().to_path_buf());
    let store: Arc<dyn SecretStore + Send + Sync> = Arc::new(InMemoryStore::default());

    let runtime = DesktopRuntime::start(&config, Arc::clone(&store))
        .await
        .unwrap();

    assert_eq!(
        get_with_session(&runtime, "/api/auth/me").await.status(),
        200
    );
    let bootstrap_login = reqwest::Client::new()
        .post(format!("{}/api/auth/login", runtime.server_url()))
        .json(&json!({"username": "admin", "password": "admin"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        bootstrap_login.status(),
        401,
        "the bootstrap admin password must be rotated"
    );
    wait_for_agent_connection(&runtime).await;
    runtime.shutdown().await.unwrap();

    let runtime = DesktopRuntime::start(&config, store).await.unwrap();
    assert_eq!(
        get_with_session(&runtime, "/api/auth/me").await.status(),
        200
    );
    wait_for_agent_connection(&runtime).await;
    runtime.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires ASSIMILATE_TEST_PG_INSTALL_DIR, ASSIMILATE_TEST_SERVER_BIN and \
            ASSIMILATE_TEST_AGENT_BIN"]
async fn a_server_that_exits_during_startup_is_reported_and_postgres_is_stopped() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let crashing_server = root.path().join("crashing-server");
    tokio::fs::write(&crashing_server, "#!/bin/sh\nexit 3\n")
        .await
        .unwrap();
    tokio::fs::set_permissions(&crashing_server, std::fs::Permissions::from_mode(0o755))
        .await
        .unwrap();
    let config = RuntimeConfig {
        server_binary: crashing_server,
        ..config(root.path().join("data"))
    };

    let result = DesktopRuntime::start(&config, Arc::new(InMemoryStore::default())).await;

    let Err(RuntimeError::ServerExited(status)) = result else {
        panic!("expected ServerExited");
    };
    assert_eq!(status.code(), Some(3));
    assert!(
        !tokio::fs::try_exists(config.paths.postgres_data().join("postmaster.pid"))
            .await
            .unwrap(),
        "postgres must be stopped again after a failed start"
    );
}
