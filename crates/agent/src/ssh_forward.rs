// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::path::PathBuf;

use futures_util::{SinkExt, StreamExt};
use shared::ssh::{RelayFrame, drain_frames_to_writer};
use tempfile::TempDir;
use tokio::{
    io::AsyncReadExt,
    net::{UnixListener, UnixStream},
};
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, warn};

pub struct SshForwardSocket {
    pub socket_path: PathBuf,
    _socket_dir: TempDir,
}

#[derive(Debug, thiserror::Error)]
pub enum SshForwardError {
    #[error("failed to create socket directory: {0}")]
    TempDir(std::io::Error),
    #[error("failed to bind unix socket: {0}")]
    Bind(std::io::Error),
    #[error("failed to build relay url: {0}")]
    Url(String),
}

impl SshForwardSocket {
    pub fn create() -> Result<Self, SshForwardError> {
        let socket_dir = TempDir::new().map_err(SshForwardError::TempDir)?;
        let socket_path = socket_dir.path().join("agent.sock");
        Ok(Self {
            socket_path,
            _socket_dir: socket_dir,
        })
    }
}

pub async fn run_ssh_forward(
    socket: &SshForwardSocket,
    server_url: &str,
    hostname: &str,
    token: &str,
) -> Result<(), SshForwardError> {
    let relay_url = build_relay_url(server_url, hostname)?;
    let listener = UnixListener::bind(&socket.socket_path).map_err(SshForwardError::Bind)?;
    tokio::spawn(accept_loop(listener, relay_url, token.to_string()));
    Ok(())
}

pub fn build_relay_url(server_url: &str, hostname: &str) -> Result<String, SshForwardError> {
    let base = server_url
        .trim_end_matches('/')
        .trim_end_matches("/ws/agent");
    if base.is_empty() {
        return Err(SshForwardError::Url(format!(
            "cannot derive relay url from '{server_url}'"
        )));
    }
    Ok(format!("{base}/ws/ssh-agent/{hostname}"))
}

async fn accept_loop(listener: UnixListener, relay_url: String, token: String) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(relay_connection(stream, relay_url.clone(), token.clone()));
            }
            Err(e) => {
                warn!(error = %e, "ssh forward: unix accept error");
                break;
            }
        }
    }
}

async fn relay_connection(unix_stream: UnixStream, relay_url: String, token: String) {
    let ws_stream = match tokio_tungstenite::connect_async(&relay_url).await {
        Ok((ws, _)) => ws,
        Err(e) => {
            error!(error = %e, "ssh forward: failed to connect to relay");
            return;
        }
    };

    let (mut unix_read, unix_write) = tokio::io::split(unix_stream);
    let (mut ws_sink, ws_stream) = ws_stream.split();

    if ws_sink.send(Message::Text(token.into())).await.is_err() {
        error!("ssh forward: failed to send auth token");
        return;
    }

    let unix_to_ws = async {
        let mut buf = vec![0u8; 4096];
        loop {
            match unix_read.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let Some(data) = buf.get(..n) else {
                        break;
                    };
                    if ws_sink
                        .send(Message::Binary(data.to_vec().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    };

    let ws_to_unix = drain_frames_to_writer(
        ws_stream.map(|msg| match msg {
            Ok(Message::Binary(data)) => RelayFrame::Data(data),
            Ok(Message::Close(_)) => RelayFrame::Stop,
            Ok(Message::Text(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {
                RelayFrame::Skip
            }
            Err(e) => {
                warn!(error = %e, "ssh forward: ws read error");
                RelayFrame::Stop
            }
        }),
        unix_write,
    );

    tokio::join!(unix_to_ws, ws_to_unix);
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests use std::fs for simple synchronous setup/assertions"
)]
mod tests {
    use super::*;

    #[test]
    fn relay_url_strips_ws_agent_path() {
        let url = build_relay_url("ws://server:8080/ws/agent", "myhost").unwrap();
        assert_eq!(url, "ws://server:8080/ws/ssh-agent/myhost");
    }

    #[test]
    fn relay_url_strips_trailing_slash() {
        let url = build_relay_url("ws://server:8080/ws/agent/", "myhost").unwrap();
        assert_eq!(url, "ws://server:8080/ws/ssh-agent/myhost");
    }

    #[test]
    fn relay_url_rejects_empty_base() {
        let err = build_relay_url("/ws/agent", "myhost").unwrap_err();
        assert!(matches!(err, SshForwardError::Url(_)));
    }

    #[test]
    fn relay_url_bare_host() {
        let url = build_relay_url("ws://server:8080", "myhost").unwrap();
        assert_eq!(url, "ws://server:8080/ws/ssh-agent/myhost");
    }

    #[tokio::test]
    async fn forward_socket_creates_temp_path() {
        let socket = SshForwardSocket::create().unwrap();
        assert!(socket.socket_path.to_str().unwrap().ends_with("agent.sock"));
        assert!(socket.socket_path.parent().unwrap().exists());
    }

    #[tokio::test]
    async fn forward_socket_dropped_cleans_up() {
        let dir_path = {
            let socket = SshForwardSocket::create().unwrap();
            socket.socket_path.parent().unwrap().to_path_buf()
        };
        assert!(!dir_path.exists());
    }

    #[tokio::test]
    async fn run_ssh_forward_binds_socket() {
        let socket = SshForwardSocket::create().unwrap();
        run_ssh_forward(&socket, "ws://127.0.0.1:9999/ws/agent", "host", "token")
            .await
            .unwrap();
        assert!(socket.socket_path.exists());
    }
}
