// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The opener's checks on frames a well-behaved sealer never produces, so
//! the frames are built here from the raw transport.

use super::*;
use crate::agent_channel::{ChannelPrivateKey, Initiator, respond};

/// The agent's sealer and the server's opener of one channel.
fn channel() -> (Sealer, Opener) {
    let key = ChannelPrivateKey::derive(b"transport test server").unwrap();
    let (initiator, first) = Initiator::start(&key.public_key().unwrap()).unwrap();
    let ((_, server_rx), reply) = respond(&key, &first).unwrap();
    let (agent_tx, _) = initiator.finish(&reply).unwrap();
    (agent_tx, server_rx)
}

#[test]
fn a_frame_longer_than_any_sealed_one_is_refused() {
    let (_, mut opener) = channel();
    let frame = vec![0u8; MAX_FRAME_LEN + 1];
    assert!(matches!(opener.open(&frame), Err(ChannelError::Decrypt)));
}

#[test]
fn a_chunk_with_an_unknown_flag_is_malformed() {
    let (mut sealer, mut opener) = channel();
    let frame = sealer.seal_chunk(FLAG_LAST_BINARY + 1, b"data").unwrap();
    assert!(matches!(opener.open(&frame), Err(ChannelError::Malformed)));
}

#[test]
fn chunks_adding_up_past_the_limit_are_refused() {
    let (mut sealer, mut opener) = channel();
    // As if the chunks of one message had already filled the limit.
    opener.pending = vec![0u8; MAX_MESSAGE_LEN];
    let frame = sealer.seal_chunk(FLAG_MORE, b"x").unwrap();
    assert!(matches!(
        opener.open(&frame),
        Err(ChannelError::MessageTooLarge)
    ));
}
