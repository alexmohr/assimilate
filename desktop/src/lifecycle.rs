// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! When the background stack starts and stops, kept free of Tauri so the
//! ordering rules can be tested without an event loop.
//!
//! There is exactly one way out of the app: the first quit request (tray
//! menu or OS exit request) stops the stack, waiting for a start that is
//! still in progress, and only then lets the process exit.

use std::{
    fmt::Display,
    sync::atomic::{AtomicBool, Ordering},
};

use desktop_core::runtime::{DesktopRuntime, RuntimeError};
use tokio::sync::Mutex;

/// A running background stack that can be stopped in order.
pub trait Stack: Send + 'static {
    /// Why stopping failed.
    type Error: Display;

    /// Stops everything the stack started.
    fn shutdown(self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

impl Stack for DesktopRuntime {
    type Error = RuntimeError;

    fn shutdown(self) -> impl Future<Output = Result<(), RuntimeError>> + Send {
        DesktopRuntime::shutdown(self)
    }
}

/// What a quit request should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitDecision {
    /// This is the first request: the caller must stop the stack and exit.
    StartShutdown,
    /// A shutdown is already running; hold the exit until it is done.
    AlreadyShuttingDown,
    /// The stack is stopped; let the process exit.
    Exit,
}

/// What [`Lifecycle::start`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    /// The stack is up.
    Running,
    /// The app was already quitting, so nothing was started.
    Skipped,
}

/// The stack's slot plus the quit state.
pub struct Lifecycle<S> {
    stack: Mutex<Option<S>>,
    quitting: AtomicBool,
    stopped: AtomicBool,
}

impl<S> Default for Lifecycle<S> {
    fn default() -> Self {
        Self {
            stack: Mutex::new(None),
            quitting: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
        }
    }
}

impl<S: Stack> Lifecycle<S> {
    /// Records a quit request and says what the caller must do.
    pub fn request_quit(&self) -> QuitDecision {
        if self.stopped.load(Ordering::SeqCst) {
            QuitDecision::Exit
        } else if self.quitting.swap(true, Ordering::SeqCst) {
            QuitDecision::AlreadyShuttingDown
        } else {
            QuitDecision::StartShutdown
        }
    }

    /// Whether a quit has been requested; background work stops then.
    pub fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::SeqCst)
    }

    /// Brings the stack up with `bring_up`, holding the slot the whole time
    /// so a concurrent [`Self::stop`] waits for it and then stops what came
    /// up. Does nothing once the app is quitting.
    ///
    /// # Errors
    ///
    /// Returns `bring_up`'s error; nothing is kept running then.
    pub async fn start<E>(
        &self,
        bring_up: impl Future<Output = Result<S, E>>,
    ) -> Result<Started, E> {
        let mut slot = self.stack.lock().await;
        if self.quitting.load(Ordering::SeqCst) {
            return Ok(Started::Skipped);
        }
        *slot = Some(bring_up.await?);
        Ok(Started::Running)
    }

    /// Waits for a start in progress, stops whatever is running and marks
    /// the app as ready to exit.
    ///
    /// # Errors
    ///
    /// Returns the stack's error if it did not stop cleanly; the app is
    /// marked ready to exit regardless.
    pub async fn stop(&self) -> Result<(), S::Error> {
        let stack = self.stack.lock().await.take();
        let result = match stack {
            Some(stack) => stack.shutdown().await,
            None => Ok(()),
        };
        self.stopped.store(true, Ordering::SeqCst);
        result
    }
}

