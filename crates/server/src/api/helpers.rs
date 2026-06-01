// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use crate::{error::ApiError, ssh};

const VALID_COMPRESSIONS: &[&str] = &["none", "lz4", "zstd", "zlib"];

pub fn default_ssh_user() -> String {
    "root".to_string()
}

/// Validates that a field value is not empty, returning a `BadRequest` error with the field name.
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
pub fn validate_compression(value: Option<&str>) -> Result<String, ApiError> {
    let Some(s) = value else {
        return Ok("lz4".to_string());
    };
    let s = s.trim();
    if s.is_empty() {
        return Ok("lz4".to_string());
    }
    let base = s.split(',').next().unwrap_or(s);
    if !VALID_COMPRESSIONS.contains(&base) {
        return Err(ApiError::BadRequest(format!(
            "invalid compression: '{s}'. Valid values: none, lz4, zstd, zstd,<level>, zlib, \
             zlib,<level>"
        )));
    }
    if let Some(level_str) = s.strip_prefix("zstd,").or_else(|| s.strip_prefix("zlib,"))
        && level_str.parse::<i32>().is_err()
    {
        return Err(ApiError::BadRequest(format!(
            "invalid compression level in '{s}'"
        )));
    }
    let normalized = match s {
        "zstd" => "zstd,3".to_string(),
        "zlib" => "zlib,6".to_string(),
        other => other.to_string(),
    };
    Ok(normalized)
}

/// Hashes a password using bcrypt in a blocking task.
pub async fn hash_password(password: String) -> Result<String, ApiError> {
    tokio::task::spawn_blocking(move || bcrypt::hash(password, 10))
        .await
        .map_err(|e| ApiError::Internal(format!("join error: {e}")))?
        .map_err(ApiError::Bcrypt)
}

/// Verifies a password against a bcrypt hash in a blocking task.
pub async fn verify_password(password: String, hash: String) -> Result<bool, ApiError> {
    tokio::task::spawn_blocking(move || bcrypt::verify(password, &hash))
        .await
        .map_err(|e| ApiError::Internal(format!("join error: {e}")))?
        .map_err(|e| ApiError::Internal(format!("bcrypt verify error: {e}")))
}

/// Generates a cryptographically random hex string of the specified byte length.
pub fn generate_random_hex(len_bytes: usize) -> String {
    use rand::rngs::OsRng;

    let mut bytes = vec![0u8; len_bytes];
    rand::RngCore::fill_bytes(&mut OsRng, &mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

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
    for hostname in state.registry.connected_agents().await {
        crate::config_assembler::push_config_to_agent(state, &hostname).await;
    }
}
