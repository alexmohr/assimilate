// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! The `BorgBase` account management types shared by the server API and the frontend.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where a `BorgBase` repository is stored.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    TS,
    utoipa::ToSchema,
    strum_macros::AsRefStr,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
#[ts(export)]
pub enum BorgbaseRegion {
    /// Europe.
    Eu,
    /// United States.
    Us,
}

/// What the server's key may do to a `BorgBase` repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BorgbaseKeyAccess {
    /// Add data, never free space.
    AppendOnly,
    /// Add data and delete it.
    Full,
}

/// A repository on `BorgBase`, with its quota and usage as `BorgBase` reports them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct BorgbaseRepo {
    /// `BorgBase`'s repository ID, which is also the SSH user and the first label of its host.
    pub id: String,
    /// The repository's name on `BorgBase`.
    pub name: String,
    /// The SSH host the repository is reached at.
    pub ssh_host: String,
    /// Whether `BorgBase` enforces the quota.
    pub quota_enabled: bool,
    /// The quota, in the unit `BorgBase` reports it in.
    pub quota: Option<f64>,
    /// The disk usage, in the unit `BorgBase` reports it in.
    pub usage: Option<f64>,
    /// When the repository last changed.
    pub last_modified: Option<String>,
}

/// The state of the `BorgBase` account the server manages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct BorgbaseStatus {
    /// Whether an API token is stored.
    pub token_set: bool,
}

/// Request to store the `BorgBase` API token.
#[derive(Clone, Deserialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct SetBorgbaseTokenRequest {
    /// The API token; it is encrypted at rest and never returned.
    pub token: String,
}

impl std::fmt::Debug for SetBorgbaseTokenRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SetBorgbaseTokenRequest { token: [REDACTED] }")
    }
}

/// Request to create a repository on `BorgBase` for the server's key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct CreateBorgbaseRepoRequest {
    /// The new repository's name.
    pub name: String,
    /// Where to store it.
    pub region: BorgbaseRegion,
    /// What the server's key may do to it.
    pub access: BorgbaseKeyAccess,
}

/// A repository created on `BorgBase`, ready to be added as a repository host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS, utoipa::ToSchema)]
#[ts(export)]
pub struct CreatedBorgbaseRepo {
    /// `BorgBase`'s repository ID.
    pub id: String,
    /// The repository's name.
    pub name: String,
    /// The SSH host to connect to.
    pub ssh_host: String,
    /// The SSH user to connect as.
    pub ssh_user: String,
    /// The path on the host.
    pub repo_path: String,
}

/// The SSH host a `BorgBase` repository is reached at.
#[must_use]
pub fn ssh_host(repo_id: &str) -> String {
    format!("{repo_id}.repo.borgbase.com")
}

/// The path of the repository on its `BorgBase` host.
pub const REPO_PATH: &str = "./repo";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repository_is_reached_at_its_own_host() {
        assert_eq!(ssh_host("abcd1234"), "abcd1234.repo.borgbase.com");
    }

    #[test]
    fn regions_and_access_use_their_wire_names() {
        assert_eq!(BorgbaseRegion::Eu.as_ref(), "eu");
        assert_eq!(
            serde_json::to_string(&BorgbaseRegion::Us).unwrap(),
            "\"us\""
        );
        assert_eq!(
            serde_json::to_string(&BorgbaseKeyAccess::AppendOnly).unwrap(),
            "\"append_only\""
        );
    }

    #[test]
    fn the_token_request_never_debug_prints_the_token() {
        let request = SetBorgbaseTokenRequest {
            token: "secret-token".to_owned(),
        };
        assert!(!format!("{request:?}").contains("secret-token"));
    }
}
