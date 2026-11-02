// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { isMaintenanceOutcome, maintenanceOutcomeBadge, maintenanceRunSeconds } from './appendOnly'

describe('appendOnly utils', () => {
  it.each([
    ['ok', 'OK', 'success'],
    ['stopped', 'Stopped', 'warning'],
    ['check_failed', 'Check failed', 'danger'],
    ['failed', 'Failed', 'danger'],
  ])('badges the %s outcome', (outcome, label, tone) => {
    expect(maintenanceOutcomeBadge({ outcome })).toEqual({ label, tone })
  })

  it('shows an unknown outcome neutrally as itself', () => {
    expect(isMaintenanceOutcome('later')).toBe(false)
    expect(maintenanceOutcomeBadge({ outcome: 'later' })).toEqual({
      label: 'later',
      tone: 'neutral',
    })
  })

  it('measures how long a finished run took', () => {
    expect(
      maintenanceRunSeconds({
        started_at: '2026-10-19T04:30:00Z',
        finished_at: '2026-10-19T04:31:30Z',
      }),
    ).toBe(90)
  })

  it('has no duration for a run still going or with odd timestamps', () => {
    expect(
      maintenanceRunSeconds({ started_at: '2026-10-19T04:30:00Z', finished_at: null }),
    ).toBeNull()
    expect(
      maintenanceRunSeconds({ started_at: '2026-10-19T04:30:00Z', finished_at: 'garbage' }),
    ).toBeNull()
  })
})
