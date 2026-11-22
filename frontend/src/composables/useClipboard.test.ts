// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { useClipboard } from './useClipboard'

describe('useClipboard', () => {
  const writeText = vi.fn().mockResolvedValue(undefined)

  beforeEach(() => {
    vi.useFakeTimers()
    writeText.mockClear()
    vi.stubGlobal('navigator', { clipboard: { writeText } })
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('writes the text and flags it as copied', async () => {
    const { copied, copy } = useClipboard()
    expect(copied.value).toBe(false)

    await copy('hello')

    expect(writeText).toHaveBeenCalledWith('hello')
    expect(copied.value).toBe(true)
  })

  it('resets the copied flag after the default timeout', async () => {
    const { copied, copy } = useClipboard()
    await copy('hello')

    vi.advanceTimersByTime(1999)
    expect(copied.value).toBe(true)

    vi.advanceTimersByTime(1)
    expect(copied.value).toBe(false)
  })

  it('honours a custom timeout', async () => {
    const { copied, copy } = useClipboard(500)
    await copy('hello')

    vi.advanceTimersByTime(499)
    expect(copied.value).toBe(true)

    vi.advanceTimersByTime(1)
    expect(copied.value).toBe(false)
  })
})
