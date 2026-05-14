// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

pub mod api;
pub mod config_assembler;
pub mod db;
pub mod error;
pub mod log_buffer;
pub mod middleware;
pub mod openapi;
pub mod rate_limit;
pub mod scheduler;
pub mod ssh;
pub mod tunnel;
pub mod ws;

use sqlx::PgPool;

use crate::{
    log_buffer::LogBuffer,
    tunnel::TunnelManager,
    ws::{registry::AgentRegistry, ui_broadcast::UiBroadcast},
};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub encryption_key: [u8; 32],
    pub registry: AgentRegistry,
    pub ui_broadcast: UiBroadcast,
    pub tunnel_manager: TunnelManager,
    pub log_buffer: LogBuffer,
}
