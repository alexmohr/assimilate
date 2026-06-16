// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::time::Duration;

use tokio::sync::broadcast;

use super::registry::AgentRegistry;

/// How often to check that the agent is still connected while waiting for it to
/// report completion. Borg backups have no fixed upper bound on duration (a large
/// repository can legitimately take many hours), so the wait itself has no timeout;
/// this poll is only a way to notice early that the agent has gone away.
const CONNECTIVITY_POLL_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct OperationOutcome {
    pub hostname: String,
    pub repo_id: i64,
    pub success: bool,
}

#[derive(Clone, Debug)]
pub struct CompletionBus {
    tx: broadcast::Sender<OperationOutcome>,
}

impl CompletionBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx }
    }

    pub fn publish(&self, outcome: OperationOutcome) {
        let _ = self.tx.send(outcome);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<OperationOutcome> {
        self.tx.subscribe()
    }
}

impl Default for CompletionBus {
    fn default() -> Self {
        Self::new()
    }
}

/// The result of waiting for a triggered operation to finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionOutcome {
    Success,
    Failed,
    /// The agent disconnected (or the bus shut down) before reporting completion.
    /// Sequential schedules treat this the same as a failure, but it's tracked
    /// separately so callers can log the distinct cause.
    AgentDisconnected,
}

/// Waits for the matching completion event for `hostname`/`repo_id` on `rx`.
///
/// There is deliberately no fixed timeout: a legitimate backup of a large
/// repository can run for many hours, and bailing out early would let the next
/// target in a sequential schedule start while the prior one is still running.
/// Instead, the wait is bounded only by the agent's connection: if the agent
/// disconnects, it can no longer report completion, so that's treated as the
/// terminal outcome.
pub async fn wait_for_completion(
    registry: &AgentRegistry,
    rx: broadcast::Receiver<OperationOutcome>,
    hostname: &str,
    repo_id: i64,
) -> CompletionOutcome {
    wait_for_completion_with_poll_interval(
        registry,
        rx,
        hostname,
        repo_id,
        CONNECTIVITY_POLL_INTERVAL,
    )
    .await
}

async fn wait_for_completion_with_poll_interval(
    registry: &AgentRegistry,
    mut rx: broadcast::Receiver<OperationOutcome>,
    hostname: &str,
    repo_id: i64,
    poll_interval: Duration,
) -> CompletionOutcome {
    loop {
        tokio::select! {
            outcome = rx.recv() => {
                match outcome {
                    Ok(o) if o.hostname == hostname && o.repo_id == repo_id => {
                        return if o.success {
                            CompletionOutcome::Success
                        } else {
                            CompletionOutcome::Failed
                        };
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => {
                        return CompletionOutcome::AgentDisconnected;
                    }
                }
            }
            () = tokio::time::sleep(poll_interval) => {
                if !registry.is_connected(hostname).await {
                    return CompletionOutcome::AgentDisconnected;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wait_for_completion_returns_success_on_matching_outcome() {
        let bus = CompletionBus::new();
        let registry = AgentRegistry::new();
        let rx = bus.subscribe();

        bus.publish(OperationOutcome {
            hostname: "host-a".to_string(),
            repo_id: 1,
            success: true,
        });

        let outcome = wait_for_completion(&registry, rx, "host-a", 1).await;
        assert_eq!(outcome, CompletionOutcome::Success);
    }

    #[tokio::test]
    async fn wait_for_completion_ignores_unrelated_outcomes() {
        let bus = CompletionBus::new();
        let registry = AgentRegistry::new();
        let rx = bus.subscribe();

        bus.publish(OperationOutcome {
            hostname: "other-host".to_string(),
            repo_id: 1,
            success: true,
        });
        bus.publish(OperationOutcome {
            hostname: "host-a".to_string(),
            repo_id: 2,
            success: true,
        });
        bus.publish(OperationOutcome {
            hostname: "host-a".to_string(),
            repo_id: 1,
            success: false,
        });

        let outcome = wait_for_completion(&registry, rx, "host-a", 1).await;
        assert_eq!(outcome, CompletionOutcome::Failed);
    }

    #[tokio::test]
    async fn wait_for_completion_keeps_waiting_while_agent_stays_connected() {
        // A long-running backup must not be abandoned just because it's slow;
        // with no completion event and a connected agent, the wait never resolves
        // on its own.
        let bus = CompletionBus::new();
        let registry = AgentRegistry::new();
        let (tx, _) = tokio::sync::mpsc::channel(1);
        registry
            .register("host-a".to_string(), tx, false, None)
            .await;
        let rx = bus.subscribe();

        let wait = tokio::spawn({
            let registry = registry.clone();
            async move {
                wait_for_completion_with_poll_interval(
                    &registry,
                    rx,
                    "host-a",
                    1,
                    Duration::from_millis(10),
                )
                .await
            }
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!wait.is_finished());
        wait.abort();
    }

    #[tokio::test]
    async fn wait_for_completion_detects_agent_disconnect() {
        let bus = CompletionBus::new();
        let registry = AgentRegistry::new();
        let (tx, _) = tokio::sync::mpsc::channel(1);
        registry
            .register("host-a".to_string(), tx, false, None)
            .await;
        let rx = bus.subscribe();

        let wait = tokio::spawn({
            let registry = registry.clone();
            async move {
                wait_for_completion_with_poll_interval(
                    &registry,
                    rx,
                    "host-a",
                    1,
                    Duration::from_millis(10),
                )
                .await
            }
        });

        tokio::time::sleep(Duration::from_millis(30)).await;
        registry.unregister("host-a").await;

        let outcome = tokio::time::timeout(Duration::from_secs(1), wait)
            .await
            .expect("wait_for_completion should resolve shortly after disconnect")
            .expect("task should not panic");
        assert_eq!(outcome, CompletionOutcome::AgentDisconnected);
    }
}
