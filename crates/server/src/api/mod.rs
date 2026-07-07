// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

/// Agent management endpoints.
pub mod agents;
/// Archive browsing, listing, and content endpoints.
pub mod archives;
/// Audit log endpoints.
pub mod audit;
/// Authentication and session endpoints.
pub mod auth;
/// Configuration import/export endpoints.
pub mod config_io;
/// Agent deployment endpoints.
pub mod deploy;
/// Archive diff endpoints.
pub mod diff;
/// Backup dry-run endpoints.
pub mod dryrun;
/// Global exclude pattern endpoints.
pub mod excludes;
/// Archive export endpoints.
pub mod export;
/// Health check endpoints.
pub mod health;
/// Shared helper functions for API handlers.
pub mod helpers;
/// Repository key management endpoints.
pub mod keys;
/// Server log endpoints.
pub mod logs;
/// Notification endpoints.
pub mod notifications;
/// Repository permission endpoints.
pub mod permissions;
/// Repository quota endpoints.
pub mod quota;
/// Role-based access control (groups, roles) endpoints.
pub mod rbac;
/// Backup report endpoints.
pub mod reports;
/// Repository management endpoints.
pub mod repos;
/// Archive restore endpoints.
pub mod restore;
/// Backup schedule endpoints.
pub mod schedules;
/// Archive search endpoints.
pub mod search;
/// Server-level quota endpoints.
pub mod server_quotas;
/// SSH connection endpoints.
pub mod ssh;
/// Statistics and dashboard endpoints.
pub mod stats;
/// System management endpoints.
pub mod system;
/// Tag management endpoints.
pub mod tags;
/// API token endpoints.
pub mod tokens;
/// SSH tunnel endpoints.
pub mod tunnels;
/// User management endpoints.
pub mod users;
