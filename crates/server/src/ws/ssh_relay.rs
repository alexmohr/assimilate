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
use shared::ssh::{RelayFrame, agent_socket, connect_agent, drain_frames_to_writer};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    time,
};
use tokio_util::io::ReaderStream;

use crate::{AppState, ws::handler::verify_agent_token};

/// WebSocket upgrade handler for SSH agent forwarding.
///
/// Authenticates the agent via its token, then relays bytes bidirectionally
/// between the WebSocket and the server's local `SSH_AUTH_SOCK`.
pub fn ssh_relay_handler(
    ws: WebSocketUpgrade,
    Path(hostname): Path<String>,
    State(state): State<AppState>,
) -> impl std::future::Future<Output = impl IntoResponse> {
    let agent = agent_socket(std::env::var("SSH_AUTH_SOCK").ok());
    std::future::ready(ws.on_upgrade(|socket| handle_ssh_relay(socket, hostname, state, agent)))
}

/// Relays `socket` to the SSH agent at `agent`, once the connection has
/// authenticated as `hostname`'s agent.
async fn handle_ssh_relay(
    mut socket: WebSocket,
    hostname: String,
    state: AppState,
    agent: Option<String>,
) {
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

    if verify_agent_token(&state.pool, &hostname, &token)
        .await
        .is_none()
    {
        tracing::warn!(hostname = %hostname, "ssh relay: authentication failed");
        let _ = socket.send(Message::Close(None)).await;
        return;
    }

    let Some(ssh_auth_sock) = agent else {
        tracing::error!("ssh relay: SSH_AUTH_SOCK not set on server");
        return;
    };

    let agent = match connect_agent(&ssh_auth_sock).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "ssh relay: failed to connect to SSH_AUTH_SOCK");
            return;
        }
    };
    relay(socket, agent, &hostname, &state).await;
}

async fn relay<S>(socket: WebSocket, agent: S, hostname: &str, state: &AppState)
where
    S: AsyncRead + AsyncWrite,
{
    tracing::info!(hostname = %hostname, "ssh relay connection opened");

    let (unix_read, unix_write) = tokio::io::split(agent);
    let mut unix_read_stream = ReaderStream::new(unix_read);
    let (mut ws_sink, ws_stream) = socket.split();

    let unix_to_ws = async {
        while let Some(Ok(data)) = unix_read_stream.next().await {
            if ws_sink.send(Message::Binary(data)).await.is_err() {
                break;
            }
        }
    };

    let ws_to_unix = drain_frames_to_writer(
        ws_stream.map(|msg| match msg {
            Ok(Message::Binary(data)) => RelayFrame::Data(data),
            Ok(Message::Close(_)) => RelayFrame::Stop,
            Ok(Message::Text(_) | Message::Ping(_) | Message::Pong(_)) => RelayFrame::Skip,
            Err(e) => {
                tracing::warn!(error = %e, "ssh relay: ws read error");
                RelayFrame::Stop
            }
        }),
        unix_write,
    );

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

/// End to end over a real WebSocket, against a fake SSH agent on a Unix
/// socket. The server's tests are Unix-only; `shared::ssh` tests connecting
/// to an agent on a Windows named pipe.
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use sqlx::PgPool;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    use super::*;

    const TOKEN: &str = "relay-agent-token";

    /// The agent's side of the relay in these tests.
    type Client = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    /// Serves the relay for `hostname`'s agent, whose token is [`TOKEN`],
    /// against the SSH agent at `agent`, and returns a client already
    /// authenticated to it.
    async fn connect(pool: PgPool, hostname: &'static str, agent: Option<String>) -> Client {
        let hash = bcrypt::hash(TOKEN, 4).expect("hash token");
        crate::db::insert_agent(&pool, hostname, None, &hash, None, None)
            .await
            .expect("insert agent");
        let state = crate::test_support::build_test_state(pool, b"ssh-relay-test-secret-key");
        let app = axum::Router::new()
            .route(
                "/ws/ssh-agent/{hostname}",
                axum::routing::get(
                    move |ws: WebSocketUpgrade,
                          Path(hostname): Path<String>,
                          State(state): State<AppState>| {
                        let agent = agent.clone();
                        std::future::ready(ws.on_upgrade(move |socket| {
                            handle_ssh_relay(socket, hostname, state, agent)
                        }))
                    },
                ),
            )
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("address");
        tokio::spawn(async move { axum::serve(listener, app).await });

        let (mut ws, _) =
            tokio_tungstenite::connect_async(format!("ws://{addr}/ws/ssh-agent/{hostname}"))
                .await
                .expect("connect");
        ws.send(WsMessage::Text(TOKEN.into()))
            .await
            .expect("send token");
        ws
    }

    /// Whether the server closes `ws` without relaying anything.
    async fn closes_without_data(ws: &mut Client) -> bool {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match ws.next().await {
                    None | Some(Err(_) | Ok(WsMessage::Close(_))) => return true,
                    Some(Ok(WsMessage::Binary(_))) => return false,
                    Some(Ok(_)) => {}
                }
            }
        })
        .await
        .expect("the relay ends")
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_authenticated_agent_is_relayed_to_the_ssh_agent(pool: PgPool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("agent.sock");
        let listener = tokio::net::UnixListener::bind(&path).expect("bind agent");
        let agent = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = vec![0; 4];
            stream.read_exact(&mut request).await.expect("read");
            stream.write_all(b"pong").await.expect("write");
            request
        });

        let mut ws = connect(
            pool,
            "relay-host",
            Some(path.to_str().expect("utf-8 path").to_owned()),
        )
        .await;
        ws.send(WsMessage::Binary(b"ping".to_vec().into()))
            .await
            .expect("send");
        let reply = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let WsMessage::Binary(data) = ws.next().await.expect("open").expect("frame") {
                    return data.to_vec();
                }
            }
        })
        .await
        .expect("a reply");

        assert_eq!(reply, b"pong");
        assert_eq!(agent.await.expect("agent"), b"ping");
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn without_an_ssh_agent_the_relay_ends(pool: PgPool) {
        let mut ws = connect(pool, "relay-no-agent-host", None).await;

        assert!(closes_without_data(&mut ws).await);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_ssh_agent_that_cannot_be_reached_ends_the_relay(pool: PgPool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nothing.sock");

        let mut ws = connect(
            pool,
            "relay-unreachable-host",
            Some(path.to_str().expect("utf-8 path").to_owned()),
        )
        .await;

        assert!(closes_without_data(&mut ws).await);
    }
}