/// Runs `step` on a freshly started `stack`. If the step fails, the stack is
/// stopped before the error is returned, so a failed start never leaves
/// processes running.
///
/// # Errors
///
/// Returns `step`'s error.
pub async fn finish_or_stop<S: Stack, E>(
    stack: S,
    step: impl FnOnce(&S) -> Result<(), E>,
) -> Result<S, E> {
    if let Err(e) = step(&stack) {
        if let Err(stop) = stack.shutdown().await {
            tracing::warn!(error = %stop, "failed to stop the stack after a failed start");
        }
        return Err(e);
    }
    Ok(stack)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use tauri::async_runtime::{block_on, spawn};
    use tokio::sync::oneshot;

    use super::*;

    /// Counts how often it was shut down.
    struct FakeStack(Arc<AtomicUsize>);

    impl Stack for FakeStack {
        type Error = std::io::Error;

        fn shutdown(self) -> impl Future<Output = Result<(), std::io::Error>> + Send {
            self.0.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(()))
        }
    }

    #[test]
    fn only_the_first_quit_request_starts_the_shutdown() {
        let lifecycle = Lifecycle::<FakeStack>::default();

        assert_eq!(lifecycle.request_quit(), QuitDecision::StartShutdown);
        assert_eq!(lifecycle.request_quit(), QuitDecision::AlreadyShuttingDown);
        block_on(lifecycle.stop()).unwrap();
        assert_eq!(lifecycle.request_quit(), QuitDecision::Exit);
    }

    #[test]
    fn quitting_is_visible_once_requested() {
        let lifecycle = Lifecycle::<FakeStack>::default();
        assert!(!lifecycle.is_quitting());

        lifecycle.request_quit();

        assert!(lifecycle.is_quitting());
    }

    #[test]
    fn stop_shuts_the_running_stack_down_once() {
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let lifecycle = Lifecycle::default();

        let started = block_on(
            lifecycle.start(async { Ok::<_, std::io::Error>(FakeStack(Arc::clone(&shutdowns))) }),
        );
        assert_eq!(started.unwrap(), Started::Running);
        assert_eq!(lifecycle.request_quit(), QuitDecision::StartShutdown);
        block_on(lifecycle.stop()).unwrap();
        block_on(lifecycle.stop()).unwrap();

        assert_eq!(shutdowns.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn nothing_starts_once_quitting() {
        let brought_up = Arc::new(AtomicBool::new(false));
        let lifecycle = Lifecycle::<FakeStack>::default();
        assert_eq!(lifecycle.request_quit(), QuitDecision::StartShutdown);

        let flag = Arc::clone(&brought_up);
        let started = block_on(lifecycle.start(async move {
            flag.store(true, Ordering::SeqCst);
            Ok::<_, std::io::Error>(FakeStack(Arc::new(AtomicUsize::new(0))))
        }));

        assert_eq!(started.unwrap(), Started::Skipped);
        assert!(!brought_up.load(Ordering::SeqCst));
    }

    #[test]
    fn a_failed_start_leaves_nothing_to_stop() {
        let lifecycle = Lifecycle::<FakeStack>::default();

        let started = block_on(lifecycle.start(async { Err::<FakeStack, _>("boom") }));

        assert_eq!(started.unwrap_err(), "boom");
        block_on(lifecycle.stop()).unwrap();
        assert_eq!(lifecycle.request_quit(), QuitDecision::Exit);
    }

    #[test]
    fn quitting_during_startup_waits_and_stops_what_started() {
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let lifecycle = Arc::new(Lifecycle::default());
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel::<()>();

        let starting = {
            let lifecycle = Arc::clone(&lifecycle);
            let stack = FakeStack(Arc::clone(&shutdowns));
            spawn(async move {
                lifecycle
                    .start(async move {
                        entered_tx.send(()).unwrap();
                        release_rx.await.unwrap();
                        Ok::<_, std::io::Error>(stack)
                    })
                    .await
            })
        };
        block_on(entered_rx).unwrap();

        // The user quits while the stack is still coming up.
        assert_eq!(lifecycle.request_quit(), QuitDecision::StartShutdown);
        let stopping = {
            let lifecycle = Arc::clone(&lifecycle);
            spawn(async move { lifecycle.stop().await })
        };
        assert_eq!(lifecycle.request_quit(), QuitDecision::AlreadyShuttingDown);

        release_tx.send(()).unwrap();
        assert_eq!(block_on(starting).unwrap().unwrap(), Started::Running);
        block_on(stopping).unwrap().unwrap();

        assert_eq!(shutdowns.load(Ordering::SeqCst), 1);
        assert_eq!(lifecycle.request_quit(), QuitDecision::Exit);
    }

    #[test]
    fn a_failed_step_stops_the_stack() {
        let shutdowns = Arc::new(AtomicUsize::new(0));

        let result = block_on(finish_or_stop(FakeStack(Arc::clone(&shutdowns)), |_| {
            Err::<(), _>("sign-in failed")
        }));

        assert_eq!(result.err(), Some("sign-in failed"));
        assert_eq!(shutdowns.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_successful_step_keeps_the_stack_running() {
        let shutdowns = Arc::new(AtomicUsize::new(0));

        let result = block_on(finish_or_stop(FakeStack(Arc::clone(&shutdowns)), |_| {
            Ok::<(), &str>(())
        }));

        assert!(result.is_ok());
        assert_eq!(shutdowns.load(Ordering::SeqCst), 0);
    }
}
