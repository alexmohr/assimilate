// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! What a new host fetches to install the agent: the install scripts and, for
//! Linux, the agent binary the server carries. These routes are public. The
//! scripts hold no secrets, and the binary is the one anyone could build from
//! the source; a host proves itself later, with its enrollment token.

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

/// The agent binary for `query`, opened for reading.
async fn open_agent(query: &DownloadQuery) -> Result<(tokio::fs::File, u64), ApiError> {
    let DownloadOs::Linux = query.os;
    let not_found =
        || ApiError::NotFound("this server carries no agent for that architecture".to_owned());
    let path = agent_binary_dir().await.join(query.arch.file_name());
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| not_found())?;
    let length = file.metadata().await.map_err(|_| not_found())?.len();
    Ok((file, length))
}

/// The agent binary this server carries, for the install script to download.
/// It is streamed, not read into memory: anyone can ask for it.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the server has no agent for the architecture.
pub async fn download_agent(Query(query): Query<DownloadQuery>) -> Result<Response, ApiError> {
    let (file, length) = open_agent(&query).await?;
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

/// The SHA-256 of that binary, in `sha256sum` format, hashed in chunks.
///
/// # Errors
///
/// Returns [`ApiError::NotFound`] if the server has no agent for the architecture, or
/// [`ApiError::Internal`] if reading it fails.
pub async fn download_agent_sha256(Query(query): Query<DownloadQuery>) -> Result<String, ApiError> {
    use std::fmt::Write as _;

    let (mut file, _length) = open_agent(&query).await?;
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
}
