// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Domain types, WebSocket protocol schema, and crypto utilities shared
//! between the `server` and `agent` crates.

/// Encryption and decryption helpers used to protect secrets (e.g. borg
/// repository passphrases) at rest.
pub mod crypto;
/// Message types exchanged over the agent/server WebSocket connection.
pub mod protocol;
/// API response DTOs returned by the server's REST endpoints.
pub mod responses;
/// Types describing backup schedules and their configuration.
pub mod schedule;
/// Types and helpers for SSH agent forwarding between server and agent.
pub mod ssh;
/// Core domain types (identifiers, hosts, repositories, backup reports, etc.)
/// used throughout the workspace.
pub mod types;
