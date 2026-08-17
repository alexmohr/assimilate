// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { onMounted, ref, watch, type Ref } from 'vue'
import { apiClient } from '../api/client'
import { logger } from '../utils/logger'

/**
 * Fetches a days+optional-repo filtered list from `endpoint`, refetching
 * whenever either filter changes. Shared by the dashboard's trend/stats
 * widgets, which all page the same `?days=&repo_id=` shape.
 */
export function useRangeFilteredFetch<T>(
  endpoint: string,
  selectedDays: Ref<number>,
  selectedRepoId: Ref<number | undefined>,
): { entries: Ref<T[]>; loading: Ref<boolean> } {
  const entries = ref<T[]>([]) as Ref<T[]>
  const loading = ref(true)

  async function fetchEntries(): Promise<void> {
    loading.value = true
    try {
      const params = new URLSearchParams({ days: String(selectedDays.value) })
      if (selectedRepoId.value !== undefined) {
        params.set('repo_id', String(selectedRepoId.value))
      }
      const response = await apiClient.get<T[]>(`${endpoint}?${params.toString()}`)
      entries.value = response.data
    } finally {
      loading.value = false
    }
  }

  onMounted(() => {
    fetchEntries().catch(logger.error)
  })

  watch([selectedDays, selectedRepoId], () => {
    fetchEntries().catch(logger.error)
  })

  return { entries, loading }
}
