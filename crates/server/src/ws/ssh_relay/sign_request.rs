// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! `SSH_AGENTC_SIGN_REQUEST`, read far enough to tell which SSH login it
//! signs.
//!
//! ```text
//! byte    SSH_AGENTC_SIGN_REQUEST
//! string  key blob
//! string  data
//! uint32  flags
//! ```
//!
//! For an SSH login `data` is the public key userauth request (RFC 4252
//! section 7), or OpenSSH's host-bound variant of it, which also names the
//! server's host key:
//!
//! ```text
//! string  session identifier
//! byte    SSH_MSG_USERAUTH_REQUEST
//! string  user name
//! string  "ssh-connection"
//! string  "publickey" | "publickey-hostbound-v00@openssh.com"
//! bool    TRUE
//! string  public key algorithm name
//! string  public key blob
//! string  server host key            (host-bound only)
//! ```
//!
//! Anything else the server's key could sign, an `SSHSIG` file signature
//! among them, is not a login and is refused.

use russh::keys::ssh_key::{PublicKey, public::KeyData};

use super::wire::{Reader, WireError};

/// `SSH_MSG_USERAUTH_REQUEST`.
const SSH_MSG_USERAUTH_REQUEST: u8 = 50;

/// The SSH service a userauth request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Service {
    /// `ssh-connection`, the only service `ssh` logs in to.
    Connection,
    Other,
}

impl From<&[u8]> for Service {
    fn from(name: &[u8]) -> Self {
        if name == b"ssh-connection" {
            Self::Connection
        } else {
            Self::Other
        }
    }
}

/// The userauth method a request uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    /// `publickey`.
    PublicKey,
    /// `publickey-hostbound-v00@openssh.com`, which OpenSSH 8.9 and later
    /// use against servers that offer it.
    PublicKeyHostBound,
    Other,
}

impl From<&[u8]> for Method {
    fn from(name: &[u8]) -> Self {
        if name == b"publickey" {
            Self::PublicKey
        } else if name == b"publickey-hostbound-v00@openssh.com" {
            Self::PublicKeyHostBound
        } else {
            Self::Other
        }
    }
}

/// Why a sign request is not an SSH login the relay can check. None of
/// them carries key material, so each can be logged as it is.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SignError {
    /// The request's own fields do not decode.
    #[error("malformed sign request: {0}")]
    Malformed(WireError),
    /// The data to sign is not a userauth request.
    #[error("the data to sign is not an SSH login request: {0}")]
    NotUserauth(WireError),
    /// The data starts like a login, but is some other SSH message.
    #[error("the data to sign is not an SSH userauth request")]
    NotUserauthMessage,
    /// The login is for a service other than `ssh-connection`.
    #[error("the SSH login is for a service other than ssh-connection")]
    Service,
    /// The login uses a method other than public key authentication.
    #[error("the SSH login uses a method other than public key authentication")]
    Method,
    /// The request asks for a signature over a request that says it has
    /// none.
    #[error("the SSH login request does not expect a signature")]
    NoSignature,
    /// The key named in the login is not the one asked to sign.
    #[error("the SSH login names a different key than the one asked to sign")]
    KeyMismatch,
    /// The host-bound login's server host key cannot be read.
    #[error("the host-bound SSH login names an unreadable server host key")]
    HostKey,
}

/// A sign request for an SSH login: whose, and on which session.
#[derive(Debug, PartialEq, Eq)]
pub struct Login<'a> {
    /// The SSH session the login happens in.
    pub session_id: &'a [u8],
    /// The user it logs in as.
    pub user: &'a [u8],
    /// The server host key a host-bound login names.
    pub server_host_key: Option<KeyData>,
}

