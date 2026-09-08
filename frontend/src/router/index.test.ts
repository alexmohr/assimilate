// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { router } from './index'

describe('router', () => {
  /** The wizard is its own view now, not the detail page in create mode. */
  it('resolves the schedule-create route to ScheduleCreateView', async () => {
    const route = router.getRoutes().find((r) => r.name === 'schedule-create')
    expect(route).toBeTruthy()
    expect(route!.path).toBe('/schedules/new')

    const loadComponent = route!.components?.default as () => Promise<{ default: unknown }>
    const module = await loadComponent()
    expect(module.default).toBeTruthy()
  })

  it('resolves the agent-detail route to AgentDetailView', async () => {
    const route = router.getRoutes().find((r) => r.name === 'agent-detail')
    expect(route).toBeTruthy()

    const loadComponent = route!.components?.default as () => Promise<{ default: unknown }>
    const module = await loadComponent()
    expect(module.default).toBeTruthy()
  })
})
