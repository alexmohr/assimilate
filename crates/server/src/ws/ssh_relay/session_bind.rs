// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! `SSH_AGENTC_EXTENSION` requests, and the one the relay acts on:
//! OpenSSH's `session-bind@openssh.com` (PROTOCOL.agent), with which `ssh`
//! tells its agent which SSH server a connection is logging in to.
//!
//! ```text
//! byte    SSH_AGENTC_EXTENSION
//! string  "session-bind@openssh.com"
//! string  hostkey
//! string  session identifier
//! string  signature
//! bool    is_forwarding
//! ```
//!
//! The signature is the SSH server's, made with `hostkey` over the session
//! identifier during key exchange, so only a connection that really reached
//! the server holding that host key can produce it. The relay checks it
//! itself; a bind it cannot verify proves nothing about where a login goes.

use russh::keys::{
    signature::Verifier as _,
    ssh_key::{PublicKey, Signature, public::KeyData},
};

use super::wire::{Reader, WireError};

/// The extensions the relay tells apart, by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionName {
    /// `session-bind@openssh.com`.
    SessionBind,
    /// Any other extension, which the relay never forwards.
    Other,
}

impl From<&[u8]> for ExtensionName {
    fn from(name: &[u8]) -> Self {
        if name == b"session-bind@openssh.com" {
            Self::SessionBind
        } else {
            Self::Other
        }
    }
}

/// An extension request: which one, and the fields after its name.
#[derive(Debug)]
pub struct Extension<'a> {
    pub name: ExtensionName,
    pub fields: Reader<'a>,
}

impl<'a> Extension<'a> {
    /// Reads the name of the extension request whose body (the bytes after
    /// the message type) is `body`.
    ///
    /// # Errors
    ///
    /// Fails if the body does not start with a name.
    pub fn parse(body: &'a [u8]) -> Result<Self, WireError> {
        let mut fields = Reader::new(body);
        let name = ExtensionName::from(fields.string()?);
        Ok(Self { name, fields })
    }
}

/// Why a `session-bind` was not accepted as proof of the server a
/// connection logs in to. None of them carries key material, so each can
/// be logged as it is.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BindError {
    /// The fields do not decode.
    #[error("malformed session-bind: {0}")]
    Malformed(#[from] WireError),
    /// The host key is not one the relay can read.
    #[error("session-bind names a host key of an unsupported or unreadable type")]
    HostKey,
    /// The signature is not one the relay can read.
    #[error("session-bind carries an unreadable signature")]
    Signature,
    /// The signature is not the host key's over the session identifier.
    #[error("session-bind signature does not verify against its host key")]
    BadSignature,
}

/// A verified `session-bind`: the connection is logged in to the SSH server
/// holding `host_key`, in the SSH session `session_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBind {
    pub host_key: KeyData,
    pub session_id: Vec<u8>,
    /// Whether `ssh` is forwarding this agent connection on to that server,
    /// so that further binds for the next hops may follow on it.
    pub forwarding: bool,
}

