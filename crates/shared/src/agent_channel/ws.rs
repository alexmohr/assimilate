// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The channel on a WebSocket: the handshake over its first two binary
//! messages, then a sink and a stream that look like the socket's own but
//! encrypt every text and binary message. Control messages (ping, pong,
//! close) pass through as they are: they carry nothing secret, and the
//! WebSocket library answers pings itself.

use std::{
    collections::VecDeque,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use futures_util::{Sink, SinkExt as _, Stream, StreamExt as _};

use super::{
    ChannelError, ChannelPrivateKey, ChannelPublicKey, Initiator, MessageKind, Opener, Sealer,
    respond,
};

/// What a WebSocket message is, as far as the channel cares.
pub enum Frame<M> {
    /// A text message.
    Text(String),
    /// A binary message.
    Binary(Vec<u8>),
    /// A ping, pong or close, passed through unencrypted.
    Control(M),
    /// The peer closed the connection.
    Close(M),
}

/// How the channel reads and builds one WebSocket library's messages, so the
/// same adapters serve the agent's `tungstenite` and the server's `axum`.
pub trait WsCodec {
    /// The library's message type.
    type Message: Unpin;
    /// Sorts a received or outgoing message.
    fn classify(message: Self::Message) -> Frame<Self::Message>;
    /// Builds a text message.
    fn text(text: String) -> Self::Message;
    /// Builds a binary message.
    fn binary(bytes: Vec<u8>) -> Self::Message;
}

/// The `tokio-tungstenite` codec, used by the agent and in tests.
pub struct TungsteniteCodec;

impl WsCodec for TungsteniteCodec {
    type Message = tokio_tungstenite::tungstenite::Message;

    fn classify(message: Self::Message) -> Frame<Self::Message> {
        use tokio_tungstenite::tungstenite::Message;
        match message {
            Message::Text(text) => Frame::Text(text.as_str().to_owned()),
            Message::Binary(bytes) => Frame::Binary(bytes.to_vec()),
            Message::Close(_) => Frame::Close(message),
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => Frame::Control(message),
        }
    }

    fn text(text: String) -> Self::Message {
        Self::Message::Text(text.into())
    }

    fn binary(bytes: Vec<u8>) -> Self::Message {
        Self::Message::Binary(bytes.into())
    }
}

/// Why the channel on a WebSocket failed.
#[derive(Debug, thiserror::Error)]
pub enum WsChannelError<E> {
    /// The WebSocket itself failed.
    #[error("websocket error: {0}")]
    Transport(E),
    /// The handshake or a frame failed.
    #[error(transparent)]
    Channel(#[from] ChannelError),
    /// The peer sent a text message where only encrypted binary frames are
    /// allowed: it speaks the unencrypted protocol.
    #[error("the peer sent an unencrypted message")]
    Unencrypted,
    /// A decrypted text message is not UTF-8.
    #[error("a decrypted text message is not UTF-8")]
    InvalidUtf8,
    /// The connection closed before the handshake completed.
    #[error("the connection closed during the handshake")]
    Closed,
}

/// Runs the agent's side of the handshake on a fresh connection.
///
/// # Errors
///
/// Returns [`WsChannelError`] if the socket fails, closes, or the server
/// does not prove it holds the private key matching `server`.
pub async fn initiate<C, Si, St, E>(
    sink: &mut Si,
    stream: &mut St,
    server: &ChannelPublicKey,
) -> Result<(Sealer, Opener), WsChannelError<E>>
where
    C: WsCodec,
    Si: Sink<C::Message, Error = E> + Unpin,
    St: Stream<Item = Result<C::Message, E>> + Unpin,
{
    let (initiator, first) = Initiator::start(server)?;
    sink.send(C::binary(first))
        .await
        .map_err(WsChannelError::Transport)?;
    let reply = next_handshake_frame::<C, St, E>(stream).await?;
    Ok(initiator.finish(&reply)?)
}

/// Runs the server's side of the handshake on a fresh connection.
///
/// # Errors
///
/// Returns [`WsChannelError`] if the socket fails, closes, or the agent's
/// first message is not a handshake for this server's key.
pub async fn accept<C, Si, St, E>(
    sink: &mut Si,
    stream: &mut St,
    key: &ChannelPrivateKey,
) -> Result<(Sealer, Opener), WsChannelError<E>>
where
    C: WsCodec,
    Si: Sink<C::Message, Error = E> + Unpin,
    St: Stream<Item = Result<C::Message, E>> + Unpin,
{
    let first = next_handshake_frame::<C, St, E>(stream).await?;
    let (channel, reply) = respond(key, &first)?;
    sink.send(C::binary(reply))
        .await
        .map_err(WsChannelError::Transport)?;
    Ok(channel)
}

async fn next_handshake_frame<C, St, E>(stream: &mut St) -> Result<Vec<u8>, WsChannelError<E>>
where
    C: WsCodec,
    St: Stream<Item = Result<C::Message, E>> + Unpin,
{
    loop {
        let message = stream
            .next()
            .await
            .ok_or(WsChannelError::Closed)?
            .map_err(WsChannelError::Transport)?;
        match C::classify(message) {
            Frame::Binary(bytes) => return Ok(bytes),
            Frame::Text(_) => return Err(WsChannelError::Unencrypted),
            Frame::Close(_) => return Err(WsChannelError::Closed),
            Frame::Control(_) => {}
        }
    }
}

/// A WebSocket sink that encrypts each text and binary message it is given
/// into one or more binary frames.
pub struct EncryptedSink<C: WsCodec, Si> {
    inner: Si,
    sealer: Sealer,
    queue: VecDeque<C::Message>,
    codec: PhantomData<fn() -> C>,
}

impl<C: WsCodec, Si> EncryptedSink<C, Si> {
    /// Wraps `inner`, encrypting with `sealer`.
    pub fn new(inner: Si, sealer: Sealer) -> Self {
        Self {
            inner,
            sealer,
            queue: VecDeque::new(),
            codec: PhantomData,
        }
    }
}

impl<C, Si> EncryptedSink<C, Si>
where
    C: WsCodec,
    Si: Sink<C::Message> + Unpin,
{
    fn poll_drain(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), WsChannelError<Si::Error>>> {
        while !self.queue.is_empty() {
            match Pin::new(&mut self.inner).poll_ready(cx) {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(e)) => return Poll::Ready(Err(WsChannelError::Transport(e))),
                Poll::Pending => return Poll::Pending,
            }
            if let Some(message) = self.queue.pop_front() {
                Pin::new(&mut self.inner)
                    .start_send(message)
                    .map_err(WsChannelError::Transport)?;
            }
        }
        Poll::Ready(Ok(()))
    }
}

impl<C, Si> Sink<C::Message> for EncryptedSink<C, Si>
where
    C: WsCodec,
    Si: Sink<C::Message> + Unpin,
{
    type Error = WsChannelError<Si::Error>;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.get_mut().poll_drain(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: C::Message) -> Result<(), Self::Error> {
        let this = self.get_mut();
        let frames = match C::classify(item) {
            Frame::Text(text) => this.sealer.seal(MessageKind::Text, text.as_bytes())?,
            Frame::Binary(bytes) => this.sealer.seal(MessageKind::Binary, &bytes)?,
            Frame::Control(message) | Frame::Close(message) => {
                this.queue.push_back(message);
                return Ok(());
            }
        };
        this.queue.extend(frames.into_iter().map(C::binary));
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.get_mut();
        match this.poll_drain(cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut this.inner)
                .poll_flush(cx)
                .map_err(WsChannelError::Transport),
            other => other,
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.get_mut();
        match this.poll_drain(cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut this.inner)
                .poll_close(cx)
                .map_err(WsChannelError::Transport),
            other => other,
        }
    }
}

