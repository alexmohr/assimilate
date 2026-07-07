// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! WebSocket protocol handlers for agent and UI connections.

/// Broadcast bus for operation completion events.
pub mod completion_bus;
/// Agent WebSocket connection handler and message routing.
pub mod handler;
/// Agent connection registry and message dispatch.
pub mod registry;
/// SSH agent forwarding relay over WebSocket.
pub mod ssh_relay;
/// Broadcast channel for UI-facing real-time events.
pub mod ui_broadcast;
/// WebSocket upgrade handler for UI client connections.
pub mod ui_handler;
