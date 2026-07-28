// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{collections::HashMap, sync::Arc};

use shared::protocol::ServerToAgent;
use tokio::sync::{RwLock, mpsc};

/// A connected agent's outbound channel and capability flags.
#[derive(Debug)]
pub struct AgentConnection {
    /// Channel for sending messages to this agent.
    pub sender: mpsc::Sender<ServerToAgent>,
    /// Whether the agent supports the restart command.
    pub supports_restart: bool,
    /// If restart is unavailable, the reason provided by the agent.
    pub restart_unavailable_reason: Option<String>,
}

/// Registry of all currently connected agents, keyed by hostname.
#[derive(Debug, Clone, Default)]
pub struct AgentRegistry {
    connections: Arc<RwLock<HashMap<String, AgentConnection>>>,
}

impl AgentRegistry {
    /// Create an empty agent registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a newly connected agent. Returns `true` if this replaced an
    /// already-registered connection for the same hostname (a reconnect) -
    /// callers use this to detect that the previous session is gone and any
    /// in-flight operations tied to it are now abandoned, since
    /// [`is_connected`](Self::is_connected) alone can't tell a fresh session
    /// apart from the one that was there before.
    pub async fn register(
        &self,
        hostname: String,
        sender: mpsc::Sender<ServerToAgent>,
        supports_restart: bool,
        restart_unavailable_reason: Option<String>,
    ) -> bool {
        let connection = AgentConnection {
            sender,
            supports_restart,
            restart_unavailable_reason,
        };
        self.connections
            .write()
            .await
            .insert(hostname, connection)
            .is_some()
    }

    /// Remove a disconnected agent from the registry.
    pub async fn unregister(&self, hostname: &str) {
        self.connections.write().await.remove(hostname);
    }

    /// # Errors
    ///
    /// Returns an error if the underlying operation fails.
    pub async fn send_to(
        &self,
        hostname: &str,
        msg: ServerToAgent,
    ) -> Result<(), Box<mpsc::error::SendError<ServerToAgent>>> {
        let connections = self.connections.read().await;
        if let Some(conn) = connections.get(hostname) {
            conn.sender.send(msg).await.map_err(Box::new)
        } else {
            Err(Box::new(mpsc::error::SendError(msg)))
        }
    }

    /// Return the hostnames of all currently connected agents.
    pub async fn connected_agents(&self) -> Vec<String> {
        self.connections.read().await.keys().cloned().collect()
    }

    /// Check whether a given agent is currently connected.
    pub async fn is_connected(&self, hostname: &str) -> bool {
        self.connections.read().await.contains_key(hostname)
    }

    /// Return the restart capability for a given agent (`supports_restart`, reason).
    pub async fn restart_capability(&self, hostname: &str) -> (bool, Option<String>) {
        let connections = self.connections.read().await;
        connections.get(hostname).map_or(
            (false, Some("agent is not connected".to_owned())),
            |conn| {
                (
                    conn.supports_restart,
                    conn.restart_unavailable_reason.clone(),
                )
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use super::AgentRegistry;

    #[tokio::test]
    async fn register_reports_first_registration_as_not_a_replacement() {
        let registry = AgentRegistry::new();
        let (tx, _rx) = mpsc::channel(1);

        let replaced = registry
            .register("agent-a".to_owned(), tx, true, None)
            .await;

        assert!(!replaced);
        assert!(registry.is_connected("agent-a").await);
    }

    #[tokio::test]
    async fn register_reports_reconnect_as_a_replacement() {
        let registry = AgentRegistry::new();
        let (tx1, _rx1) = mpsc::channel(1);
        let (tx2, _rx2) = mpsc::channel(1);

        let first = registry
            .register("agent-a".to_owned(), tx1, true, None)
            .await;
        let second = registry
            .register("agent-a".to_owned(), tx2, true, None)
            .await;

        assert!(!first);
        assert!(second);
    }

    #[tokio::test]
    async fn register_does_not_report_replacement_for_a_different_hostname() {
        let registry = AgentRegistry::new();
        let (tx1, _rx1) = mpsc::channel(1);
        let (tx2, _rx2) = mpsc::channel(1);

        registry
            .register("agent-a".to_owned(), tx1, true, None)
            .await;
        let replaced = registry
            .register("agent-b".to_owned(), tx2, true, None)
            .await;

        assert!(!replaced);
    }
}
