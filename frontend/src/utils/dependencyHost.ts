// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import type { BadgeTone } from './badge'
import type { DependencyTestResponse } from '../types/generated'

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
    ? { tone: 'success', label: 'Reachable' }
    : { tone: 'warning', label: 'Not answering' }
}

/** The sentence a connection test is reported with, in a dialog and in a toast alike. */
export function testResultText(result: DependencyTestResponse): string {
  return result.reachable
    ? `${result.address} answered on port ${result.port}`
    : `${result.address} did not answer on port ${result.port} within 5 seconds`
}
