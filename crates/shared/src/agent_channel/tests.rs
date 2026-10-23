// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use super::*;

fn server_key() -> ChannelPrivateKey {
    ChannelPrivateKey::derive(b"test server secret").unwrap()
}

/// Runs a full handshake and returns the agent's and the server's channel.
fn connect() -> ((Sealer, Opener), (Sealer, Opener)) {
    let key = server_key();
    let (initiator, first) = Initiator::start(&key.public_key().unwrap()).unwrap();
    let (server, reply) = respond(&key, &first).unwrap();
    let agent = initiator.finish(&reply).unwrap();
    (agent, server)
}

fn deliver(sealer: &mut Sealer, opener: &mut Opener, message: &[u8]) -> Vec<u8> {
    let frames = sealer.seal(MessageKind::Text, message).unwrap();
    let (last, rest) = frames.split_last().unwrap();
    for frame in rest {
        assert_eq!(opener.open(frame).unwrap(), None);
    }
    let (kind, message) = opener.open(last).unwrap().unwrap();
    assert_eq!(kind, MessageKind::Text);
    message
}

#[test]
fn messages_travel_both_ways() {
    let ((mut agent_tx, mut agent_rx), (mut server_tx, mut server_rx)) = connect();
    assert_eq!(deliver(&mut agent_tx, &mut server_rx, b"hello"), b"hello");
    assert_eq!(deliver(&mut server_tx, &mut agent_rx, b"config"), b"config");
    assert_eq!(deliver(&mut agent_tx, &mut server_rx, b"again"), b"again");
}

#[test]
fn handshake_frames_are_small() {
    let key = server_key();
    let (initiator, first) = Initiator::start(&key.public_key().unwrap()).unwrap();
    let (_, reply) = respond(&key, &first).unwrap();
    assert_eq!(first.len(), 48);
    assert_eq!(reply.len(), 48);
    drop(initiator);
}

#[test]
fn frames_do_not_contain_the_plaintext() {
    let ((mut agent_tx, _), _) = connect();
    let frames = agent_tx
        .seal(MessageKind::Text, b"super secret token value")
        .unwrap();
    let frame = frames.first().unwrap();
    assert!(
        !frame
            .windows(b"secret".len())
            .any(|window| window == b"secret")
    );
}

#[test]
fn an_empty_message_round_trips() {
    let ((mut agent_tx, _), (_, mut server_rx)) = connect();
    assert_eq!(deliver(&mut agent_tx, &mut server_rx, b""), b"");
}

#[test]
fn a_large_message_is_split_and_joined() {
    let ((mut agent_tx, _), (_, mut server_rx)) = connect();
    let message: Vec<u8> = (0..200_000u32)
        .map(|i| u8::try_from(i % 251).unwrap())
        .collect();
    let frames = agent_tx.seal(MessageKind::Binary, &message).unwrap();
    assert_eq!(frames.len(), 4);
    assert!(frames.iter().all(|frame| frame.len() <= 65_535));
    let (last, rest) = frames.split_last().unwrap();
    for frame in rest {
        assert_eq!(server_rx.open(frame).unwrap(), None);
    }
    assert_eq!(
        server_rx.open(last).unwrap().unwrap(),
        (MessageKind::Binary, message)
    );
}

#[test]
fn a_message_over_the_limit_is_refused() {
    let ((mut agent_tx, _), _) = connect();
    let message = vec![0u8; MAX_MESSAGE_LEN + 1];
    assert!(matches!(
        agent_tx.seal(MessageKind::Binary, &message),
        Err(ChannelError::MessageTooLarge)
    ));
}

#[test]
fn an_impostor_server_cannot_complete_the_handshake() {
    let real = server_key();
    let impostor = ChannelPrivateKey::derive(b"impostor").unwrap();
    let (initiator, first) = Initiator::start(&real.public_key().unwrap()).unwrap();
    assert!(matches!(
        respond(&impostor, &first),
        Err(ChannelError::Handshake)
    ));
    // Even a reply the impostor builds from a handshake of its own fails.
    let (_, own_first) = Initiator::start(&impostor.public_key().unwrap()).unwrap();
    let (_, reply) = respond(&impostor, &own_first).unwrap();
    assert!(matches!(
        initiator.finish(&reply),
        Err(ChannelError::Handshake)
    ));
}

