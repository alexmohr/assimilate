// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Which SSH logins one relay connection may have the server's key sign.
//!
//! Every agent machine borrows the same key, and every repository host
//! trusts it, so a signature is good for logging in to any of them. The
//! relay therefore only signs a login once the connection has proven, with
//! a verified `session-bind`, which SSH server it is logged in to, and only
//! when that server is a repository host this agent backs up to and the
//! login is as a user one of those repositories uses.

use russh::keys::ssh_key::{PublicKey, public::KeyData};

use super::{
    session_bind::{BindError, SessionBind},
    sign_request::{Login, SignError},
    wire::Reader,
};
use crate::db::RelayDestinationRow;

/// The most binds one connection may hold, OpenSSH's own limit
/// (`AGENT_MAX_SESSION_IDS`): one per hop of a forwarded agent.
const MAX_BINDS: usize = 16;

/// A repository host this agent may log in to, as one of its users.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Destination {
    host_key: KeyData,
    user: String,
}

/// The repository hosts, and users on them, one agent's repositories use.
#[derive(Debug, Default)]
pub struct RelayScope {
    destinations: Vec<Destination>,
}

impl RelayScope {
    /// The destinations among `rows` whose pinned host key can be read.
    /// One that cannot is left out, so logins to it are refused.
    pub fn new(rows: Vec<RelayDestinationRow>, hostname: &str) -> Self {
        let destinations = rows
            .into_iter()
            .filter_map(|row| match PublicKey::from_openssh(&row.ssh_host_key) {
                Ok(key) => Some(Destination {
                    host_key: key.key_data().clone(),
                    user: row.ssh_user,
                }),
                Err(e) => {
                    tracing::warn!(
                        hostname = %hostname,
                        error = %e,
                        "ssh relay: a repository host's pinned host key cannot be read; \
                         logins to it are refused"
                    );
                    None
                }
            })
            .collect();
        Self { destinations }
    }

    fn allows_host(&self, host_key: &KeyData) -> bool {
        self.destinations.iter().any(|d| d.host_key == *host_key)
    }

    fn allows_login(&self, host_key: &KeyData, user: &[u8]) -> bool {
        self.destinations
            .iter()
            .any(|d| d.host_key == *host_key && d.user.as_bytes() == user)
    }
}

