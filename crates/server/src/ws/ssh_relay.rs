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

    // Either side closing ends the relay: the SSH agent never closes a
    // connection on its own, and a closed WebSocket gets no further replies.
    tokio::select! {
        () = unix_to_ws => {}
        () = ws_to_unix => {}
        () = state.shutdown_token.cancelled() => {
            tracing::debug!(hostname = %hostname, "shutdown signal received, closing relay");
        }
    }
    // Dropping the agent's halves on return closes its connection.
    let _ = ws_sink.close().await;

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
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::{UnixListener, UnixStream},
    };
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    use tokio_util::sync::CancellationToken;

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
        connect_with_shutdown(pool, hostname, agent).await.0
    }

    /// As [`connect`], also returning the server's shutdown token.
    async fn connect_with_shutdown(
        pool: PgPool,
        hostname: &'static str,
        agent: Option<String>,
    ) -> (Client, CancellationToken) {
        let hash = bcrypt::hash(TOKEN, 4).expect("hash token");
        crate::db::insert_agent(&pool, hostname, None, &hash, None, None)
            .await
            .expect("insert agent");
        let state = crate::test_support::build_test_state(pool, b"ssh-relay-test-secret-key");
        let shutdown = state.shutdown_token.clone();
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
        (ws, shutdown)
    }

    /// A fake SSH agent listening on a Unix socket in `dir`, and its path.
    fn fake_agent(dir: &tempfile::TempDir) -> (UnixListener, String) {
        let path = dir.path().join("agent.sock");
        let listener = UnixListener::bind(&path).expect("bind agent");
        (listener, path.to_str().expect("utf-8 path").to_owned())
    }

    /// Reads binary frames from `ws` until `len` bytes have arrived.
    async fn receive(ws: &mut Client, len: usize) -> Vec<u8> {
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut received = Vec::new();
            while received.len() < len {
                if let WsMessage::Binary(data) = ws.next().await.expect("open").expect("frame") {
                    received.extend_from_slice(&data);
                }
            }
            received
        })
        .await
        .expect("a reply")
    }

    /// Whether `stream` reaches its end, its peer having closed it.
    async fn reaches_eof(stream: &mut UnixStream) -> bool {
        let mut rest = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut rest))
            .await
            .expect("the agent connection ends")
            .is_ok()
    }

    /// Whether the server closes `ws` without relaying anything.
    async fn closes_without_data(ws: &mut Client) -> bool {
        let frames: Vec<WsMessage> = tokio::time::timeout(
            Duration::from_secs(10),
            ws.filter_map(|m| async { m.ok() }).collect(),
        )
        .await
        .expect("the relay ends");
        !frames.iter().any(WsMessage::is_binary)
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
        // The relay sends only what the SSH agent wrote, so its first frame
        // is the reply.
        let reply = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("a reply")
            .expect("open")
            .expect("frame");

        assert_eq!(reply, WsMessage::Binary(b"pong".to_vec().into()));
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

    /// Several requests in one connection, each answered in turn, with a
    /// text frame in between that the relay skips and a reply too large for
    /// one read of the agent socket.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn bytes_flow_both_ways_for_several_exchanges(pool: PgPool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let (listener, path) = fake_agent(&dir);
        let large: Vec<u8> = (0..=u8::MAX).cycle().take(64 * 1024).collect();
        let large_reply = large.clone();
        let agent = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut requests = Vec::new();
            for reply in [b"one!".as_slice(), large_reply.as_slice()] {
                let mut request = vec![0; 4];
                stream.read_exact(&mut request).await.expect("read");
                stream.write_all(reply).await.expect("write");
                requests.push(request);
            }
            requests
        });

        let mut ws = connect(pool, "relay-exchanges-host", Some(path)).await;
        ws.send(WsMessage::Binary(b"req1".to_vec().into()))
            .await
            .expect("send");
        let first = receive(&mut ws, 4).await;
        ws.send(WsMessage::Text("not for the agent".into()))
            .await
            .expect("send text");
        ws.send(WsMessage::Binary(b"req2".to_vec().into()))
            .await
            .expect("send");
        let second = receive(&mut ws, large.len()).await;

        assert_eq!(first, b"one!");
        assert_eq!(second, large);
        assert_eq!(
            agent.await.expect("agent"),
            vec![b"req1".to_vec(), b"req2".to_vec()]
        );
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn closing_the_websocket_closes_the_ssh_agent_connection(pool: PgPool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let (listener, path) = fake_agent(&dir);
        let agent = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            reaches_eof(&mut stream).await
        });

        let mut ws = connect(pool, "relay-ws-close-host", Some(path)).await;
        ws.close(None).await.expect("close");

        assert!(agent.await.expect("agent"));
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn the_ssh_agent_closing_closes_the_websocket(pool: PgPool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let (listener, path) = fake_agent(&dir);
        let agent = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            drop(stream);
        });

        let mut ws = connect(pool, "relay-agent-close-host", Some(path)).await;
        agent.await.expect("agent");

        assert!(closes_without_data(&mut ws).await);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn shutting_down_ends_the_relay(pool: PgPool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let (listener, path) = fake_agent(&dir);
        let (connected, accepted) = tokio::sync::oneshot::channel();
        let agent = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            connected.send(()).expect("test waits");
            reaches_eof(&mut stream).await
        });

        let (mut ws, shutdown) =
            connect_with_shutdown(pool, "relay-shutdown-host", Some(path)).await;
        accepted.await.expect("agent connected");
        shutdown.cancel();

        assert!(closes_without_data(&mut ws).await);
        assert!(agent.await.expect("agent"));
    }
}
