// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

import { ref, type Ref } from 'vue'
import { listAgents } from '../api/agents'
import { listRepos } from '../api/repos'
import { listSchedules } from '../api/schedules'
import { logger } from '../utils/logger'
import type { ScopeOption } from '../components/ScopeSelector.vue'

/**
 * The repositories, hosts and schedules a notification channel can be
 * scoped to, labelled for `ScopeSelector`. Shared by the Notifications page's
 * add-channel wizard and a channel's own page, which both offer the same
 * choice.
 */
export function useScopeOptions(): {
  scopeRepos: Ref<ScopeOption[]>
  scopeAgents: Ref<ScopeOption[]>
  scopeSchedules: Ref<ScopeOption[]>
  scopeSearch: Ref<string>
  loadScopeOptions: () => Promise<void>
} {
  const scopeRepos = ref<ScopeOption[]>([])
  const scopeAgents = ref<ScopeOption[]>([])
  const scopeSchedules = ref<ScopeOption[]>([])
  const scopeSearch = ref('')

  async function loadScopeOptions(): Promise<void> {
    try {
      const [repoRows, agentRows, scheduleRows] = await Promise.all([
        listRepos(),
        listAgents(),
        listSchedules(),
      ])
      scopeRepos.value = repoRows.map((r) => ({ id: r.id, label: r.name }))
      scopeAgents.value = agentRows.map((c) => ({ id: c.id, label: c.display_name ?? c.hostname }))
      scopeSchedules.value = scheduleRows.map((s) => ({
        id: s.id,
        label: s.name || `Schedule #${s.id}`,
      }))
    } catch (e: unknown) {
      logger.error('loadScopeOptions failed', e)
    }
  }

  return { scopeRepos, scopeAgents, scopeSchedules, scopeSearch, loadScopeOptions }
}
