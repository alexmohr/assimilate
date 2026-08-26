// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::collections::HashMap;

use serde::Deserialize;
use shared::{
    ssh::{borg_rsh, borg_rsh_with_known_hosts},
    types::Compression,
};
use tracing::warn;

use crate::{error::ApiError, ssh};

/// Query parameter for disambiguating a hostname shared by agents in
/// different domains. An agent never reports its own domain (it has no
/// reliable way to learn it), so this is only ever supplied by an admin
/// acting through the UI/API, not by the agent itself.
#[derive(Debug, Default, Deserialize)]
pub struct DomainQuery {
    /// Domain to disambiguate the hostname with, if it is shared by more
    /// than one agent.
    pub domain: Option<String>,
}

/// Builds the base borg environment shared by all server-side borg invocations:
/// the repository passphrase, the SSH command, and the server's `SSH_AUTH_SOCK`
/// when one is present so borg can use the forwarded agent.
///
/// `SSH_AUTH_SOCK` is always forwarded when set, because the server reaches the
/// repository over its own SSH agent. The relocation override
/// (`BORG_RELOCATED_REPO_ACCESS_IS_OK`) is intentionally *not* part of the base:
/// it is added only by callers that have a confirmed pending relocation, so that
/// unrelated operations never silently accept a moved repository.
///
/// This uses `StrictHostKeyChecking=accept-new` against the process's own
/// default `known_hosts` file, so it is only appropriate before a repository
/// has an accepted host key on record (initial connection tests, `borg init`).
/// Once a repository has a pinned `ssh_host_key`, use [`borg_env_for_repo`]
/// instead so the pin is actually enforced.
#[must_use]
pub fn borg_base_env(passphrase: &str) -> HashMap<String, String> {
    let mut env = HashMap::from([
        ("BORG_PASSPHRASE".to_owned(), passphrase.to_owned()),
        ("BORG_RSH".to_owned(), borg_rsh()),
    ]);
    if let Ok(sock) = std::env::var("SSH_AUTH_SOCK") {
        env.insert("SSH_AUTH_SOCK".to_owned(), sock);
    }
    env
}

/// Builds the borg environment for an operation against an existing
/// repository, pinning SSH host key verification to `ssh_host_key` when one
/// is on record (via a stable, repo-scoped `known_hosts` file - see
/// [`crate::ssh::write_pinned_known_hosts`]) instead of relying on the
/// process's own default `known_hosts` file.
///
/// Without this, a repo's "Accept SSH key" flow only updates the database and
/// never actually took effect: every server-side borg call still fell back to
/// the shared, unpinned `known_hosts` file, which could be left holding a
/// stale entry for the host, causing every subsequent operation to fail with
/// the same host-key-changed error the user had just accepted.
///
/// If writing the pinned file fails, this falls back to the unpinned
/// `accept-new` behavior (logging a warning) rather than failing the whole
/// operation outright, matching the agent's equivalent fallback.
pub async fn borg_env_for_repo(
    passphrase: &str,
    repo_id: i64,
    ssh_host: &str,
    ssh_port: u16,
    ssh_host_key: Option<&str>,
) -> HashMap<String, String> {
    let mut env = borg_base_env(passphrase);

    let Some(host_key) = ssh_host_key else {
        return env;
    };

    match ssh::write_pinned_known_hosts(repo_id, ssh_host, ssh_port, host_key).await {
        Ok(path) => {
            env.insert("BORG_RSH".to_owned(), borg_rsh_with_known_hosts(&path));
        }
        Err(e) => {
            warn!(
                repo_id,
                error = %e,
                "failed to write pinned SSH known_hosts, falling back to unpinned SSH"
            );
        }
    }

    env
}

/// Default SSH user for borg repository connections.
#[must_use]
pub fn default_ssh_user() -> String {
    "borg".to_string()
}

/// Validates that a field value is not empty, returning a `BadRequest` error with the field name.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid.
pub fn validate_non_empty(value: &str, field_name: &str) -> Result<(), ApiError> {
    if value.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "{field_name} must not be empty"
        )));
    }
    Ok(())
}

/// Validates and normalizes the compression string from API requests.
/// Accepts: "none", "lz4", "zstd", "zstd,3", "zlib", "zlib,6", or None (defaults to "lz4").
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid.
pub fn validate_compression(value: Option<&str>) -> Result<String, ApiError> {
    let s = value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("lz4");
    s.parse::<Compression>()
        .map(|c| c.to_string())
        .map_err(|e| {
            ApiError::BadRequest(format!(
                "invalid compression: {e}. Valid values: none, lz4, zstd, zstd,<level>, zlib, \
                 zlib,<level>"
            ))
        })
}

