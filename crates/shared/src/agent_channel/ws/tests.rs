// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, accept_async, connect_async, tungstenite::Message,
};

use super::*;

type ClientSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type ServerSocket = WebSocketStream<TcpStream>;

fn server_key() -> ChannelPrivateKey {
    ChannelPrivateKey::derive(b"ws test server secret").unwrap()
}

async fn sockets() -> (ClientSocket, ServerSocket) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        accept_async(tcp).await.unwrap()
    });
    let (client, _) = connect_async(&url).await.unwrap();
    (client, server.await.unwrap())
}

#[tokio::test]
async fn messages_pass_encrypted_in_both_directions() {
    let (client, server) = sockets().await;
    let key = server_key();
    let public = key.public_key().unwrap();

    let server_task = tokio::spawn(async move {
        let (mut sink, mut stream) = server.split();
        let (sealer, opener) = accept::<TungsteniteCodec, _, _, _>(&mut sink, &mut stream, &key)
            .await
            .unwrap();
        let mut sink = EncryptedSink::<TungsteniteCodec, _>::new(sink, sealer);
        let mut stream = DecryptedStream::<TungsteniteCodec, _>::new(stream, opener);
        let hello = stream.next().await.unwrap().unwrap();
        let blob = stream.next().await.unwrap().unwrap();
        sink.send(Message::Text("welcome".into())).await.unwrap();
        sink.send(blob.clone()).await.unwrap();
        (hello, blob)
    });

    let (mut sink, mut stream) = client.split();
    let (sealer, opener) = initiate::<TungsteniteCodec, _, _, _>(&mut sink, &mut stream, &public)
        .await
        .unwrap();
    let mut sink = EncryptedSink::<TungsteniteCodec, _>::new(sink, sealer);
    let mut stream = DecryptedStream::<TungsteniteCodec, _>::new(stream, opener);
    let large: Vec<u8> = (0..300_000u32)
        .map(|i| u8::try_from(i % 7).unwrap())
        .collect();
    sink.send(Message::Text("hello".into())).await.unwrap();
    sink.send(Message::Binary(large.clone().into()))
        .await
        .unwrap();

    assert_eq!(
        stream.next().await.unwrap().unwrap(),
        Message::Text("welcome".into())
    );
    assert_eq!(
        stream.next().await.unwrap().unwrap(),
        Message::Binary(large.clone().into())
    );
    let (hello, blob) = server_task.await.unwrap();
    assert_eq!(hello, Message::Text("hello".into()));
    assert_eq!(blob, Message::Binary(large.into()));
}

#[tokio::test]
async fn what_crosses_the_wire_is_binary_and_unreadable() {
    let (client, mut server) = sockets().await;
    let key = server_key();
    let public = key.public_key().unwrap();

    let client_task = tokio::spawn(async move {
        let (mut sink, mut stream) = client.split();
        let (sealer, _opener) =
            initiate::<TungsteniteCodec, _, _, _>(&mut sink, &mut stream, &public)
                .await
                .unwrap();
        let mut sink = EncryptedSink::<TungsteniteCodec, _>::new(sink, sealer);
        sink.send(Message::Text("token=hunter2".into()))
            .await
            .unwrap();
        (sink, stream)
    });

    let first = server.next().await.unwrap().unwrap();
    let (_, reply) = respond(&key, &first.into_data()).unwrap();
    server.send(Message::Binary(reply.into())).await.unwrap();
    let frame = server.next().await.unwrap().unwrap();
    let Message::Binary(bytes) = frame else {
        panic!("expected a binary frame, got {frame:?}");
    };
    assert!(!bytes.windows(7).any(|window| window == b"hunter2"));
    drop(client_task.await.unwrap());
}

#[tokio::test]
async fn an_unencrypted_hello_is_refused() {
    let (mut client, server) = sockets().await;
    let key = server_key();
    let server_task = tokio::spawn(async move {
        let (mut sink, mut stream) = server.split();
        accept::<TungsteniteCodec, _, _, _>(&mut sink, &mut stream, &key)
            .await
            .map(|_| ())
    });
    client
        .send(Message::Text(r#"{"type":"Hello"}"#.into()))
        .await
        .unwrap();
    assert!(matches!(
        server_task.await.unwrap(),
        Err(WsChannelError::Unencrypted)
    ));
}

#[tokio::test]
async fn an_agent_pinned_to_another_key_fails_the_handshake() {
    let (client, server) = sockets().await;
    let key = server_key();
    let pinned = ChannelPrivateKey::derive(b"another server")
        .unwrap()
        .public_key()
        .unwrap();
    let server_task = tokio::spawn(async move {
        let (mut sink, mut stream) = server.split();
        accept::<TungsteniteCodec, _, _, _>(&mut sink, &mut stream, &key)
            .await
            .map(|_| ())
    });
    let (mut sink, mut stream) = client.split();
    let result = initiate::<TungsteniteCodec, _, _, _>(&mut sink, &mut stream, &pinned).await;
    assert!(matches!(
        server_task.await.unwrap(),
        Err(WsChannelError::Channel(ChannelError::Handshake))
    ));
    drop(sink);
    assert!(result.is_err() || stream.next().await.is_none());
}

#[tokio::test]
async fn a_server_that_closes_during_the_handshake_is_reported() {
    let (client, mut server) = sockets().await;
    let public = server_key().public_key().unwrap();
    let server_task = tokio::spawn(async move {
        let _first = server.next().await;
        server.close(None).await.unwrap();
    });
    let (mut sink, mut stream) = client.split();
    let result = initiate::<TungsteniteCodec, _, _, _>(&mut sink, &mut stream, &public).await;
    assert!(matches!(result, Err(WsChannelError::Closed)));
    server_task.await.unwrap();
}

#[tokio::test]
async fn a_plain_text_message_after_the_handshake_ends_the_stream() {
    let (client, server) = sockets().await;
    let key = server_key();
    let public = key.public_key().unwrap();
    let server_task = tokio::spawn(async move {
        let (mut sink, mut stream) = server.split();
        let (_sealer, opener) = accept::<TungsteniteCodec, _, _, _>(&mut sink, &mut stream, &key)
            .await
            .unwrap();
        let mut stream = DecryptedStream::<TungsteniteCodec, _>::new(stream, opener);
        let first = stream.next().await.unwrap();
        let after = stream.next().await;
        (first.is_err(), after.is_none())
    });
    let (mut sink, mut stream) = client.split();
    initiate::<TungsteniteCodec, _, _, _>(&mut sink, &mut stream, &public)
        .await
        .unwrap();
    sink.send(Message::Text("plain".into())).await.unwrap();
    sink.send(Message::Text("more".into())).await.unwrap();
    assert_eq!(server_task.await.unwrap(), (true, true));
}
