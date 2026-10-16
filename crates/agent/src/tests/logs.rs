// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Capturing what the agent logs while a test runs.

use std::{
    future::Future,
    io::Write,
    sync::{Arc, Mutex},
};

use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Captured {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Everything logged, at any level, while `run` ran, along with its result.
/// The subscriber is the thread's default, so this suits the current-thread
/// runtime `#[tokio::test]` uses.
pub async fn logs_while<F: Future>(run: F) -> (F::Output, String) {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(captured.clone())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    let output = run.await;
    drop(guard);
    let text = String::from_utf8_lossy(
        &captured
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
    .into_owned();
    (output, text)
}
