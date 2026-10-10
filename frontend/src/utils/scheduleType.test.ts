// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { scheduleTypeLabel, scheduleTypeShortLabel } from './scheduleType'

describe('scheduleTypeLabel', () => {
  it('names every schedule type in full', () => {
    expect(scheduleTypeLabel('backup')).toBe('Backup')
    expect(scheduleTypeLabel('check')).toBe('Integrity check')
    expect(scheduleTypeLabel('verify')).toBe('Verify (extract dry-run)')
  })
})

describe('scheduleTypeShortLabel', () => {
  it('drops only the verify parenthetical', () => {
    expect(scheduleTypeShortLabel('backup')).toBe('Backup')
    expect(scheduleTypeShortLabel('check')).toBe('Integrity check')
    expect(scheduleTypeShortLabel('verify')).toBe('Verify')
  })
})
