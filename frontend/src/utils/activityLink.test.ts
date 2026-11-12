// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { runLogLink } from './activityLink'

describe('runLogLink', () => {
  it('opens the backup runs of the Activity log with nothing else given', () => {
    expect(runLogLink({})).toEqual({ path: '/activity', query: { category: 'backup' } })
  })

  it('narrows the log to everything it is told about one run', () => {
    expect(
      runLogLink({
        hostname: 'web-01',
        scheduleId: 7,
        target: 'server-daily',
        status: 'failed',
        reportId: 99,
      }),
    ).toEqual({
      path: '/activity',
      query: {
        category: 'backup',
        hostname: 'web-01',
        schedule_id: '7',
        target: 'server-daily',
        status: 'failed',
        report: '99',
      },
    })
  })

  it('carries a warned run’s status too', () => {
    expect(runLogLink({ status: 'warning' })).toEqual({
      path: '/activity',
      query: { category: 'backup', status: 'warning' },
    })
  })

  it('leaves out every field that is null, empty or zero', () => {
    expect(
      runLogLink({
        hostname: null,
        scheduleId: null,
        target: '',
        status: null,
        reportId: null,
      }),
    ).toEqual({ path: '/activity', query: { category: 'backup' } })
    expect(runLogLink({ hostname: '', scheduleId: 0, reportId: 0 })).toEqual({
      path: '/activity',
      query: { category: 'backup' },
    })
  })

  it('keeps each field independent of the others', () => {
    expect(runLogLink({ scheduleId: 3 })).toEqual({
      path: '/activity',
      query: { category: 'backup', schedule_id: '3' },
    })
    expect(runLogLink({ reportId: 12 })).toEqual({
      path: '/activity',
      query: { category: 'backup', report: '12' },
    })
  })
})
