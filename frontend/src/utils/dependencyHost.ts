// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { BadgeTone } from './badge'
import type {
  DependencyHostResponse,
  DependencyTestResponse,
  MachineWaitResponse,
} from '../types/generated'

/**
 * The well-known ports a dependency is usually checked on. Anything else is
 * a plain TCP port: the check is the same either way, the name only says what
 * the machine is probably serving.
 */
export type PortPreset = 'smb' | 'nfs' | 'ssh' | 'other'

interface KnownPort {
  preset: Exclude<PortPreset, 'other'>
  port: number
  protocol: string
}

const KNOWN_PORTS: readonly KnownPort[] = [
  { preset: 'smb', port: 445, protocol: 'SMB' },
  { preset: 'nfs', port: 2049, protocol: 'NFS' },
  { preset: 'ssh', port: 22, protocol: 'SSH' },
]

/** The choices of the port picker, in the order it shows them. */
export const PORT_PRESET_OPTIONS: readonly { value: PortPreset; label: string }[] = [
  ...KNOWN_PORTS.map((known) => ({ value: known.preset, label: known.protocol })),
  { value: 'other', label: 'Other port' },
]

/** The port a preset stands for; `null` for "Other port", which keeps whatever was typed. */
export function presetPort(preset: PortPreset): number | null {
  return KNOWN_PORTS.find((known) => known.preset === preset)?.port ?? null
}

export function presetForPort(port: number): PortPreset {
  return KNOWN_PORTS.find((known) => known.port === port)?.preset ?? 'other'
}

/** "SMB", "NFS", "SSH", or "TCP" for any other port. */
export function protocolLabel(port: number): string {
  return KNOWN_PORTS.find((known) => known.port === port)?.protocol ?? 'TCP'
}

/** "SMB, TCP port 445" - what the check actually does, and what it is for. */
export function checkDescription(port: number): string {
  const known = KNOWN_PORTS.find((k) => k.port === port)
  return known ? `${known.protocol}, TCP port ${port}` : `TCP port ${port}`
}

export interface ReachabilityBadge {
  tone: BadgeTone
  label: string
}

/**
 * What the last check found. `null` is a dependency nothing has asked yet,
 * which is not the same as one that did not answer.
 */
export function reachabilityBadge(lastCheckReachable: boolean | null): ReachabilityBadge {
  if (lastCheckReachable === null) return { tone: 'neutral', label: 'Not checked yet' }
  return lastCheckReachable
    ? { tone: 'success', label: 'Answering' }
    : { tone: 'warning', label: 'Not answering' }
}

/** The sentence a connection test is reported with, in a dialog and in a toast alike. */
export function testResultText(result: DependencyTestResponse): string {
  return result.reachable
    ? `${result.address} answered on port ${result.port}`
    : `${result.address} did not answer on port ${result.port} within ${result.timeout_seconds} seconds`
}

/**
 * The Dependencies list's filters, each a question the list can answer from
 * what it already has: did the last check fail, is a run waiting on it, has
 * it never been asked.
 */
export type DependencyFilter = 'all' | 'not_answering' | 'waiting' | 'not_checked'

export function matchesDependencyFilter(
  host: DependencyHostResponse,
  filter: DependencyFilter,
): boolean {
  switch (filter) {
    case 'all':
      return true
    case 'not_answering':
      return host.last_check_reachable === false
    case 'waiting':
      return host.waiting_count > 0
    case 'not_checked':
      return host.last_check_reachable === null
  }
}

/**
 * How urgently a dependency wants looking at: one that did not answer, then
 * one runs are waiting on, then one never checked, then the rest.
 */
function attentionRank(host: DependencyHostResponse): number {
  if (host.last_check_reachable === false) return 0
  if (host.waiting_count > 0) return 1
  if (host.last_check_reachable === null) return 2
  return 3
}

/** Problems first, then by name. Returns a new array. */
export function sortDependencies(
  hosts: readonly DependencyHostResponse[],
): DependencyHostResponse[] {
  return [...hosts].sort(
    (a, b) => attentionRank(a) - attentionRank(b) || a.name.localeCompare(b.name),
  )
}

/** Whether the name or the address contains the search text, ignoring case. */
export function matchesDependencySearch(host: DependencyHostResponse, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  return host.name.toLowerCase().includes(q) || host.address.toLowerCase().includes(q)
}

/**
 * The waits on a machine that are waits on this dependency. A machine lists
 * the waits of every role on it, and other dependencies may share it.
 */
export function dependencyWaits(
  waits: readonly MachineWaitResponse[],
  dependencyName: string,
): MachineWaitResponse[] {
  return waits.filter((w) => w.role === 'dependency' && w.dependency_name === dependencyName)
}

/** "1 run waiting", "3 runs waiting". */
export function waitingRunsText(count: number): string {
  return count === 1 ? '1 run waiting' : `${count} runs waiting`
}

/** "1 schedule", "2 schedules". */
export function countLabel(count: number, singular: string, plural: string): string {
  return `${count} ${count === 1 ? singular : plural}`
}
