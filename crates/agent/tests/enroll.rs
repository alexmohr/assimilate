// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! `assimilate-agent --enroll`: the real binary trades a single-use enrollment
//! token for an agent token over the encrypted channel and prints that token,
//! and nothing else, on standard output.

use std::{process::Stdio, time::Duration};

use futures_util::{SinkExt as _, StreamExt as _};
use shared::{
    agent_channel::{
        ChannelPrivateKey,
        ws::{DecryptedStream, EncryptedSink, TungsteniteCodec, accept},
    },
    protocol::ServerToAgent,
};
use tokio::{net::TcpListener, process::Command};
use tokio_tungstenite::tungstenite::Message;

/// A test helper's failure, as text for the test that asked.
type TestResult<T> = Result<T, String>;

/// Turns any displayable failure into a [`TestResult`] failure.
trait Context<T> {
    fn context(self, what: &str) -> TestResult<T>;
}

impl<T, E: std::fmt::Display> Context<T> for Result<T, E> {
    fn context(self, what: &str) -> TestResult<T> {
        self.map_err(|e| format!("{what}: {e}"))
    }
}

impl<T> Context<T> for Option<T> {
    fn context(self, what: &str) -> TestResult<T> {
        self.ok_or_else(|| what.to_owned())
    }
}

/// A server that answers the first message with `answer`, returning that message.
async fn serve_one(
    listener: TcpListener,
    key: ChannelPrivateKey,
    answer: ServerToAgent,
) -> TestResult<serde_json::Value> {
    let (tcp, _) = listener.accept().await.context("accept")?;
    let ws = tokio_tungstenite::accept_async(tcp)
        .await
        .context("websocket")?;
    let (mut sink, mut stream) = ws.split();
    let (sealer, opener) = accept::<TungsteniteCodec, _, _, _>(&mut sink, &mut stream, &key)
        .await
        .context("handshake")?;
    let mut sink = EncryptedSink::<TungsteniteCodec, _>::new(sink, sealer);
    let mut stream = DecryptedStream::<TungsteniteCodec, _>::new(stream, opener);
    let first = stream
        .next()
        .await
        .context("no first message")?
        .context("first message")?
        .into_text()
        .context("text")?;
    let json = serde_json::to_string(&answer).context("serialize")?;
    sink.send(Message::Text(json.into()))
        .await
        .context("send")?;
    // Hold the connection open: the agent ends it, not the server.
    while stream.next().await.is_some() {}
    serde_json::from_str(&first).context("parse")
}

/// Runs `agent --enroll` against a server answering with `answer`.
async fn enroll_against(
    answer: ServerToAgent,
    token: Option<&str>,
) -> TestResult<(std::process::Output, serde_json::Value)> {
    let listener = TcpListener::bind("127.0.0.1:0").await.context("bind")?;
    let url = format!("ws://{}", listener.local_addr().context("address")?);
    let key = ChannelPrivateKey::derive(b"enroll test server").context("key")?;
    let server_key = key.public_key().context("public key")?.to_string();
    let mut server = tokio::spawn(serve_one(listener, key, answer));

    let mut command = Command::new(env!("CARGO_BIN_EXE_agent"));
    command
        .args([
            "--enroll",
            "--server-url",
            &url,
            "--server-key",
            &server_key,
        ])
        .env("BORG_HOSTNAME", "cli-test-host")
        .env_remove("BORG_ENROLLMENT_TOKEN")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    if let Some(token) = token {
        command.env("BORG_ENROLLMENT_TOKEN", token);
    }
    let output = tokio::time::timeout(Duration::from_mins(1), command.output())
        .await
        .context("the agent did not finish")?
        .context("run the agent")?;
    // The server returns what it received once the agent has gone; with no
    // connection at all it would wait for ever.
    let sent = if let Ok(joined) = tokio::time::timeout(Duration::from_secs(5), &mut server).await {
        joined.context("server task")??
    } else {
        server.abort();
        serde_json::Value::Null
    };
    Ok((output, sent))
}

#[tokio::test]
async fn an_accepted_enrollment_prints_only_the_agent_token() {
    let (output, sent) = enroll_against(
        ServerToAgent::Enrolled {
            token: "agent-token".to_owned(),
        },
        Some("enr_secret"),
    )
    .await
    .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "agent-token\n");
    // The server saw the host's name and the single-use token, inside the channel.
    assert_eq!(sent.pointer("/type"), Some(&serde_json::json!("Enroll")));
    assert_eq!(
        sent.pointer("/payload/hostname"),
        Some(&serde_json::json!("cli-test-host"))
    );
    assert_eq!(
        sent.pointer("/payload/enrollment_token"),
        Some(&serde_json::json!("enr_secret"))
    );
}

#[tokio::test]
async fn a_refused_enrollment_fails_and_prints_no_token() {
    let (output, _) = enroll_against(
        ServerToAgent::EnrollRefused {
            reason: "token already used".to_owned(),
        },
        Some("enr_secret"),
    )
    .await
    .unwrap();

    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("token already used")
    );
}

#[tokio::test]
async fn enrolling_without_a_token_is_refused_before_connecting() {
    let (output, sent) = enroll_against(
        ServerToAgent::Enrolled {
            token: "never sent".to_owned(),
        },
        None,
    )
    .await
    .unwrap();

    assert_eq!(sent, serde_json::Value::Null, "the agent never connected");
    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    // Whatever it says, it says it on stderr and prints no token.
    assert!(!output.stderr.is_empty(), "{output:?}");
}
