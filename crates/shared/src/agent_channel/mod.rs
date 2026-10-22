// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Every agent connection, the main one and each SSH relay, opens with a
//! `Noise_NK_25519_ChaChaPoly_BLAKE2s` handshake in two binary WebSocket
//! frames: the agent knows the server's static public key, so only the
//! server holding the matching private key can complete it, and both sides'
//! fresh ephemeral keys give the session forward secrecy. Nothing is sent
//! before the handshake completes; the agent's token and every message after
//! it travel as encrypted frames.
//!
//! A message is split into chunks that each fit one Noise message. Every
//! chunk starts with an encrypted flag saying whether more follow, so where
//! one message ends is authenticated too, and frames carry no nonce: each
//! direction counts its own, which works because a WebSocket delivers frames
//! in order. Any error leaves the channel unusable, and the connection must
//! be closed.

mod key;
mod transport;
pub mod ws;

use std::sync::Arc;

pub use key::{ChannelPrivateKey, ChannelPublicKey};
use snow::{Builder, HandshakeState};
pub use transport::{MessageKind, Opener, Sealer};

/// The Noise protocol name. Changing it, or [`PROLOGUE`], makes every
/// handshake with a peer on the old value fail.
pub const NOISE_PARAMS: &str = "Noise_NK_25519_ChaChaPoly_BLAKE2s";

/// Bound into the handshake hash, so a handshake made for another protocol
/// or another version of this one cannot be replayed here.
pub const PROLOGUE: &[u8] = b"assimilate agent channel v1";

/// The largest message either side accepts once its chunks are joined.
pub const MAX_MESSAGE_LEN: usize = 64 * 1024 * 1024;

/// The largest handshake frame. Both handshake messages carry an empty
/// payload, so they are 48 bytes; anything much larger is not a handshake.
pub const MAX_HANDSHAKE_LEN: usize = 1024;

/// Ways setting up or using the encrypted channel can fail.
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    /// Deriving the server's static key failed.
    #[error("failed to derive the agent channel key")]
    KeyDerivation,
    /// The configured server public key is not 32 bytes of base64.
    #[error("the server key is not a valid agent channel key (expected 44 characters of base64)")]
    InvalidPublicKey,
    /// The Noise handshake failed: the peer does not hold the expected key,
    /// speaks another protocol version, or sent a malformed message.
    #[error("the encrypted channel handshake failed")]
    Handshake,
    /// A frame failed authentication: it was altered, replayed, reordered or
    /// is not from this session.
    #[error("an encrypted frame failed to decrypt")]
    Decrypt,
    /// Encrypting a frame failed.
    #[error("failed to encrypt a frame")]
    Encrypt,
    /// A message, or the chunks received of one so far, exceeds
    /// [`MAX_MESSAGE_LEN`].
    #[error("a message exceeds the {MAX_MESSAGE_LEN} byte limit")]
    MessageTooLarge,
    /// A decrypted chunk did not start with a valid flag.
    #[error("an encrypted frame is malformed")]
    Malformed,
    /// One direction has used all 2^64 nonces.
    #[error("the channel has used all of its nonces")]
    NonceExhausted,
}

/// An agent's half-done handshake, waiting for the server's reply.
pub struct Initiator(HandshakeState);

impl Initiator {
    /// Starts a handshake with the server whose static key is `server`,
    /// returning the state to finish it with and the first frame to send.
    ///
    /// # Errors
    ///
    /// Returns [`ChannelError::Handshake`] if the handshake cannot be built.
    pub fn start(server: &ChannelPublicKey) -> Result<(Self, Vec<u8>), ChannelError> {
        let mut state = builder()?
            .remote_public_key(server.as_bytes())
            .and_then(Builder::build_initiator)
            .map_err(|_| ChannelError::Handshake)?;
        let first = write_handshake(&mut state)?;
        Ok((Self(state), first))
    }

    /// Reads the server's reply and switches to transport mode.
    ///
    /// # Errors
    ///
    /// Returns [`ChannelError::Handshake`] if the reply does not come from
    /// the holder of the server's private key.
    pub fn finish(mut self, reply: &[u8]) -> Result<(Sealer, Opener), ChannelError> {
        read_handshake(&mut self.0, reply)?;
        into_transport(self.0)
    }
}

/// Answers an agent's first handshake frame with the server's static key,
/// returning the channel and the reply frame to send.
///
/// # Errors
///
/// Returns [`ChannelError::Handshake`] if `first` is not a handshake for
/// this key and protocol.
pub fn respond(
    key: &ChannelPrivateKey,
    first: &[u8],
) -> Result<((Sealer, Opener), Vec<u8>), ChannelError> {
    let mut state = builder()?
        .local_private_key(key.as_bytes())
        .and_then(Builder::build_responder)
        .map_err(|_| ChannelError::Handshake)?;
    read_handshake(&mut state, first)?;
    let reply = write_handshake(&mut state)?;
    Ok((into_transport(state)?, reply))
}

fn builder() -> Result<Builder<'static>, ChannelError> {
    let params = NOISE_PARAMS.parse().map_err(|_| ChannelError::Handshake)?;
    Builder::new(params)
        .prologue(PROLOGUE)
        .map_err(|_| ChannelError::Handshake)
}

fn write_handshake(state: &mut HandshakeState) -> Result<Vec<u8>, ChannelError> {
    let mut frame = vec![0u8; MAX_HANDSHAKE_LEN];
    let len = state
        .write_message(&[], &mut frame)
        .map_err(|_| ChannelError::Handshake)?;
    frame.truncate(len);
    Ok(frame)
}

fn read_handshake(state: &mut HandshakeState, frame: &[u8]) -> Result<(), ChannelError> {
    if frame.len() > MAX_HANDSHAKE_LEN {
        return Err(ChannelError::Handshake);
    }
    let mut payload = vec![0u8; MAX_HANDSHAKE_LEN];
    let len = state
        .read_message(frame, &mut payload)
        .map_err(|_| ChannelError::Handshake)?;
    // Both sides send an empty payload; anything else is not this protocol.
    if len == 0 {
        Ok(())
    } else {
        Err(ChannelError::Handshake)
    }
}

fn into_transport(state: HandshakeState) -> Result<(Sealer, Opener), ChannelError> {
    let transport = Arc::new(
        state
            .into_stateless_transport_mode()
            .map_err(|_| ChannelError::Handshake)?,
    );
    Ok((Sealer::new(Arc::clone(&transport)), Opener::new(transport)))
}

#[cfg(test)]
mod tests;
