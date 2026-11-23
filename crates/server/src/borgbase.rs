// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! A client for the `BorgBase` GraphQL API.
//!
//! `BorgBase` is a borg hosting service whose repositories, SSH keys and disk usage are managed
//! through a GraphQL API behind a bearer token. The documents below are the ones `BorgBase`'s own
//! API client sends. Everything else about the API (the units of quota and usage, an error
//! body's exact shape) is undocumented, so the client reports numbers as they arrive and treats
//! any non-success status or any `errors` entry as a failure.
//!
//! The token is a secret: it never appears in a `Debug` rendering, a log line or an error.

use std::{fmt, time::Duration};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use shared::borgbase::{BorgbaseKeyAccess, BorgbaseRegion, BorgbaseRepo};

/// Where `BorgBase` serves its GraphQL API.
pub const DEFAULT_API_URL: &str = "https://api.borgbase.com/graphql";

/// The environment variable that points the server at another GraphQL endpoint, used by the demo
/// and the tests to talk to a stand-in for `BorgBase`.
pub const API_URL_ENV_KEY: &str = "ASSIMILATE_BORGBASE_API_URL";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The GraphQL endpoint to use: [`API_URL_ENV_KEY`] when set and not empty, else `BorgBase`.
#[must_use]
pub fn api_url() -> String {
    std::env::var(API_URL_ENV_KEY)
        .ok()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_API_URL.to_owned())
}

/// A `BorgBase` API token.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiToken(String);

impl ApiToken {
    /// A token an admin entered, with surrounding whitespace removed.
    ///
    /// # Errors
    ///
    /// Returns [`BorgbaseError::InvalidToken`] if it is empty or contains whitespace or control
    /// characters, which can never be part of a bearer token.
    pub fn parse(entered: &str) -> Result<Self, BorgbaseError> {
        let token = entered.trim();
        if token.is_empty() || token.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(BorgbaseError::InvalidToken);
        }
        Ok(Self(token.to_owned()))
    }

    /// The token itself, for sending or encrypting.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiToken([REDACTED])")
    }
}

/// Why a `BorgBase` request failed.
#[derive(Debug, thiserror::Error)]
pub enum BorgbaseError {
    /// The entered token is empty or malformed.
    #[error("the BorgBase API token must be a single word without whitespace")]
    InvalidToken,
    /// The request never got an answer.
    #[error("could not reach the BorgBase API: {0}")]
    Unreachable(String),
    /// `BorgBase` answered with a failure status.
    #[error("the BorgBase API answered with status {0}; check the API token and its permissions")]
    Status(u16),
    /// `BorgBase` answered with GraphQL errors.
    #[error("the BorgBase API refused the request: {0}")]
    Api(String),
    /// The answer was not the shape the documents ask for.
    #[error("the BorgBase API sent an answer this server does not understand")]
    Malformed,
}

/// A repository as `BorgBase`'s `repoList` returns it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct WireRepo {
    id: String,
    name: String,
    #[serde(default, rename = "quotaEnabled")]
    quota_enabled: Option<bool>,
    #[serde(default)]
    quota: Option<f64>,
    #[serde(default, rename = "currentUsage")]
    current_usage: Option<f64>,
    #[serde(default, rename = "lastModified")]
    last_modified: Option<String>,
}

impl From<WireRepo> for BorgbaseRepo {
    fn from(repo: WireRepo) -> Self {
        Self {
            ssh_host: shared::borgbase::ssh_host(&repo.id),
            id: repo.id,
            name: repo.name,
            quota_enabled: repo.quota_enabled.unwrap_or(false),
            quota: repo.quota,
            usage: repo.current_usage,
            last_modified: repo.last_modified,
        }
    }
}

/// An SSH key on a `BorgBase` account.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RemoteKey {
    /// `BorgBase`'s key ID.
    pub id: String,
    /// The key's name.
    pub name: String,
    /// The public key, if `BorgBase` returns it.
    #[serde(default, rename = "keyData")]
    pub key_data: Option<String>,
}

/// A repository `BorgBase` just created.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CreatedRepo {
    /// `BorgBase`'s repository ID.
    pub id: String,
    /// The repository's name.
    pub name: String,
}

const REPO_LIST: &str =
    "query repoList { repoList { id name quota quotaEnabled lastModified currentUsage } }";
const SSH_LIST: &str = "query sshList { sshList { id name keyData } }";
const SSH_ADD: &str = "mutation sshAdd($name: String!, $keyData: String!) { sshAdd(name: $name, \
                       keyData: $keyData) { keyAdded { id name keyData } } }";
const REPO_ADD: &str = "mutation repoAdd($name: String!, $region: String!, $appendOnlyKeys: \
                        [String], $fullAccessKeys: [String], $quotaEnabled: Boolean) { \
                        repoAdd(name: $name, region: $region, appendOnlyKeys: $appendOnlyKeys, \
                        fullAccessKeys: $fullAccessKeys, quotaEnabled: $quotaEnabled) { repoAdded \
                        { id name } } }";

#[derive(Deserialize)]
struct RepoListData {
    #[serde(rename = "repoList")]
    repo_list: Vec<WireRepo>,
}

