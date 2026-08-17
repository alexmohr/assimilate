// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { describe, expect, it, vi, beforeEach } from 'vitest'
import { defineComponent, h, nextTick, ref } from 'vue'
import { mount } from '@vue/test-utils'
import { useRangeFilteredFetch } from './useRangeFilteredFetch'

vi.mock('../api/client', () => ({
  apiClient: { get: vi.fn() },
}))

vi.mock('../utils/logger', () => ({
  logger: { error: vi.fn() },
}))

import { apiClient } from '../api/client'

const mockGet = apiClient.get as ReturnType<typeof vi.fn>

interface Entry {
  id: number
}

function mountFetch(
  endpoint: string,
  initialDays = 30,
  initialRepoId: number | undefined = undefined,
): {
  days: ReturnType<typeof ref<number>>
  repoId: ReturnType<typeof ref<number | undefined>>
  entries: ReturnType<typeof useRangeFilteredFetch<Entry>>['entries']
  loading: ReturnType<typeof useRangeFilteredFetch<Entry>>['loading']
} {
  const days = ref(initialDays)
  const repoId = ref<number | undefined>(initialRepoId)
  let result: ReturnType<typeof useRangeFilteredFetch<Entry>> | undefined
  mount(
    defineComponent({
      setup() {
        result = useRangeFilteredFetch<Entry>(endpoint, days, repoId)
        return () => h('div')
      },
    }),
  )
  return { days, repoId, entries: result!.entries, loading: result!.loading }
}

beforeEach(() => {
  vi.clearAllMocks()
  mockGet.mockResolvedValue({ data: [{ id: 1 }] })
})

describe('useRangeFilteredFetch', () => {
  it('fetches on mount with the days param and starts loading true', async () => {
    const { entries, loading } = mountFetch('/stats/activity')

    expect(loading.value).toBe(true)
    await nextTick()
    await nextTick()

    expect(mockGet).toHaveBeenCalledWith('/stats/activity?days=30')
    expect(entries.value).toEqual([{ id: 1 }])
    expect(loading.value).toBe(false)
  })

  it('omits repo_id when undefined and includes it when set', async () => {
    const { repoId } = mountFetch('/stats/activity', 14, 7)
    await nextTick()
    await nextTick()

    expect(mockGet).toHaveBeenLastCalledWith('/stats/activity?days=14&repo_id=7')

    repoId.value = undefined
    await nextTick()
    await nextTick()

    expect(mockGet).toHaveBeenLastCalledWith('/stats/activity?days=14')
  })

  it('refetches when days or repoId change', async () => {
    const { days, repoId } = mountFetch('/stats/trends')
    await nextTick()
    await nextTick()
    expect(mockGet).toHaveBeenCalledTimes(1)

    days.value = 90
    await nextTick()
    await nextTick()
    expect(mockGet).toHaveBeenCalledTimes(2)
    expect(mockGet).toHaveBeenLastCalledWith('/stats/trends?days=90')

    repoId.value = 3
    await nextTick()
    await nextTick()
    expect(mockGet).toHaveBeenCalledTimes(3)
    expect(mockGet).toHaveBeenLastCalledWith('/stats/trends?days=90&repo_id=3')
  })

  it('resets loading to false even when the fetch fails', async () => {
    mockGet.mockRejectedValue(new Error('network down'))
    const { loading } = mountFetch('/stats/activity')

    await nextTick()
    await nextTick()

    expect(loading.value).toBe(false)
  })
})
