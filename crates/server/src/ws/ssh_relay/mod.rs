// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use axum::{
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use shared::ssh::{agent_socket, connect_agent};
use tokio::{
    io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _},
    time,
};

use self::agent_protocol::{FAILURE_REPLY, FrameError, ReplyError, RequestBuffer};
use crate::{AppState, ws::handler::verify_agent_token};

mod agent_protocol;

/// WebSocket upgrade handler for SSH agent forwarding.
///
/// Opens the encrypted agent channel, authenticates the agent by the token it
/// sends inside it, then relays the SSH agent requests it is allowed to make
/// (listing keys and signing) to the server's local `SSH_AUTH_SOCK`.
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
    socket: WebSocket,
    hostname: String,
    state: AppState,
    agent: Option<String>,
) {
    let (sink, stream) = socket.split();
    let Some((mut sink, mut stream)) =
        crate::ws::channel::establish(sink, stream, &state.channel_keys, &hostname).await
    else {
        return;
    };
    let token =
        match time::timeout(time::Duration::from_secs(5), read_auth_token(&mut stream)).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                tracing::warn!(hostname = %hostname, "ssh relay: no auth token received");
                let _ = sink.send(Message::Close(None)).await;
                return;
            }
            Err(_elapsed) => {
                tracing::warn!(hostname = %hostname, "ssh relay: auth token timeout");
                let _ = sink.send(Message::Close(None)).await;
                return;
            }
        };

    if verify_agent_token(&state.pool, &hostname, &token)
        .await
        .is_none()
    {
        tracing::warn!(hostname = %hostname, "ssh relay: authentication failed");
        let _ = sink.send(Message::Close(None)).await;
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
    relay(sink, stream, agent, &hostname, &state).await;
}

