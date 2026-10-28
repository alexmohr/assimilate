// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'

type ResponseHandler = (response: unknown) => unknown
type ErrorHandler = (error: {
  response?: { status?: number }
  config?: { url?: string; method?: string }
}) => Promise<never>

const locationAssign = vi.fn()
const responseUse = vi.hoisted(() => vi.fn())
const requestMock = vi.hoisted(() => vi.fn())

vi.mock('axios', () => ({
  isCancel: (e: { cancelled?: boolean }) => e.cancelled === true,
  default: {
    request: requestMock,
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
    requestMock.mockReset()
  })

  it('retries a GET that failed without a response', async () => {
    vi.useFakeTimers()
    requestMock.mockResolvedValue('ok')
    const config = { url: '/repos/1/archives', method: 'get' }

    const result = errorHandler({ config })
    await vi.runAllTimersAsync()
    await expect(result).resolves.toBe('ok')
    vi.useRealTimers()

    expect(requestMock).toHaveBeenCalledTimes(1)
    expect(requestMock).toHaveBeenCalledWith(config)
  })

  it('gives up after the retry budget and rejects with the last error', async () => {
    vi.useFakeTimers()
    const error = { config: { url: '/agents', method: 'get' } }
    requestMock.mockRejectedValue(error)

    const result = expect(errorHandler(error)).rejects.toBe(error)
    await vi.runAllTimersAsync()
    await result
    vi.useRealTimers()

    expect(requestMock).toHaveBeenCalledTimes(2)
  })

  it('redirects to login when a retry comes back unauthorized', async () => {
    vi.useFakeTimers()
    const unauthorized = { response: { status: 401 }, config: { url: '/agents', method: 'get' } }
    requestMock.mockRejectedValue(unauthorized)

    const result = expect(errorHandler({ config: { url: '/agents', method: 'get' } })).rejects.toBe(
      unauthorized,
    )
    await vi.runAllTimersAsync()
    await result
    vi.useRealTimers()

    expect(requestMock).toHaveBeenCalledTimes(1)
    expect(locationAssign).toHaveBeenCalledWith('/login')
  })

  it('never retries a GET the caller cancelled', async () => {
    const error = { cancelled: true, config: { url: '/repos/1/archives/x/search', method: 'get' } }

    await expect(errorHandler(error)).rejects.toBe(error)

    expect(requestMock).not.toHaveBeenCalled()
  })

  it('never replays a write that failed without a response', async () => {
    const error = { config: { url: '/agents', method: 'post' } }

    await expect(errorHandler(error)).rejects.toBe(error)

    expect(requestMock).not.toHaveBeenCalled()
  })

  it('does not retry a GET that got an HTTP response', async () => {
    const error = { response: { status: 500 }, config: { url: '/agents', method: 'get' } }

    await expect(errorHandler(error)).rejects.toBe(error)

    expect(requestMock).not.toHaveBeenCalled()
  })

  it('redirects 401 responses to login', async () => {
    await expect(
      errorHandler({ response: { status: 401 }, config: { url: '/agents' } }),
    ).rejects.toEqual({
      response: { status: 401 },
      config: { url: '/agents' },
    })

    expect(locationAssign).toHaveBeenCalledWith('/login')
  })

  it('does not redirect non-401 responses', async () => {
    await expect(
      errorHandler({ response: { status: 500 }, config: { url: '/agents' } }),
    ).rejects.toEqual({
      response: { status: 500 },
      config: { url: '/agents' },
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
