// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import {
  backupStatusBadgeClass,
  backupStatusTone,
  badgeClass,
  logLevelTone,
  thresholdTone,
  type BadgeTone,
} from './badge'

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
