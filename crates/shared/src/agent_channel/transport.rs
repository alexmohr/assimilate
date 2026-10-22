// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::sync::Arc;

use snow::StatelessTransportState;

use super::{ChannelError, MAX_MESSAGE_LEN};

/// Largest Noise message, ciphertext and tag included.
const MAX_FRAME_LEN: usize = 65_535;
const TAG_LEN: usize = 16;
const FLAG_LEN: usize = 1;
/// Plaintext bytes of a message that fit in one frame beside the flag.
const CHUNK_LEN: usize = MAX_FRAME_LEN - TAG_LEN - FLAG_LEN;

/// The flag before each chunk: more chunks follow, or this is the last one
/// of a text or a binary message. Carrying the kind keeps the WebSocket's
/// own text/binary distinction, which every encrypted frame otherwise hides.
const FLAG_MORE: u8 = 0;
const FLAG_LAST_TEXT: u8 = 1;
const FLAG_LAST_BINARY: u8 = 2;

/// Whether a message is text or binary, as a WebSocket message would be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    /// UTF-8 text, such as a JSON protocol message.
    Text,
    /// Raw bytes, such as relayed SSH agent traffic.
    Binary,
}

impl MessageKind {
    fn last_flag(self) -> u8 {
        match self {
            Self::Text => FLAG_LAST_TEXT,
            Self::Binary => FLAG_LAST_BINARY,
        }
    }
}

/// The sending half of an established channel: turns messages into frames.
pub struct Sealer {
    transport: Arc<StatelessTransportState>,
    nonce: u64,
}

impl Sealer {
    pub(super) fn new(transport: Arc<StatelessTransportState>) -> Self {
        Self {
            transport,
            nonce: 0,
        }
    }

    /// Encrypts `message` of the given `kind` into the frames to send, in
    /// order, each as one binary WebSocket message.
    ///
    /// # Errors
    ///
    /// Returns [`ChannelError::MessageTooLarge`] for a message over
    /// [`MAX_MESSAGE_LEN`], or another [`ChannelError`] if encryption fails;
    /// the channel cannot be used after an error.
    pub fn seal(
        &mut self,
        kind: MessageKind,
        message: &[u8],
    ) -> Result<Vec<Vec<u8>>, ChannelError> {
        if message.len() > MAX_MESSAGE_LEN {
            return Err(ChannelError::MessageTooLarge);
        }
        let chunks: Vec<&[u8]> = if message.is_empty() {
            vec![&[]]
        } else {
            message.chunks(CHUNK_LEN).collect()
        };
        let last = chunks.len().saturating_sub(1);
        chunks
            .into_iter()
            .enumerate()
            .map(|(i, chunk)| {
                let flag = if i == last {
                    kind.last_flag()
                } else {
                    FLAG_MORE
                };
                self.seal_chunk(flag, chunk)
            })
            .collect()
    }

    fn seal_chunk(&mut self, flag: u8, chunk: &[u8]) -> Result<Vec<u8>, ChannelError> {
        let mut plain = Vec::with_capacity(chunk.len().saturating_add(FLAG_LEN));
        plain.push(flag);
        plain.extend_from_slice(chunk);
        let mut frame = vec![0u8; plain.len().saturating_add(TAG_LEN)];
        let len = self
            .transport
            .write_message(self.nonce, &plain, &mut frame)
            .map_err(|_| ChannelError::Encrypt)?;
        self.nonce = self
            .nonce
            .checked_add(1)
            .ok_or(ChannelError::NonceExhausted)?;
        frame.truncate(len);
        Ok(frame)
    }
}

/// The receiving half of an established channel: turns frames back into
/// messages.
pub struct Opener {
    transport: Arc<StatelessTransportState>,
    nonce: u64,
    pending: Vec<u8>,
}

impl Opener {
    pub(super) fn new(transport: Arc<StatelessTransportState>) -> Self {
        Self {
            transport,
            nonce: 0,
            pending: Vec::new(),
        }
    }

    /// Decrypts the next received frame. Returns the whole message and its
    /// kind once its last chunk has arrived, and `None` while more chunks
    /// are due.
    ///
    /// # Errors
    ///
    /// Returns [`ChannelError::Decrypt`] for a frame that fails
    /// authentication (altered, replayed, reordered or foreign),
    /// [`ChannelError::MessageTooLarge`] once a message grows past
    /// [`MAX_MESSAGE_LEN`], and [`ChannelError::Malformed`] for a chunk
    /// without a valid flag; the channel cannot be used after an error.
    pub fn open(&mut self, frame: &[u8]) -> Result<Option<(MessageKind, Vec<u8>)>, ChannelError> {
        if frame.len() > MAX_FRAME_LEN {
            return Err(ChannelError::Decrypt);
        }
        let mut plain = vec![0u8; frame.len()];
        let len = self
            .transport
            .read_message(self.nonce, frame, &mut plain)
            .map_err(|_| ChannelError::Decrypt)?;
        self.nonce = self
            .nonce
            .checked_add(1)
            .ok_or(ChannelError::NonceExhausted)?;
        let plain = plain.get(..len).ok_or(ChannelError::Malformed)?;
        let (&flag, chunk) = plain.split_first().ok_or(ChannelError::Malformed)?;
        if self.pending.len().saturating_add(chunk.len()) > MAX_MESSAGE_LEN {
            return Err(ChannelError::MessageTooLarge);
        }
        self.pending.extend_from_slice(chunk);
        let kind = match flag {
            FLAG_MORE => return Ok(None),
            FLAG_LAST_TEXT => MessageKind::Text,
            FLAG_LAST_BINARY => MessageKind::Binary,
            _ => return Err(ChannelError::Malformed),
        };
        Ok(Some((kind, std::mem::take(&mut self.pending))))
    }
}
