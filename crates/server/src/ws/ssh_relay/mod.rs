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

use self::{
    agent_protocol::{
        AgentRequest, FAILURE_REPLY, FrameError, ReplyError, RequestBuffer, SUCCESS_REPLY,
    },
    scope::{RelayScope, Session},
    session_bind::{Extension, ExtensionName},
};
use crate::{AppState, ws::handler::verify_agent_token};

mod agent_protocol;
mod scope;
mod session_bind;
mod sign_request;
#[cfg(test)]
mod test_messages;
mod wire;

/// WebSocket upgrade handler for SSH agent forwarding.
///
/// Opens the encrypted agent channel, authenticates the agent by the token it
/// sends inside it, then relays the SSH agent requests it is allowed to make
/// (listing keys, and signing a login to one of its own repository hosts) to
/// the server's local `SSH_AUTH_SOCK`.
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

    let Some(authenticated) = verify_agent_token(&state.pool, &hostname, &token).await else {
        tracing::warn!(hostname = %hostname, "ssh relay: authentication failed");
        let _ = sink.send(Message::Close(None)).await;
        return;
    };

    let scope =
        match crate::db::list_relay_destinations_for_agent(&state.pool, authenticated.id).await {
            Ok(destinations) => RelayScope::new(destinations, &hostname),
            Err(e) => {
                tracing::error!(
                    hostname = %hostname,
                    error = %e,
                    "ssh relay: failed to load the agent's repository hosts"
                );
                let _ = sink.send(Message::Close(None)).await;
                return;
            }
        };

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
    relay(sink, stream, agent, Session::new(scope), &hostname, &state).await;
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
    mut session: Session,
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
        result = relay_requests(&mut ws_sink, &mut ws_stream, agent, &mut session, hostname) => {
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
/// `SSH_AGENT_FAILURE` itself. The key every repository host trusts would
/// sign a login to any of them, so a sign request is forwarded only for a
/// login `session` allows, and refused like the rest otherwise. Taking one
/// request at a time keeps those replies in order with the agent's own, as
/// the protocol requires.
async fn relay_requests<S, Si, St, E>(
    ws_sink: &mut Si,
    ws_stream: &mut St,
    mut agent: S,
    session: &mut Session,
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
        let reply = match request.kind {
            AgentRequest::RequestIdentities => forward(&mut agent, &request.bytes).await?,
            AgentRequest::SignRequest => match session.authorize_sign(request.body()) {
                Ok(()) => forward(&mut agent, &request.bytes).await?,
                Err(refusal) => {
                    tracing::warn!(
                        hostname = %hostname,
                        reason = %refusal,
                        "ssh relay: refused a sign request"
                    );
                    FAILURE_REPLY.to_vec()
                }
            },
            AgentRequest::Extension => extension_reply(session, request.body(), hostname).to_vec(),
            AgentRequest::Refused(message_type) => {
                tracing::warn!(
                    hostname = %hostname,
                    message_type,
                    "ssh relay: refused an SSH agent request that is not a key listing or signature"
                );
                FAILURE_REPLY.to_vec()
            }
        };
        ws_sink
            .send(Message::Binary(reply.into()))
            .await
            .map_err(|_| RelayError::WebSocketClosed)?;
    }
    Ok(())
}

/// Writes `request` to the server's SSH agent and reads its reply.
async fn forward<S>(agent: &mut S, request: &[u8]) -> Result<Vec<u8>, RelayError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    agent
        .write_all(request)
        .await
        .map_err(RelayError::AgentWrite)?;
    Ok(agent_protocol::read_reply(agent).await?)
}

