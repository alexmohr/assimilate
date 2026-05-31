// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'

type ResponseHandler = (response: unknown) => unknown
type ErrorHandler = (error: {
  response?: { status?: number }
  config?: { url?: string }
}) => Promise<never>

const routerPush = vi.hoisted(() => vi.fn().mockResolvedValue(undefined))
const responseUse = vi.hoisted(() => vi.fn())

vi.mock('../router', () => ({
  router: {
    push: routerPush,
    currentRoute: { value: { fullPath: '/' } },
  },
}))

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

import { apiClient } from './client'

describe('apiClient response interceptor', () => {
  const [successHandler, errorHandler] = responseUse.mock.calls[0] as [
    ResponseHandler,
    ErrorHandler,
  ]

  beforeEach(() => {
    routerPush.mockClear()
  })

  it('redirects 401 responses to login', async () => {
    const err = { response: { status: 401 }, config: { url: '/clients' } }
    const rejected = errorHandler(err).catch((e: unknown) => e)
    // flush the dynamic import('../router') microtask before asserting
    await vi.dynamicImportSettled()
    await rejected

    expect(routerPush).toHaveBeenCalledWith({ name: 'login', query: { next: '/' } })
  })

  it('does not redirect non-401 responses', async () => {
    await expect(
      errorHandler({ response: { status: 500 }, config: { url: '/clients' } }),
    ).rejects.toEqual({
      response: { status: 500 },
      config: { url: '/clients' },
    })

    expect(routerPush).not.toHaveBeenCalled()
  })

  it('does not redirect 401 for auth endpoints', async () => {
    await expect(
      errorHandler({ response: { status: 401 }, config: { url: '/auth/login' } }),
    ).rejects.toEqual({
      response: { status: 401 },
      config: { url: '/auth/login' },
    })

    expect(routerPush).not.toHaveBeenCalled()
  })

  it('keeps the success handler wired', () => {
    expect(successHandler).toBeTypeOf('function')
    expect(apiClient).toBeDefined()
  })
})
