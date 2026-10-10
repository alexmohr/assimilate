// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi } from 'vitest'
import {
  ClientLogBuffer,
  DEFAULT_CLIENT_LOG_CAPACITY,
  MAX_CLIENT_LOG_CAPACITY,
  callerFrame,
  captureGlobalErrors,
  formatClientLogs,
} from './clientLog'
import { REDACTED } from './redact'

function messages(buffer: ClientLogBuffer): string[] {
  return buffer.entries().map((e) => e.message)
}

describe('ClientLogBuffer', () => {
  it('defaults to the documented capacity', () => {
    expect(new ClientLogBuffer().capacity).toBe(DEFAULT_CLIENT_LOG_CAPACITY)
  })

  it('records level, message, source and an ISO timestamp', () => {
    const buffer = new ClientLogBuffer(5)
    const entry = buffer.record('warn', ['disk', 'low'], 'fn (app.js:1:2)')
    expect(entry.level).toBe('warn')
    expect(entry.message).toBe('disk low')
    expect(entry.source).toBe('fn (app.js:1:2)')
    expect(entry.stack).toBeNull()
    expect(new Date(entry.timestamp).toISOString()).toBe(entry.timestamp)
  })

  it('evicts the oldest entry once full', () => {
    const buffer = new ClientLogBuffer(3)
    for (const m of ['a', 'b', 'c', 'd', 'e']) buffer.record('debug', [m])
    expect(buffer.size).toBe(3)
    expect(messages(buffer)).toEqual(['c', 'd', 'e'])
  })

  it('keeps ids increasing across evictions and clears', () => {
    const buffer = new ClientLogBuffer(2)
    const ids = ['a', 'b', 'c'].map((m) => buffer.record('debug', [m]).id)
    buffer.clear()
    ids.push(buffer.record('debug', ['d']).id)
    expect(ids).toEqual([1, 2, 3, 4])
  })

  it('stores redacted text, never the logged object', () => {
    const buffer = new ClientLogBuffer()
    const payload = { username: 'admin', password: 'hunter2' }
    const err = new Error('request failed Authorization: Bearer abc')
    buffer.record('error', ['login', payload, err])
    const [entry] = buffer.entries()
    expect(entry!.message).not.toContain('hunter2')
    expect(entry!.message).not.toContain('abc')
    expect(entry!.message).toContain(REDACTED)
    expect(entry!.stack).not.toContain('abc')
    // Mutating the original afterwards does not reach the buffer.
    payload.username = 'changed'
    expect(buffer.entries()[0]!.message).toContain('admin')
  })

  it('returns a copy from entries()', () => {
    const buffer = new ClientLogBuffer()
    buffer.record('debug', ['a'])
    buffer.entries().pop()
    expect(buffer.size).toBe(1)
  })

  it('clears every entry', () => {
    const buffer = new ClientLogBuffer()
    buffer.record('debug', ['a'])
    buffer.clear()
    expect(buffer.size).toBe(0)
    expect(buffer.entries()).toEqual([])
  })

  it('keeps the newest entries when shrunk and all of them when grown', () => {
    const buffer = new ClientLogBuffer(4)
    for (const m of ['a', 'b', 'c', 'd', 'e']) buffer.record('debug', [m])
    buffer.setCapacity(2)
    expect(messages(buffer)).toEqual(['d', 'e'])
    buffer.setCapacity(10)
    buffer.record('debug', ['f'])
    expect(messages(buffer)).toEqual(['d', 'e', 'f'])
  })

  it('clamps the capacity to a sane range', () => {
    expect(new ClientLogBuffer(0).capacity).toBe(1)
    expect(new ClientLogBuffer(Number.MAX_SAFE_INTEGER).capacity).toBe(MAX_CLIENT_LOG_CAPACITY)
    expect(new ClientLogBuffer(Number.NaN).capacity).toBe(DEFAULT_CLIENT_LOG_CAPACITY)
    expect(new ClientLogBuffer(2.7).capacity).toBe(2)
  })

  it('notifies subscribers and stops after unsubscribe', () => {
    const buffer = new ClientLogBuffer()
    const listener = vi.fn()
    const stop = buffer.subscribe(listener)
    buffer.record('debug', ['a'])
    buffer.clear()
    expect(listener).toHaveBeenCalledTimes(2)
    stop()
    expect(buffer.listenerCount).toBe(0)
    buffer.record('debug', ['b'])
    expect(listener).toHaveBeenCalledTimes(2)
  })

  it('survives a listener that throws or logs from inside the callback', () => {
    const buffer = new ClientLogBuffer()
    buffer.subscribe(() => {
      throw new Error('listener broke')
    })
    const reentrant = vi.fn(() => {
      buffer.record('debug', ['from listener'])
    })
    buffer.subscribe(reentrant)
    expect(() => buffer.record('error', ['a'])).not.toThrow()
    // The nested record lands in the buffer but does not notify again.
    expect(reentrant).toHaveBeenCalledTimes(1)
    expect(messages(buffer)).toEqual(['a', 'from listener'])
  })
})

