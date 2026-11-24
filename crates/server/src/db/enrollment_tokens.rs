// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Enrollment tokens: single-use tokens a new host trades for an agent token.

use shared::{
    enrollment::{EnrollmentTokenResponse, EnrollmentTokenStatus},
    time::Timestamp,
};
use sqlx::PgPool;

use crate::error::ApiError;

/// A row of `enrollment_tokens`, joined with the names the list shows.
#[derive(Debug, Clone)]
pub struct EnrollmentTokenRow {
    /// Unique identifier.
    pub id: i64,
    /// The token's first characters.
    pub hint: String,
    /// Who created it, if they still exist.
    pub created_by: Option<String>,
    /// The hostname it is restricted to, if any.
    pub hostname: Option<String>,
    /// The agent tags a host receives on enrolling.
    pub tag_ids: Vec<i64>,
    /// When it was created.
    pub created_at: Timestamp,
    /// When it stops working.
    pub expires_at: Timestamp,
    /// When a host used it.
    pub used_at: Option<Timestamp>,
    /// When it was revoked.
    pub revoked_at: Option<Timestamp>,
    /// The hostname of the agent that used it.
    pub used_by: Option<String>,
}

impl EnrollmentTokenRow {
    /// Where the token is in its life at `now`. A used or revoked token stays
    /// so; a pending one whose time is up is expired.
    #[must_use]
    pub fn status(&self, now: Timestamp) -> EnrollmentTokenStatus {
        if self.used_at.is_some() {
            EnrollmentTokenStatus::Used
        } else if self.revoked_at.is_some() {
            EnrollmentTokenStatus::Revoked
        } else if self.expires_at <= now {
            EnrollmentTokenStatus::Expired
        } else {
            EnrollmentTokenStatus::Pending
        }
    }

    /// The row as the API lists it, judged at `now`.
    #[must_use]
    pub fn into_response(self, now: Timestamp) -> EnrollmentTokenResponse {
        EnrollmentTokenResponse {
            status: self.status(now),
            id: self.id,
            hint: self.hint,
            created_by: self.created_by,
            hostname: self.hostname,
            tag_ids: self.tag_ids,
            created_at: self.created_at,
            expires_at: self.expires_at,
            used_by: self.used_by,
        }
    }
}

/// What a new token is made of.
pub struct NewEnrollmentToken<'a> {
    /// The SHA-256 of the token.
    pub token_hash: &'a str,
    /// The token's first characters.
    pub hint: &'a str,
    /// The user creating it.
    pub created_by: i64,
    /// When it stops working.
    pub expires_at: Timestamp,
    /// The hostname it is restricted to, if any.
    pub hostname: Option<&'a str>,
    /// The agent tags a host receives on enrolling.
    pub tag_ids: &'a [i64],
}

/// Inserts a token and returns its list entry.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn insert_enrollment_token(
    pool: &PgPool,
    new: &NewEnrollmentToken<'_>,
) -> Result<EnrollmentTokenRow, ApiError> {
    let id = sqlx::query_scalar!(
        "INSERT INTO enrollment_tokens (token_hash, hint, created_by, expires_at, hostname, \
         tag_ids) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        new.token_hash,
        new.hint,
        new.created_by,
        new.expires_at as Timestamp,
        new.hostname,
        new.tag_ids,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)?;
    get_enrollment_token(pool, id).await
}

/// One token with the names the list shows.
///
/// # Errors
///
/// Returns an error if:
/// - [`ApiError::Database`]: the database query fails
/// - [`ApiError::NotFound`]: the token does not exist
pub async fn get_enrollment_token(pool: &PgPool, id: i64) -> Result<EnrollmentTokenRow, ApiError> {
    sqlx::query_as!(
        EnrollmentTokenRow,
        r#"SELECT t.id, t.hint, u.username AS "created_by?", t.hostname, t.tag_ids,
           t.created_at, t.expires_at, t.used_at, t.revoked_at, a.hostname AS "used_by?"
           FROM enrollment_tokens t
           LEFT JOIN users u ON u.id = t.created_by
           LEFT JOIN agents a ON a.id = t.used_by_agent_id
           WHERE t.id = $1"#,
        id,
    )
    .fetch_optional(pool)
    .await
    .map_err(ApiError::Database)?
    .ok_or_else(|| ApiError::NotFound(format!("enrollment token {id} not found")))
}

/// Every token, newest first.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn list_enrollment_tokens(pool: &PgPool) -> Result<Vec<EnrollmentTokenRow>, ApiError> {
    sqlx::query_as!(
        EnrollmentTokenRow,
        r#"SELECT t.id, t.hint, u.username AS "created_by?", t.hostname, t.tag_ids,
           t.created_at, t.expires_at, t.used_at, t.revoked_at, a.hostname AS "used_by?"
           FROM enrollment_tokens t
           LEFT JOIN users u ON u.id = t.created_by
           LEFT JOIN agents a ON a.id = t.used_by_agent_id
           ORDER BY t.created_at DESC, t.id DESC"#,
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::Database)
}

