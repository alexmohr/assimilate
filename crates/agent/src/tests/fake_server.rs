// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The accept loop the fake API servers share.

use std::{
    future::Future,
    io,
    sync::{Arc, Mutex},
};

use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};

/// A listener a fake server takes connections from.
pub trait Listener: Send + 'static {
    /// One accepted connection.
    type Stream: Send + 'static;

    /// Waits for the next connection.
    fn next(&self) -> impl Future<Output = io::Result<Self::Stream>> + Send;
}

impl Listener for TcpListener {
    type Stream = TcpStream;

    async fn next(&self) -> io::Result<TcpStream> {
        self.accept().await.map(|(stream, _)| stream)
    }
}

impl Listener for UnixListener {
    type Stream = UnixStream;

    async fn next(&self) -> io::Result<UnixStream> {
        self.accept().await.map(|(stream, _)| stream)
    }
}

/// Serves every connection `listener` accepts with `serve`, each with the
/// returned shared state.
pub fn spawn<L, S, F, Fut>(listener: L, state: S, serve: F) -> Arc<Mutex<S>>
where
    L: Listener,
    S: Send + 'static,
    F: Fn(L::Stream, Arc<Mutex<S>>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let state = Arc::new(Mutex::new(state));
    let shared = Arc::clone(&state);
    tokio::spawn(async move {
        while let Ok(stream) = listener.next().await {
            tokio::spawn(serve(stream, Arc::clone(&shared)));
        }
    });
    state
}
