// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

pub mod api;
pub mod borg;
pub mod config_assembler;
pub mod db;
pub mod error;
pub mod log_buffer;
pub mod middleware;
pub mod notifications;
pub mod openapi;
pub mod rate_limit;
pub mod scheduler;
pub mod ssh;
pub mod tunnel;
pub mod ws;

use std::{collections::HashMap, sync::Arc};

use shared::types::DryRunFile;
use sqlx::PgPool;
use tokio::sync::{Mutex, oneshot};

use crate::{
    log_buffer::LogBuffer,
    notifications::NotificationService,
    tunnel::TunnelManager,
    ws::{registry::AgentRegistry, ui_broadcast::UiBroadcast},
};

pub type PendingDryRuns =
    Arc<Mutex<HashMap<String, oneshot::Sender<(Vec<DryRunFile>, i64, Option<String>)>>>>;

/// (success, files_restored, error_message)
pub type PendingRestores =
    Arc<Mutex<HashMap<String, oneshot::Sender<(bool, u64, Option<String>)>>>>;

pub type PendingMigrations = Arc<Mutex<HashMap<String, oneshot::Sender<(bool, Option<String>)>>>>;

/// (success, deleted_count, error_message)
pub type PendingDeletes = Arc<Mutex<HashMap<String, oneshot::Sender<(bool, u32, Option<String>)>>>>;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub encryption_key: [u8; 32],
    pub registry: AgentRegistry,
    pub ui_broadcast: UiBroadcast,
    pub tunnel_manager: TunnelManager,
    pub log_buffer: LogBuffer,
    pub notification_service: NotificationService,
    pub pending_dryruns: PendingDryRuns,
    pub pending_restores: PendingRestores,
    pub pending_migrations: PendingMigrations,
    pub pending_deletes: PendingDeletes,
}
