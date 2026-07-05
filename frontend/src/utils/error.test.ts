// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'

const { mockIsAxiosError } = vi.hoisted(() => ({
  mockIsAxiosError: vi.fn(),
}))

vi.mock('./logger', () => ({
  logger: { error: vi.fn() },
}))

vi.mock('axios', () => ({
  default: { isAxiosError: mockIsAxiosError },
  isAxiosError: mockIsAxiosError,
}))

const { extractBlobError, extractError } = await import('./error')

beforeEach(() => {
  vi.clearAllMocks()
})

describe('extractError', () => {
  it('extracts message from Error instance', () => {
    const result = extractError(new Error('something broke'))
    expect(result).toBe('something broke')
  })

  it('extracts message with context prefix', () => {
    const result = extractError(new Error('something broke'), 'Test context')
    expect(result).toBe('Test context: something broke')
  })

  it('extracts message from axios response with string data', () => {
    mockIsAxiosError.mockReturnValue(true)
    const result = extractError({
      isAxiosError: true,
      response: { status: 400, data: 'bad request' },
      message: 'Request failed',
      config: { url: '/test' },
    })
    expect(result).toContain('bad request')
  })

  it('extracts error_id from axios response with object data', () => {
    mockIsAxiosError.mockReturnValue(true)
    const result = extractError({
      isAxiosError: true,
      response: { status: 500, data: { error: 'server error', error_id: 'abc123' } },
      message: 'Request failed',
      config: { url: '/test' },
    })
    expect(result).toContain('server error')
    expect(result).toContain('ref: abc123')
  })

  it('falls back to axios message when there is no response (network error)', () => {
    mockIsAxiosError.mockReturnValue(true)
    const result = extractError({
      isAxiosError: true,
      message: 'Network Error',
      config: { url: '/test' },
    })
    expect(result).toContain('Network Error')
  })

  it('falls back to axios message when response has no data', () => {
    mockIsAxiosError.mockReturnValue(true)
    const result = extractError({
      isAxiosError: true,
      response: { status: 500 },
      message: 'Request failed with status code 500',
      config: { url: '/test' },
    })
    expect(result).toContain('Request failed with status code 500')
  })
})

function createAxiosError(data: unknown, message = 'Request failed'): Record<string, unknown> {
  return {
    isAxiosError: true,
    response: { status: 500, data },
    message,
    config: { url: '/test' },
  }
}

describe('extractBlobError', () => {
  it('decodes a Blob response body with JSON error and error_id', async () => {
    const blob = new Blob([JSON.stringify({ error: 'archive not found', error_id: 'e123' })], {
      type: 'application/json',
    })
    mockIsAxiosError.mockReturnValue(true)

    const result = await extractBlobError(createAxiosError(blob))

    expect(result).toContain('archive not found')
    expect(result).toContain('ref: e123')
  })

  it('falls through to extractError for non-Blob axios errors', async () => {
    mockIsAxiosError.mockReturnValue(true)

    const result = await extractBlobError(createAxiosError({ error: 'bad request' }))

    expect(result).toContain('bad request')
  })

  it('falls through to extractError for non-axios errors', async () => {
    mockIsAxiosError.mockReturnValue(false)

    const result = await extractBlobError(new Error('generic error'))

    expect(result).toBe('generic error')
  })

  it('falls back to axios message when there is no response (network error)', async () => {
    mockIsAxiosError.mockReturnValue(true)

    const result = await extractBlobError({
      isAxiosError: true,
      message: 'Network Error',
      config: { url: '/test' },
    })

    expect(result).toContain('Network Error')
  })

  it('decodes a Blob with non-JSON body', async () => {
    const blob = new Blob(['Internal server error'], { type: 'text/plain' })
    mockIsAxiosError.mockReturnValue(true)

    const result = await extractBlobError(createAxiosError(blob))

    expect(result).toContain('Internal server error')
  })

  it('falls back to axios message when Blob text is empty', async () => {
    const blob = new Blob([''], { type: 'text/plain' })
    mockIsAxiosError.mockReturnValue(true)

    const result = await extractBlobError(
      createAxiosError(blob, 'Request failed with status code 500'),
    )

    expect(result).toContain('Request failed with status code 500')
  })

  it('handles Blob with JSON number body, falling back to axios message', async () => {
    const blob = new Blob(['42'], { type: 'application/json' })
    mockIsAxiosError.mockReturnValue(true)

    const result = await extractBlobError(
      createAxiosError(blob, 'Request failed with status code 500'),
    )

    expect(result).toContain('Request failed with status code 500')
  })

  it('handles Blob with JSON boolean body, falling back to axios message', async () => {
    const blob = new Blob(['true'], { type: 'application/json' })
    mockIsAxiosError.mockReturnValue(true)

    const result = await extractBlobError(
      createAxiosError(blob, 'Request failed with status code 500'),
    )

    expect(result).toContain('Request failed with status code 500')
  })

  it('handles Blob with JSON array body, falling back to axios message', async () => {
    const blob = new Blob(['[1, 2, 3]'], { type: 'application/json' })
    mockIsAxiosError.mockReturnValue(true)

    const result = await extractBlobError(
      createAxiosError(blob, 'Request failed with status code 500'),
    )

    expect(result).toContain('Request failed with status code 500')
  })

  it('handles Blob with nested JSON object (error not at top level)', async () => {
    const blob = new Blob([JSON.stringify({ data: { error: 'deep error' } })], {
      type: 'application/json',
    })
    mockIsAxiosError.mockReturnValue(true)

    const result = await extractBlobError(
      createAxiosError(blob, 'Request failed with status code 500'),
    )

    expect(result).toContain('Request failed with status code 500')
  })
})
