// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect } from 'vitest'
import { effectScope } from 'vue'
import { ClientLogBuffer } from '../utils/clientLog'
import { useClientLogs } from './useClientLogs'

describe('useClientLogs', () => {
  it('starts with the existing entries, newest first', () => {
    const buffer = new ClientLogBuffer()
    buffer.record('debug', ['old'])
    buffer.record('debug', ['new'])
    const { entries, stop } = useClientLogs(buffer)
    expect(entries.value.map((e) => e.message)).toEqual(['new', 'old'])
    stop()
  })

  it('follows new entries and clears', () => {
    const buffer = new ClientLogBuffer()
    const { entries, clear, stop } = useClientLogs(buffer)
    buffer.record('warn', ['live'])
    expect(entries.value.map((e) => e.message)).toEqual(['live'])
    clear()
    expect(entries.value).toEqual([])
    expect(buffer.size).toBe(0)
    stop()
  })

  it('unsubscribes when its scope is disposed', () => {
    const buffer = new ClientLogBuffer()
    const scope = effectScope()
    const result = scope.run(() => useClientLogs(buffer))!
    expect(buffer.listenerCount).toBe(1)
    scope.stop()
    expect(buffer.listenerCount).toBe(0)
    buffer.record('debug', ['after'])
    expect(result.entries.value).toEqual([])
  })
})
