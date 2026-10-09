// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import type { logger as Logger } from './logger'
import type { clientLogBuffer as Buffer } from './clientLog'

describe('logger', () => {
  beforeEach(() => {
    vi.resetModules()
  })

  afterEach(() => {
    vi.restoreAllMocks()
  })

  /** Spies first: the logger binds the console methods when it loads. */
  async function load(): Promise<{
    logger: typeof Logger
    buffer: typeof Buffer
    spies: Record<'error' | 'warn' | 'debug', ReturnType<typeof vi.spyOn>>
  }> {
    const spies = {
      error: vi.spyOn(console, 'error').mockImplementation(() => {}),
      warn: vi.spyOn(console, 'warn').mockImplementation(() => {}),
      debug: vi.spyOn(console, 'debug').mockImplementation(() => {}),
    }
    const { logger } = await import('./logger')
    const { clientLogBuffer } = await import('./clientLog')
    clientLogBuffer.clear()
    return { logger, buffer: clientLogBuffer, spies }
  }

  it('still forwards every call, unredacted, to the console', async () => {
    const { logger, spies } = await load()
    const payload = { password: 'pw' }
    logger.error('a', payload)
    logger.warn('b')
    logger.debug('c')
    expect(spies.error).toHaveBeenCalledWith('a', payload)
    expect(spies.warn).toHaveBeenCalledWith('b')
    expect(spies.debug).toHaveBeenCalledWith('c')
  })

  it('records each call in the buffer at its level, redacted', async () => {
    const { logger, buffer } = await load()
    logger.error('login failed', { password: 'pw' })
    logger.warn('slow')
    logger.debug('detail')
    const entries = buffer.entries()
    expect(entries.map((e) => e.level)).toEqual(['error', 'warn', 'debug'])
    expect(entries[0]!.message).toBe('login failed {password: [REDACTED]}')
  })

  it("names the logger's caller as the source", async () => {
    const { logger, buffer } = await load()
    function someCaller(): void {
      logger.warn('from caller')
    }
    someCaller()
    expect(buffer.entries()[0]!.source).toContain('someCaller')
  })

  it('works as a bare callback, e.g. .catch(logger.error)', async () => {
    const { logger, buffer } = await load()
    await Promise.reject(new Error('rejected')).catch(logger.error)
    expect(buffer.entries()[0]!.message).toBe('Error: rejected')
    expect(buffer.entries()[0]!.stack).toContain('rejected')
  })
})
