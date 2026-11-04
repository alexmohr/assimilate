// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { TunnelStatus, TunnelWithStatus } from '../types/tunnel'

/**
 * Where a tunnel stands, as the Tunnels list groups it: up, on its way back
 * up, or needing someone to look at it (disconnected, or failed with an error).
 */
export type TunnelState = 'connected' | 'reconnecting' | 'attention'

/** The Tunnels list's filters: everything, or one of the three states. */
export type TunnelFilter = 'all' | TunnelState

export function tunnelState(status: TunnelStatus): TunnelState {
  if (status === 'connected') return 'connected'
  if (status === 'reconnecting') return 'reconnecting'
  return 'attention'
}

export function matchesTunnelFilter(tunnel: TunnelWithStatus, filter: TunnelFilter): boolean {
  return filter === 'all' || tunnelState(tunnel.status) === filter
}

/** Whether any of the given names contains the search text, ignoring case. */
export function matchesTunnelSearch(query: string, ...names: (string | null)[]): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  return names.some((name) => name !== null && name.toLowerCase().includes(q))
}

/** The tone of a row's status dot; `null` for a switched-off tunnel, which is grey. */
export function tunnelDotTone(tunnel: TunnelWithStatus): 'success' | 'warning' | 'danger' | null {
  if (!tunnel.enabled) return null
  switch (tunnelState(tunnel.status)) {
    case 'connected':
      return 'success'
    case 'reconnecting':
      return 'warning'
    case 'attention':
      return 'danger'
  }
}

/**
 * How urgently a tunnel wants looking at: an error, then an enabled tunnel
 * that is down, then one reconnecting, then a connected one, and last one
 * that is switched off and so down on purpose.
 */
function attentionRank(tunnel: TunnelWithStatus): number {
  if (!tunnel.enabled) return 4
  if (typeof tunnel.status === 'object') return 0
  switch (tunnelState(tunnel.status)) {
    case 'attention':
      return 1
    case 'reconnecting':
      return 2
    case 'connected':
      return 3
  }
}

/** Problems first, then by the agent's name. Returns a new array. */
export function sortTunnels(
  tunnels: readonly TunnelWithStatus[],
  label: (tunnel: TunnelWithStatus) => string,
): TunnelWithStatus[] {
  return [...tunnels].sort(
    (a, b) => attentionRank(a) - attentionRank(b) || label(a).localeCompare(label(b)),
  )
}
