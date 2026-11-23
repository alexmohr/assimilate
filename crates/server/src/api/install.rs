// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! What a new host fetches to install the agent: the install scripts and, for
//! Linux, the agent binary the server carries. These routes are public. The
//! scripts hold no secrets, and the binary is the one anyone could build from
//! the source; a host proves itself later, with its enrollment token.

use std::path::Path;

use axum::{
    body::Body,
    extract::Query,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt as _;
use tokio_util::io::ReaderStream;

use super::deploy::agent_binary_dir;
use crate::error::ApiError;

const INSTALL_SH: &str = include_str!("install/install.sh");
const INSTALL_PS1: &str = include_str!("install/install.ps1");

/// How much of the binary is read at a time to hash it.
const HASH_CHUNK_BYTES: usize = 64 * 1024;

/// What the scripts carry in place of the version, until the server fills it in.
const VERSION_PLACEHOLDER: &str = "@VERSION@";

/// The operating systems the server itself carries an agent for.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadOs {
    /// Linux hosts. macOS and Windows agents come from the GitHub release.
    Linux,
}

/// The CPU architectures the server carries an agent for.
#[derive(Debug, Clone, Copy, Deserialize)]
pub enum DownloadArch {
    /// 64-bit x86.
    #[serde(rename = "x86_64")]
    X86_64,
    /// 64-bit ARM.
    #[serde(rename = "aarch64")]
    Aarch64,
}

impl DownloadArch {
    /// The name of the binary in the agent binary directory.
    const fn file_name(self) -> &'static str {
        match self {
            Self::X86_64 => "agent-x86_64",
            Self::Aarch64 => "agent-aarch64",
        }
    }
}

/// Query parameters of the agent download.
#[derive(Debug, Deserialize)]
pub struct DownloadQuery {
    /// The operating system of the host.
    pub os: DownloadOs,
    /// The CPU architecture of the host.
    pub arch: DownloadArch,
}

fn script_response(script: &str, content_type: &'static str) -> Response {
    let body = script.replace(VERSION_PLACEHOLDER, env!("APP_VERSION"));
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            // The script names the release this server was built from.
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        body,
    )
        .into_response()
}

/// The shell install script for Linux and macOS.
#[must_use]
pub fn install_sh() -> Response {
    script_response(INSTALL_SH, "text/x-shellscript; charset=utf-8")
}

/// The PowerShell install script for Windows.
#[must_use]
pub fn install_ps1() -> Response {
    script_response(INSTALL_PS1, "text/plain; charset=utf-8")
}

/// The agent binary for `query` in `dir`, opened for reading.
async fn open_agent(dir: &Path, query: &DownloadQuery) -> Result<(tokio::fs::File, u64), ApiError> {
    let DownloadOs::Linux = query.os;
    let not_found =
        || ApiError::NotFound("this server carries no agent for that architecture".to_owned());
    let file = tokio::fs::File::open(dir.join(query.arch.file_name()))
        .await
        .map_err(|_| not_found())?;
    let length = file.metadata().await.map_err(|_| not_found())?.len();
    Ok((file, length))
}

/// The agent binary in `dir`, streamed, not read into memory: anyone can ask for it.
async fn agent_response(dir: &Path, query: &DownloadQuery) -> Result<Response, ApiError> {
    let (file, length) = open_agent(dir, query).await?;
    Ok((
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            ),
            (header::CONTENT_LENGTH, HeaderValue::from(length)),
        ],
        Body::from_stream(ReaderStream::new(file)),
    )
        .into_response())
}

