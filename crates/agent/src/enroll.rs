// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! `assimilate-agent --enroll`: a host without an agent token trades a
//! single-use enrollment token for one. The exchange runs inside the encrypted
//! channel, so it only completes with a server holding the key the host pins.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use shared::{
    agent_channel::{
        ChannelPublicKey,
        ws::{DecryptedStream, EncryptedSink, TungsteniteCodec, initiate},
    },
    protocol::{AgentToServer, ServerToAgent},
};
use tokio_tungstenite::tungstenite::Message;

use crate::ws::ChannelFailure;

/// How long the whole exchange may take before it is given up on.
const ENROLL_TIMEOUT: Duration = Duration::from_secs(30);

/// Why a host could not enroll.
#[derive(Debug, thiserror::Error)]
pub enum EnrollError {
    #[error("connection failed: {0}")]
    Connect(Box<tokio_tungstenite::tungstenite::Error>),
    /// The server does not hold the key this host pins.
    #[error(
        "encrypted channel handshake failed: {0}; check that the server key matches the agent \
         channel key shown on the server's System page"
    )]
    Handshake(Box<ChannelFailure>),
    #[error("send failed: {0}")]
    Send(Box<ChannelFailure>),
    #[error("receive failed: {0}")]
    Receive(Box<ChannelFailure>),
    #[error("serialization failed: {0}")]
    Serialize(serde_json::Error),
    /// The server turned the token down.
    #[error("the server refused to enroll this host: {0}")]
    Refused(String),
    /// The server ended the connection, or answered with something else.
    #[error("the server did not answer the enrollment request")]
    NoAnswer,
    #[error("the server did not answer within {} seconds", ENROLL_TIMEOUT.as_secs())]
    TimedOut,
}

/// What an enrollment needs to know.
pub struct EnrollRequest<'a> {
    /// WebSocket base URL of the server, without a path.
    pub url: &'a str,
    /// The server's agent channel key.
    pub key: &'a ChannelPublicKey,
    /// The single-use enrollment token.
    pub enrollment_token: &'a str,
    /// This host's name, as the server will know the agent.
    pub hostname: String,
}

/// Trades the enrollment token for the agent's own token.
///
/// # Errors
///
/// Returns an [`EnrollError`] if the server cannot be reached or does not hold
/// the pinned key, refuses the token, or does not answer in time.
pub async fn enroll(request: &EnrollRequest<'_>) -> Result<String, EnrollError> {
    tokio::time::timeout(ENROLL_TIMEOUT, exchange(request))
        .await
        .map_err(|_| EnrollError::TimedOut)?
}