#[test]
fn a_plain_text_hello_is_not_a_handshake() {
    let hello = br#"{"type":"Hello","token":"abc"}"#;
    assert!(matches!(
        respond(&server_key(), hello),
        Err(ChannelError::Handshake)
    ));
}

#[test]
fn an_oversized_handshake_frame_is_refused() {
    let frame = vec![0u8; MAX_HANDSHAKE_LEN + 1];
    assert!(matches!(
        respond(&server_key(), &frame),
        Err(ChannelError::Handshake)
    ));
}

#[test]
fn a_handshake_carrying_a_payload_is_refused() {
    let key = server_key();
    let mut state = builder()
        .unwrap()
        .remote_public_key(key.public_key().unwrap().as_bytes())
        .and_then(Builder::build_initiator)
        .unwrap();
    let mut first = vec![0u8; MAX_HANDSHAKE_LEN];
    let len = state.write_message(b"hello", &mut first).unwrap();
    first.truncate(len);
    assert!(matches!(
        respond(&key, &first),
        Err(ChannelError::Handshake)
    ));
}

#[test]
fn a_tampered_frame_fails_to_open() {
    let ((mut agent_tx, _), (_, mut server_rx)) = connect();
    let mut frame = agent_tx
        .seal(MessageKind::Text, b"hello")
        .unwrap()
        .pop()
        .unwrap();
    if let Some(byte) = frame.first_mut() {
        *byte ^= 1;
    }
    assert!(matches!(server_rx.open(&frame), Err(ChannelError::Decrypt)));
}

#[test]
fn a_replayed_frame_fails_to_open() {
    let ((mut agent_tx, _), (_, mut server_rx)) = connect();
    let frame = agent_tx
        .seal(MessageKind::Text, b"hello")
        .unwrap()
        .pop()
        .unwrap();
    assert!(server_rx.open(&frame).unwrap().is_some());
    assert!(matches!(server_rx.open(&frame), Err(ChannelError::Decrypt)));
}

#[test]
fn reordered_frames_fail_to_open() {
    let ((mut agent_tx, _), (_, mut server_rx)) = connect();
    let _first = agent_tx.seal(MessageKind::Text, b"one").unwrap();
    let second = agent_tx
        .seal(MessageKind::Text, b"two")
        .unwrap()
        .pop()
        .unwrap();
    assert!(matches!(
        server_rx.open(&second),
        Err(ChannelError::Decrypt)
    ));
}

#[test]
fn a_frame_from_another_session_fails_to_open() {
    let ((mut agent_tx, _), _) = connect();
    let (_, (_, mut other_rx)) = connect();
    let frame = agent_tx
        .seal(MessageKind::Text, b"hello")
        .unwrap()
        .pop()
        .unwrap();
    assert!(matches!(other_rx.open(&frame), Err(ChannelError::Decrypt)));
}

#[test]
fn a_frame_echoed_back_to_its_sender_fails_to_open() {
    let ((mut agent_tx, mut agent_rx), _) = connect();
    let frame = agent_tx
        .seal(MessageKind::Text, b"hello")
        .unwrap()
        .pop()
        .unwrap();
    assert!(matches!(agent_rx.open(&frame), Err(ChannelError::Decrypt)));
}

#[test]
fn a_truncated_message_never_completes() {
    let ((mut agent_tx, _), (_, mut server_rx)) = connect();
    let frames = agent_tx
        .seal(MessageKind::Binary, &vec![7u8; 100_000])
        .unwrap();
    assert_eq!(frames.len(), 2);
    assert_eq!(server_rx.open(frames.first().unwrap()).unwrap(), None);
    // Dropping the last chunk and sending a new message instead cannot be
    // mistaken for the end of the first: the next frame's nonce is wrong.
    let next = agent_tx
        .seal(MessageKind::Text, b"next")
        .unwrap()
        .pop()
        .unwrap();
    assert!(matches!(server_rx.open(&next), Err(ChannelError::Decrypt)));
}
