// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Test-only helpers shared across this crate's unit test modules.

/// Generates a fresh ed25519 key pair for use in SSH-related tests.
/// `russh::keys::PrivateKey` (used in `tunnel.rs`'s tests) pins its own
/// `ssh-key`/`rand_core` versions internally, distinct from this crate's
/// direct `ssh-key` dependency used here and in `ssh.rs`'s tests - callers
/// that need a `russh` key round-trip this through an OpenSSH PEM instead of
/// converting it directly.
pub(crate) fn generate_ed25519_key() -> ssh_key::PrivateKey {
    ssh_key::PrivateKey::random(&mut ssh_key::rand_core::OsRng, ssh_key::Algorithm::Ed25519)
        .expect("generate test key")
}
