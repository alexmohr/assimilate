// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::path::Path;

use futures_util::{Stream, StreamExt as _};
use tokio::io::{AsyncWrite, AsyncWriteExt as _};

/// Outcome of classifying a single inbound WebSocket frame in an
/// SSH-forwarding relay loop: forward a binary payload, ignore an unrelated
/// frame type, or stop relaying (close frame, read error, or anything else
/// that should end the loop).
pub enum RelayFrame<T> {
    /// Forward this payload to the local writer.
    Data(T),
    /// An unrelated frame type (text, ping, pong, ...); keep waiting for more.
    Skip,
    /// Stop relaying - a close frame, a read error, or the stream ending.
    Stop,
}

/// A connection to a local SSH agent: a Unix socket, or a named pipe on
/// Windows.
#[cfg(unix)]
pub type AgentStream = tokio::net::UnixStream;
/// A connection to a local SSH agent: a Unix socket, or a named pipe on
/// Windows.
#[cfg(windows)]
pub type AgentStream = tokio::net::windows::named_pipe::NamedPipeClient;

/// Where a local SSH agent listens without `SSH_AUTH_SOCK`: nowhere on Unix,
/// Windows OpenSSH's agent service pipe on Windows.
#[cfg(unix)]
pub const DEFAULT_SSH_AUTH_SOCK: Option<&str> = None;
/// Where a local SSH agent listens without `SSH_AUTH_SOCK`: nowhere on Unix,
/// Windows OpenSSH's agent service pipe on Windows.
#[cfg(windows)]
pub const DEFAULT_SSH_AUTH_SOCK: Option<&str> = Some(r"\\.\pipe\openssh-ssh-agent");

/// The local SSH agent to use: `ssh_auth_sock` (the value of
/// `SSH_AUTH_SOCK`) if set, else [`DEFAULT_SSH_AUTH_SOCK`].
#[must_use]
pub fn agent_socket(ssh_auth_sock: Option<String>) -> Option<String> {
    ssh_auth_sock.or_else(|| DEFAULT_SSH_AUTH_SOCK.map(str::to_owned))
}

/// Connects to the SSH agent at `path`.
///
/// # Errors
///
/// Fails if nothing is listening there.
#[cfg(unix)]
pub async fn connect_agent(path: &str) -> std::io::Result<AgentStream> {
    tokio::net::UnixStream::connect(path).await
}

/// Connects to the SSH agent at `path`. While every instance of the pipe is
/// serving another client, it waits and tries again, for up to a second.
///
/// # Errors
///
/// Fails if no pipe of that name is open, or it stays busy.
#[cfg(windows)]
pub async fn connect_agent(path: &str) -> std::io::Result<AgentStream> {
    use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;
    let busy = i32::try_from(ERROR_PIPE_BUSY).ok();
    for _ in 0..20 {
        match tokio::net::windows::named_pipe::ClientOptions::new().open(path) {
            Err(e) if e.raw_os_error() == busy => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            connected => return connected,
        }
    }
    tokio::net::windows::named_pipe::ClientOptions::new().open(path)
}

/// Drains `frames` into `writer`, forwarding each [`RelayFrame::Data`]
/// payload and stopping on the first [`RelayFrame::Stop`] or once the stream
/// ends. Both the agent's SSH-agent-forwarding relay (`ssh_forward.rs`) and
/// the server's SSH relay handler (`ws/ssh_relay.rs`) drive the same "binary
/// WebSocket frame -> local byte stream" loop over their own respective
/// WebSocket message types (`tokio-tungstenite` and `axum`, which aren't a
/// shared type this crate can match on directly), so callers classify their
/// own frames into [`RelayFrame`] before handing the mapped stream here.
pub async fn drain_frames_to_writer<T, S, W>(mut frames: S, mut writer: W)
where
    T: AsRef<[u8]>,
    S: Stream<Item = RelayFrame<T>> + Unpin,
    W: AsyncWrite + Unpin,
{
    while let Some(frame) = frames.next().await {
        match frame {
            RelayFrame::Data(data) => {
                if writer.write_all(data.as_ref()).await.is_err() {
                    break;
                }
            }
            RelayFrame::Skip => {}
            RelayFrame::Stop => break,
        }
    }
}

