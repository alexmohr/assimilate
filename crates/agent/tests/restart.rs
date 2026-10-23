// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The server's "Restart agent": the agent shuts down like on a stop
//! request, then exits successfully for its service manager to start it
//! again, rather than exiting on the spot.

use std::{process::Stdio, time::Duration};

use futures_util::{SinkExt as _, StreamExt as _};
use shared::agent_channel::{
    ChannelPrivateKey,
    ws::{DecryptedStream, EncryptedSink, TungsteniteCodec, accept},
};
use tokio::{net::TcpListener, process::Command};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn a_restart_request_makes_the_agent_exit_successfully() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let key = ChannelPrivateKey::derive(b"restart test server").unwrap();
    let server_key = key.public_key().unwrap().to_string();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let (mut sink, mut stream) = ws.split();
        let (sealer, opener) = accept::<TungsteniteCodec, _, _, _>(&mut sink, &mut stream, &key)
            .await
            .unwrap();
        let mut sink = EncryptedSink::<TungsteniteCodec, _>::new(sink, sealer);
        let mut stream = DecryptedStream::<TungsteniteCodec, _>::new(stream, opener);
        let hello = stream.next().await.unwrap().unwrap();
        sink.send(Message::Text(r#"{"type":"RestartAgent"}"#.into()))
            .await
            .unwrap();
        // Hold the connection open: the agent ends it, not the server.
        while stream.next().await.is_some() {}
        hello
    });

    let mut agent = Command::new(env!("CARGO_BIN_EXE_agent"))
        .args(["--server-url", &url, "--server-key", &server_key])
        .env("BORG_AGENT_TOKEN", "restart-test-token")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let status = tokio::time::timeout(Duration::from_mins(1), agent.wait())
        .await
        .expect("the agent exits after the restart request")
        .unwrap();

    assert!(status.success(), "{status:?}");
    let hello = server.await.unwrap();
    assert!(
        hello.to_text().unwrap().contains(r#""type":"Hello""#),
        "{hello:?}"
    );
}
