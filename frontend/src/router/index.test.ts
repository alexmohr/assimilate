// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it } from 'vitest'
import { router } from './index'

describe('router', () => {
  it('resolves the agent-detail route to AgentDetailView', async () => {
    const route = router.getRoutes().find((r) => r.name === 'agent-detail')
    expect(route).toBeTruthy()

    const loadComponent = route!.components?.default as () => Promise<{ default: unknown }>
    const module = await loadComponent()
    expect(module.default).toBeTruthy()
  })
})
