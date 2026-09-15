// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, it, expect, beforeEach } from 'vitest'
import { nextTick, ref } from 'vue'
import { usePersistedRef, usePersistedBoolean, useQueryOverride } from './usePersistedRef'

type Mode = 'time' | 'agent' | 'repo'
function isMode(value: string): value is Mode {
  return value === 'time' || value === 'agent' || value === 'repo'
}

describe('usePersistedRef', () => {
  beforeEach(() => {
    localStorage.clear()
  })

  it('starts from the given initial value when nothing is stored', () => {
    const mode = usePersistedRef('assimilate-test-mode', 'time', isMode)
    expect(mode.value).toBe('time')
  })

  it('restores a previously persisted value', () => {
    localStorage.setItem('assimilate-test-mode', 'repo')
    const mode = usePersistedRef('assimilate-test-mode', 'time', isMode)
    expect(mode.value).toBe('repo')
  })

  it('falls back to initial when storage holds a value the type no longer allows', () => {
    localStorage.setItem('assimilate-test-mode', 'host')
    const mode = usePersistedRef('assimilate-test-mode', 'time', isMode)
    expect(mode.value).toBe('time')
  })

  it('writes every change back to storage', async () => {
    const mode = usePersistedRef('assimilate-test-mode', 'time', isMode)
    mode.value = 'agent'
    await nextTick()
    expect(localStorage.getItem('assimilate-test-mode')).toBe('agent')
  })

  it('prefers a valid override over both storage and initial', () => {
    localStorage.setItem('assimilate-test-mode', 'repo')
    const mode = usePersistedRef('assimilate-test-mode', 'time', isMode, 'agent')
    expect(mode.value).toBe('agent')
  })

  it('writes a valid override back to storage immediately, not just on the next change', () => {
    localStorage.setItem('assimilate-test-mode', 'repo')
    usePersistedRef('assimilate-test-mode', 'time', isMode, 'agent')
    expect(localStorage.getItem('assimilate-test-mode')).toBe('agent')
  })

  it('ignores an invalid override and falls through to storage', () => {
    localStorage.setItem('assimilate-test-mode', 'repo')
    const mode = usePersistedRef('assimilate-test-mode', 'time', isMode, 'bogus')
    expect(mode.value).toBe('repo')
  })

  it('ignores a null override', () => {
    localStorage.setItem('assimilate-test-mode', 'repo')
    const mode = usePersistedRef('assimilate-test-mode', 'time', isMode, null)
    expect(mode.value).toBe('repo')
  })
})

describe('usePersistedBoolean', () => {
  beforeEach(() => {
    localStorage.clear()
  })

  it('starts from the given initial value when nothing is stored', () => {
    const flag = usePersistedBoolean('assimilate-test-flag', false)
    expect(flag.value).toBe(false)
  })

  it('restores a persisted true', () => {
    localStorage.setItem('assimilate-test-flag', 'true')
    const flag = usePersistedBoolean('assimilate-test-flag', false)
    expect(flag.value).toBe(true)
  })

  it('restores a persisted false, overriding a truthy initial', () => {
    localStorage.setItem('assimilate-test-flag', 'false')
    const flag = usePersistedBoolean('assimilate-test-flag', true)
    expect(flag.value).toBe(false)
  })

  it('writes every change back to storage', async () => {
    const flag = usePersistedBoolean('assimilate-test-flag', false)
    flag.value = true
    await nextTick()
    expect(localStorage.getItem('assimilate-test-flag')).toBe('true')
  })
})

describe('useQueryOverride', () => {
  it('ignores the query being absent at setup', async () => {
    const query = ref<string | undefined>(undefined)
    const target = usePersistedRef('assimilate-test-mode', 'time', isMode)
    useQueryOverride(() => query.value, isMode, target, 'time')
    await nextTick()
    expect(target.value).toBe('time')
  })

  it('applies a valid query value that arrives after setup', async () => {
    const query = ref<string | undefined>(undefined)
    const target = usePersistedRef('assimilate-test-mode', 'time', isMode)
    useQueryOverride(() => query.value, isMode, target, 'time')

    query.value = 'agent'
    await nextTick()
    expect(target.value).toBe('agent')
  })

  it('falls back when an invalid query value arrives', async () => {
    const query = ref<string | undefined>(undefined)
    const target = usePersistedRef('assimilate-test-mode', 'repo', isMode)
    useQueryOverride(() => query.value, isMode, target, 'time')

    query.value = 'bogus'
    await nextTick()
    expect(target.value).toBe('time')
  })

  it('leaves the target alone once the query goes back to absent', async () => {
    const query = ref<string | undefined>('agent')
    const target = usePersistedRef('assimilate-test-mode', 'time', isMode)
    useQueryOverride(() => query.value, isMode, target, 'time')

    query.value = 'repo'
    await nextTick()
    expect(target.value).toBe('repo')

    query.value = undefined
    await nextTick()
    expect(target.value).toBe('repo')
  })
})
