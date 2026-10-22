// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Where the agent reaches its server and how it proves both sides: the
//! token authenticates the agent, the pinned channel key the server.

use std::fmt;

use shared::agent_channel::ChannelPublicKey;

/// The server this agent works for.
#[derive(Clone)]
pub struct ServerEndpoint {
    /// WebSocket base URL (`ws://` or `wss://`), without a path.
    pub url: String,
    /// The agent's token, sent only inside the encrypted channel.
    pub token: String,
    /// The server's agent channel key: a server that cannot complete the
    /// handshake for it is refused.
    pub key: ChannelPublicKey,
}

impl fmt::Debug for ServerEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerEndpoint")
            .field("url", &self.url)
            .field("token", &"[REDACTED]")
            .field("key", &self.key.to_string())
            .finish()
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use shared::agent_channel::ChannelPrivateKey;

    use super::ServerEndpoint;

    /// The private key every test server uses.
    pub(crate) fn server_key() -> ChannelPrivateKey {
        ChannelPrivateKey::derive(b"agent test server").unwrap()
    }

    /// An endpoint at `url` that pins [`server_key`].
    pub(crate) fn endpoint(url: &str) -> ServerEndpoint {
        ServerEndpoint {
            url: url.to_owned(),
            token: "token".to_owned(),
            key: server_key().public_key().unwrap(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_is_redacted_in_debug_output() {
        let endpoint = test_support::endpoint("ws://backup.example");
        let debug = format!("{endpoint:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("\"token\""));
        assert!(debug.contains("ws://backup.example"));
    }
}
