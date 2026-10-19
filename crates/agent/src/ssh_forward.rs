// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! SSH agent forwarding for borg's `ssh`: a local endpoint `SSH_AUTH_SOCK`
//! points at, whose connections are relayed to the server over a WebSocket.
//! On Unix that is a socket in a private temporary directory; on Windows,
//! where OpenSSH reads `SSH_AUTH_SOCK` as a named pipe, a pipe with a random
//! name that only the agent's own user (and administrators) can open.

use std::{path::PathBuf, sync::OnceLock};

use futures_util::{SinkExt, StreamExt};
use shared::ssh::{RelayFrame, drain_frames_to_writer};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite},
    task::AbortHandle,
};
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, warn};

pub struct SshForwardSocket {
    pub socket_path: PathBuf,
    #[cfg(unix)]
    _socket_dir: tempfile::TempDir,
    /// The pipe's first instance, created up front so the name is claimed
    /// before anyone is told about it; [`run_ssh_forward`] starts serving it.
    #[cfg(windows)]
    first_instance: std::sync::Mutex<Option<tokio::net::windows::named_pipe::NamedPipeServer>>,
    /// The task accepting connections, stopped with the socket so a finished
    /// operation leaves no endpoint behind.
    accept_task: OnceLock<AbortHandle>,
}

