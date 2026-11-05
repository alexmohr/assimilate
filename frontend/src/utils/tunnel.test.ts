// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import type { TunnelStatus, TunnelWithStatus } from '../types/tunnel'
import {
  matchesTunnelFilter,
  matchesTunnelSearch,
  sortTunnels,
  tunnelDotTone,
  tunnelErrorMessage,
  tunnelState,
} from './tunnel'

function tunnel(
  id: number,
  status: TunnelStatus,
  overrides: Partial<TunnelWithStatus> = {},
): TunnelWithStatus {
  return {
    id,
    agent_id: id,
    agent_hostname: `agent-${id}`,
    ssh_host: '10.0.0.1',
    ssh_user: 'root',
    ssh_port: 22,
    tunnel_port: 2200 + id,
    enabled: true,
    created_at: '2026-05-31T00:00:00Z',
    status,
    ...overrides,
  }
}

const failed: TunnelStatus = { error: { message: 'handshake failed' } }

describe('tunnel list helpers', () => {
  it('puts disconnected and failed tunnels under needs attention', () => {
    expect(tunnelState('connected')).toBe('connected')
    expect(tunnelState('reconnecting')).toBe('reconnecting')
    expect(tunnelState('disconnected')).toBe('attention')
    expect(tunnelState(failed)).toBe('attention')
  })

  it('filters by state, with all matching everything', () => {
    const up = tunnel(1, 'connected')
    const down = tunnel(2, failed)
    expect(matchesTunnelFilter(up, 'all')).toBe(true)
    expect(matchesTunnelFilter(down, 'all')).toBe(true)
    expect(matchesTunnelFilter(up, 'connected')).toBe(true)
    expect(matchesTunnelFilter(down, 'connected')).toBe(false)
    expect(matchesTunnelFilter(down, 'attention')).toBe(true)
    expect(matchesTunnelFilter(up, 'reconnecting')).toBe(false)
  })

  it('searches any of the names given, ignoring case, nulls and blank queries', () => {
    expect(matchesTunnelSearch(' ', 'web-01', null)).toBe(true)
    expect(matchesTunnelSearch('WEB', 'web-01', null)).toBe(true)
    expect(matchesTunnelSearch('nas', 'web-01', null, 'nas.lan')).toBe(true)
    expect(matchesTunnelSearch('media', 'web-01', null, 'nas.lan')).toBe(false)
  })

  it('tones the dot by state, grey for a switched-off tunnel', () => {
    expect(tunnelDotTone(tunnel(1, 'connected'))).toBe('success')
    expect(tunnelDotTone(tunnel(1, 'reconnecting'))).toBe('warning')
    expect(tunnelDotTone(tunnel(1, 'disconnected'))).toBe('danger')
    expect(tunnelDotTone(tunnel(1, failed))).toBe('danger')
    expect(tunnelDotTone(tunnel(1, 'disconnected', { enabled: false }))).toBeNull()
  })

  it('sorts problems first and switched-off tunnels last, then by name', () => {
    const input = [
      tunnel(1, 'connected', { agent_hostname: 'b-up' }),
      tunnel(2, 'disconnected', { enabled: false, agent_hostname: 'off' }),
      tunnel(3, 'connected', { agent_hostname: 'a-up' }),
      tunnel(4, 'reconnecting', { agent_hostname: 'coming-back' }),
      tunnel(5, 'disconnected', { agent_hostname: 'down' }),
      tunnel(6, failed, { agent_hostname: 'broken' }),
    ]
    const order = sortTunnels(input, (t) => t.agent_hostname ?? '').map((t) => t.agent_hostname)
    expect(order).toEqual(['broken', 'down', 'coming-back', 'a-up', 'b-up', 'off'])
    expect(input[0].agent_hostname).toBe('b-up')
  })
})

describe('tunnelErrorMessage', () => {
  it('returns the message of a failed tunnel', () => {
    expect(tunnelErrorMessage({ error: { message: 'refused' } })).toBe('refused')
  })

  it.each(['connected', 'disconnected', 'reconnecting'] as const)(
    'returns null for a %s tunnel',
    (status) => {
      expect(tunnelErrorMessage(status)).toBeNull()
    },
  )
})