/// The relay's own answer to an extension request whose body is `body`:
/// success for a `session-bind` it accepts, failure for anything else.
fn extension_reply(session: &mut Session, body: &[u8], hostname: &str) -> [u8; 5] {
    let extension = match Extension::parse(body) {
        Ok(extension) => extension,
        Err(e) => {
            tracing::warn!(
                hostname = %hostname,
                error = %e,
                "ssh relay: refused a malformed SSH agent extension"
            );
            return FAILURE_REPLY;
        }
    };
    match extension.name {
        ExtensionName::SessionBind => match session.bind(extension.fields) {
            Ok(()) => {
                tracing::debug!(hostname = %hostname, "ssh relay: accepted a session-bind");
                SUCCESS_REPLY
            }
            Err(refusal) => {
                tracing::warn!(
                    hostname = %hostname,
                    reason = %refusal,
                    "ssh relay: refused a session-bind"
                );
                FAILURE_REPLY
            }
        },
        ExtensionName::Other => {
            tracing::debug!(hostname = %hostname, "ssh relay: refused an SSH agent extension");
            FAILURE_REPLY
        }
    }
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

    use super::{
        agent_protocol::tests::message,
        test_messages::{HostKey, LoginRequest, session_bind},
        *,
    };
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
        authenticate(open(pool, hostname, agent).await).await
    }

    /// Runs the handshake on an opened connection and sends the token.
    async fn authenticate(
        (ws, shutdown): (Socket, CancellationToken),
    ) -> (Client, CancellationToken) {
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

    /// As [`connect`], for an agent with a backup schedule to a repository
    /// on each of `repo_hosts`, logging in there as the user given with it.
    async fn connect_with_repo_hosts(
        pool: PgPool,
        hostname: &'static str,
        agent: Option<String>,
        repo_hosts: &[(&HostKey, &str)],
    ) -> Client {
        authenticate(open_with_repo_hosts(pool, hostname, agent, repo_hosts).await)
            .await
            .0
    }

    /// Gives the agent `agent_id` a backup schedule to a repository on a
    /// host pinned to `host_key`, logging in as `ssh_user`.
    async fn add_repo_host(
        pool: &PgPool,
        agent_id: i64,
        repo_name: &str,
        host_key: &HostKey,
        ssh_user: &str,
    ) {
        let repo = crate::db::insert_repo(
            pool,
            &crate::db::InsertRepoParams {
                name: repo_name,
                repo_path: "/backups/relay",
                ssh_user,
                ssh_host: &format!("{repo_name}.local"),
                ssh_port: 22,
                passphrase_encrypted: b"encrypted",
                compression: "lz4",
                encryption: "repokey",
                owner_id: None,
                sync_schedule: None,
            },
        )
        .await
        .expect("insert repo");
        crate::db::update_repo_ssh_host_key(pool, repo.id, &host_key.openssh())
            .await
            .expect("pin host key");
        let schedule = crate::db::insert_schedule(
            pool,
            repo.id,
            &crate::db::ScheduleParams::for_test(repo_name, "0 3 * * *"),
            None,
        )
        .await
        .expect("insert schedule");
        crate::db::insert_schedule_targets(pool, schedule.id, &[(agent_id, 0)])
            .await
            .expect("target the agent");
    }

    /// Serves the relay as [`connect`] does and returns the raw, not yet
    /// encrypted, connection to it.
    async fn open(
        pool: PgPool,
        hostname: &'static str,
        agent: Option<String>,
    ) -> (Socket, CancellationToken) {
        open_with_repo_hosts(pool, hostname, agent, &[]).await
    }

    /// As [`open`], for an agent with a repository on each of `repo_hosts`
    /// as [`connect_with_repo_hosts`] describes.
    async fn open_with_repo_hosts(
        pool: PgPool,
        hostname: &'static str,
        agent: Option<String>,
        repo_hosts: &[(&HostKey, &str)],
    ) -> (Socket, CancellationToken) {
        let hash = bcrypt::hash(TOKEN, 4).expect("hash token");
        let agent_row = crate::db::insert_agent(&pool, hostname, None, &hash, None, None)
            .await
            .expect("insert agent");
        for (index, (host_key, ssh_user)) in repo_hosts.iter().enumerate() {
            let repo_name = format!("{hostname}-repo-{index}");
            add_repo_host(&pool, agent_row.id, &repo_name, host_key, ssh_user).await;
        }
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
    /// `SSH_AGENTC_REMOVE_ALL_IDENTITIES`, then `SSH_AGENTC_LOCK`), and an
    /// extension the relay cannot accept (here a `session-bind` with nothing
    /// after its name to verify), are answered with `SSH_AGENT_FAILURE` and
    /// never reach it, while a key listing sent after them, split across two
    /// frames, still does.
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
        let session_bind = message(27, b"\0\0\0\x18session-bind@openssh.com");
        send(&mut ws, &[message(19, b""), lock, session_bind].concat()).await;
        let failures = [FAILURE_REPLY, FAILURE_REPLY, FAILURE_REPLY].concat();
        let refusals = receive(&mut ws, failures.len()).await;
        let keys = list_keys();
        let (head, tail) = keys.split_at(2);
        send(&mut ws, head).await;
        send(&mut ws, tail).await;
        let listing = receive(&mut ws, no_keys().len()).await;
        ws.sink.close().await.expect("close");

        assert_eq!(refusals, failures);
        assert_eq!(listing, no_keys());
        assert_eq!(agent.await.expect("agent"), list_keys());
    }

    /// An extension whose name cannot be read, and one the relay does not
    /// know, are answered with `SSH_AGENT_FAILURE` by the relay itself and
    /// never reach the SSH agent, which the relay keeps serving.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn extensions_the_relay_does_not_accept_never_reach_the_ssh_agent(pool: PgPool) {
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

        let mut ws = connect(pool, "relay-extension-host", Some(path)).await;
        // A name said to be 255 bytes long with none of them sent.
        let malformed = message(27, b"\0\0\0\xff");
        let unknown = message(27, b"\0\0\0\x15query@example.invalid");
        send(&mut ws, &[malformed, unknown].concat()).await;
        let failures = [FAILURE_REPLY, FAILURE_REPLY].concat();
        let refusals = receive(&mut ws, failures.len()).await;
        send(&mut ws, &list_keys()).await;
        let listing = receive(&mut ws, no_keys().len()).await;
        ws.sink.close().await.expect("close");

        assert_eq!(refusals, failures);
        assert_eq!(listing, no_keys());
        assert_eq!(agent.await.expect("agent"), list_keys());
    }

    /// When the agent's repository hosts cannot be read, the relay cannot
    /// tell which logins it may sign, so it closes before it connects to the
    /// SSH agent at all.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_relay_whose_repository_hosts_cannot_be_read_ends(pool: PgPool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let (listener, path) = fake_agent(&dir);
        let host = HostKey::generate();
        let opened = open_with_repo_hosts(
            pool.clone(),
            "relay-unreadable-hosts",
            Some(path),
            &[(&host, "borg")],
        )
        .await;
        // Broken before the token is sent, which is when the relay reads the
        // agent's repository hosts; the token itself is still checked.
        sqlx::query!("ALTER TABLE repo_hosts RENAME TO repo_hosts_unreadable")
            .execute(&pool)
            .await
            .expect("hide the repository hosts");
        let (mut ws, _shutdown) = authenticate(opened).await;

        assert!(closes_without_data(&mut ws).await);
        let accepted = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
        assert!(accepted.is_err(), "nothing connects to the SSH agent");
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
    /// one read of the agent socket. The sign request is a login to the
    /// agent's own repository host, bound to it first, so the relay lets it
    /// through.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn bytes_flow_both_ways_for_several_exchanges(pool: PgPool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let (listener, path) = fake_agent(&dir);
        let signature: Vec<u8> = (0..=u8::MAX).cycle().take(64 * 1024).collect();
        let large = message(14, &signature);
        let large_reply = large.clone();
        let repo_host = HostKey::generate();
        let sign = LoginRequest::new(b"exchanges-session", b"borg").encode();
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

        let mut ws = connect_with_repo_hosts(
            pool,
            "relay-exchanges-host",
            Some(path),
            &[(&repo_host, "borg")],
        )
        .await;
        send(&mut ws, &list_keys()).await;
        let first = receive(&mut ws, no_keys().len()).await;
        ws.sink
            .send(WsMessage::Text("not for the agent".into()))
            .await
            .expect("send text");
        // The relay answers the bind itself, so it never reaches the agent.
        send(
            &mut ws,
            &session_bind(&repo_host, b"exchanges-session", false),
        )
        .await;
        let bound = receive(&mut ws, SUCCESS_REPLY.len()).await;
        assert_eq!(bound, SUCCESS_REPLY);
        send(&mut ws, &sign).await;
        let second = receive(&mut ws, large.len()).await;

        assert_eq!(first, no_keys());
        assert_eq!(second, large);
        assert_eq!(agent.await.expect("agent"), vec![list_keys(), sign]);
    }

    /// The attack the bind check closes: one agent machine having the
    /// server's key sign a login to a repository host only another agent
    /// backs up to. Every way of asking is refused without reaching the SSH
    /// agent - before any bind, with a bind to that host, and after a bind
    /// to its own host but for another session, user or host key - while a
    /// login to its own host, as the user its repository uses, is signed.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn only_a_login_to_the_agents_own_repository_host_is_signed(pool: PgPool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let (listener, path) = fake_agent(&dir);
        let own = HostKey::generate();
        let unrelated = HostKey::generate();
        let signature = message(14, b"signature");
        let signature_reply = signature.clone();
        let allowed = LoginRequest::new(b"own-session", b"borg").encode();
        let allowed_len = allowed.len();
        let agent = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = vec![0; allowed_len];
            stream.read_exact(&mut request).await.expect("read");
            stream.write_all(&signature_reply).await.expect("write");
            let mut rest = Vec::new();
            stream.read_to_end(&mut rest).await.expect("read rest");
            [request, rest].concat()
        });
        let other = crate::db::insert_agent(&pool, "relay-other-host", None, "hash", None, None)
            .await
            .expect("insert other agent");
        add_repo_host(&pool, other.id, "relay-other-repo", &unrelated, "borg").await;

        let mut ws =
            connect_with_repo_hosts(pool, "relay-scoped-host", Some(path), &[(&own, "borg")]).await;
        let unbound = [
            LoginRequest::new(b"unrelated-session", b"borg").encode(),
            session_bind(&unrelated, b"unrelated-session", false),
            LoginRequest::new(b"unrelated-session", b"borg").encode(),
        ];
        let mut refusals = Vec::new();
        for request in &unbound {
            send(&mut ws, request).await;
            refusals.extend(receive(&mut ws, FAILURE_REPLY.len()).await);
        }
        send(&mut ws, &session_bind(&own, b"own-session", false)).await;
        let bound = receive(&mut ws, SUCCESS_REPLY.len()).await;
        let mismatched = [
            LoginRequest::new(b"unrelated-session", b"borg").encode(),
            LoginRequest::new(b"own-session", b"root").encode(),
            LoginRequest {
                host_bound: Some(unrelated.public_blob()),
                ..LoginRequest::new(b"own-session", b"borg")
            }
            .encode(),
        ];
        for request in &mismatched {
            send(&mut ws, request).await;
            refusals.extend(receive(&mut ws, FAILURE_REPLY.len()).await);
        }
        send(&mut ws, &allowed).await;
        let signed = receive(&mut ws, signature.len()).await;
        ws.sink.close().await.expect("close");

        assert_eq!(
            refusals,
            FAILURE_REPLY.repeat(unbound.len().saturating_add(mismatched.len()))
        );
        assert_eq!(bound, SUCCESS_REPLY);
        assert_eq!(signed, signature);
        assert_eq!(agent.await.expect("agent"), allowed);
    }

    /// A repository host without a pinned key cannot be told apart from any
    /// other, so an agent whose only repository is on one has nothing it
    /// can bind to, and its logins are refused.
    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_repository_host_without_a_pinned_key_is_never_signed_for(pool: PgPool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let (listener, path) = fake_agent(&dir);
        let agent = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut received = Vec::new();
            stream.read_to_end(&mut received).await.expect("read");
            received
        });
        let host = HostKey::generate();
        let opened = open_with_repo_hosts(
            pool.clone(),
            "relay-unpinned-host",
            Some(path),
            &[(&host, "borg")],
        )
        .await;
        // Unpinned before the token is sent, which is when the relay reads
        // the agent's repository hosts.
        sqlx::query!("UPDATE repo_hosts SET ssh_host_key = NULL")
            .execute(&pool)
            .await
            .expect("unpin");
        let (mut ws, _shutdown) = authenticate(opened).await;

        send(&mut ws, &session_bind(&host, b"session", false)).await;
        let refused_bind = receive(&mut ws, FAILURE_REPLY.len()).await;
        send(&mut ws, &LoginRequest::new(b"session", b"borg").encode()).await;
        let refused_sign = receive(&mut ws, FAILURE_REPLY.len()).await;
        ws.sink.close().await.expect("close");

        assert_eq!(refused_bind, FAILURE_REPLY);
        assert_eq!(refused_sign, FAILURE_REPLY);
        assert_eq!(agent.await.expect("agent"), Vec::<u8>::new());
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
