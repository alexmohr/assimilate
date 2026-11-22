// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! A host with no agent token trading a single-use enrollment token for one.
//! This runs inside the encrypted agent channel, so the exchange only happens
//! with a server holding the key the host pinned.

use axum::extract::ws::Message;
use futures_util::{Sink, SinkExt};
use shared::{audit::AuditEvent, enrollment::ENROLLMENT_REFUSED, protocol::ServerToAgent};

use crate::{
    AppState,
    api::{
        audit_trail::{self, AuditTarget},
        helpers::{self, MaxLen},
        tokens::hash_token,
    },
    db::enrollment_tokens::{EnrollError, enroll_agent, enrollment_token_is_usable},
};

/// What an agent sends to enroll.
pub(super) struct EnrollRequest {
    pub(super) hostname: String,
    pub(super) domain: Option<String>,
    pub(super) enrollment_token: String,
}

/// Why a request was turned down, and what the host is told.
enum Refusal {
    /// The token cannot be used. The same words for every cause.
    Token,
    /// The host's name or domain is not acceptable.
    Invalid(String),
    /// The agent already exists, or the server failed.
    Other(&'static str),
}

impl Refusal {
    fn reason(self) -> String {
        match self {
            Self::Token => ENROLLMENT_REFUSED.to_owned(),
            Self::Invalid(reason) => reason,
            Self::Other(reason) => reason.to_owned(),
        }
    }
}

fn validate(request: &EnrollRequest) -> Result<(), Refusal> {
    helpers::validate_non_empty(&request.hostname, "hostname")
        .and_then(|()| helpers::validate_max_len(&request.hostname, "hostname", MaxLen::Hostname))
        .and_then(|()| {
            helpers::validate_opt_max_len(request.domain.as_deref(), "domain", MaxLen::Hostname)
        })
        .map_err(|e| Refusal::Invalid(e.to_string()))
}

async fn send<Si>(ws_sink: &mut Si, message: &ServerToAgent)
where
    Si: Sink<Message> + Unpin,
    Si::Error: std::fmt::Display,
{
    let Ok(json) = serde_json::to_string(message) else {
        tracing::error!("failed to serialize the enrollment answer");
        return;
    };
    if let Err(e) = ws_sink.send(Message::Text(json.into())).await {
        tracing::debug!(error = %e, "ws send failed");
    }
}

/// Spends the token on a new agent and returns the agent's own token.
async fn enroll(state: &AppState, request: &EnrollRequest) -> Result<String, Refusal> {
    validate(request)?;

    // Before the bcrypt hash below, which costs real CPU: this endpoint answers
    // anyone who can reach the server, so a token that does not exist must not.
    let token_hash = hash_token(&request.enrollment_token);
    let usable = enrollment_token_is_usable(&state.pool, &token_hash, &request.hostname).await;
    if !usable.map_err(|_| Refusal::Other("enrollment failed"))? {
        return Err(Refusal::Token);
    }

    let agent_token = helpers::generate_random_hex(32);
    let to_hash = agent_token.clone();
    let agent_token_hash =
        tokio::task::spawn_blocking(move || bcrypt::hash(to_hash, bcrypt::DEFAULT_COST))
            .await
            .map_err(|_| Refusal::Other("enrollment failed"))?
            .map_err(|_| Refusal::Other("enrollment failed"))?;

    let enrolled = enroll_agent(
        &state.pool,
        &token_hash,
        &request.hostname,
        request.domain.as_deref(),
        &agent_token_hash,
    )
    .await
    .map_err(|e| match e {
        EnrollError::Refused => Refusal::Token,
        EnrollError::AgentExists => {
            Refusal::Other("an agent with this hostname already exists on the server")
        }
        EnrollError::Database(error) => {
            tracing::error!(error = %error, "enrollment failed");
            Refusal::Other("enrollment failed")
        }
    })?;

    audit_trail::record_system(
        &state.pool,
        Some(AuditTarget::EnrollmentToken(enrolled.token_id)),
        AuditEvent::UseEnrollmentToken {
            hostname: request.hostname.clone(),
            domain: request.domain.clone(),
        },
    )
    .await;
    tracing::info!(hostname = %request.hostname, "agent enrolled");
    Ok(agent_token)
}

/// Answers an enrollment request. The connection ends after this: the agent
/// reconnects with its new token and a `Hello`.
pub(super) async fn handle_enroll<Si>(ws_sink: &mut Si, state: &AppState, request: EnrollRequest)
where
    Si: Sink<Message> + Unpin,
    Si::Error: std::fmt::Display,
{
    let answer = match enroll(state, &request).await {
        Ok(token) => ServerToAgent::Enrolled { token },
        Err(refusal) => {
            tracing::warn!(hostname = %request.hostname, "enrollment refused");
            ServerToAgent::EnrollRefused {
                reason: refusal.reason(),
            }
        }
    };
    send(ws_sink, &answer).await;
}