impl SessionBind {
    /// Decodes a `session-bind`'s fields and checks the host's signature.
    ///
    /// # Errors
    ///
    /// Fails if the fields do not decode, the host key or signature cannot
    /// be read, or the signature does not verify.
    pub fn verify(mut fields: Reader<'_>) -> Result<Self, BindError> {
        let host_key = fields.string()?;
        let session_id = fields.string()?;
        let signature = fields.string()?;
        let forwarding = fields.boolean()?;
        fields.finish()?;

        let host_key = PublicKey::from_bytes(host_key).map_err(|_| BindError::HostKey)?;
        let host_key = host_key.key_data();
        let signature = Signature::try_from(signature).map_err(|_| BindError::Signature)?;
        host_key
            .verify(session_id, &signature)
            .map_err(|_| BindError::BadSignature)?;
        Ok(Self {
            host_key: host_key.clone(),
            session_id: session_id.to_vec(),
            forwarding,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::test_messages::{HostKey, session_bind_body, ssh_string},
        *,
    };

    /// `body` with its extension name read off, as the relay hands it on.
    fn bind_fields(body: &[u8]) -> Reader<'_> {
        let extension = Extension::parse(body).unwrap();
        assert_eq!(extension.name, ExtensionName::SessionBind);
        extension.fields
    }

    #[test]
    fn extensions_are_told_apart_by_name() {
        let other = ssh_string(b"query");

        assert_eq!(Extension::parse(&other).unwrap().name, ExtensionName::Other);
        assert_eq!(
            Extension::parse(&ssh_string(b"session-bind@openssh.com"))
                .unwrap()
                .name,
            ExtensionName::SessionBind
        );
        assert_eq!(
            Extension::parse(&[0, 0, 0, 9]).unwrap_err(),
            WireError::Truncated
        );
    }

    #[test]
    fn a_bind_signed_by_its_host_key_is_verified() {
        let host = HostKey::generate();
        let body = session_bind_body(&host, b"session-1", false);

        let bind = SessionBind::verify(bind_fields(&body)).unwrap();

        assert_eq!(bind.host_key, host.key_data());
        assert_eq!(bind.session_id, b"session-1");
        assert!(!bind.forwarding);
    }

    #[test]
    fn the_forwarding_flag_is_read() {
        let host = HostKey::generate();
        let body = session_bind_body(&host, b"session-1", true);

        assert!(SessionBind::verify(bind_fields(&body)).unwrap().forwarding);
    }

    /// A bind naming one host key but signed by another, the shape a client
    /// claiming an allowed host for a session with some other host has.
    #[test]
    fn a_bind_signed_by_another_key_is_refused() {
        let claimed = HostKey::generate();
        let signer = HostKey::generate();
        let body = [
            ssh_string(b"session-bind@openssh.com"),
            ssh_string(&claimed.public_blob()),
            ssh_string(b"session-1"),
            ssh_string(&signer.sign(b"session-1")),
            vec![0],
        ]
        .concat();

        assert_eq!(
            SessionBind::verify(bind_fields(&body)),
            Err(BindError::BadSignature)
        );
    }

    #[test]
    fn a_bind_whose_signature_covers_another_session_is_refused() {
        let host = HostKey::generate();
        let body = [
            ssh_string(b"session-bind@openssh.com"),
            ssh_string(&host.public_blob()),
            ssh_string(b"session-2"),
            ssh_string(&host.sign(b"session-1")),
            vec![0],
        ]
        .concat();

        assert_eq!(
            SessionBind::verify(bind_fields(&body)),
            Err(BindError::BadSignature)
        );
    }

    #[test]
    fn a_malformed_bind_is_refused() {
        let host = HostKey::generate();
        let body = session_bind_body(&host, b"session-1", false);
        let truncated = body.get(..body.len().saturating_sub(1)).unwrap();
        let trailing = [body.as_slice(), &[0]].concat();
        let name_only = ssh_string(b"session-bind@openssh.com");

        assert_eq!(
            SessionBind::verify(bind_fields(truncated)),
            Err(BindError::Malformed(WireError::Truncated))
        );
        assert_eq!(
            SessionBind::verify(bind_fields(&trailing)),
            Err(BindError::Malformed(WireError::TrailingBytes))
        );
        assert_eq!(
            SessionBind::verify(bind_fields(&name_only)),
            Err(BindError::Malformed(WireError::Truncated))
        );
    }

    #[test]
    fn an_unreadable_host_key_or_signature_is_refused() {
        let host = HostKey::generate();
        let bad_key = [
            ssh_string(b"session-bind@openssh.com"),
            ssh_string(b"not a key"),
            ssh_string(b"session-1"),
            ssh_string(&host.sign(b"session-1")),
            vec![0],
        ]
        .concat();
        let bad_signature = [
            ssh_string(b"session-bind@openssh.com"),
            ssh_string(&host.public_blob()),
            ssh_string(b"session-1"),
            ssh_string(b"not a signature"),
            vec![0],
        ]
        .concat();

        assert_eq!(
            SessionBind::verify(bind_fields(&bad_key)),
            Err(BindError::HostKey)
        );
        assert_eq!(
            SessionBind::verify(bind_fields(&bad_signature)),
            Err(BindError::Signature)
        );
    }
}