impl Drop for SshForwardSocket {
    fn drop(&mut self) {
        if let Some(task) = self.accept_task.get() {
            task.abort();
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SshForwardError {
    #[cfg(unix)]
    #[error("failed to create socket directory: {0}")]
    TempDir(std::io::Error),
    #[error("failed to bind the forwarding socket: {0}")]
    Bind(std::io::Error),
    #[error("failed to build relay url: {0}")]
    Url(String),
}

impl SshForwardSocket {
    #[cfg(unix)]
    pub fn create() -> Result<Self, SshForwardError> {
        let socket_dir = tempfile::TempDir::new().map_err(SshForwardError::TempDir)?;
        let socket_path = socket_dir.path().join("agent.sock");
        Ok(Self {
            socket_path,
            _socket_dir: socket_dir,
            accept_task: OnceLock::new(),
        })
    }

    #[cfg(windows)]
    pub fn create() -> Result<Self, SshForwardError> {
        let name = format!(
            r"\\.\pipe\assimilate-ssh-agent-{}",
            uuid::Uuid::new_v4().simple()
        );
        let first = pipe_instance(&name, true).map_err(SshForwardError::Bind)?;
        Ok(Self {
            socket_path: PathBuf::from(name),
            first_instance: std::sync::Mutex::new(Some(first)),
            accept_task: OnceLock::new(),
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
    let task = listen(socket, relay_url, token.to_string())?;
    // `create` hands out a fresh socket each time, so this is its only task.
    let _ = socket.accept_task.set(task.abort_handle());
    Ok(())
}

#[cfg(unix)]
fn listen(
    socket: &SshForwardSocket,
    relay_url: String,
    token: String,
) -> Result<tokio::task::JoinHandle<()>, SshForwardError> {
    let listener =
        tokio::net::UnixListener::bind(&socket.socket_path).map_err(SshForwardError::Bind)?;
    Ok(tokio::spawn(accept_loop(listener, relay_url, token)))
}

#[cfg(windows)]
fn listen(
    socket: &SshForwardSocket,
    relay_url: String,
    token: String,
) -> Result<tokio::task::JoinHandle<()>, SshForwardError> {
    let name = socket.socket_path.to_string_lossy().into_owned();
    let first = socket
        .first_instance
        .lock()
        .ok()
        .and_then(|mut first| first.take())
        .ok_or_else(|| {
            SshForwardError::Bind(std::io::Error::other("the pipe is already being served"))
        })?;
    Ok(tokio::spawn(accept_pipe_loop(
        first, name, relay_url, token,
    )))
}

/// One instance of the forwarding pipe. The first claims the name, so an
/// existing pipe of that name makes setup fail instead of being joined.
#[cfg(windows)]
fn pipe_instance(
    name: &str,
    first: bool,
) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    let mut options = tokio::net::windows::named_pipe::ServerOptions::new();
    options
        .first_pipe_instance(first)
        .reject_remote_clients(true);
    shared::windows_acl::create_private_pipe(
        &options,
        name,
        shared::windows_acl::Access::SystemAdministratorsAndOwner,
    )
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

#[cfg(unix)]
async fn accept_loop(listener: tokio::net::UnixListener, relay_url: String, token: String) {
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

/// How often, and how far apart, creating the next pipe instance is tried
/// in one round: a shortage of handles can pass in a moment.
#[cfg(windows)]
const NEXT_PIPE_ATTEMPTS: u32 = 5;
#[cfg(windows)]
const NEXT_PIPE_DELAY: std::time::Duration = std::time::Duration::from_millis(200);
/// How long to wait before another round of [`NEXT_PIPE_ATTEMPTS`] once one
/// has failed.
#[cfg(windows)]
const NEXT_PIPE_BACKOFF: std::time::Duration = std::time::Duration::from_secs(5);

/// Runs `attempt` up to `attempts` times, `delay` apart, until it succeeds,
/// returning its last error if it never does.
#[cfg(any(windows, test))]
async fn retrying<T>(
    attempts: u32,
    delay: std::time::Duration,
    mut attempt: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    let mut left = attempts.max(1);
    loop {
        match attempt() {
            Ok(value) => return Ok(value),
            Err(e) if left <= 1 => return Err(e),
            Err(e) => {
                warn!(error = %e, "ssh forward: retrying");
                left = left.saturating_sub(1);
                tokio::time::sleep(delay).await;
            }
        }
    }
}

/// A pipe instance serves one client, so a fresh one is created as soon as
/// a client connects, before its connection is handed off. While no fresh
/// one can be created, the connected client is served anyway and creating
/// the next keeps being retried, for the later borg runs of the same job;
/// the task ends with the forward, which aborts it.
#[cfg(windows)]
async fn accept_pipe_loop(
    mut server: tokio::net::windows::named_pipe::NamedPipeServer,
    name: String,
    relay_url: String,
    token: String,
) {
    loop {
        if let Err(e) = server.connect().await {
            warn!(error = %e, "ssh forward: pipe accept error");
            break;
        }
        let mut connected = Some(server);
        server = loop {
            match retrying(NEXT_PIPE_ATTEMPTS, NEXT_PIPE_DELAY, || {
                pipe_instance(&name, false)
            })
            .await
            {
                Ok(next) => break next,
                Err(e) => {
                    warn!(error = %e, "ssh forward: failed to create the next pipe instance");
                    if let Some(connected) = connected.take() {
                        tokio::spawn(relay_connection(
                            connected,
                            relay_url.clone(),
                            token.clone(),
                        ));
                    }
                    tokio::time::sleep(NEXT_PIPE_BACKOFF).await;
                }
            }
        };
        if let Some(connected) = connected {
            tokio::spawn(relay_connection(
                connected,
                relay_url.clone(),
                token.clone(),
            ));
        }
    }
}

async fn relay_connection<S>(local_stream: S, relay_url: String, token: String)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let ws_stream = match tokio_tungstenite::connect_async(&relay_url).await {
        Ok((ws, _)) => ws,
        Err(e) => {
            error!(error = %e, "ssh forward: failed to connect to relay");
            return;
        }
    };

    let (ws_sink, ws_stream) = ws_stream.split();
    relay_over(ws_sink, ws_stream, local_stream, token).await;
}

/// Relays between `local_stream` and a relay connection already open, as
/// `ws_sink` and `ws_stream`, after sending `token` as its first frame.
async fn relay_over<S, W, R>(mut ws_sink: W, ws_stream: R, local_stream: S, token: String)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
    W: futures_util::Sink<Message> + Unpin,
    R: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let (mut local_read, local_write) = tokio::io::split(local_stream);

    if ws_sink.send(Message::Text(token.into())).await.is_err() {
        error!("ssh forward: failed to send auth token");
        return;
    }

    let local_to_ws = async {
        let mut buf = vec![0u8; 4096];
        loop {
            match local_read.read(&mut buf).await {
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

    let ws_to_local = drain_frames_to_writer(
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
        local_write,
    );

    tokio::join!(local_to_ws, ws_to_local);
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests use std::fs for simple synchronous setup/assertions"
)]
mod tests {
    use tokio::io::AsyncWriteExt as _;

    use super::*;

    /// A relay server that takes one connection: it reads the token and a
    /// frame, sends a text frame the agent must skip, echoes the frame and
    /// closes. Returns its URL and, once done, the token and frame it got.
    async fn echo_relay() -> (String, tokio::task::JoinHandle<(Message, Message)>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let token = ws.next().await.unwrap().unwrap();
            let frame = ws.next().await.unwrap().unwrap();
            ws.send(Message::Text("ignored".into())).await.unwrap();
            ws.send(frame.clone()).await.unwrap();
            ws.close(None).await.unwrap();
            (token, frame)
        });
        (url, server)
    }

    #[tokio::test]
    async fn a_connection_is_relayed_to_the_server_and_back() {
        let (url, server) = echo_relay().await;
        let (local, mut agent) = tokio::io::duplex(64);
        let relay = tokio::spawn(relay_connection(local, url, "secret".to_owned()));

        agent.write_all(b"ping").await.unwrap();
        let mut reply = [0; 4];
        agent.read_exact(&mut reply).await.unwrap();
        drop(agent);
        tokio::time::timeout(std::time::Duration::from_secs(10), relay)
            .await
            .expect("the relay ends once both sides are done")
            .unwrap();

        assert_eq!(&reply, b"ping");
        let (token, frame) = server.await.unwrap();
        assert_eq!(token, Message::Text("secret".into()));
        assert_eq!(frame, Message::Binary(b"ping".to_vec().into()));
    }

    #[tokio::test]
    async fn a_relay_that_cannot_be_reached_ends_the_connection() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/", listener.local_addr().unwrap());
        drop(listener);
        let (local, mut agent) = tokio::io::duplex(64);

        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            relay_connection(local, url, "secret".to_owned()),
        )
        .await
        .expect("the relay gives up");

        let mut rest = Vec::new();
        agent.read_to_end(&mut rest).await.unwrap();
        assert_eq!(rest, b"");
    }

    /// A relay connection whose sending side takes `accepted` frames and
    /// then fails, and whose receiving side has nothing to say.
    fn failing_relay(
        accepted: usize,
    ) -> (
        impl futures_util::Sink<Message, Error = std::io::Error> + Unpin,
        futures_util::stream::Empty<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    ) {
        let sink = Box::pin(futures_util::sink::unfold(
            0,
            move |sent, _: Message| async move {
                if sent < accepted {
                    Ok(sent.saturating_add(1))
                } else {
                    Err(std::io::Error::other("relay closed"))
                }
            },
        ));
        (sink, futures_util::stream::empty())
    }

    #[tokio::test]
    async fn a_token_that_cannot_be_sent_ends_the_connection() {
        let (sink, stream) = failing_relay(0);
        let (local, mut agent) = tokio::io::duplex(64);

        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            relay_over(sink, stream, local, "secret".to_owned()),
        )
        .await
        .expect("the relay gives up");

        let mut rest = Vec::new();
        agent.read_to_end(&mut rest).await.unwrap();
        assert_eq!(rest, b"");
    }

    #[tokio::test]
    async fn a_frame_that_cannot_be_sent_ends_the_connection() {
        let (sink, stream) = failing_relay(1);
        let (local, mut agent) = tokio::io::duplex(64);
        let relay = tokio::spawn(relay_over(sink, stream, local, "secret".to_owned()));

        agent.write_all(b"ping").await.unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(10), relay)
            .await
            .expect("the relay stops once a frame can't be sent")
            .unwrap();
    }

    #[tokio::test]
    async fn a_failing_step_is_retried_until_it_succeeds_or_runs_out() {
        let delay = std::time::Duration::from_millis(1);
        let mut calls = 0;
        let result = retrying(5, delay, || {
            calls += 1;
            if calls < 3 {
                Err(std::io::Error::other("busy"))
            } else {
                Ok(calls)
            }
        })
        .await;
        assert_eq!(result.unwrap(), 3);

        let mut calls = 0;
        let result: std::io::Result<()> = retrying(2, delay, || {
            calls += 1;
            Err(std::io::Error::other("still busy"))
        })
        .await;
        assert_eq!(result.unwrap_err().to_string(), "still busy");
        assert_eq!(calls, 2);
    }

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

    #[cfg(unix)]
    #[tokio::test]
    async fn forward_socket_creates_temp_path() {
        let socket = SshForwardSocket::create().unwrap();
        assert!(socket.socket_path.to_str().unwrap().ends_with("agent.sock"));
        assert!(socket.socket_path.parent().unwrap().exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn forward_socket_dropped_cleans_up() {
        let dir_path = {
            let socket = SshForwardSocket::create().unwrap();
            socket.socket_path.parent().unwrap().to_path_buf()
        };
        assert!(!dir_path.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_ssh_forward_binds_socket() {
        let socket = SshForwardSocket::create().unwrap();
        run_ssh_forward(&socket, "ws://127.0.0.1:9999/ws/agent", "host", "token")
            .await
            .unwrap();
        assert!(socket.socket_path.exists());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn forward_pipe_names_are_unique_pipes() {
        let first = SshForwardSocket::create().unwrap();
        let second = SshForwardSocket::create().unwrap();
        let name = first.socket_path.to_str().unwrap();
        assert!(
            name.starts_with(r"\\.\pipe\assimilate-ssh-agent-"),
            "{name}"
        );
        assert_ne!(first.socket_path, second.socket_path);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn run_ssh_forward_serves_the_pipe_until_dropped() {
        use tokio::net::windows::named_pipe::ClientOptions;

        let socket = SshForwardSocket::create().unwrap();
        let name = socket.socket_path.clone();
        run_ssh_forward(&socket, "ws://127.0.0.1:9999/ws/agent", "host", "token")
            .await
            .unwrap();
        // Two clients in a row: each connection gets its own pipe instance.
        ClientOptions::new().open(&name).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        ClientOptions::new().open(&name).unwrap();

        drop(socket);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(ClientOptions::new().open(&name).is_err());
    }
}
