// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'

type ResponseHandler = (response: unknown) => unknown
type ErrorHandler = (error: {
  response?: { status?: number }
  config?: { url?: string }
}) => Promise<never>

const locationAssign = vi.fn()
const responseUse = vi.hoisted(() => vi.fn())

vi.mock('axios', () => ({
  default: {
    create: vi.fn(() => ({
      interceptors: {
        response: {
          use: responseUse,
        },
      },
    })),
  },
}))

vi.stubGlobal('window', {
  location: { assign: locationAssign },
})

import { apiClient } from './client'

describe('apiClient response interceptor', () => {
  const [successHandler, errorHandler] = responseUse.mock.calls[0] as [
    ResponseHandler,
    ErrorHandler,
  ]

  beforeEach(() => {
    locationAssign.mockClear()
  })

  it('redirects 401 responses to login', async () => {
    await expect(
      errorHandler({ response: { status: 401 }, config: { url: '/clients' } }),
    ).rejects.toEqual({
      response: { status: 401 },
      config: { url: '/clients' },
    })

    expect(locationAssign).toHaveBeenCalledWith('/login')
  })

  it('does not redirect non-401 responses', async () => {
    await expect(
      errorHandler({ response: { status: 500 }, config: { url: '/clients' } }),
    ).rejects.toEqual({
      response: { status: 500 },
      config: { url: '/clients' },
    })

    expect(locationAssign).not.toHaveBeenCalled()
  })

  it('does not redirect 401 for auth endpoints', async () => {
    await expect(
      errorHandler({ response: { status: 401 }, config: { url: '/auth/login' } }),
    ).rejects.toEqual({
      response: { status: 401 },
      config: { url: '/auth/login' },
    })

    expect(locationAssign).not.toHaveBeenCalled()
  })

  it('keeps the success handler wired', () => {
    expect(successHandler).toBeTypeOf('function')
    expect(apiClient).toBeDefined()
  })
})