/// The SHA-256 of the agent binary in `dir`, in `sha256sum` format, hashed in chunks.
async fn agent_sha256(dir: &Path, query: &DownloadQuery) -> Result<String, ApiError> {
    use std::fmt::Write as _;

    let (mut file, _length) = open_agent(dir, query).await?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; HASH_CHUNK_BYTES];
    loop {
        let read = file
            .read(&mut chunk)
            .await
            .map_err(|e| ApiError::Internal(format!("could not read the agent binary: {e}")))?;
        let Some(bytes) = chunk.get(..read).filter(|bytes| !bytes.is_empty()) else {
            break;
        };
        hasher.update(bytes);
    }
    let hex = hasher.finalize().iter().fold(String::new(), |mut acc, b| {
        let _ = write!(acc, "{b:02x}");
        acc
    });
    Ok(format!("{hex}  assimilate-agent\n"))
}

/// The agent binary this server carries, for the install script to download.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the server has no agent for the architecture.
pub async fn download_agent(Query(query): Query<DownloadQuery>) -> Result<Response, ApiError> {
    agent_response(&agent_binary_dir().await, &query).await
}

/// The SHA-256 of that binary, in `sha256sum` format.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the server has no agent for the architecture, or
/// [`ApiError::Internal`] if reading it fails.
pub async fn download_agent_sha256(Query(query): Query<DownloadQuery>) -> Result<String, ApiError> {
    agent_sha256(&agent_binary_dir().await, &query).await
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::*;

    async fn body_of(response: Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn the_scripts_name_the_servers_release() {
        let sh = body_of(install_sh()).await;
        let ps1 = body_of(install_ps1()).await;

        for script in [sh, ps1] {
            assert!(
                script.contains(&format!("'{}'", env!("APP_VERSION")))
                    || script.contains(&format!("\"{}\"", env!("APP_VERSION")))
            );
        }
    }

    #[tokio::test]
    async fn the_scripts_are_served_as_text() {
        let response = install_sh();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/x-shellscript; charset=utf-8"
        );
    }

    #[test]
    fn the_scripts_still_hold_the_version_placeholder_to_replace() {
        assert!(INSTALL_SH.contains(VERSION_PLACEHOLDER));
        assert!(INSTALL_PS1.contains(VERSION_PLACEHOLDER));
    }

    #[test]
    fn an_architecture_maps_to_the_binary_the_deploy_stages() {
        assert_eq!(DownloadArch::X86_64.file_name(), "agent-x86_64");
        assert_eq!(DownloadArch::Aarch64.file_name(), "agent-aarch64");
    }

    fn x86_64() -> DownloadQuery {
        DownloadQuery {
            os: DownloadOs::Linux,
            arch: DownloadArch::X86_64,
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;

        Sha256::digest(bytes)
            .iter()
            .fold(String::new(), |mut acc, b| {
                let _ = write!(acc, "{b:02x}");
                acc
            })
    }

    #[tokio::test]
    async fn the_agent_is_streamed_with_its_length() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("agent-x86_64"), b"agent bytes")
            .await
            .unwrap();

        let response = agent_response(dir.path(), &x86_64()).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_LENGTH).unwrap(),
            "11"
        );
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/octet-stream"
        );
        assert_eq!(body_of(response).await, "agent bytes");
    }

    #[tokio::test]
    async fn the_checksum_is_in_sha256sum_format_and_covers_a_file_larger_than_a_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = vec![7u8; HASH_CHUNK_BYTES * 2 + 1];
        tokio::fs::write(dir.path().join("agent-aarch64"), &bytes)
            .await
            .unwrap();
        let query = DownloadQuery {
            os: DownloadOs::Linux,
            arch: DownloadArch::Aarch64,
        };

        let line = agent_sha256(dir.path(), &query).await.unwrap();

        assert_eq!(line, format!("{}  assimilate-agent\n", sha256_hex(&bytes)));
    }

    #[tokio::test]
    async fn an_architecture_the_server_does_not_carry_is_not_found() {
        let dir = tempfile::tempdir().unwrap();

        let download = agent_response(dir.path(), &x86_64()).await;
        let checksum = agent_sha256(dir.path(), &x86_64()).await;

        assert!(matches!(download, Err(ApiError::NotFound(_))));
        assert!(matches!(checksum, Err(ApiError::NotFound(_))));
    }
}