/// Why the relay refused a `session-bind` or a sign request. None of them
/// carries key material, so each can be logged as it is.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Refusal {
    #[error(transparent)]
    Bind(#[from] BindError),
    /// The bound host key is not pinned for any repository host this agent
    /// backs up to.
    #[error("session-bind names a host that is not one of this agent's repository hosts")]
    HostNotAllowed,
    /// A bind after one that was not for forwarding: `ssh` binds a
    /// connection once, and only a forwarded one carries the next hop's.
    #[error("session-bind follows a bind that was not for agent forwarding")]
    BindAfterNonForwarding,
    #[error("session-bind exceeds the {MAX_BINDS} binds one connection may hold")]
    TooManyBinds,
    #[error(transparent)]
    Sign(#[from] SignError),
    /// A sign request on a connection with no accepted bind, such as one
    /// from an OpenSSH client older than 8.9.
    #[error("sign request on a connection without an accepted session-bind")]
    NotBound,
    #[error("sign request is for another SSH session than the bound one")]
    SessionMismatch,
    #[error("host-bound sign request names another host key than the bound one")]
    HostKeyMismatch,
    /// The login is as a user none of this agent's repositories on the
    /// bound host uses.
    #[error("sign request logs in as a user no repository of this agent uses on the bound host")]
    UserNotAllowed,
}

/// One relay connection's scope and the binds it has proven.
#[derive(Debug)]
pub struct Session {
    scope: RelayScope,
    binds: Vec<SessionBind>,
}

impl Session {
    pub fn new(scope: RelayScope) -> Self {
        Self {
            scope,
            binds: Vec::new(),
        }
    }

    /// Accepts the `session-bind` whose fields after its name are `fields`.
    ///
    /// # Errors
    ///
    /// Refuses a bind that does not verify, names a host outside the scope,
    /// or cannot follow the binds already held.
    pub fn bind(&mut self, fields: Reader<'_>) -> Result<(), Refusal> {
        let bind = SessionBind::verify(fields)?;
        if !self.scope.allows_host(&bind.host_key) {
            return Err(Refusal::HostNotAllowed);
        }
        match self.binds.last() {
            // `ssh` may repeat the bind it already made.
            Some(last) if *last == bind => return Ok(()),
            Some(last) if !last.forwarding => return Err(Refusal::BindAfterNonForwarding),
            Some(_) | None => {}
        }
        if self.binds.len() >= MAX_BINDS {
            return Err(Refusal::TooManyBinds);
        }
        self.binds.push(bind);
        Ok(())
    }

    /// Checks that the sign request whose body (the bytes after the message
    /// type) is `body` signs a login the latest bind allows.
    ///
    /// # Errors
    ///
    /// Refuses a request that is not a login, comes before any bind, or
    /// does not match the latest bind's session, host and allowed users.
    pub fn authorize_sign(&self, body: &[u8]) -> Result<(), Refusal> {
        let login = Login::parse(body)?;
        let bind = self.binds.last().ok_or(Refusal::NotBound)?;
        if login.session_id != bind.session_id.as_slice() {
            return Err(Refusal::SessionMismatch);
        }
        if login
            .server_host_key
            .is_some_and(|host_key| host_key != bind.host_key)
        {
            return Err(Refusal::HostKeyMismatch);
        }
        if !self.scope.allows_login(&bind.host_key, login.user) {
            return Err(Refusal::UserNotAllowed);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::{
            session_bind::Extension,
            test_messages::{HostKey, LoginRequest, session_bind_body},
        },
        *,
    };

    fn row(host: &HostKey, user: &str) -> RelayDestinationRow {
        RelayDestinationRow {
            ssh_host_key: host.openssh(),
            ssh_user: user.to_owned(),
        }
    }

    fn session(rows: Vec<RelayDestinationRow>) -> Session {
        Session::new(RelayScope::new(rows, "scope-test-host"))
    }

    fn bind(
        session: &mut Session,
        host: &HostKey,
        id: &[u8],
        forwarding: bool,
    ) -> Result<(), Refusal> {
        let body = session_bind_body(host, id, forwarding);
        session.bind(Extension::parse(&body).unwrap().fields)
    }

    fn sign(session: &Session, request: LoginRequest) -> Result<(), Refusal> {
        let request = request.encode();
        session.authorize_sign(request.get(5..).unwrap())
    }

    #[test]
    fn a_login_to_a_bound_allowed_host_as_its_user_is_signed() {
        let host = HostKey::generate();
        let mut session = session(vec![row(&host, "borg")]);

        bind(&mut session, &host, b"session-1", false).unwrap();

        assert_eq!(
            sign(&session, LoginRequest::new(b"session-1", b"borg")),
            Ok(())
        );
        assert_eq!(
            sign(
                &session,
                LoginRequest {
                    host_bound: Some(host.public_blob()),
                    ..LoginRequest::new(b"session-1", b"borg")
                }
            ),
            Ok(())
        );
    }

    #[test]
    fn a_sign_request_before_any_bind_is_refused() {
        let host = HostKey::generate();
        let session = session(vec![row(&host, "borg")]);

        assert_eq!(
            sign(&session, LoginRequest::new(b"session-1", b"borg")),
            Err(Refusal::NotBound)
        );
    }

    #[test]
    fn a_bind_to_a_host_outside_the_scope_is_refused() {
        let allowed = HostKey::generate();
        let other = HostKey::generate();
        let mut session = session(vec![row(&allowed, "borg")]);

        assert_eq!(
            bind(&mut session, &other, b"session-1", false),
            Err(Refusal::HostNotAllowed)
        );
        assert_eq!(
            sign(&session, LoginRequest::new(b"session-1", b"borg")),
            Err(Refusal::NotBound)
        );
    }

    #[test]
    fn a_bind_that_does_not_verify_is_refused() {
        let host = HostKey::generate();
        let mut session = session(vec![row(&host, "borg")]);
        let mut body = session_bind_body(&host, b"session-1", false);
        // The forwarding flag is the last byte, so the one before it is the
        // signature's.
        let at = body.len().saturating_sub(2);
        *body.get_mut(at).unwrap() ^= 1;

        assert_eq!(
            session.bind(Extension::parse(&body).unwrap().fields),
            Err(Refusal::Bind(BindError::BadSignature))
        );
    }

    #[test]
    fn a_login_for_another_session_or_host_is_refused() {
        let host = HostKey::generate();
        let other = HostKey::generate();
        let mut session = session(vec![row(&host, "borg"), row(&other, "borg")]);
        bind(&mut session, &host, b"session-1", false).unwrap();

        assert_eq!(
            sign(&session, LoginRequest::new(b"session-2", b"borg")),
            Err(Refusal::SessionMismatch)
        );
        assert_eq!(
            sign(
                &session,
                LoginRequest {
                    host_bound: Some(other.public_blob()),
                    ..LoginRequest::new(b"session-1", b"borg")
                }
            ),
            Err(Refusal::HostKeyMismatch)
        );
    }

    /// The user is checked against the bound host's own repositories: a
    /// user another allowed host has is not good on this one.
    #[test]
    fn a_login_as_a_user_the_bound_host_has_no_repository_for_is_refused() {
        let host = HostKey::generate();
        let other = HostKey::generate();
        let mut session = session(vec![row(&host, "borg"), row(&other, "root")]);
        bind(&mut session, &host, b"session-1", false).unwrap();

        assert_eq!(
            sign(&session, LoginRequest::new(b"session-1", b"root")),
            Err(Refusal::UserNotAllowed)
        );
    }

    #[test]
    fn a_sign_request_that_is_not_a_login_is_refused() {
        let host = HostKey::generate();
        let mut session = session(vec![row(&host, "borg")]);
        bind(&mut session, &host, b"session-1", false).unwrap();

        assert!(matches!(
            sign(
                &session,
                LoginRequest {
                    method: b"password".to_vec(),
                    ..LoginRequest::new(b"session-1", b"borg")
                }
            ),
            Err(Refusal::Sign(SignError::Method))
        ));
    }

    #[test]
    fn a_connection_binds_once_unless_it_forwards() {
        let host = HostKey::generate();
        let next = HostKey::generate();
        let mut session = session(vec![row(&host, "borg"), row(&next, "borg")]);
        bind(&mut session, &host, b"session-1", false).unwrap();

        assert_eq!(bind(&mut session, &host, b"session-1", false), Ok(()));
        assert_eq!(
            bind(&mut session, &next, b"session-2", false),
            Err(Refusal::BindAfterNonForwarding)
        );
        assert_eq!(
            sign(&session, LoginRequest::new(b"session-1", b"borg")),
            Ok(())
        );
    }

    /// A forwarded connection carries the next hop's bind after its own,
    /// and only the latest one's session may be signed for.
    #[test]
    fn a_forwarded_connection_signs_for_its_latest_hop() {
        let host = HostKey::generate();
        let next = HostKey::generate();
        let mut session = session(vec![row(&host, "borg"), row(&next, "borg")]);
        bind(&mut session, &host, b"session-1", true).unwrap();
        bind(&mut session, &next, b"session-2", false).unwrap();

        assert_eq!(
            sign(&session, LoginRequest::new(b"session-2", b"borg")),
            Ok(())
        );
        assert_eq!(
            sign(&session, LoginRequest::new(b"session-1", b"borg")),
            Err(Refusal::SessionMismatch)
        );
    }

    #[test]
    fn a_connection_holds_at_most_sixteen_binds() {
        let host = HostKey::generate();
        let mut session = session(vec![row(&host, "borg")]);
        for hop in 0..MAX_BINDS {
            bind(
                &mut session,
                &host,
                format!("session-{hop}").as_bytes(),
                true,
            )
            .unwrap();
        }

        assert_eq!(
            bind(&mut session, &host, b"one-too-many", true),
            Err(Refusal::TooManyBinds)
        );
    }

    /// A repository whose pinned key cannot be read, such as the
    /// placeholder some tests pin, leaves only its host out of the scope.
    #[test]
    fn a_pinned_key_that_cannot_be_read_is_left_out() {
        let host = HostKey::generate();
        let mut session = session(vec![
            RelayDestinationRow {
                ssh_host_key: "ssh-ed25519 AAAATEST".to_owned(),
                ssh_user: "borg".to_owned(),
            },
            row(&host, "borg"),
        ]);

        assert_eq!(bind(&mut session, &host, b"session-1", false), Ok(()));
    }

    #[test]
    fn an_empty_scope_refuses_every_bind() {
        let host = HostKey::generate();
        let mut session = session(Vec::new());

        assert_eq!(
            bind(&mut session, &host, b"session-1", false),
            Err(Refusal::HostNotAllowed)
        );
    }
}