describe('callerFrame', () => {
  const v8 = [
    'Error',
    '    at record (http://host/assets/logger.js:1:10)',
    '    at Object.error (http://host/assets/logger.js:1:50)',
    '    at fetchLogs (http://host/assets/ActivityLogView.js:9:3)',
  ].join('\n')

  it('skips the logger frames of a V8 stack and drops the origin', () => {
    expect(callerFrame(v8, 2, 'http://host')).toBe('fetchLogs (/assets/ActivityLogView.js:9:3)')
  })

  it('reads a Gecko/WebKit stack', () => {
    const gecko =
      'record@http://host/a.js:1:1\nerror@http://host/a.js:2:2\nload@http://host/b.js:3:3'
    expect(callerFrame(gecko, 2, 'http://host')).toBe('load@/b.js:3:3')
  })

  it('returns an empty string when the frame is missing', () => {
    expect(callerFrame(undefined, 2)).toBe('')
    expect(callerFrame('Error', 2)).toBe('')
  })
})

describe('formatClientLogs', () => {
  it('writes one line per entry, followed by its stack', () => {
    const text = formatClientLogs([
      {
        id: 1,
        timestamp: '2026-01-01T00:00:00.000Z',
        level: 'error',
        message: 'boom',
        source: 'f (a.js:1:1)',
        stack: 'Error: boom\n    at f',
      },
      {
        id: 2,
        timestamp: '2026-01-01T00:00:01.000Z',
        level: 'debug',
        message: 'quiet',
        source: '',
        stack: null,
      },
    ])
    expect(text).toBe(
      '[2026-01-01T00:00:00.000Z] ERROR f (a.js:1:1): boom\nError: boom\n    at f\n' +
        '[2026-01-01T00:00:01.000Z] DEBUG: quiet',
    )
  })
})

describe('captureGlobalErrors', () => {
  it('records uncaught errors and rejections, and removes both listeners', () => {
    const target = new EventTarget()
    const buffer = new ClientLogBuffer()
    const remove = captureGlobalErrors(target as unknown as Window, buffer)

    target.dispatchEvent(
      new ErrorEvent('error', {
        error: new Error('uncaught token=abc'),
        message: 'uncaught',
        filename: 'app.js',
        lineno: 3,
        colno: 4,
      }),
    )
    const rejection = new Event('unhandledrejection') as PromiseRejectionEvent
    Object.defineProperty(rejection, 'reason', { value: { password: 'pw' } })
    target.dispatchEvent(rejection)

    const [uncaught, unhandled] = buffer.entries()
    expect(uncaught!.message).toBe(`Uncaught Error: uncaught token=${REDACTED}`)
    expect(uncaught!.source).toBe('app.js:3:4')
    expect(uncaught!.stack).not.toBeNull()
    expect(unhandled!.message).toBe(`Unhandled rejection {password: ${REDACTED}}`)

    remove()
    target.dispatchEvent(new ErrorEvent('error', { message: 'after' }))
    expect(buffer.size).toBe(2)
  })

  it('falls back to the event message when there is no error object', () => {
    const target = new EventTarget()
    const buffer = new ClientLogBuffer()
    captureGlobalErrors(target as unknown as Window, buffer)
    target.dispatchEvent(new ErrorEvent('error', { message: 'Script error.' }))
    expect(buffer.entries()[0]!.message).toBe('Uncaught Script error.')
    expect(buffer.entries()[0]!.source).toBe('')
  })
})
