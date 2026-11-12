// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { SCOPE_AGENTS, SCOPE_REPOS, SCOPE_SCHEDULES, scopeOptionsGet } from './notificationFixtures'

describe('scopeOptionsGet', () => {
  it('serves the scope lists by URL', async () => {
    expect((await scopeOptionsGet('/repos')).data).toBe(SCOPE_REPOS)
    expect((await scopeOptionsGet('/agents')).data).toBe(SCOPE_AGENTS)
    expect((await scopeOptionsGet('/schedules')).data).toBe(SCOPE_SCHEDULES)
  })

  it('answers any other URL with an empty list', async () => {
    expect((await scopeOptionsGet('/notifications/deliveries')).data).toEqual([])
  })
})
