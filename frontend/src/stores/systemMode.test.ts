// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { getSystemMode } from '../api/system'
import { useSystemModeStore } from './systemMode'

vi.mock('../api/system', () => ({
  getSystemMode: vi.fn(),
}))

const mockedGetSystemMode = vi.mocked(getSystemMode)

describe('system mode store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()
  })

  it('hides server-only routes once the server reports desktop mode', async () => {
    mockedGetSystemMode.mockResolvedValue({ mode: 'desktop' })
    const store = useSystemModeStore()

    await store.load()

    expect(store.isDesktop).toBe(true)
    expect(store.hidesRoute({ serverOnly: true })).toBe(true)
    expect(store.hidesRoute({ requiresAdmin: true })).toBe(false)
  })

  it('hides nothing in server mode', async () => {
    mockedGetSystemMode.mockResolvedValue({ mode: 'server' })
    const store = useSystemModeStore()

    await store.load()

    expect(store.isDesktop).toBe(false)
    expect(store.hidesRoute({ serverOnly: true })).toBe(false)
  })

  it('hides nothing before the mode is known', () => {
    const store = useSystemModeStore()

    expect(store.mode).toBeNull()
    expect(store.hidesRoute({ serverOnly: true })).toBe(false)
  })

  it('asks the server once, even for concurrent and repeated loads', async () => {
    mockedGetSystemMode.mockResolvedValue({ mode: 'desktop' })
    const store = useSystemModeStore()

    await Promise.all([store.load(), store.load()])
    await store.load()

    expect(mockedGetSystemMode).toHaveBeenCalledTimes(1)
  })

  it('falls back to showing everything on failure and retries on the next load', async () => {
    mockedGetSystemMode.mockRejectedValueOnce(new Error('offline'))
    mockedGetSystemMode.mockResolvedValueOnce({ mode: 'desktop' })
    const store = useSystemModeStore()

    await store.load()
    expect(store.mode).toBeNull()
    expect(store.hidesRoute({ serverOnly: true })).toBe(false)

    await store.load()
    expect(store.isDesktop).toBe(true)
    expect(mockedGetSystemMode).toHaveBeenCalledTimes(2)
  })
})