/// Why a relay ended early.
#[derive(Debug, thiserror::Error)]
enum RelayError {
    /// The agent machine sent a message the protocol does not allow.
    #[error("agent machine broke the SSH agent protocol: {0}")]
    Request(#[from] FrameError),
    /// The server's SSH agent's reply could not be read.
    #[error(transparent)]
    Reply(#[from] ReplyError),
    /// Writing a request to the server's SSH agent failed.
    #[error("writing to the SSH agent: {0}")]
    AgentWrite(std::io::Error),
    /// The server's SSH agent sent bytes nobody asked for.
    #[error("the SSH agent sent an unsolicited reply")]
    Unsolicited,
    /// The WebSocket would not take a reply.
    #[error("the WebSocket closed before a reply was sent")]
    WebSocketClosed,
}

async fn relay<S, Si, St, E>(
    mut ws_sink: Si,
    mut ws_stream: St,
    agent: S,
    hostname: &str,
    state: &AppState,
) where
    S: AsyncRead + AsyncWrite + Unpin,
    Si: Sink<Message> + Unpin,
    St: Stream<Item = Result<Message, E>> + Unpin,
    E: std::fmt::Display,
{
    tracing::info!(hostname = %hostname, "ssh relay connection opened");

    // Either side closing ends the relay: the SSH agent never closes a
    // connection on its own, and a closed WebSocket gets no further replies.
    tokio::select! {
        result = relay_requests(&mut ws_sink, &mut ws_stream, agent, hostname) => {
            if let Err(e) = result {
                tracing::warn!(hostname = %hostname, error = %e, "ssh relay ended");
            }
        }
        () = state.shutdown_token.cancelled() => {
            tracing::debug!(hostname = %hostname, "shutdown signal received, closing relay");
        }
    }
    // Dropping the agent connection on return closes it.
    let _ = ws_sink.close().await;

    tracing::info!(hostname = %hostname, "ssh relay connection closed");
}

/// Relays requests from the agent machine to the server's SSH agent one at a
/// time, each followed by its reply, until either side closes.
///
/// Only requests to list the keys and to sign with one reach the SSH agent;
/// any other request (adding, removing or locking keys, extensions) would
/// change the agent every host shares, so the relay answers it with
/// `SSH_AGENT_FAILURE` itself. Taking one request at a time keeps those
/// replies in order with the agent's own, as the protocol requires.
async fn relay_requests<S, Si, St, E>(
    ws_sink: &mut Si,
    ws_stream: &mut St,
    mut agent: S,
    hostname: &str,
) -> Result<(), RelayError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    Si: Sink<Message> + Unpin,
    St: Stream<Item = Result<Message, E>> + Unpin,
    E: std::fmt::Display,
{
    let mut requests = RequestBuffer::default();
    while let Some(request) = next_request(ws_stream, &mut requests, &mut agent).await? {
        let reply = if request.kind.is_allowed() {
            agent
                .write_all(&request.bytes)
                .await
                .map_err(RelayError::AgentWrite)?;
            agent_protocol::read_reply(&mut agent).await?
        } else {
            tracing::warn!(
                hostname = %hostname,
                request = ?request.kind,
                "ssh relay: refused an SSH agent request that is not a key listing or signature"
            );
            FAILURE_REPLY.to_vec()
        };
        ws_sink
            .send(Message::Binary(reply.into()))
            .await
            .map_err(|_| RelayError::WebSocketClosed)?;
    }
    Ok(())
}

/// The agent machine's next complete request, or `None` once it has closed
/// the WebSocket or the server's SSH agent has closed its connection.
async fn next_request<S, St, E>(
    ws_stream: &mut St,
    requests: &mut RequestBuffer,
    agent: &mut S,
) -> Result<Option<agent_protocol::Request>, RelayError>
where
    S: AsyncRead + Unpin,
    St: Stream<Item = Result<Message, E>> + Unpin,
    E: std::fmt::Display,
{
    loop {
        if let Some(request) = requests.next_request()? {
            return Ok(Some(request));
        }
        // While waiting for a request the SSH agent has nothing to say, so
        // anything read from it is either its connection closing or a
        // protocol violation.
        let mut unsolicited = [0; 1];
        tokio::select! {
            message = ws_stream.next() => match message {
                Some(Ok(Message::Binary(data))) => requests.extend(&data),
                Some(Ok(Message::Text(_) | Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | None => return Ok(None),
                Some(Err(e)) => {
                    tracing::warn!(error = %e, "ssh relay: ws read error");
                    return Ok(None);
                }
            },
            read = agent.read(&mut unsolicited) => {
                let read = read.map_err(|e| RelayError::Reply(ReplyError::Io(e)))?;
                return if read == 0 { Ok(None) } else { Err(RelayError::Unsolicited) };
            }
        }
    }
}

async fn read_auth_token<St, E>(stream: &mut St) -> Option<String>
where
    St: Stream<Item = Result<Message, E>> + Unpin,
{
    loop {
        match stream.next().await? {
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

    use shared::agent_channel::ws::{DecryptedStream, EncryptedSink, TungsteniteCodec, initiate};
    use sqlx::PgPool;
    use tokio::net::{UnixListener, UnixStream};
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    use tokio_util::sync::CancellationToken;

    use super::{agent_protocol::tests::message, *};
    use crate::ws::channel::UNENCRYPTED_REASON;

    const TOKEN: &str = "relay-agent-token";

    type Socket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    /// The agent's side of the relay in these tests, over the encrypted
    /// channel.
    struct Client {
        sink: EncryptedSink<TungsteniteCodec, futures_util::stream::SplitSink<Socket, WsMessage>>,
        stream: DecryptedStream<TungsteniteCodec, futures_util::stream::SplitStream<Socket>>,
    }

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
        let (ws, shutdown) = open(pool, hostname, agent).await;
        let mut client = encrypt(ws).await;
        client
            .sink
            .send(WsMessage::Text(TOKEN.into()))
            .await
            .expect("send token");
        (client, shutdown)
    }

    /// Runs the agent's side of the handshake on `ws`.
    async fn encrypt(ws: Socket) -> Client {
        let (mut sink, mut stream) = ws.split();
        let key = crate::test_support::test_channel_key()
            .public_key()
            .unwrap();
        let (sealer, opener) = initiate::<TungsteniteCodec, _, _, _>(&mut sink, &mut stream, &key)
            .await
            .expect("handshake");
        Client {
            sink: EncryptedSink::new(sink, sealer),
            stream: DecryptedStream::new(stream, opener),
        }
    }

    /// Serves the relay as [`connect`] does and returns the raw, not yet
    /// encrypted, connection to it.
    async fn open(
        pool: PgPool,
        hostname: &'static str,
        agent: Option<String>,
    ) -> (Socket, CancellationToken) {
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
        let addr = crate::test_support::serve_on_loopback(app).await;

        let (ws, _) =
            tokio_tungstenite::connect_async(format!("ws://{addr}/ws/ssh-agent/{hostname}"))
                .await
                .expect("connect");
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
                if let WsMessage::Binary(data) =
                    ws.stream.next().await.expect("open").expect("frame")
                {
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
            (&mut ws.stream).filter_map(|m| async { m.ok() }).collect(),
        )
        .await
        .expect("the relay ends");
        !frames.iter().any(WsMessage::is_binary)
    }

    /// `SSH_AGENTC_REQUEST_IDENTITIES`.
    fn list_keys() -> Vec<u8> {
        message(11, b"")
    }

    /// `SSH_AGENT_IDENTITIES_ANSWER` with no keys.
    fn no_keys() -> Vec<u8> {
        message(12, &[0, 0, 0, 0])
    }

    /// Sends `bytes` to the relay as one binary frame.
    async fn send(ws: &mut Client, bytes: &[u8]) {
        ws.sink
            .send(WsMessage::Binary(bytes.to_vec().into()))
            .await
            .expect("send");
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_authenticated_agent_is_relayed_to_the_ssh_agent(pool: PgPool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("agent.sock");
        let listener = tokio::net::UnixListener::bind(&path).expect("bind agent");
        let agent = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = vec![0; list_keys().len()];
            stream.read_exact(&mut request).await.expect("read");
            stream.write_all(&no_keys()).await.expect("write");
            request
        });

        let mut ws = connect(
            pool,
            "relay-host",
            Some(path.to_str().expect("utf-8 path").to_owned()),
        )
        .await;
        send(&mut ws, &list_keys()).await;
        // The relay sends only what the SSH agent wrote, so its first frame
        // is the reply.
        let reply = tokio::time::timeout(Duration::from_secs(10), ws.stream.next())
            .await
            .expect("a reply")
            .expect("open")
            .expect("frame");

        assert_eq!(reply, WsMessage::Binary(no_keys().into()));
        assert_eq!(agent.await.expect("agent"), list_keys());
    }

    /// A request that would change the SSH agent every host shares (here
    /// `SSH_AGENTC_REMOVE_ALL_IDENTITIES`, then `SSH_AGENTC_LOCK`) is answered
    /// with `SSH_AGENT_FAILURE` and never reaches it, while a key listing
    /// sent after them, split across two frames, still does.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn requests_that_change_the_ssh_agent_never_reach_it(pool: PgPool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let (listener, path) = fake_agent(&dir);
        let agent = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = vec![0; list_keys().len()];
            stream.read_exact(&mut request).await.expect("read");
            stream.write_all(&no_keys()).await.expect("write");
            let mut rest = Vec::new();
            stream.read_to_end(&mut rest).await.expect("read rest");
            [request, rest].concat()
        });

        let mut ws = connect(pool, "relay-refuse-host", Some(path)).await;
        let lock = message(22, b"\0\0\0\x08password");
        send(&mut ws, &[message(19, b""), lock].concat()).await;
        let two_failures = [FAILURE_REPLY, FAILURE_REPLY].concat();
        let refusals = receive(&mut ws, two_failures.len()).await;
        let keys = list_keys();
        let (head, tail) = keys.split_at(2);
        send(&mut ws, head).await;
        send(&mut ws, tail).await;
        let listing = receive(&mut ws, no_keys().len()).await;
        ws.sink.close().await.expect("close");

        assert_eq!(refusals, two_failures);
        assert_eq!(listing, no_keys());
        assert_eq!(agent.await.expect("agent"), list_keys());
    }

    /// The SSH agent never speaks unasked, so bytes from it while no request
    /// is waiting for a reply end the relay instead of being passed on as if
    /// they answered the next one.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_unsolicited_reply_from_the_ssh_agent_ends_the_relay(pool: PgPool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let (listener, path) = fake_agent(&dir);
        let agent = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            stream.write_all(&no_keys()).await.expect("write");
            // The relay drops the connection with the unsolicited bytes
            // unread, so this side may see a reset rather than a clean end;
            // either way the read returns once the connection is closed.
            let mut rest = Vec::new();
            tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut rest))
                .await
                .is_ok()
        });

        let mut ws = connect(pool, "relay-unsolicited-host", Some(path)).await;

        assert!(closes_without_data(&mut ws).await);
        assert!(
            agent.await.expect("agent"),
            "the relay left the SSH agent connection open"
        );
    }

    /// A request announcing more than the protocol's 256 KiB limit ends the
    /// relay from its length prefix alone, without reaching the SSH agent.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn an_oversized_request_ends_the_relay(pool: PgPool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let (listener, path) = fake_agent(&dir);
        let agent = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut received = Vec::new();
            stream.read_to_end(&mut received).await.expect("read");
            received
        });

        let mut ws = connect(pool, "relay-oversized-host", Some(path)).await;
        let len = u32::try_from(agent_protocol::MAX_MESSAGE_LEN + 1).expect("length");
        send(&mut ws, &len.to_be_bytes()).await;

        assert!(closes_without_data(&mut ws).await);
        assert_eq!(agent.await.expect("agent"), Vec::<u8>::new());
    }

    /// An agent from before the encrypted channel sends its token as plain
    /// text; it is refused before the token is even looked at, and nothing
    /// is relayed.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_plain_text_token_is_refused(pool: PgPool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let (listener, path) = fake_agent(&dir);
        let (mut ws, _shutdown) = open(pool, "relay-plain-host", Some(path)).await;
        ws.send(WsMessage::Text(TOKEN.into()))
            .await
            .expect("send token");

        let frames: Vec<WsMessage> = tokio::time::timeout(
            Duration::from_secs(10),
            ws.filter_map(|m| async { m.ok() }).collect(),
        )
        .await
        .expect("the relay ends");

        let refused = |frame: &WsMessage| matches!(frame, WsMessage::Close(Some(close)) if close.reason.as_str() == UNENCRYPTED_REASON);
        assert!(frames.iter().any(refused));
        assert!(!frames.iter().any(WsMessage::is_binary));
        let accepted = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
        assert!(accepted.is_err(), "nothing connects to the SSH agent");
    }

    /// The relay ends without reaching the SSH agent when the encrypted
    /// connection closes before a token, sends a wrong one, or sends none
    /// before the deadline.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_connection_without_a_valid_token_is_never_relayed(pool: PgPool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let (listener, path) = fake_agent(&dir);

        let (ws, _shutdown) = open(pool.clone(), "relay-closed-host", Some(path.clone())).await;
        let mut closed = encrypt(ws).await;
        closed.sink.close().await.expect("close");
        assert!(closes_without_data(&mut closed).await);

        let (ws, _shutdown) = open(pool.clone(), "relay-wrong-host", Some(path.clone())).await;
        let mut wrong = encrypt(ws).await;
        wrong
            .sink
            .send(WsMessage::Text("not-the-token".into()))
            .await
            .expect("send token");
        assert!(closes_without_data(&mut wrong).await);

        let (ws, _shutdown) = open(pool, "relay-silent-host", Some(path)).await;
        let mut silent = encrypt(ws).await;
        assert!(closes_without_data(&mut silent).await);

        let accepted = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
        assert!(accepted.is_err(), "nothing connects to the SSH agent");
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
        let signature: Vec<u8> = (0..=u8::MAX).cycle().take(64 * 1024).collect();
        let large = message(14, &signature);
        let large_reply = large.clone();
        let sign = message(13, b"key-and-data");
        let sign_len = sign.len();
        let agent = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut requests = Vec::new();
            for (reply, len) in [(no_keys(), list_keys().len()), (large_reply, sign_len)] {
                let mut request = vec![0; len];
                stream.read_exact(&mut request).await.expect("read");
                stream.write_all(&reply).await.expect("write");
                requests.push(request);
            }
            requests
        });

        let mut ws = connect(pool, "relay-exchanges-host", Some(path)).await;
        send(&mut ws, &list_keys()).await;
        let first = receive(&mut ws, no_keys().len()).await;
        ws.sink
            .send(WsMessage::Text("not for the agent".into()))
            .await
            .expect("send text");
        send(&mut ws, &sign).await;
        let second = receive(&mut ws, large.len()).await;

        assert_eq!(first, no_keys());
        assert_eq!(second, large);
        assert_eq!(agent.await.expect("agent"), vec![list_keys(), sign]);
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
        ws.sink.close().await.expect("close");

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