impl<'a> Login<'a> {
    /// Reads the login that the sign request whose body (the bytes after
    /// the message type) is `body` asks to sign.
    ///
    /// # Errors
    ///
    /// Fails if the request is malformed or what it asks to sign is not a
    /// public key login with the key it names.
    pub fn parse(body: &'a [u8]) -> Result<Self, SignError> {
        let mut request = Reader::new(body);
        let key_blob = request.string().map_err(SignError::Malformed)?;
        let data = request.string().map_err(SignError::Malformed)?;
        request.u32().map_err(SignError::Malformed)?;
        request.finish().map_err(SignError::Malformed)?;

        let mut data = Reader::new(data);
        let session_id = data.string().map_err(SignError::NotUserauth)?;
        if data.byte().map_err(SignError::NotUserauth)? != SSH_MSG_USERAUTH_REQUEST {
            return Err(SignError::NotUserauthMessage);
        }
        let user = data.string().map_err(SignError::NotUserauth)?;
        match Service::from(data.string().map_err(SignError::NotUserauth)?) {
            Service::Connection => {}
            Service::Other => return Err(SignError::Service),
        }
        let method = Method::from(data.string().map_err(SignError::NotUserauth)?);
        if !data.boolean().map_err(SignError::NotUserauth)? {
            return Err(SignError::NoSignature);
        }
        data.string().map_err(SignError::NotUserauth)?;
        if data.string().map_err(SignError::NotUserauth)? != key_blob {
            return Err(SignError::KeyMismatch);
        }
        let server_host_key = match method {
            Method::PublicKey => None,
            Method::PublicKeyHostBound => {
                let host_key = data.string().map_err(SignError::NotUserauth)?;
                let host_key = PublicKey::from_bytes(host_key).map_err(|_| SignError::HostKey)?;
                Some(host_key.key_data().clone())
            }
            Method::Other => return Err(SignError::Method),
        };
        data.finish().map_err(SignError::NotUserauth)?;
        Ok(Self {
            session_id,
            user,
            server_host_key,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::test_messages::{HostKey, LoginRequest, ssh_string},
        *,
    };

    /// `request`'s body, the bytes after the message type.
    fn body(request: &[u8]) -> &[u8] {
        request.get(5..).unwrap()
    }

    #[test]
    fn a_public_key_login_is_read() {
        let request = LoginRequest::new(b"session-1", b"borg").encode();

        assert_eq!(
            Login::parse(body(&request)),
            Ok(Login {
                session_id: b"session-1",
                user: b"borg",
                server_host_key: None,
            })
        );
    }

    #[test]
    fn a_host_bound_login_names_the_server_host_key() {
        let host = HostKey::generate();
        let request = LoginRequest {
            host_bound: Some(host.public_blob()),
            ..LoginRequest::new(b"session-1", b"borg")
        }
        .encode();

        let login = Login::parse(body(&request)).unwrap();

        assert_eq!(login.server_host_key, Some(host.key_data()));
    }

    /// An `SSHSIG` file signature, which `ssh-keygen -Y sign` asks the
    /// agent for, is not a login at all.
    #[test]
    fn a_file_signature_is_not_a_login() {
        let request = LoginRequest {
            data: Some(b"SSHSIG\0\0\0\x04file".to_vec()),
            ..LoginRequest::new(b"session-1", b"borg")
        }
        .encode();

        assert!(matches!(
            Login::parse(body(&request)),
            Err(SignError::NotUserauth(_))
        ));
    }

    #[test]
    fn logins_the_relay_cannot_check_are_refused() {
        let base = || LoginRequest::new(b"session-1", b"borg");
        let cases = [
            (
                LoginRequest {
                    message_type: 51,
                    ..base()
                },
                SignError::NotUserauthMessage,
            ),
            (
                LoginRequest {
                    service: b"ssh-userauth".to_vec(),
                    ..base()
                },
                SignError::Service,
            ),
            (
                LoginRequest {
                    method: b"hostbased".to_vec(),
                    ..base()
                },
                SignError::Method,
            ),
            (
                LoginRequest {
                    has_signature: false,
                    ..base()
                },
                SignError::NoSignature,
            ),
            (
                LoginRequest {
                    login_key_blob: Some(b"another key".to_vec()),
                    ..base()
                },
                SignError::KeyMismatch,
            ),
            (
                LoginRequest {
                    host_bound: Some(b"not a key".to_vec()),
                    ..base()
                },
                SignError::HostKey,
            ),
        ];

        for (request, refusal) in cases {
            assert_eq!(Login::parse(body(&request.encode())), Err(refusal));
        }
    }

    #[test]
    fn a_malformed_sign_request_is_refused() {
        let request = LoginRequest::new(b"session-1", b"borg").encode();
        let trailing = [body(&request), &[0]].concat();
        let no_flags = body(&request)
            .get(..body(&request).len().saturating_sub(4))
            .unwrap();
        let truncated_login = [
            ssh_string(b"key"),
            ssh_string(&[ssh_string(b"session-1"), vec![50]].concat()),
            vec![0, 0, 0, 0],
        ]
        .concat();

        assert_eq!(
            Login::parse(&trailing),
            Err(SignError::Malformed(WireError::TrailingBytes))
        );
        assert_eq!(
            Login::parse(no_flags),
            Err(SignError::Malformed(WireError::Truncated))
        );
        assert_eq!(
            Login::parse(&truncated_login),
            Err(SignError::NotUserauth(WireError::Truncated))
        );
    }
}
