// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use futures_util::StreamExt as _;
use shared::agent_channel::ws::initiate;
use tokio::sync::mpsc;

use super::*;
use crate::test_support::test_channel_key;

type Pipe = (
    std::pin::Pin<Box<dyn Sink<Message, Error = std::io::Error> + Send>>,
    std::pin::Pin<Box<dyn Stream<Item = Result<Message, std::io::Error>> + Send>>,
);

/// Two connected in-memory socket ends: what one sends, the other receives.
fn socket_pair() -> (Pipe, Pipe) {
    fn end(tx: mpsc::UnboundedSender<Message>, rx: mpsc::UnboundedReceiver<Message>) -> Pipe {
        let sink = futures_util::sink::unfold(tx, |tx, message: Message| async move {
            tx.send(message)
                .map_err(|_| std::io::Error::other("peer gone"))?;
            Ok::<_, std::io::Error>(tx)
        });
        let stream = futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|message| (Ok(message), rx))
        });
        (Box::pin(sink), Box::pin(stream))
    }
    let (a_tx, a_rx) = mpsc::unbounded_channel();
    let (b_tx, b_rx) = mpsc::unbounded_channel();
    (end(a_tx, b_rx), end(b_tx, a_rx))
}

fn close_of(message: Option<Result<Message, std::io::Error>>) -> Option<(u16, String)> {
    match message {
        Some(Ok(Message::Close(Some(frame)))) => Some((frame.code, frame.reason.to_string())),
        _ => None,
    }
}

#[tokio::test]
async fn an_agent_with_the_servers_key_gets_an_encrypted_channel() {
    let ((server_sink, server_stream), (mut agent_sink, mut agent_stream)) = socket_pair();
    let key = test_channel_key();
    let public = key.public_key().unwrap();

    let agent = async {
        let (sealer, opener) =
            initiate::<AxumCodec, _, _, _>(&mut agent_sink, &mut agent_stream, &public)
                .await
                .unwrap();
        let mut sink = EncryptedSink::<AxumCodec, _>::new(agent_sink, sealer);
        let mut stream = DecryptedStream::<AxumCodec, _>::new(agent_stream, opener);
        sink.send(Message::Text("hello".into())).await.unwrap();
        stream.next().await.unwrap().unwrap()
    };
    let server = async {
        let (mut sink, mut stream) = establish(server_sink, server_stream, &key, "test")
            .await
            .unwrap();
        let hello = stream.next().await.unwrap().unwrap();
        sink.send(Message::Text("welcome".into())).await.unwrap();
        hello
    };

    let (reply, hello) = tokio::join!(agent, server);
    assert_eq!(hello, Message::Text("hello".into()));
    assert_eq!(reply, Message::Text("welcome".into()));
}

#[tokio::test]
async fn an_agent_from_before_the_encrypted_channel_is_told_to_update() {
    let ((server_sink, server_stream), (mut agent_sink, mut agent_stream)) = socket_pair();
    agent_sink
        .send(Message::Text(r#"{"type":"Hello","payload":{}}"#.into()))
        .await
        .unwrap();

    let (channel, logs) = crate::test_support::formatted_logs_while(establish(
        server_sink,
        server_stream,
        &test_channel_key(),
        "old-agent",
    ))
    .await;

    assert!(channel.is_none());
    assert_eq!(
        close_of(agent_stream.next().await),
        Some((CLOSE_CODE_REFUSED, UNENCRYPTED_REASON.to_owned()))
    );
    assert!(logs.contains("needs updating"), "{logs}");
}

#[tokio::test]
async fn a_refusal_to_an_agent_already_gone_is_only_logged() {
    let ((server_sink, server_stream), (mut agent_sink, agent_stream)) = socket_pair();
    agent_sink
        .send(Message::Text(r#"{"type":"Hello","payload":{}}"#.into()))
        .await
        .unwrap();
    drop(agent_stream);

    let (channel, logs) = crate::test_support::formatted_logs_while(establish(
        server_sink,
        server_stream,
        &test_channel_key(),
        "gone-agent",
    ))
    .await;

    assert!(channel.is_none());
    assert!(logs.contains("ws send failed"), "{logs}");
}

#[tokio::test]
async fn an_agent_pinned_to_another_key_is_refused() {
    let ((server_sink, server_stream), (mut agent_sink, mut agent_stream)) = socket_pair();
    let other = shared::agent_channel::ChannelPrivateKey::derive(b"another server")
        .unwrap()
        .public_key()
        .unwrap();
    let (start, first) = shared::agent_channel::Initiator::start(&other).unwrap();
    agent_sink
        .send(Message::Binary(first.into()))
        .await
        .unwrap();

    let channel = establish(server_sink, server_stream, &test_channel_key(), "test").await;

    assert!(channel.is_none());
    assert_eq!(
        close_of(agent_stream.next().await),
        Some((
            CLOSE_CODE_REFUSED,
            "encrypted channel handshake failed".to_owned()
        ))
    );
    drop(start);
}

#[tokio::test(start_paused = true)]
async fn an_agent_that_never_starts_the_handshake_is_closed() {
    let ((server_sink, server_stream), (_agent_sink, mut agent_stream)) = socket_pair();

    let channel = establish(server_sink, server_stream, &test_channel_key(), "test").await;

    assert!(channel.is_none());
    assert_eq!(
        close_of(agent_stream.next().await),
        Some((
            CLOSE_CODE_REFUSED,
            "encrypted channel handshake timed out".to_owned()
        ))
    );
}

#[test]
fn control_messages_pass_through_the_codec() {
    assert!(matches!(
        AxumCodec::classify(Message::Ping(vec![1].into())),
        Frame::Control(Message::Ping(_))
    ));
    assert!(matches!(
        AxumCodec::classify(Message::Pong(vec![1].into())),
        Frame::Control(Message::Pong(_))
    ));
    assert!(matches!(
        AxumCodec::classify(Message::Close(None)),
        Frame::Close(Message::Close(None))
    ));
}
