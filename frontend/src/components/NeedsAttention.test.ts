// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { renderWithPlugins } from '../test-utils'
import type { DashboardFinding } from '../types/dashboard'
import NeedsAttention from './NeedsAttention.vue'

function finding(overrides: Partial<DashboardFinding>): DashboardFinding {
  return {
    id: 'finding-1',
    kind: 'host_offline_due_soon',
    severity: 'warning',
    status: 'warning',
    hostname: 'web 01',
    schedule_id: 4,
    schedule_name: 'nightly',
    repo_id: 9,
    repo_name: 'offsite',
    reason: 'Host is offline',
    occurred_at: '2026-06-01T00:00:00Z',
    deadline: null,
    destination: { kind: 'schedule', schedule_id: 4 },
    ...overrides,
  }
}

function openHref(f: DashboardFinding): string | undefined {
  const wrapper = renderWithPlugins(NeedsAttention, { props: { findings: [f] } })
  return wrapper.find('.finding-action').attributes('href')
}

describe('NeedsAttention destinations', () => {
  it('opens a host finding on the agent page, with the hostname encoded', () => {
    const href = openHref(finding({ destination: { kind: 'host', hostname: 'web 01' } }))
    expect(href).toBe('/agents/web%2001')
  })

  it('opens a repository finding on the repository page', () => {
    const href = openHref(
      finding({ kind: 'repository_unscheduled', destination: { kind: 'repository', repo_id: 9 } }),
    )
    expect(href).toBe('/repos/9')
  })

  it('opens an activity finding that is not a failed run on its schedule in the Activity log', () => {
    const href = openHref(
      finding({
        kind: 'schedule_target_overdue',
        destination: { kind: 'activity', report_id: 3 },
      }),
    )
    // Not a failed or warned backup, so no hostname or status filter is added.
    expect(href).toBe('/activity?category=backup&schedule_id=4')
  })
})
