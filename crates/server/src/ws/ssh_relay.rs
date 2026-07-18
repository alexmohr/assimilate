// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use tokio::{io::AsyncWriteExt, net::UnixStream, time};
use tokio_util::io::ReaderStream;

use crate::{AppState, db};

/// WebSocket upgrade handler for SSH agent forwarding.
///
/// Authenticates the agent via its token, then relays bytes bidirectionally
/// between the WebSocket and the server's local `SSH_AUTH_SOCK`.
pub fn ssh_relay_handler(
    ws: WebSocketUpgrade,
    Path(hostname): Path<String>,
    State(state): State<AppState>,
) -> impl std::future::Future<Output = impl IntoResponse> {
    std::future::ready(ws.on_upgrade(|socket| handle_ssh_relay(socket, hostname, state)))
}

async fn handle_ssh_relay(mut socket: WebSocket, hostname: String, state: AppState) {
    let token =
        match time::timeout(time::Duration::from_secs(5), read_auth_token(&mut socket)).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                tracing::warn!(hostname = %hostname, "ssh relay: no auth token received");
                let _ = socket.send(Message::Close(None)).await;
                return;
            }
            Err(_elapsed) => {
                tracing::warn!(hostname = %hostname, "ssh relay: auth token timeout");
                let _ = socket.send(Message::Close(None)).await;
                return;
            }
        };

    let authenticated = match db::get_agent_token_hash(&state.pool, &hostname).await {
        Ok((_, hash)) => tokio::task::spawn_blocking(move || bcrypt::verify(&token, &hash))
            .await
            .map_err(|e| tracing::error!(hostname = %hostname, error = %e, "bcrypt task panicked"))
            .and_then(|r| {
                r.inspect_err(|e| {
                    tracing::error!(hostname = %hostname, error = %e, "bcrypt verify failed");
                })
                .map_err(|_| ())
            })
            .unwrap_or(false),
        Err(e) => {
            tracing::warn!(hostname = %hostname, error = %e, "ssh relay: agent lookup failed");
            false
        }
    };

    if !authenticated {
        tracing::warn!(hostname = %hostname, "ssh relay: authentication failed");
        let _ = socket.send(Message::Close(None)).await;
        return;
    }

    let Ok(ssh_auth_sock) = std::env::var("SSH_AUTH_SOCK") else {
        tracing::error!("ssh relay: SSH_AUTH_SOCK not set on server");
        return;
    };

    let unix_stream = match UnixStream::connect(&ssh_auth_sock).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "ssh relay: failed to connect to SSH_AUTH_SOCK");
            return;
        }
    };

    tracing::info!(hostname = %hostname, "ssh relay connection opened");

    let (unix_read, mut unix_write) = tokio::io::split(unix_stream);
    let mut unix_read_stream = ReaderStream::new(unix_read);
    let (mut ws_sink, mut ws_stream) = socket.split();

    let unix_to_ws = async {
        while let Some(Ok(data)) = unix_read_stream.next().await {
            if ws_sink.send(Message::Binary(data)).await.is_err() {
                break;
            }
        }
    };

    let ws_to_unix = async {
        loop {
            match ws_stream.next().await {
                Some(Ok(Message::Binary(data))) => {
                    if unix_write.write_all(&data).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(e)) => {
                    tracing::warn!(error = %e, "ssh relay: ws read error");
                    break;
                }
                Some(Ok(Message::Text(_) | Message::Ping(_) | Message::Pong(_))) => {}
            }
        }
    };

    tokio::select! {
        () = async {
            tokio::join!(unix_to_ws, ws_to_unix);
        } => {}
        () = state.shutdown_token.cancelled() => {
            tracing::debug!(hostname = %hostname, "shutdown signal received, closing relay");
        }
    }

    tracing::info!(hostname = %hostname, "ssh relay connection closed");
}

async fn read_auth_token(socket: &mut WebSocket) -> Option<String> {
    loop {
        match socket.recv().await? {
            Ok(Message::Text(text)) => return Some(text.to_string()),
            Ok(Message::Binary(data)) => return String::from_utf8(data.to_vec()).ok(),
            Ok(Message::Ping(_) | Message::Pong(_)) => {}
            Ok(Message::Close(_)) | Err(_) => return None,
        }
    }
}
