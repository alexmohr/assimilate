// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! What the agent's system service (the Windows service, the macOS launchd
//! daemon) shares: the `service` command line, and the configuration
//! `service install` stores for `service run`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use shared::agent_channel::ChannelPublicKey;

use crate::ServerEndpoint;

/// `assimilate-agent service ...`.
#[derive(clap::Subcommand)]
pub enum Command {
    /// Install, remove or run the agent as a system service.
    Service {
        #[command(subcommand)]
        action: Action,
    },
}

#[derive(clap::Subcommand)]
pub enum Action {
    /// Register the service, store its server URL, token and server key,
    /// and start it. Run again to change any of them; the service is
    /// restarted.
    ///
    /// The token is read from the `BORG_AGENT_TOKEN` environment variable
    /// only: a command line can be read by other local users while it runs.
    Install {
        #[arg(long, env = "BORG_SERVER_URL")]
        server_url: String,
        /// The server's agent channel key.
        #[arg(long, env = "BORG_SERVER_KEY")]
        server_key: ChannelPublicKey,
    },
    /// Stop and remove the service and its stored configuration.
    Uninstall,
    /// Run as the service. Only the service manager starts this.
    #[command(hide = true)]
    Run,
}

/// Why a stored configuration couldn't be made or used.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("serializing the service configuration: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("invalid service configuration {}: {source}", path.display())]
    Invalid {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("set the agent token in the BORG_AGENT_TOKEN environment variable")]
    NoToken,
    #[error(
        "the stored configuration {} has no valid server key; run `service install` again with \
         --server-key",
        path.display()
    )]
    NoServerKey { path: PathBuf },
}

/// What `install` stores for `run`.
#[derive(Serialize, Deserialize)]
pub struct StoredConfig {
    server_url: String,
    token: String,
    /// Missing from configurations stored before the agent channel was
    /// encrypted; such an agent refuses to start until installed again.
    #[serde(default)]
    server_key: Option<String>,
}

impl StoredConfig {
    /// What `install` stores: the server, its key and the agent `token`,
    /// which must be set.
    pub fn install(
        server_url: String,
        server_key: &ChannelPublicKey,
        token: Option<String>,
    ) -> Result<Self, ConfigError> {
        Ok(Self {
            server_url,
            token: token.ok_or(ConfigError::NoToken)?,
            server_key: Some(server_key.to_string()),
        })
    }

    /// The JSON to store.
    pub fn to_json(&self) -> Result<Vec<u8>, ConfigError> {
        Ok(serde_json::to_vec_pretty(self)?)
    }

    /// The configuration `json`, read from `path`, holds.
    pub fn from_json(json: &[u8], path: &Path) -> Result<Self, ConfigError> {
        serde_json::from_slice(json).map_err(|source| ConfigError::Invalid {
            path: path.to_path_buf(),
            source,
        })
    }

    /// The server to connect to; `path` is where this was read from.
    pub fn endpoint(self, path: &Path) -> Result<ServerEndpoint, ConfigError> {
        let key = self
            .server_key
            .and_then(|key| key.parse::<ChannelPublicKey>().ok())
            .ok_or_else(|| ConfigError::NoServerKey {
                path: path.to_path_buf(),
            })?;
        Ok(ServerEndpoint {
            url: self.server_url,
            token: self.token,
            key,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

    #[test]
    fn the_stored_config_round_trips() {
        let path = Path::new("agent.json");
        let key: ChannelPublicKey = KEY.parse().unwrap();
        let stored = StoredConfig::install(
            "wss://backup.example".to_owned(),
            &key,
            Some("t0ken".to_owned()),
        )
        .unwrap();

        let json = stored.to_json().unwrap();
        let endpoint = StoredConfig::from_json(&json, path)
            .unwrap()
            .endpoint(path)
            .unwrap();

        assert_eq!(endpoint.url, "wss://backup.example");
        assert_eq!(endpoint.token, "t0ken");
        assert_eq!(endpoint.key.to_string(), KEY);
    }

    #[test]
    fn install_needs_the_token_from_the_environment() {
        let key: ChannelPublicKey = KEY.parse().unwrap();

        let missing = StoredConfig::install("wss://backup.example".to_owned(), &key, None);

        assert!(matches!(missing, Err(ConfigError::NoToken)));
    }

    #[test]
    fn a_config_stored_without_a_server_key_is_refused() {
        let path = Path::new("agent.json");
        let json = br#"{"server_url":"wss://backup.example","token":"t0ken"}"#;

        let error = StoredConfig::from_json(json, path)
            .unwrap()
            .endpoint(path)
            .err()
            .unwrap();

        assert!(error.to_string().contains("--server-key"), "{error}");
    }

    #[test]
    fn a_malformed_config_names_its_file() {
        let error = StoredConfig::from_json(b"{", Path::new("agent.json"))
            .err()
            .unwrap();

        assert!(error.to_string().contains("agent.json"), "{error}");
    }
}