/// Hashes a password using bcrypt in a blocking task.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Internal`]: an internal error occurs
/// - [`ApiError::Bcrypt`]: password hashing fails
pub async fn hash_password(password: String) -> Result<String, ApiError> {
    tokio::task::spawn_blocking(move || bcrypt::hash(password, 10))
        .await
        .map_err(|e| ApiError::Internal(format!("join error: {e}")))?
        .map_err(ApiError::Bcrypt)
}

/// Verifies a password against a bcrypt hash in a blocking task.
///
/// # Errors
///
/// Returns [`ApiError::Internal`] if an internal error occurs.
pub async fn verify_password(password: String, hash: String) -> Result<bool, ApiError> {
    tokio::task::spawn_blocking(move || bcrypt::verify(password, &hash))
        .await
        .map_err(|e| ApiError::Internal(format!("join error: {e}")))?
        .map_err(|e| ApiError::Internal(format!("bcrypt verify error: {e}")))
}

/// Generates a cryptographically random hex string of the specified byte length.
pub fn generate_random_hex(len_bytes: usize) -> String {
    use std::fmt::Write as _;

    use rand::rngs::OsRng;

    let mut bytes = vec![0u8; len_bytes];
    rand::RngCore::fill_bytes(&mut OsRng, &mut bytes);
    bytes.iter().fold(String::new(), |mut acc, b| {
        let _ = write!(acc, "{b:02x}");
        acc
    })
}

/// # Errors
///
/// Returns [`ApiError::BadRequest`] if the request is invalid.
pub async fn validate_path_exists(
    ssh_host: &str,
    ssh_user: &str,
    ssh_port: u16,
    path: &str,
) -> Result<(), ApiError> {
    let req = ssh::ListDirRequest {
        ssh_host: ssh_host.to_string(),
        ssh_user: ssh_user.to_string(),
        ssh_port: Some(ssh_port),
        path: path.to_string(),
    };
    let res = ssh::list_dir(&req).await;
    if let Some(err) = res.error {
        return Err(ApiError::BadRequest(format!(
            "repo_path '{path}' does not exist or is not accessible: {err}"
        )));
    }
    Ok(())
}

/// Pushes configuration to all currently connected agents.
pub async fn push_config_to_all_agents(state: &crate::AppState) {
    for agent_id in state.registry.connected_agents().await {
        crate::config_assembler::push_config_to_agent(state, agent_id).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{borg_env_for_repo, validate_compression};

    // borg_env_for_repo_pins_known_hosts_when_host_key_present lives in
    // ssh.rs's ssh_key_dir_scoped_helpers instead of here: it writes a real
    // pinned known_hosts file under SSH_KEY_DIR, and that env var is already
    // exclusively owned by ssh.rs's combined test to avoid races between
    // parallel tests mutating the same process-global var.

    #[tokio::test]
    async fn borg_env_for_repo_falls_back_to_accept_new_without_host_key() {
        let env = borg_env_for_repo("hunter2", 555_000_002, "repo.example.com", 22, None).await;

        let rsh = env.get("BORG_RSH").unwrap();
        assert!(rsh.contains("StrictHostKeyChecking=accept-new"));
        assert!(!rsh.contains("UserKnownHostsFile="));
    }

    #[test]
    fn validate_compression_defaults_to_lz4_when_absent() {
        assert_eq!(validate_compression(None).unwrap(), "lz4");
        assert_eq!(validate_compression(Some("")).unwrap(), "lz4");
        assert_eq!(validate_compression(Some("  ")).unwrap(), "lz4");
    }

    #[test]
    fn validate_compression_normalizes_bare_zstd_and_zlib() {
        assert_eq!(validate_compression(Some("zstd")).unwrap(), "zstd,3");
        assert_eq!(validate_compression(Some("zlib")).unwrap(), "zlib,6");
    }

    #[test]
    fn validate_compression_passes_through_explicit_levels() {
        assert_eq!(validate_compression(Some("zstd,9")).unwrap(), "zstd,9");
        assert_eq!(validate_compression(Some("none")).unwrap(), "none");
        assert_eq!(validate_compression(Some("lz4")).unwrap(), "lz4");
    }

    #[test]
    fn validate_compression_rejects_unknown_algorithm() {
        assert!(validate_compression(Some("brotli")).is_err());
    }

    #[test]
    fn validate_compression_rejects_invalid_level() {
        assert!(validate_compression(Some("zstd,abc")).is_err());
    }
}
