// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Registries of requests the server sent to one agent and is waiting on.

use std::{collections::HashMap, sync::Arc};

use tokio::sync::{Mutex, oneshot};

/// A request in flight: the agent it was sent to, and the channel its answer
/// resolves.
struct PendingRequest<T> {
    agent_id: i64,
    tx: oneshot::Sender<T>,
}

/// What looking up an agent's answer in a [`PendingRequests`] found.
pub enum Claim<T> {
    /// The request was sent to this agent; it is no longer pending, and the
    /// answer goes out on the returned channel.
    Claimed(oneshot::Sender<T>),
    /// No request is waiting under this id.
    Unknown,
    /// The request was sent to another agent. It stays pending, so the agent
    /// it was sent to can still answer it.
    WrongAgent {
        /// The agent the request was sent to.
        expected_agent_id: i64,
    },
}

/// One-shot channels for requests sent to a specific agent, keyed by request
/// id.
///
/// An answer only resolves a request when it comes from the agent the request
/// was sent to, so one agent cannot fulfill or fail another agent's operation
/// by replaying its request id.
pub struct PendingRequests<T> {
    requests: Arc<Mutex<HashMap<String, PendingRequest<T>>>>,
}

impl<T> Clone for PendingRequests<T> {
    fn clone(&self) -> Self {
        Self {
            requests: Arc::clone(&self.requests),
        }
    }
}

impl<T> Default for PendingRequests<T> {
    fn default() -> Self {
        Self {
            requests: Arc::default(),
        }
    }
}

impl<T> PendingRequests<T> {
    /// Waits for `agent_id`'s answer to `request_id` on `tx`.
    pub async fn insert(&self, request_id: String, agent_id: i64, tx: oneshot::Sender<T>) {
        self.requests
            .lock()
            .await
            .insert(request_id, PendingRequest { agent_id, tx });
    }

    /// Stops waiting on `request_id`, whichever agent it was sent to - for
    /// the caller that gave up on it (send failure, timeout).
    pub async fn remove(&self, request_id: &str) {
        self.requests.lock().await.remove(request_id);
    }

    /// Takes the channel for `request_id` if `agent_id` is the agent it was
    /// sent to.
    pub async fn claim(&self, request_id: &str, agent_id: i64) -> Claim<T> {
        let mut requests = self.requests.lock().await;
        let Some(expected_agent_id) = requests.get(request_id).map(|pending| pending.agent_id)
        else {
            return Claim::Unknown;
        };
        if expected_agent_id != agent_id {
            return Claim::WrongAgent { expected_agent_id };
        }
        requests
            .remove(request_id)
            .map_or(Claim::Unknown, |pending| Claim::Claimed(pending.tx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn claim_by_target_agent_resolves_the_request() {
        let pending = PendingRequests::<u32>::default();
        let (tx, rx) = oneshot::channel();
        pending.insert("req".to_owned(), 1, tx).await;

        let Claim::Claimed(tx) = pending.claim("req", 1).await else {
            panic!("the target agent should claim its own request");
        };
        tx.send(7).unwrap();
        assert_eq!(rx.await.unwrap(), 7);
        assert!(matches!(pending.claim("req", 1).await, Claim::Unknown));
    }

    #[tokio::test]
    async fn claim_by_other_agent_leaves_the_request_pending() {
        let pending = PendingRequests::<u32>::default();
        let (tx, mut rx) = oneshot::channel();
        pending.insert("req".to_owned(), 1, tx).await;

        assert!(matches!(
            pending.claim("req", 2).await,
            Claim::WrongAgent {
                expected_agent_id: 1
            }
        ));
        assert!(rx.try_recv().is_err(), "nothing was sent on the channel");
        assert!(matches!(pending.claim("req", 1).await, Claim::Claimed(_)));
    }

    #[tokio::test]
    async fn claim_of_unknown_request_finds_nothing() {
        let pending = PendingRequests::<u32>::default();
        assert!(matches!(pending.claim("missing", 1).await, Claim::Unknown));
    }

    #[tokio::test]
    async fn remove_drops_the_request_for_any_agent() {
        let pending = PendingRequests::<u32>::default();
        let (tx, rx) = oneshot::channel();
        pending.insert("req".to_owned(), 1, tx).await;

        pending.remove("req").await;
        assert!(rx.await.is_err(), "the channel closes once removed");
        assert!(matches!(pending.claim("req", 1).await, Claim::Unknown));
    }
}
