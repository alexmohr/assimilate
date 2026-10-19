// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! Domain types, WebSocket protocol schema, and crypto utilities shared
//! between the `server` and `agent` crates.

/// The actions recorded in the audit log, each with its own details.
pub mod audit;
/// Shared borg process handling: the `Borg` wrappers' common spawn/wait/log
/// pattern and graceful SIGTERM/SIGKILL child termination, environment and
/// pattern-file building (`borg::env`), and typed `--log-json` parsing
/// (`borg::log_json`).
pub mod borg;
/// Stopping a console child the way Ctrl+C would, including on Windows,
/// where that needs the child on a console of its own.
pub mod console;
/// The session cookie's name and parsing of `Cookie`/`Set-Cookie` headers,
/// shared by the server and the desktop app's provisioning client.
pub mod cookies;
/// Encryption and decryption helpers used to protect secrets (e.g. borg
/// repository passphrases) at rest.
pub mod crypto;
/// Database dumps: the engines and Kubernetes workloads an agent dumps into a
/// backup, its per-host settings, and the manifest written into each archive.
pub mod database;
/// Postgres `TEXT` column support for domain enums, so the server's rows hold
/// the enums themselves and an unknown stored value is a read error.
#[cfg(feature = "sqlx")]
pub mod db_text;
/// Wire types for dependency hosts: machines a backup needs besides its agent
/// and repository.
pub mod dependency_hosts;
/// Human-readable formatting shared by the agent's VM staging output and the
/// server's notification content.
pub mod format;
/// Pre- and post-backup hook commands and their per-command timeouts.
pub mod hooks;
/// Windows job objects: a child and everything it starts, killed as one.
#[cfg(windows)]
pub mod job;
/// Machines: what is woken, shut down, and expected to be online or not.
pub mod machines;
/// Notification channel, rule and delivery wire types.
pub mod notifications;
/// Message types exchanged over the agent/server WebSocket connection.
pub mod protocol;
/// API response DTOs returned by the server's REST endpoints.
pub mod responses;
/// Types describing backup schedules and their configuration.
pub mod schedule;
/// Types and helpers for SSH agent forwarding between server and agent.
pub mod ssh;
/// Tracks fire-and-forget background tasks so shutdown can join them.
pub mod task_registry;
/// Core domain types (identifiers, hosts, repositories, backup reports, etc.)
/// used throughout the workspace.
pub mod types;
/// Types describing the libvirt/QEMU domains an agent stages before a backup.
pub mod vm;
/// Files and named pipes with a DACL that keeps other local users out.
#[cfg(windows)]
pub mod windows_acl;
