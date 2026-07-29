// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'
import type { Router } from 'vue-router'
import {
  navigateToScheduleIssue,
  scheduleIssuesFromEntries,
  scheduleRunStatus,
  withErrorTitles,
  type ScheduleHealthEntry,
} from './scheduleHealth'

function makeRouter(): Router {
  return { push: vi.fn() } as unknown as Router
}

function makeEntry(overrides: Partial<ScheduleHealthEntry> = {}): ScheduleHealthEntry {
  return {
    schedule_id: 1,
    hostname: 'bell',
    target_name: 'offsite',
    last_status: 'success',
    last_backup_at: '2026-01-01T00:00:00Z',
    is_overdue: false,
    last_error_message: null,
    cron_expression: '0 2 * * *',
    schedule_enabled: true,
    ...overrides,
  }
}

describe('scheduleRunStatus', () => {
  it('normalizes a raw status string', () => {
    expect(scheduleRunStatus(makeEntry({ last_status: 'FAILED' }))).toBe('failed')
  })

  it('returns null when the entry has never run', () => {
    expect(scheduleRunStatus(makeEntry({ last_status: null }))).toBeNull()
  })

  it('returns null for a null/undefined entry', () => {
    expect(scheduleRunStatus(null)).toBeNull()
    expect(scheduleRunStatus(undefined)).toBeNull()
  })
})

describe('navigateToScheduleIssue', () => {
  it('routes overdue to the schedule detail page', () => {
    const router = makeRouter()
    navigateToScheduleIssue(router, 42, 'overdue')
    expect(router.push).toHaveBeenCalledWith('/schedules/42')
  })

  it('routes failed/warning to the activity log filtered to this schedule', () => {
    const router = makeRouter()
    navigateToScheduleIssue(router, 42, 'failed')
    expect(router.push).toHaveBeenCalledWith(
      '/activity?category=backup&schedule_id=42&status=failed',
    )

    navigateToScheduleIssue(router, 42, 'warning')
    expect(router.push).toHaveBeenCalledWith(
      '/activity?category=backup&schedule_id=42&status=warning',
    )
  })
})

describe('scheduleIssuesFromEntries', () => {
  it('returns no issues for a healthy, non-overdue schedule', () => {
    const issues = scheduleIssuesFromEntries([makeEntry()], 1, makeRouter())
    expect(issues).toEqual([])
  })

  it('returns an Overdue chip when any entry is overdue', () => {
    const issues = scheduleIssuesFromEntries([makeEntry({ is_overdue: true })], 1, makeRouter())
    expect(issues).toHaveLength(1)
    expect(issues[0]).toMatchObject({ key: 'overdue', label: 'Overdue', severity: 'warning' })
  })

  it('returns a Warning chip when no entry has failed but one has a warning', () => {
    const router = makeRouter()
    const issues = scheduleIssuesFromEntries([makeEntry({ last_status: 'warning' })], 1, router)
    expect(issues).toHaveLength(1)
    expect(issues[0]).toMatchObject({ key: 'warning', label: 'Warning', severity: 'warning' })

    issues[0].onClick()
    expect(router.push).toHaveBeenCalledWith(
      '/activity?category=backup&schedule_id=1&status=warning',
    )
  })

  it('prefers a Failed chip over Warning when both are present across entries', () => {
    const issues = scheduleIssuesFromEntries(
      [makeEntry({ last_status: 'warning' }), makeEntry({ last_status: 'failed' })],
      1,
      makeRouter(),
    )
    expect(issues.map((i) => i.key)).toEqual(['failed'])
  })

  it('returns both Overdue and Failed chips when both apply', () => {
    const issues = scheduleIssuesFromEntries(
      [makeEntry({ is_overdue: true, last_status: 'failed' })],
      1,
      makeRouter(),
    )
    expect(issues.map((i) => i.key)).toEqual(['overdue', 'failed'])
  })

  it('wires each chip to navigate for the right schedule id and kind', () => {
    const router = makeRouter()
    const issues = scheduleIssuesFromEntries(
      [makeEntry({ is_overdue: true, last_status: 'failed' })],
      7,
      router,
    )

    issues.find((i) => i.key === 'overdue')!.onClick()
    expect(router.push).toHaveBeenCalledWith('/schedules/7')

    issues.find((i) => i.key === 'failed')!.onClick()
    expect(router.push).toHaveBeenCalledWith(
      '/activity?category=backup&schedule_id=7&status=failed',
    )
  })
})

describe('withErrorTitles', () => {
  it('sets the title of a Failed/Warning issue from the matching entry', () => {
    const entries = [makeEntry({ last_status: 'failed', last_error_message: 'disk full' })]
    const issues = scheduleIssuesFromEntries(entries, 1, makeRouter())

    expect(withErrorTitles(issues, entries)).toEqual([
      expect.objectContaining({ title: 'disk full' }),
    ])
  })

  it('leaves the issue untouched when the matching entry has no error message', () => {
    const entries = [makeEntry({ last_status: 'failed', last_error_message: null })]
    const issues = scheduleIssuesFromEntries(entries, 1, makeRouter())

    expect(withErrorTitles(issues, entries)[0].title).toBeUndefined()
  })

  it('leaves the Overdue issue untouched', () => {
    const entries = [makeEntry({ is_overdue: true })]
    const issues = scheduleIssuesFromEntries(entries, 1, makeRouter())

    expect(withErrorTitles(issues, entries)).toEqual(issues)
  })
})
