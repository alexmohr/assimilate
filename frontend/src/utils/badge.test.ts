// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  agentPowerPhase,
  backupStatusBadgeClass,
  backupStatusTone,
  badgeClass,
  logLevelTone,
  systemEventTone,
  thresholdTone,
  type BadgeTone,
} from './badge'
import type { RunEventType, SystemEventSeverity } from '../types/generated'

const TONES: BadgeTone[] = ['success', 'warning', 'danger', 'info', 'accent', 'neutral']

describe('badgeClass', () => {
  it('maps every tone to its modifier class', () => {
    expect(TONES.map(badgeClass)).toEqual([
      'badge--success',
      'badge--warning',
      'badge--danger',
      'badge--info',
      'badge--accent',
      'badge--neutral',
    ])
  })
})

describe('backupStatusTone', () => {
  // One case per NormalizedBackupStatus arm, so a new status added to the
  // union without a tone shows up here rather than silently rendering as a
  // failure.
  it.each([
    ['success', 'success'],
    ['warning', 'warning'],
    ['started', 'info'],
    ['pending', 'neutral'],
    ['cancelled', 'neutral'],
    ['failed', 'danger'],
  ])('renders %s as the %s tone', (status, tone) => {
    expect(backupStatusTone(status)).toBe(tone)
  })

  it('reads the status case-insensitively, as the wire format is Display-derived', () => {
    expect(backupStatusTone('SUCCESS')).toBe('success')
    expect(backupStatusTone('Started')).toBe('info')
  })

  it('treats an unrecognized status as a failure rather than hiding it', () => {
    expect(backupStatusTone('exploded')).toBe('danger')
  })
})

describe('backupStatusBadgeClass', () => {
  it('composes the tone into the badge modifier', () => {
    expect(backupStatusBadgeClass('success')).toBe('badge--success')
    expect(backupStatusBadgeClass('failed')).toBe('badge--danger')
  })
})

describe('thresholdTone', () => {
  it.each([
    ['ok', 'success'],
    ['warning', 'warning'],
    ['critical', 'danger'],
  ] as const)('renders %s as the %s tone', (level, tone) => {
    expect(thresholdTone(level)).toBe(tone)
  })
})

describe('logLevelTone', () => {
  it.each([
    ['error', 'danger'],
    ['warn', 'warning'],
    ['warning', 'warning'],
    ['info', 'info'],
  ])('renders %s as the %s tone', (level, tone) => {
    expect(logLevelTone(level)).toBe(tone)
  })

  it('accepts the upper-case spellings the agent log emits', () => {
    expect(logLevelTone('ERROR')).toBe('danger')
    expect(logLevelTone('WARN')).toBe('warning')
  })

  it('falls back to neutral for levels it does not style', () => {
    expect(logLevelTone('debug')).toBe('neutral')
    expect(logLevelTone('trace')).toBe('neutral')
    expect(logLevelTone('')).toBe('neutral')
  })
})

describe('agentPowerPhase', () => {
  // One case per RunEventType arm, so a new event type added to the union
  // without a phase mapping shows up here rather than silently falling
  // through to the type checker only.
  it.each([
    ['reachability_check', { label: 'Checking...', tone: 'neutral' }],
    ['wake_sent', { label: 'Waking host...', tone: 'info' }],
    ['host_online', { label: 'Waking host...', tone: 'info' }],
    ['agent_start_sent', { label: 'Starting agent...', tone: 'info' }],
    ['shutdown_sent', { label: 'Shutting down...', tone: 'neutral' }],
    ['agent_stop_sent', { label: 'Shutting down...', tone: 'neutral' }],
  ] as [RunEventType, { label: string; tone: BadgeTone }][])(
    'maps %s to %o',
    (eventType, phase) => {
      expect(agentPowerPhase(eventType)).toEqual(phase)
    },
  )

  it.each(['agent_connected', 'host_offline', 'agent_stopped'] as RunEventType[])(
    'maps %s to null, ending the transient phase',
    (eventType) => {
      expect(agentPowerPhase(eventType)).toBeNull()
    },
  )
})

describe('systemEventTone', () => {
  it('maps every severity the server can report', () => {
    expect(systemEventTone('success')).toBe('success')
    expect(systemEventTone('warning')).toBe('warning')
    expect(systemEventTone('failed')).toBe('danger')
    expect(systemEventTone('info')).toBe('info')
  })

  // A tab left open across a deploy that adds a severity variant would
  // otherwise get `undefined` back and render a broken tone class.
  it('falls back to a neutral tone for a severity it does not know', () => {
    expect(systemEventTone('nonsense' as SystemEventSeverity)).toBe('neutral')
  })
})
