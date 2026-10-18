// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The server's "Restart agent": the agent shuts down like on a stop
//! request, then exits successfully for its service manager to start it
//! again, rather than exiting on the spot.

use std::{process::Stdio, time::Duration};

use futures_util::{SinkExt as _, StreamExt as _};
use tokio::{net::TcpListener, process::Command};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn a_restart_request_makes_the_agent_exit_successfully() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        let hello = ws.next().await.unwrap().unwrap();
        ws.send(Message::Text(r#"{"type":"RestartAgent"}"#.into()))
            .await
            .unwrap();
        // Hold the connection open: the agent ends it, not the server.
        while ws.next().await.is_some() {}
        hello
    });

    let mut agent = Command::new(env!("CARGO_BIN_EXE_agent"))
        .args(["--server-url", &url, "--token", "restart-test-token"])
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
