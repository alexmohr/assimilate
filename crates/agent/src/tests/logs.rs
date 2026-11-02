// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Capturing what the agent logs while a test runs.

use std::{
    future::Future,
    io::Write,
    sync::{Arc, Mutex, Once},
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

/// Installs, once per test process, a global subscriber that takes every
/// event and writes it nowhere.
///
/// `tracing` caches, per log statement, whether any subscriber wants it, and
/// works that out the first time the statement runs, on whichever thread gets
/// there first. Tests run in parallel, and most of them have no subscriber at
/// all. So when another test reached a statement first while this test was
/// installing its thread-local one, the statement could be cached as wanted
/// by nobody, and it then stayed silent for the rest of the process,
/// including inside `logs_while`. That is what made
/// `the_extract_is_logged_with_its_path_count` fail in most parallel runs.
/// With a global subscriber that wants everything, no statement can be cached
/// as unwanted, and each event goes to whatever subscriber is current on its
/// thread.
fn keep_every_callsite_live() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let discard = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(std::io::sink)
            .finish();
        // Only fails if a global subscriber is already set, which wants the
        // events too.
        let _ = tracing::subscriber::set_global_default(discard);
        // Statements whose interest was worked out before the global
        // subscriber existed are worked out again with it.
        tracing::callsite::rebuild_interest_cache();
    });
}

/// Everything logged, at any level, while `run` ran, along with its result.
/// The subscriber is the thread's default, so this suits the current-thread
/// runtime `#[tokio::test]` uses.
pub async fn logs_while<F: Future>(run: F) -> (F::Output, String) {
    keep_every_callsite_live();
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
