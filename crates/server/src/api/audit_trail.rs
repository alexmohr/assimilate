// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Writing audit log entries from the handlers that change who can do what:
//! who acted, from where, on what, and the [`AuditEvent`] they caused.

use std::{convert::Infallible, net::IpAddr};

use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::request::Parts,
};
use shared::audit::AuditEvent;
use sqlx::PgPool;

use super::auth::AuthUser;
use crate::{
    AppState,
    db::audit::{NewAuditEntry, insert_audit_entry},
};

/// The caller's IP address, resolved through the trusted-proxy configuration.
///
/// `None` when the request carries no peer address, as when a router is
/// called directly rather than served from a socket.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClientIp(pub Option<IpAddr>);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = Infallible;

    fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(Ok(Self(
            parts
                .extensions
                .get::<ConnectInfo<std::net::SocketAddr>>()
                .map(|peer| {
                    state
                        .client_ip_resolver
                        .resolve(peer.0.ip(), &parts.headers)
                }),
        )))
    }
}

/// The resource an audited action was taken on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditTarget {
    /// A user account, by ID.
    User(i64),
    /// A group, by ID.
    Group(i64),
    /// A role, by ID.
    Role(i64),
    /// A repository, by ID.
    Repo(i64),
    /// An API token, by ID.
    ApiToken(i64),
    /// An agent, by ID.
    Agent(i64),
    /// A notification channel, by ID.
    NotificationChannel(i64),
    /// A notification rule, by ID.
    NotificationRule(i64),
}

impl AuditTarget {
    /// The `target_type` column value.
    #[must_use]
    pub const fn kind(self) -> &'static str {
        match self {
            Self::User(_) => "user",
            Self::Group(_) => "group",
            Self::Role(_) => "role",
            Self::Repo(_) => "repo",
            Self::ApiToken(_) => "api_token",
            Self::Agent(_) => "agent",
            Self::NotificationChannel(_) => "notification_channel",
            Self::NotificationRule(_) => "notification_rule",
        }
    }

    /// The `target_id` column value.
    #[must_use]
    pub const fn id(self) -> i64 {
        match self {
            Self::User(id)
            | Self::Group(id)
            | Self::Role(id)
            | Self::Repo(id)
            | Self::ApiToken(id)
            | Self::Agent(id)
            | Self::NotificationChannel(id)
            | Self::NotificationRule(id) => id,
        }
    }
}

/// Who performed an audited action, and from where.
#[derive(Debug, Clone, Copy)]
pub struct Actor<'a> {
    /// The acting user's ID.
    pub user_id: i64,
    /// The acting user's username.
    pub username: &'a str,
    /// The address the request came from, if known.
    pub ip: Option<IpAddr>,
}

impl<'a> Actor<'a> {
    /// The authenticated caller of a request, from `ip`.
    #[must_use]
    pub fn new(user: &'a AuthUser, ip: ClientIp) -> Self {
        Self {
            user_id: user.user_id,
            username: &user.username,
            ip: ip.0,
        }
    }
}

/// Records `event`, performed by `actor` on `target`.
///
/// The action it records has already happened by the time this runs, so a
/// failure to write the entry is logged rather than failing the request.
pub async fn record(
    pool: &PgPool,
    actor: Actor<'_>,
    target: Option<AuditTarget>,
    event: AuditEvent,
) {
    let ip = actor.ip.map(|ip| ip.to_string());
    let entry = NewAuditEntry {
        user_id: Some(actor.user_id),
        username: actor.username,
        event,
        target_type: target.map(AuditTarget::kind),
        target_id: target.map(AuditTarget::id),
        ip_address: ip.as_deref(),
    };
    if let Err(e) = insert_audit_entry(pool, &entry).await {
        tracing::warn!(user_id = actor.user_id, error = %e, "failed to write audit log entry");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_is_stored_as_its_kind_and_id() {
        assert_eq!(AuditTarget::User(3).kind(), "user");
        assert_eq!(AuditTarget::ApiToken(7).kind(), "api_token");
        assert_eq!(
            AuditTarget::NotificationChannel(2).kind(),
            "notification_channel"
        );
        assert_eq!(AuditTarget::NotificationRule(9).id(), 9);
        assert_eq!(AuditTarget::Repo(11).id(), 11);
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_recorded_entry_keeps_the_actor_target_and_address(pool: PgPool) {
        let user = crate::db::insert_user(&pool, "auditor", "hash")
            .await
            .unwrap();
        record(
            &pool,
            Actor {
                user_id: user.id,
                username: "auditor",
                ip: Some("10.0.0.7".parse().unwrap()),
            },
            Some(AuditTarget::Role(4)),
            AuditEvent::DeleteRole {
                name: "old-role".to_owned(),
            },
        )
        .await;

        let entries = crate::test_support::audit_entries(&pool).await;
        let [entry] = entries.as_slice() else {
            panic!("expected exactly one audit entry, got {entries:?}");
        };
        assert_eq!(entry.user_id, Some(user.id));
        assert_eq!(entry.username, "auditor");
        assert_eq!(
            entry.event,
            AuditEvent::DeleteRole {
                name: "old-role".to_owned()
            }
        );
        assert_eq!(entry.target_type.as_deref(), Some("role"));
        assert_eq!(entry.target_id, Some(4));
        assert_eq!(entry.ip_address.as_deref(), Some("10.0.0.7"));
    }

    /// Collects everything a `tracing` subscriber writes, for asserting on logs.
    #[derive(Clone, Default)]
    struct CapturedLogs(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for CapturedLogs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[ignore = "requires DATABASE_URL"]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_write_is_logged_instead_of_failing_the_action(pool: PgPool) {
        sqlx::query!("ALTER TABLE audit_log ADD CONSTRAINT reject_every_entry CHECK (false)")
            .execute(&pool)
            .await
            .unwrap();
        let logs = CapturedLogs::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer({
                let logs = logs.clone();
                move || logs.clone()
            })
            .with_ansi(false)
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);

        record(
            &pool,
            Actor {
                user_id: 42,
                username: "auditor",
                ip: None,
            },
            Some(AuditTarget::Role(4)),
            AuditEvent::DeleteRole {
                name: "old-role".to_owned(),
            },
        )
        .await;
        drop(guard);

        let logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
        assert!(
            logs.contains("WARN") && logs.contains("failed to write audit log entry"),
            "{logs}"
        );
        assert!(logs.contains("user_id=42"), "{logs}");
        assert!(logs.contains("reject_every_entry"), "{logs}");
        assert_eq!(
            crate::test_support::audit_events(&pool).await,
            Vec::<AuditEvent>::new()
        );
    }
}
