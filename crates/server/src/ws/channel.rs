// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The server's side of the encrypted agent channel on axum's WebSocket.

use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message};
use futures_util::{Sink, SinkExt as _, Stream};
use shared::agent_channel::{
    ChannelPrivateKey,
    ws::{DecryptedStream, EncryptedSink, Frame, WsChannelError, WsCodec, accept},
};

/// How long an agent has to send its handshake after the upgrade.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// Close code agents treat as a fatal authentication failure: an agent from
/// before the encrypted channel stops with the reason instead of retrying.
const CLOSE_CODE_REFUSED: u16 = 4001;

/// Why an agent from before the encrypted channel is turned away.
pub const UNENCRYPTED_REASON: &str =
    "this server requires an encrypted agent channel; update the agent";

/// The axum message codec.
pub struct AxumCodec;

impl WsCodec for AxumCodec {
    type Message = Message;

    fn classify(message: Message) -> Frame<Message> {
        match message {
            Message::Text(text) => Frame::Text(text.as_str().to_owned()),
            Message::Binary(bytes) => Frame::Binary(bytes.to_vec()),
            Message::Close(_) => Frame::Close(message),
            Message::Ping(_) | Message::Pong(_) => Frame::Control(message),
        }
    }

    fn text(text: String) -> Message {
        Message::Text(text.into())
    }

    fn binary(bytes: Vec<u8>) -> Message {
        Message::Binary(bytes.into())
    }
}

/// An established channel's two halves.
pub type Encrypted<Si, St> = (EncryptedSink<AxumCodec, Si>, DecryptedStream<AxumCodec, St>);

/// Runs the handshake on a fresh agent connection and wraps its halves.
/// Returns `None`, after closing the connection where the agent can still
/// hear it, if the agent does not complete a handshake for `key` in time;
/// `peer` names the connection in the log.
pub async fn establish<Si, St, E>(
    mut sink: Si,
    mut stream: St,
    key: &ChannelPrivateKey,
    peer: &str,
) -> Option<Encrypted<Si, St>>
where
    Si: Sink<Message, Error = E> + Unpin,
    St: Stream<Item = Result<Message, E>> + Unpin,
    E: std::fmt::Display,
{
    let handshake = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        accept::<AxumCodec, _, _, _>(&mut sink, &mut stream, key),
    )
    .await;
    match handshake {
        Ok(Ok((sealer, opener))) => Some((
            EncryptedSink::new(sink, sealer),
            DecryptedStream::new(stream, opener),
        )),
        Ok(Err(WsChannelError::Unencrypted)) => {
            tracing::warn!(
                peer,
                "refused an agent without the encrypted channel; it needs updating"
            );
            refuse(&mut sink, UNENCRYPTED_REASON).await;
            None
        }
        Ok(Err(e)) => {
            tracing::warn!(peer, error = %e, "agent channel handshake failed");
            refuse(&mut sink, "encrypted channel handshake failed").await;
            None
        }
        Err(_) => {
            tracing::warn!(peer, "agent did not complete the channel handshake in time");
            refuse(&mut sink, "encrypted channel handshake timed out").await;
            None
        }
    }
}

async fn refuse<Si, E>(sink: &mut Si, reason: &'static str)
where
    Si: Sink<Message, Error = E> + Unpin,
    E: std::fmt::Display,
{
    let close = Message::Close(Some(CloseFrame {
        code: CLOSE_CODE_REFUSED,
        reason: reason.into(),
    }));
    if let Err(e) = sink.send(close).await {
        tracing::debug!(error = %e, "ws send failed");
    }
}

#[cfg(test)]
mod tests;