/// Revokes a token that is still pending, returning whether it was.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn revoke_enrollment_token(pool: &PgPool, id: i64) -> Result<bool, ApiError> {
    let revoked = sqlx::query!(
        "UPDATE enrollment_tokens SET revoked_at = NOW() WHERE id = $1 AND used_at IS NULL AND \
         revoked_at IS NULL AND expires_at > NOW()",
        id,
    )
    .execute(pool)
    .await
    .map_err(ApiError::Database)?;
    Ok(revoked.rows_affected() > 0)
}

/// Whether `token_hash` names a token a host called `hostname` could use right
/// now. A cheap read, so the answer to a bad token costs the server almost
/// nothing: [`enroll_agent`] decides for good, inside its transaction.
///
/// # Errors
///
/// Returns [`ApiError::Database`] if the database query fails.
pub async fn enrollment_token_is_usable(
    pool: &PgPool,
    token_hash: &str,
    hostname: &str,
) -> Result<bool, ApiError> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM enrollment_tokens
               WHERE token_hash = $1 AND used_at IS NULL AND revoked_at IS NULL
                 AND expires_at > NOW() AND (hostname IS NULL OR lower(hostname) = lower($2))
           ) AS "usable!""#,
        token_hash,
        hostname,
    )
    .fetch_one(pool)
    .await
    .map_err(ApiError::Database)
}

/// Why a host could not be enrolled.
#[derive(Debug, thiserror::Error)]
pub enum EnrollError {
    /// The token is unknown, expired, revoked, already used, or restricted to
    /// another hostname. One variant for all of them, so the caller cannot tell
    /// which tokens exist.
    #[error("enrollment token refused")]
    Refused,
    /// An agent with that hostname and domain already exists.
    #[error("an agent with that hostname already exists")]
    AgentExists,
    /// The database query failed.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// The agent a successful enrollment created.
#[derive(Debug, Clone)]
pub struct EnrolledAgent {
    /// The new agent's ID.
    pub agent_id: i64,
    /// The token's ID, for the audit entry.
    pub token_id: i64,
}

/// Why inserting the agent failed: a taken hostname, or the database itself.
fn insert_error(error: sqlx::Error) -> EnrollError {
    if error
        .as_database_error()
        .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
    {
        EnrollError::AgentExists
    } else {
        EnrollError::Database(error)
    }
}

/// Trades the token with `token_hash` for a new agent called `hostname`.
///
/// Claiming the token and creating the agent happen in one transaction, so a
/// token is only ever spent on an agent that exists, and a failed registration
/// leaves it usable. The claim is a single conditional `UPDATE`: of two hosts
/// racing for one token, exactly one wins.
///
/// # Errors
///
/// Returns an error if:
/// - [`EnrollError::Refused`]: no pending token matches, or it is restricted to another hostname
/// - [`EnrollError::AgentExists`]: an agent with that hostname and domain exists
/// - [`EnrollError::Database`]: a query fails
pub async fn enroll_agent(
    pool: &PgPool,
    token_hash: &str,
    hostname: &str,
    domain: Option<&str>,
    agent_token_hash: &str,
) -> Result<EnrolledAgent, EnrollError> {
    let mut tx = pool.begin().await?;

    let claimed = sqlx::query!(
        "UPDATE enrollment_tokens SET used_at = NOW() WHERE token_hash = $1 AND used_at IS NULL \
         AND revoked_at IS NULL AND expires_at > NOW() AND (hostname IS NULL OR lower(hostname) = \
         lower($2)) RETURNING id, created_by, tag_ids",
        token_hash,
        hostname,
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(EnrollError::Refused)?;

    let agent_id = sqlx::query_scalar!(
        "INSERT INTO agents (hostname, agent_token_hash, owner_id, domain) VALUES ($1, $2, $3, \
         $4) RETURNING id",
        hostname,
        agent_token_hash,
        claimed.created_by,
        domain,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(insert_error)?;

    sqlx::query!(
        "INSERT INTO agent_tags (agent_id, tag_id) SELECT $1, id FROM tags WHERE id = ANY($2)",
        agent_id,
        &claimed.tag_ids,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE enrollment_tokens SET used_by_agent_id = $2 WHERE id = $1",
        claimed.id,
        agent_id,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(EnrolledAgent {
        agent_id,
        token_id: claimed.id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_that_is_not_a_taken_hostname_is_a_database_error() {
        assert!(matches!(
            insert_error(sqlx::Error::PoolTimedOut),
            EnrollError::Database(sqlx::Error::PoolTimedOut)
        ));
    }
}
