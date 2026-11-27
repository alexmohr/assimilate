// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi } from 'vitest'

// Recorded as the module loads, before any per-test mock reset can clear it.
const createdWith = vi.hoisted(() => [] as unknown[])

vi.mock('axios', () => ({
  isCancel: () => false,
  default: {
    request: vi.fn(),
    create: (config: unknown) => {
      createdWith.push(config)
      return { interceptors: { response: { use: vi.fn() } } }
    },
  },
}))

import './client'

describe('apiClient defaults', () => {
  it('never looks up an XSRF cookie, so a request is sent without waiting on one', () => {
    expect(createdWith).toEqual([expect.objectContaining({ withXSRFToken: false })])
  })
})
