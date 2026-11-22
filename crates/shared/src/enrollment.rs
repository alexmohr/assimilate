// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Enrollment tokens: short-lived, single-use tokens that let a new host
//! register itself as an agent without the server reaching it over SSH.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

use crate::time::Timestamp;

/// What every enrollment token starts with, so a pasted one is recognisable
/// and a secret scanner can match it.
pub const ENROLLMENT_TOKEN_PREFIX: &str = "enr_";

/// How many random bytes an enrollment token carries.
pub const ENROLLMENT_TOKEN_BYTES: usize = 32;

/// How long a token works when the request names no lifetime.
pub const DEFAULT_ENROLLMENT_LIFETIME_SECS: i64 = 60 * 60;

/// The longest lifetime a token may be given.
pub const MAX_ENROLLMENT_LIFETIME_SECS: i64 = 24 * 60 * 60;

/// How many characters of a token the list shows to tell tokens apart.
pub const ENROLLMENT_TOKEN_HINT_LEN: usize = 8;

/// Where an enrollment token is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, ToSchema)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentTokenStatus {
    /// It can still be used.
    Pending,
    /// A host enrolled with it.
    Used,
    /// Its lifetime ran out before a host used it.
    Expired,
    /// A user revoked it before a host used it.
    Revoked,
}

/// Request to create an enrollment token.
#[derive(Debug, Clone, Deserialize, TS, ToSchema)]
#[ts(export)]
pub struct CreateEnrollmentTokenRequest {
    /// How long the token works, in seconds. Defaults to one hour.
    #[serde(default)]
    #[ts(optional, type = "number")]
    pub lifetime_secs: Option<i64>,
    /// Only a host with exactly this hostname can use the token.
    #[serde(default)]
    #[ts(optional)]
    pub hostname: Option<String>,
    /// Agent tags the enrolling host receives.
    #[serde(default)]
    #[ts(type = "number[]")]
    pub tag_ids: Vec<i64>,
}

/// An enrollment token as the list shows it. The token itself is never here.
#[derive(Debug, Clone, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct EnrollmentTokenResponse {
    /// Unique identifier.
    #[ts(type = "number")]
    pub id: i64,
    /// The token's first characters, to tell tokens apart.
    pub hint: String,
    /// The user who created it, if they still exist.
    pub created_by: Option<String>,
    /// The hostname the token is restricted to, if any.
    pub hostname: Option<String>,
    /// The agent tags a host receives on enrolling.
    #[ts(type = "number[]")]
    pub tag_ids: Vec<i64>,
    /// Where the token is in its life.
    pub status: EnrollmentTokenStatus,
    /// When it was created.
    pub created_at: Timestamp,
    /// When it stops working.
    pub expires_at: Timestamp,
    /// The hostname of the agent that used it, once one did.
    pub used_by: Option<String>,
}

/// A created token: the list entry and the one time its secret is shown.
#[derive(Debug, Clone, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct CreateEnrollmentTokenResponse {
    /// The token's list entry.
    pub token: EnrollmentTokenResponse,
    /// The token itself. It is not stored and cannot be shown again.
    pub plaintext: String,
}

/// Response listing enrollment tokens.
#[derive(Debug, Clone, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct ListEnrollmentTokensResponse {
    /// Every token, newest first.
    pub tokens: Vec<EnrollmentTokenResponse>,
}

/// Response to revoking an enrollment token.
#[derive(Debug, Clone, Serialize, TS, ToSchema)]
#[ts(export)]
pub struct RevokeEnrollmentTokenResponse {
    /// Whether the token was revoked.
    pub revoked: bool,
}

/// Why the server refused to enroll a host. One answer for every cause, so a
/// caller learns nothing about which tokens exist.
pub const ENROLLMENT_REFUSED: &str =
    "the enrollment token is invalid, expired, revoked or already used";