#[derive(Deserialize)]
struct SshListData {
    #[serde(rename = "sshList")]
    ssh_list: Vec<RemoteKey>,
}

#[derive(Deserialize)]
struct SshAddData {
    #[serde(rename = "sshAdd")]
    ssh_add: SshAdded,
}

#[derive(Deserialize)]
struct SshAdded {
    #[serde(rename = "keyAdded")]
    key_added: RemoteKey,
}

#[derive(Deserialize)]
struct RepoAddData {
    #[serde(rename = "repoAdd")]
    repo_add: RepoAdded,
}

#[derive(Deserialize)]
struct RepoAdded {
    #[serde(rename = "repoAdded")]
    repo_added: CreatedRepo,
}

#[derive(Serialize)]
struct Request<'a, V: Serialize> {
    query: &'a str,
    variables: V,
}

#[derive(Deserialize)]
struct Response<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GraphQlError>,
}

#[derive(Deserialize)]
struct GraphQlError {
    message: String,
}

/// A `BorgBase` API client for one token.
#[derive(Debug, Clone)]
pub struct BorgbaseClient {
    http: reqwest::Client,
    url: String,
    token: ApiToken,
}

impl BorgbaseClient {
    /// A client sending `token` to `url`.
    ///
    /// # Errors
    ///
    /// Returns [`BorgbaseError::Unreachable`] if the HTTP client cannot be built.
    pub fn new(url: String, token: ApiToken) -> Result<Self, BorgbaseError> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| BorgbaseError::Unreachable(e.without_url().to_string()))?;
        Ok(Self { http, url, token })
    }

    async fn execute<V: Serialize, T: DeserializeOwned>(
        &self,
        query: &str,
        variables: V,
    ) -> Result<T, BorgbaseError> {
        let response = self
            .http
            .post(&self.url)
            .bearer_auth(self.token.expose())
            .json(&Request { query, variables })
            .send()
            .await
            .map_err(|e| BorgbaseError::Unreachable(e.without_url().to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(BorgbaseError::Status(status.as_u16()));
        }
        let body: Response<T> = response
            .json()
            .await
            .map_err(|_| BorgbaseError::Malformed)?;
        if !body.errors.is_empty() {
            let messages: Vec<&str> = body.errors.iter().map(|e| e.message.as_str()).collect();
            return Err(BorgbaseError::Api(messages.join("; ")));
        }
        body.data.ok_or(BorgbaseError::Malformed)
    }

    /// Every repository on the account, with its quota and usage.
    ///
    /// # Errors
    ///
    /// Returns a [`BorgbaseError`] if the request fails.
    pub async fn list_repos(&self) -> Result<Vec<BorgbaseRepo>, BorgbaseError> {
        let data: RepoListData = self.execute(REPO_LIST, serde_json::json!({})).await?;
        Ok(data.repo_list.into_iter().map(BorgbaseRepo::from).collect())
    }

    /// Every SSH key on the account.
    ///
    /// # Errors
    ///
    /// Returns a [`BorgbaseError`] if the request fails.
    pub async fn list_keys(&self) -> Result<Vec<RemoteKey>, BorgbaseError> {
        let data: SshListData = self.execute(SSH_LIST, serde_json::json!({})).await?;
        Ok(data.ssh_list)
    }

    /// Adds `public_key` to the account, or returns the key already there with the same key
    /// material.
    ///
    /// # Errors
    ///
    /// Returns a [`BorgbaseError`] if a request fails.
    pub async fn ensure_key(
        &self,
        name: &str,
        public_key: &str,
    ) -> Result<RemoteKey, BorgbaseError> {
        let wanted = key_material(public_key);
        let existing = self.list_keys().await?;
        if let Some(found) = existing.into_iter().find(|key| {
            key.key_data
                .as_deref()
                .is_some_and(|data| key_material(data) == wanted)
        }) {
            return Ok(found);
        }
        let data: SshAddData = self
            .execute(
                SSH_ADD,
                serde_json::json!({ "name": name, "keyData": public_key.trim() }),
            )
            .await?;
        Ok(data.ssh_add.key_added)
    }

    /// Creates a repository that `key_id` may use with `access`.
    ///
    /// # Errors
    ///
    /// Returns a [`BorgbaseError`] if the request fails.
    pub async fn create_repo(
        &self,
        name: &str,
        region: BorgbaseRegion,
        key_id: &str,
        access: BorgbaseKeyAccess,
    ) -> Result<CreatedRepo, BorgbaseError> {
        let (append_only, full) = match access {
            BorgbaseKeyAccess::AppendOnly => (vec![key_id], Vec::new()),
            BorgbaseKeyAccess::Full => (Vec::new(), vec![key_id]),
        };
        let data: RepoAddData = self
            .execute(
                REPO_ADD,
                serde_json::json!({
                    "name": name,
                    "region": region.as_ref(),
                    "appendOnlyKeys": append_only,
                    "fullAccessKeys": full,
                    "quotaEnabled": false,
                }),
            )
            .await?;
        Ok(data.repo_add.repo_added)
    }
}

/// `type base64` of a public key, without its comment, which is what identifies the key.
fn key_material(public_key: &str) -> String {
    public_key
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests;
