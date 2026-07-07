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

    /// Register a newly connected agent.
    pub async fn register(
        &self,
        hostname: String,
        sender: mpsc::Sender<ServerToAgent>,
        supports_restart: bool,
        restart_unavailable_reason: Option<String>,
    ) {
        let connection = AgentConnection {
            sender,
            supports_restart,
            restart_unavailable_reason,
        };
        self.connections.write().await.insert(hostname, connection);
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
