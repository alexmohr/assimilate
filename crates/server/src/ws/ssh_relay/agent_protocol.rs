// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The parts of the SSH agent protocol (draft-miller-ssh-agent) the relay
//! needs to decide which requests an agent machine may make of the server's
//! SSH agent.
//!
//! Every message is a 4-byte big-endian length followed by that many bytes,
//! the first of which is the message type. An agent machine only ever needs
//! to list the server's keys, bind its connection to the SSH server it logs
//! in to, and have one of the keys sign that login; every other request
//! (adding, removing or locking keys, other extensions) changes the server's
//! agent for every host and is answered with a failure without reaching it.

use tokio::io::{AsyncRead, AsyncReadExt as _};

/// The largest message the relay accepts in either direction, the same
/// limit OpenSSH's `ssh-agent` applies (`AGENT_MAX_LEN`).
pub const MAX_MESSAGE_LEN: usize = 256 * 1024;

/// Length of the big-endian length prefix in front of every message.
const LENGTH_PREFIX: usize = 4;

/// `SSH_AGENT_FAILURE`, the agent's generic "request refused" reply.
const SSH_AGENT_FAILURE: u8 = 5;

/// `SSH_AGENT_SUCCESS`, the agent's generic "done" reply.
const SSH_AGENT_SUCCESS: u8 = 6;

/// The reply the relay sends for a request it refuses: a one-byte
/// `SSH_AGENT_FAILURE` message.
pub const FAILURE_REPLY: [u8; 5] = [0, 0, 0, 1, SSH_AGENT_FAILURE];

/// The reply the relay sends for a `session-bind` it accepts: a one-byte
/// `SSH_AGENT_SUCCESS` message.
pub const SUCCESS_REPLY: [u8; 5] = [0, 0, 0, 1, SSH_AGENT_SUCCESS];

/// The kind of request an agent machine sent, by its message type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRequest {
    /// `SSH_AGENTC_REQUEST_IDENTITIES` (11): list the loaded keys.
    RequestIdentities,
    /// `SSH_AGENTC_SIGN_REQUEST` (13): sign an SSH login with one of them,
    /// forwarded only for a login the connection's session-bind allows.
    SignRequest,
    /// `SSH_AGENTC_EXTENSION` (27), such as the `session-bind@openssh.com`
    /// that OpenSSH 8.9 and later sends on every connection. Never
    /// forwarded: the relay answers a session-bind itself and refuses any
    /// other extension.
    Extension,
    /// Any other message type, adding, removing or locking keys among them,
    /// which the relay never forwards.
    Refused(u8),
}

impl From<u8> for AgentRequest {
    fn from(message_type: u8) -> Self {
        match message_type {
            11 => Self::RequestIdentities,
            13 => Self::SignRequest,
            27 => Self::Extension,
            other => Self::Refused(other),
        }
    }
}

/// A message that breaks the protocol, after which the relay ends.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FrameError {
    /// A message of length zero has no message type.
    #[error("empty SSH agent message")]
    Empty,
    /// A message longer than [`MAX_MESSAGE_LEN`].
    #[error("SSH agent message of {0} bytes exceeds the {MAX_MESSAGE_LEN}-byte limit")]
    TooLong(usize),
}

/// One complete request from an agent machine.
#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    /// What the request asks for.
    pub kind: AgentRequest,
    /// The whole message, length prefix included, as it is forwarded.
    pub bytes: Vec<u8>,
}

impl Request {
    /// The message's fields: the bytes after its length and message type.
    pub fn body(&self) -> &[u8] {
        self.bytes.get(LENGTH_PREFIX + 1..).unwrap_or_default()
    }
}

/// Reassembles requests from the bytes an agent machine sends, which arrive
/// split across WebSocket frames however its SSH client wrote them.
#[derive(Debug, Default)]
pub struct RequestBuffer {
    pending: Vec<u8>,
}

impl RequestBuffer {
    /// Appends bytes received from the agent machine.
    pub fn extend(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
    }

    /// Takes the next complete request, or `None` until more bytes arrive.
    ///
    /// # Errors
    ///
    /// Fails on a message the protocol does not allow: empty, or longer than
    /// [`MAX_MESSAGE_LEN`]. A too-long message is refused from its length
    /// prefix alone, before its body is buffered.
    pub fn next_request(&mut self) -> Result<Option<Request>, FrameError> {
        let Some(len) = self.pending.first_chunk::<LENGTH_PREFIX>() else {
            return Ok(None);
        };
        // At most MAX_MESSAGE_LEN, so this cannot saturate.
        let frame_len = message_len(*len)?.saturating_add(LENGTH_PREFIX);
        let Some(&message_type) = self.pending.get(LENGTH_PREFIX) else {
            return Ok(None);
        };
        if self.pending.len() < frame_len {
            return Ok(None);
        }
        let rest = self.pending.split_off(frame_len);
        let bytes = std::mem::replace(&mut self.pending, rest);
        Ok(Some(Request {
            kind: AgentRequest::from(message_type),
            bytes,
        }))
    }
}

/// The body length a message's length prefix announces, checked against
/// the protocol's limits.
fn message_len(prefix: [u8; LENGTH_PREFIX]) -> Result<usize, FrameError> {
    let len = usize::try_from(u32::from_be_bytes(prefix)).unwrap_or(usize::MAX);
    match len {
        0 => Err(FrameError::Empty),
        len if len > MAX_MESSAGE_LEN => Err(FrameError::TooLong(len)),
        len => Ok(len),
    }
}

