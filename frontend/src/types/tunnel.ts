// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { TunnelResponse } from './generated'

export type SshTunnel = Omit<TunnelResponse, 'status'>
export type TunnelStatus =
  | 'connected'
  | 'disconnected'
  | 'reconnecting'
  | { error: { message: string } }

export interface TunnelWithStatus extends SshTunnel {
  status: TunnelStatus
  agent_hostname?: string
}

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
