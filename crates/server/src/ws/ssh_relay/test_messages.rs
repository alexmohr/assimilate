// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Builders for the `session-bind` and sign request messages the relay's
//! tests send, encoded as OpenSSH's `ssh` encodes them.

use russh::keys::{
    PrivateKey,
    signature::Signer as _,
    ssh_encoding::Encode as _,
    ssh_key::{Signature, public::KeyData},
};

use super::agent_protocol::tests::message;

/// `SSH_AGENTC_SIGN_REQUEST`.
const SIGN_REQUEST: u8 = 13;
/// `SSH_AGENTC_EXTENSION`.
const EXTENSION: u8 = 27;

/// The key blob of the client key the test logins sign with.
pub const CLIENT_KEY_BLOB: &[u8] = b"client-key-blob";

/// `bytes` as an SSH `string`, length prefix included.
pub fn ssh_string(bytes: &[u8]) -> Vec<u8> {
    let len = u32::try_from(bytes.len()).unwrap();
    [len.to_be_bytes().as_slice(), bytes].concat()
}

/// A repository host's SSH host key pair.
pub struct HostKey(PrivateKey);

impl HostKey {
    pub fn generate() -> Self {
        Self(crate::test_support::generate_test_key())
    }

    /// The public key as `known_hosts` and the database pin it.
    pub fn openssh(&self) -> String {
        self.0.public_key().to_openssh().unwrap()
    }

    /// The public key blob, as SSH sends it.
    pub fn public_blob(&self) -> Vec<u8> {
        self.0.public_key().to_bytes().unwrap()
    }

    pub fn key_data(&self) -> KeyData {
        self.0.public_key().key_data().clone()
    }

    /// The host's signature over `data`, as SSH encodes it.
    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        let signature: Signature = self.0.try_sign(data).unwrap();
        signature.encode_vec().unwrap()
    }
}

/// A `session-bind@openssh.com` request's body, the bytes after the message
/// type, binding `session_id` to `host`.
pub fn session_bind_body(host: &HostKey, session_id: &[u8], forwarding: bool) -> Vec<u8> {
    [
        ssh_string(b"session-bind@openssh.com"),
        ssh_string(&host.public_blob()),
        ssh_string(session_id),
        ssh_string(&host.sign(session_id)),
        vec![u8::from(forwarding)],
    ]
    .concat()
}

/// A whole `session-bind@openssh.com` message, length prefix included.
pub fn session_bind(host: &HostKey, session_id: &[u8], forwarding: bool) -> Vec<u8> {
    message(EXTENSION, &session_bind_body(host, session_id, forwarding))
}

/// A sign request for a public key login, each field of which a test may
/// change.
pub struct LoginRequest {
    pub session_id: Vec<u8>,
    pub message_type: u8,
    pub user: Vec<u8>,
    pub service: Vec<u8>,
    pub method: Vec<u8>,
    pub has_signature: bool,
    /// The key the login names, when it differs from the one asked to sign.
    pub login_key_blob: Option<Vec<u8>>,
    /// The server host key of a host-bound login, which also makes the
    /// method the host-bound one.
    pub host_bound: Option<Vec<u8>>,
    /// Data to sign in place of a login.
    pub data: Option<Vec<u8>>,
}

impl LoginRequest {
    pub fn new(session_id: &[u8], user: &[u8]) -> Self {
        Self {
            session_id: session_id.to_vec(),
            message_type: 50,
            user: user.to_vec(),
            service: b"ssh-connection".to_vec(),
            method: b"publickey".to_vec(),
            has_signature: true,
            login_key_blob: None,
            host_bound: None,
            data: None,
        }
    }

    /// The whole sign request, length prefix included.
    pub fn encode(self) -> Vec<u8> {
        let method = if self.host_bound.is_some() {
            b"publickey-hostbound-v00@openssh.com".to_vec()
        } else {
            self.method
        };
        let login = [
            ssh_string(&self.session_id),
            vec![self.message_type],
            ssh_string(&self.user),
            ssh_string(&self.service),
            ssh_string(&method),
            vec![u8::from(self.has_signature)],
            ssh_string(b"ssh-ed25519"),
            ssh_string(self.login_key_blob.as_deref().unwrap_or(CLIENT_KEY_BLOB)),
            self.host_bound.as_deref().map_or_else(Vec::new, ssh_string),
        ]
        .concat();
        let data = self.data.unwrap_or(login);
        message(
            SIGN_REQUEST,
            &[
                ssh_string(CLIENT_KEY_BLOB),
                ssh_string(&data),
                vec![0, 0, 0, 0],
            ]
            .concat(),
        )
    }
}