async fn exchange(request: &EnrollRequest<'_>) -> Result<String, EnrollError> {
    let url = format!("{}/ws/agent", request.url.trim_end_matches('/'));
    let (ws_stream, _response) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| EnrollError::Connect(Box::new(e)))?;
    let (mut sink, mut stream) = ws_stream.split();
    let (sealer, opener) =
        initiate::<TungsteniteCodec, _, _, _>(&mut sink, &mut stream, request.key)
            .await
            .map_err(|e| EnrollError::Handshake(Box::new(e)))?;
    let mut sink = EncryptedSink::<TungsteniteCodec, _>::new(sink, sealer);
    let mut stream = DecryptedStream::<TungsteniteCodec, _>::new(stream, opener);

    let enroll = AgentToServer::Enroll {
        hostname: request.hostname.clone(),
        domain: None,
        enrollment_token: request.enrollment_token.to_owned(),
    };
    let json = serde_json::to_string(&enroll).map_err(EnrollError::Serialize)?;
    sink.send(Message::Text(json.into()))
        .await
        .map_err(|e| EnrollError::Send(Box::new(e)))?;

    let answer = match stream.next().await {
        Some(Ok(Message::Text(text))) => serde_json::from_str::<ServerToAgent>(&text).ok(),
        Some(Err(e)) => return Err(EnrollError::Receive(Box::new(e))),
        _ => None,
    };
    match answer {
        Some(ServerToAgent::Enrolled { token }) => Ok(token),
        Some(ServerToAgent::EnrollRefused { reason }) => Err(EnrollError::Refused(reason)),
        _ => Err(EnrollError::NoAnswer),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ws::tests::accept_encrypted;

    const KEY_TOKEN: &str = "enr_0123";

    fn request<'a>(url: &'a str, key: &'a ChannelPublicKey) -> EnrollRequest<'a> {
        EnrollRequest {
            url,
            key,
            enrollment_token: KEY_TOKEN,
            hostname: "new-host".to_owned(),
        }
    }

    async fn serve_answer(
        listener: tokio::net::TcpListener,
        answer: Option<ServerToAgent>,
    ) -> serde_json::Value {
        let (mut sink, mut stream) = accept_encrypted(&listener).await;
        let text = stream.next().await.unwrap().unwrap().into_text().unwrap();
        match answer {
            Some(answer) => {
                let json = serde_json::to_string(&answer).unwrap();
                sink.send(Message::Text(json.into())).await.unwrap();
            }
            None => sink.send(Message::Close(None)).await.unwrap(),
        }
        serde_json::from_str(&text).unwrap()
    }

    #[tokio::test]
    async fn the_agent_token_comes_back_for_an_accepted_enrollment() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let key = crate::endpoint::test_support::server_key()
            .public_key()
            .unwrap();

        let server = serve_answer(
            listener,
            Some(ServerToAgent::Enrolled {
                token: "agent-token".to_owned(),
            }),
        );
        let request = request(&url, &key);
        let (sent, token) = tokio::join!(server, enroll(&request));

        assert_eq!(token.unwrap(), "agent-token");
        assert_eq!(sent.pointer("/type"), Some(&serde_json::json!("Enroll")));
        assert_eq!(
            sent.pointer("/payload/enrollment_token"),
            Some(&serde_json::json!(KEY_TOKEN))
        );
        assert_eq!(
            sent.pointer("/payload/hostname"),
            Some(&serde_json::json!("new-host"))
        );
    }

    #[tokio::test]
    async fn a_refused_enrollment_reports_the_servers_reason() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let key = crate::endpoint::test_support::server_key()
            .public_key()
            .unwrap();

        let server = serve_answer(
            listener,
            Some(ServerToAgent::EnrollRefused {
                reason: "expired".to_owned(),
            }),
        );
        let request = request(&url, &key);
        let (_, result) = tokio::join!(server, enroll(&request));

        let Err(EnrollError::Refused(reason)) = result else {
            panic!("the enrollment was not refused: {result:?}");
        };
        assert_eq!(reason, "expired");
    }

    #[tokio::test]
    async fn a_connection_closed_without_an_answer_is_reported() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let key = crate::endpoint::test_support::server_key()
            .public_key()
            .unwrap();

        let server = serve_answer(listener, None);
        let request = request(&url, &key);
        let (_, result) = tokio::join!(server, enroll(&request));

        assert!(matches!(result, Err(EnrollError::NoAnswer)));
    }

    #[tokio::test]
    async fn a_server_with_another_key_is_not_trusted() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let other = shared::agent_channel::ChannelPrivateKey::derive(b"another server")
            .unwrap()
            .public_key()
            .unwrap();

        let server = async {
            let (tcp, _) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let (mut sink, mut stream) = ws.split();
            let key = crate::endpoint::test_support::server_key();
            let _ = shared::agent_channel::ws::accept::<TungsteniteCodec, _, _, _>(
                &mut sink,
                &mut stream,
                &key,
            )
            .await;
        };
        let request = request(&url, &other);
        let ((), result) = tokio::join!(server, enroll(&request));

        assert!(matches!(result, Err(EnrollError::Handshake(_))));
    }
}
