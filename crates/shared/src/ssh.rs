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
    use super::*;

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

        assert!(buf.is_empty());
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
