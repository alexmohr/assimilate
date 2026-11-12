// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { legacyDetailLink } from './legacyDetailLink'

describe('legacyDetailLink', () => {
  it('leaves a current link alone', () => {
    expect(legacyDetailLink({}, { hostname: 'web-01' })).toBeNull()
    expect(legacyDetailLink({ tab: 'overview' }, { hostname: 'web-01' })).toBeNull()
    expect(
      legacyDetailLink({ tab: 'settings', section: 'identity' }, { hostname: 'web-01' }),
    ).toBeNull()
    expect(legacyDetailLink({ tab: 'sources', section: 'vms' }, { hostname: 'web-01' })).toBeNull()
  })

  it("sends an agent's Logs tab to its runs in the Activity log", () => {
    expect(legacyDetailLink({ tab: 'logs' }, { hostname: 'web-01' })).toEqual({
      path: '/activity',
      query: { category: 'backup', hostname: 'web-01' },
    })
  })

  it('keeps the run and status a Logs tab link pinned', () => {
    expect(
      legacyDetailLink({ tab: 'logs', report: '42', status: 'failed' }, { hostname: 'web-01' }),
    ).toEqual({
      path: '/activity',
      query: { category: 'backup', hostname: 'web-01', status: 'failed', report: '42' },
    })
  })

  it('drops a status or report it cannot read', () => {
    expect(
      legacyDetailLink({ tab: 'logs', report: 'abc', status: 'success' }, { hostname: 'web-01' }),
    ).toEqual({ path: '/activity', query: { category: 'backup', hostname: 'web-01' } })
    expect(legacyDetailLink({ tab: 'logs', report: '0' }, { hostname: 'web-01' })).toEqual({
      path: '/activity',
      query: { category: 'backup', hostname: 'web-01' },
    })
  })

  it("sends a schedule's Logs tab to its runs in the Activity log", () => {
    expect(legacyDetailLink({ tab: 'logs' }, { scheduleId: 7 })).toEqual({
      path: '/activity',
      query: { category: 'backup', schedule_id: '7' },
    })
  })

  it('moves the VM and database settings to the Sources tab, section kept', () => {
    expect(
      legacyDetailLink({ tab: 'settings', section: 'vms', domain: 'lan' }, { hostname: 'web-01' }),
    ).toEqual({ query: { tab: 'sources', section: 'vms', domain: 'lan' } })
    expect(
      legacyDetailLink({ tab: 'settings', section: 'databases' }, { hostname: 'web-01' }),
    ).toEqual({ query: { tab: 'sources', section: 'databases' } })
  })

  it('opens the overview for the tags section, which moved to the header', () => {
    expect(legacyDetailLink({ tab: 'settings', section: 'tags' }, { hostname: 'web-01' })).toEqual({
      query: { tab: 'overview' },
    })
  })

  it('only moves agent settings sections, not a schedule', () => {
    expect(legacyDetailLink({ tab: 'settings', section: 'vms' }, { scheduleId: 7 })).toBeNull()
  })
})