/// Builds the `--rsh` command line passed to `borg` for SSH transport,
/// using batch mode and auto-accepting unknown host keys on first
/// connection. Suitable when no pinned `known_hosts` file is available.
#[must_use]
pub fn borg_rsh() -> String {
    [
        "ssh",
        "-o BatchMode=yes",
        "-o StrictHostKeyChecking=accept-new",
        "-o ServerAliveInterval=15",
        "-o ServerAliveCountMax=3",
        "-o ConnectTimeout=30",
    ]
    .join(" ")
}

/// Builds the `--rsh` command line passed to `borg` for SSH transport,
/// pinning host key verification to the `known_hosts` file at `path` instead
/// of accepting new keys automatically.
#[must_use]
pub fn borg_rsh_with_known_hosts(path: &Path) -> String {
    [
        "ssh",
        "-o BatchMode=yes",
        "-o StrictHostKeyChecking=yes",
        "-o ServerAliveInterval=15",
        "-o ServerAliveCountMax=3",
        "-o ConnectTimeout=30",
        &format!("-o UserKnownHostsFile={}", path.display()),
    ]
    .join(" ")
}

/// Formats `host` as it should appear in a `known_hosts` file entry: the
/// bare hostname for the default SSH port (22), or a bracketed
/// `[host]:port` form for any non-default port.
#[must_use]
pub fn known_hosts_host(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncReadExt as _;

    use super::*;

    #[test]
    fn ssh_auth_sock_wins_over_the_default() {
        assert_eq!(
            agent_socket(Some("/run/agent.sock".to_owned())).as_deref(),
            Some("/run/agent.sock")
        );
        assert_eq!(agent_socket(None).as_deref(), DEFAULT_SSH_AUTH_SOCK);
    }

    #[cfg(unix)]
    #[test]
    fn unix_has_no_default_agent() {
        assert_eq!(DEFAULT_SSH_AUTH_SOCK, None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_defaults_to_the_openssh_agent_pipe() {
        assert_eq!(DEFAULT_SSH_AUTH_SOCK, Some(r"\\.\pipe\openssh-ssh-agent"));
    }

    /// Answers one request on `agent` with `reply`, returning the request.
    async fn answer<S>(mut agent: S, reply: &[u8]) -> Vec<u8>
    where
        S: tokio::io::AsyncRead + AsyncWrite + Unpin,
    {
        let mut request = vec![0; 4];
        agent.read_exact(&mut request).await.unwrap();
        agent.write_all(reply).await.unwrap();
        request
    }

    /// Sends `ping` through `client` and returns what comes back.
    async fn exchange(mut client: AgentStream) -> Vec<u8> {
        client.write_all(b"ping").await.unwrap();
        let mut reply = vec![0; 4];
        client.read_exact(&mut reply).await.unwrap();
        reply
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn connects_to_an_agent_listening_on_a_unix_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let agent = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            answer(stream, b"pong").await
        });

        let client = connect_agent(path.to_str().unwrap()).await.unwrap();

        assert_eq!(exchange(client).await, b"pong");
        assert_eq!(agent.await.unwrap(), b"ping");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn connects_to_an_agent_listening_on_a_named_pipe() {
        let path = format!(r"\\.\pipe\assimilate-test-agent-{}", std::process::id());
        let server = tokio::net::windows::named_pipe::ServerOptions::new()
            .first_pipe_instance(true)
            .create(&path)
            .unwrap();
        let agent = tokio::spawn(async move {
            server.connect().await.unwrap();
            answer(server, b"pong").await
        });

        let client = connect_agent(&path).await.unwrap();

        assert_eq!(exchange(client).await, b"pong");
        assert_eq!(agent.await.unwrap(), b"ping");
    }

    /// A client that finds the only instance busy waits for the agent to
    /// open another, as an agent serving one client at a time does.
    #[cfg(windows)]
    #[tokio::test]
    async fn connecting_while_the_agent_pipe_is_busy_waits_for_a_free_instance() {
        use tokio::net::windows::named_pipe::ServerOptions;
        let path = format!(r"\\.\pipe\assimilate-busy-agent-{}", std::process::id());
        let first = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&path)
            .unwrap();
        let _taken = connect_agent(&path).await.unwrap();
        first.connect().await.unwrap();
        let second_path = path.clone();
        let agent = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let second = ServerOptions::new().create(&second_path).unwrap();
            second.connect().await.unwrap();
            answer(second, b"pong").await
        });

        let client = connect_agent(&path).await.unwrap();

        assert_eq!(exchange(client).await, b"pong");
        assert_eq!(agent.await.unwrap(), b"ping");
        drop(first);
    }

    /// A pipe that stays busy fails the connection with the busy error
    /// once the wait is over, rather than waiting for good.
    #[cfg(windows)]
    #[tokio::test]
    async fn connecting_while_the_agent_pipe_stays_busy_gives_up() {
        use tokio::net::windows::named_pipe::ServerOptions;
        use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;
        let path = format!(r"\\.\pipe\assimilate-busy-forever-{}", std::process::id());
        let only = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&path)
            .unwrap();
        let _taken = connect_agent(&path).await.unwrap();
        only.connect().await.unwrap();

        let refused =
            tokio::time::timeout(std::time::Duration::from_secs(10), connect_agent(&path))
                .await
                .expect("gave up within the wait")
                .unwrap_err();

        assert_eq!(refused.raw_os_error(), i32::try_from(ERROR_PIPE_BUSY).ok());
        drop(only);
    }

    #[tokio::test]
    async fn connecting_where_no_agent_listens_fails() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let path = dir.path().join("nothing.sock").to_str().unwrap().to_owned();
        #[cfg(windows)]
        let path = {
            let _ = &dir;
            format!(r"\\.\pipe\assimilate-no-agent-{}", std::process::id())
        };

        assert!(connect_agent(&path).await.is_err());
    }

    #[tokio::test]
    async fn drain_forwards_data_frames_in_order() {
        let frames = futures_util::stream::iter(vec![
            RelayFrame::Data(b"hel".to_vec()),
            RelayFrame::Skip,
            RelayFrame::Data(b"lo".to_vec()),
        ]);
        let mut buf: Vec<u8> = Vec::new();

        drain_frames_to_writer(frames, &mut buf).await;

        assert_eq!(buf, b"hello");
    }

    #[tokio::test]
    async fn drain_stops_on_stop_frame_without_writing_later_data() {
        let frames = futures_util::stream::iter(vec![
            RelayFrame::Data(b"before".to_vec()),
            RelayFrame::Stop,
            RelayFrame::Data(b"after".to_vec()),
        ]);
        let mut buf: Vec<u8> = Vec::new();

        drain_frames_to_writer(frames, &mut buf).await;

        assert_eq!(buf, b"before");
    }

    #[tokio::test]
    async fn drain_returns_once_the_stream_ends() {
        let frames = futures_util::stream::iter(Vec::<RelayFrame<Vec<u8>>>::new());
        let mut buf: Vec<u8> = Vec::new();

        drain_frames_to_writer(frames, &mut buf).await;

        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn borg_rsh_uses_default_known_hosts_file() {
        let ssh = borg_rsh();

        assert!(ssh.contains("BatchMode=yes"));
        assert!(ssh.contains("StrictHostKeyChecking=accept-new"));
        assert!(ssh.contains("ServerAliveInterval=15"));
        assert!(ssh.contains("ServerAliveCountMax=3"));
        assert!(ssh.contains("ConnectTimeout=30"));
        assert!(!ssh.contains("UserKnownHostsFile"));
    }

    #[test]
    fn pinned_borg_rsh_requires_the_provided_known_hosts_file() {
        let ssh = borg_rsh_with_known_hosts(Path::new("/tmp/known-hosts"));

        assert!(ssh.contains("StrictHostKeyChecking=yes"));
        assert!(ssh.contains("ServerAliveInterval=15"));
        assert!(ssh.contains("UserKnownHostsFile=/tmp/known-hosts"));
    }

    #[test]
    fn known_hosts_host_includes_nonstandard_ports() {
        assert_eq!(known_hosts_host("repo.example.com", 22), "repo.example.com");
        assert_eq!(
            known_hosts_host("repo.example.com", 2222),
            "[repo.example.com]:2222"
        );
    }
}
