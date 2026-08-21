// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { TunnelResponse, TunnelStatus as GeneratedTunnelStatus } from './generated'

export type SshTunnel = TunnelResponse
export type TunnelStatus = GeneratedTunnelStatus

export interface TunnelWithStatus extends SshTunnel {
  agent_hostname?: string
}

// Mirror unexported request structs in crates/server/src/api/tunnels.rs - simple POST/PUT
// bodies with no Rust-side compile-time-checked counterpart on the ts-rs export surface.
export interface CreateTunnelRequest {
  agent_id: number
  ssh_host: string
  ssh_user: string
  ssh_port: number
  tunnel_port: number
  enabled: boolean
}
export interface UpdateTunnelRequest {
  ssh_host?: string
  ssh_user?: string
  ssh_port?: number
  tunnel_port?: number
  enabled?: boolean
}