/// A WebSocket stream that decrypts the binary frames it receives back into
/// the text and binary messages they carry. It ends after its first error,
/// since a channel that failed once cannot be trusted again.
pub struct DecryptedStream<C: WsCodec, St> {
    inner: St,
    opener: Opener,
    failed: bool,
    codec: PhantomData<fn() -> C>,
}

impl<C: WsCodec, St> DecryptedStream<C, St> {
    /// Wraps `inner`, decrypting with `opener`.
    pub fn new(inner: St, opener: Opener) -> Self {
        Self {
            inner,
            opener,
            failed: false,
            codec: PhantomData,
        }
    }

    fn decrypt<E>(&mut self, frame: &[u8]) -> Result<Option<C::Message>, WsChannelError<E>> {
        let Some((kind, bytes)) = self.opener.open(frame)? else {
            return Ok(None);
        };
        Ok(Some(match kind {
            MessageKind::Text => {
                C::text(String::from_utf8(bytes).map_err(|_| WsChannelError::InvalidUtf8)?)
            }
            MessageKind::Binary => C::binary(bytes),
        }))
    }
}

impl<C, St, E> Stream for DecryptedStream<C, St>
where
    C: WsCodec,
    St: Stream<Item = Result<C::Message, E>> + Unpin,
{
    type Item = Result<C::Message, WsChannelError<E>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if this.failed {
                return Poll::Ready(None);
            }
            let message = match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(message))) => message,
                Poll::Ready(Some(Err(e))) => {
                    this.failed = true;
                    return Poll::Ready(Some(Err(WsChannelError::Transport(e))));
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            };
            let decrypted = match C::classify(message) {
                Frame::Binary(frame) => this.decrypt(&frame),
                Frame::Text(_) => Err(WsChannelError::Unencrypted),
                Frame::Control(message) | Frame::Close(message) => Ok(Some(message)),
            };
            match decrypted {
                Ok(Some(message)) => return Poll::Ready(Some(Ok(message))),
                Ok(None) => {}
                Err(e) => {
                    this.failed = true;
                    return Poll::Ready(Some(Err(e)));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