/// Why the server's SSH agent's reply could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ReplyError {
    /// The connection to the SSH agent failed or closed mid-reply.
    #[error("reading the SSH agent's reply: {0}")]
    Io(#[from] std::io::Error),
    /// The reply broke the protocol.
    #[error(transparent)]
    Frame(#[from] FrameError),
}

/// Reads one complete reply, length prefix included, from the server's SSH
/// agent.
///
/// # Errors
///
/// Fails if the connection ends before a whole reply arrives, or the reply
/// is empty or longer than [`MAX_MESSAGE_LEN`].
pub async fn read_reply<R: AsyncRead + Unpin>(agent: &mut R) -> Result<Vec<u8>, ReplyError> {
    let mut prefix = [0; LENGTH_PREFIX];
    agent.read_exact(&mut prefix).await?;
    let len = message_len(prefix)?;
    let mut body = vec![0; len];
    agent.read_exact(&mut body).await?;
    Ok([prefix.as_slice(), &body].concat())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A message of `message_type` with `body` after it, length prefix
    /// included.
    pub(crate) fn message(message_type: u8, body: &[u8]) -> Vec<u8> {
        let content = [&[message_type], body].concat();
        let len = u32::try_from(content.len()).expect("message length");
        [len.to_be_bytes().as_slice(), &content].concat()
    }

    #[test]
    fn requests_are_told_apart_by_their_message_type() {
        assert_eq!(AgentRequest::from(11), AgentRequest::RequestIdentities);
        assert_eq!(AgentRequest::from(13), AgentRequest::SignRequest);
        assert_eq!(AgentRequest::from(27), AgentRequest::Extension);
        // ADD_IDENTITY, REMOVE_IDENTITY, REMOVE_ALL_IDENTITIES, LOCK, UNLOCK,
        // ADD_ID_CONSTRAINED.
        for refused in [17, 18, 19, 22, 23, 25] {
            assert_eq!(AgentRequest::from(refused), AgentRequest::Refused(refused));
        }
    }

    #[test]
    fn a_request_split_across_reads_is_reassembled() {
        let request = message(13, b"key-and-data");
        let mut buffer = RequestBuffer::default();

        // Part of the length, then the length alone, then part of the body.
        for end in [3, 4, 7] {
            buffer.extend(request.get(buffer.pending.len()..end).unwrap());
            assert_eq!(buffer.next_request(), Ok(None));
        }
        buffer.extend(request.get(7..).unwrap());

        assert_eq!(
            buffer.next_request(),
            Ok(Some(Request {
                kind: AgentRequest::SignRequest,
                bytes: request,
            }))
        );
        assert_eq!(buffer.next_request(), Ok(None));
    }

    #[test]
    fn several_requests_in_one_read_come_out_in_order() {
        let mut buffer = RequestBuffer::default();
        buffer.extend(&[message(11, b""), message(19, b"")].concat());

        let first = buffer.next_request().unwrap().unwrap();
        let second = buffer.next_request().unwrap().unwrap();

        assert_eq!(first.kind, AgentRequest::RequestIdentities);
        assert_eq!(second.kind, AgentRequest::Refused(19));
        assert_eq!(buffer.next_request(), Ok(None));
    }

    #[test]
    fn an_empty_message_is_a_protocol_error() {
        let mut buffer = RequestBuffer::default();
        buffer.extend(&[0, 0, 0, 0]);

        assert_eq!(buffer.next_request(), Err(FrameError::Empty));
    }

    #[test]
    fn an_oversized_message_is_refused_from_its_length_alone() {
        let mut buffer = RequestBuffer::default();
        let len = u32::try_from(MAX_MESSAGE_LEN + 1).unwrap();
        buffer.extend(&len.to_be_bytes());

        assert_eq!(
            buffer.next_request(),
            Err(FrameError::TooLong(MAX_MESSAGE_LEN + 1))
        );
    }

    #[test]
    fn the_failure_reply_is_a_one_byte_failure_message() {
        assert_eq!(FAILURE_REPLY.to_vec(), message(SSH_AGENT_FAILURE, b""));
    }

    #[test]
    fn the_success_reply_is_a_one_byte_success_message() {
        assert_eq!(SUCCESS_REPLY.to_vec(), message(SSH_AGENT_SUCCESS, b""));
    }

    #[test]
    fn a_request_body_leaves_out_its_length_and_type() {
        let mut buffer = RequestBuffer::default();
        buffer.extend(&message(13, b"fields"));

        assert_eq!(buffer.next_request().unwrap().unwrap().body(), b"fields");
    }

    #[tokio::test]
    async fn a_whole_reply_is_read_with_its_length_prefix() {
        let reply = message(12, &[0, 0, 0, 0]);
        let mut agent = [reply.clone(), message(5, b"")].concat();
        let mut agent = agent.as_mut_slice() as &[u8];

        assert_eq!(read_reply(&mut agent).await.unwrap(), reply);
        assert_eq!(read_reply(&mut agent).await.unwrap(), FAILURE_REPLY);
    }

    #[tokio::test]
    async fn a_truncated_or_oversized_reply_is_an_error() {
        let reply = message(12, b"body");
        let mut truncated = reply.get(..6).unwrap();
        assert!(matches!(
            read_reply(&mut truncated).await,
            Err(ReplyError::Io(_))
        ));

        let len = u32::try_from(MAX_MESSAGE_LEN + 1).unwrap().to_be_bytes();
        let mut oversized: &[u8] = &len;
        assert!(matches!(
            read_reply(&mut oversized).await,
            Err(ReplyError::Frame(FrameError::TooLong(_)))
        ));
    }
}
