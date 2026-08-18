<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { apiClient } from '../api/client'
import { formatDateShort } from '../utils/format'
import { extractError } from '../utils/error'
import { useWebSocket } from '../composables/useWebSocket'
import { useMobile } from '../composables/useMobile'
import { useListSort } from '../composables/useListSort'
import { useToast } from '../composables/useToast'
import { useScheduleRun } from '../composables/useScheduleRun'
import { useAsyncAction } from '../composables/useAsyncAction'
import { logger } from '../utils/logger'
import { normalizeBackupStatus } from '../utils/backupStatus'
import { isAgentOffline, lastSeenText } from '../utils/agent'
import {
  scheduleRunStatus,
  scheduleIssuesFromEntries,
  withErrorTitles,
  type ScheduleHealthEntry,
} from '../utils/scheduleHealth'
import { Plus, Clock, SlidersHorizontal } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import SortControls from '../components/SortControls.vue'
import { type EntityIssue } from '../components/EntityStatusBadges.vue'
import ScheduleCard from '../components/ScheduleCard.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import type { AgentRow } from '../types/agent'
import type { ScheduleRow, ScheduleType } from '../types/schedule'
import type { Repo } from '../types/repo'

const schedules = ref<ScheduleRow[]>([])
const repos = ref<Repo[]>([])
const agents = ref<AgentRow[]>([])
const health = ref<ScheduleHealthEntry[]>([])
const { loading, error, run } = useAsyncAction('Failed to load schedules.')
const router = useRouter()
type SortField = 'agent' | 'next_run' | 'last_run' | 'type'

const SORT_OPTIONS: readonly { field: SortField; label: string }[] = [
  { field: 'agent', label: 'Agent' },
  { field: 'next_run', label: 'Next Run' },
  { field: 'last_run', label: 'Last Run' },
  { field: 'type', label: 'Type' },
]
type FilterStatus = 'all' | 'enabled' | 'disabled'
type FilterType = 'all' | 'backup' | 'check' | 'verify'
type FilterHealth = 'all' | 'overdue' | 'success' | 'warning' | 'failed'

const {
  field: sortField,
  direction: sortDir,
  toggle: toggleSort,
  sign: sortSign,
} = useListSort<SortField>('agent')
const filterStatus = ref<FilterStatus>('all')
const filterType = ref<FilterType>('all')
const filterText = ref('')
const filterHealth = ref<FilterHealth>(
  (() => {
    const q = useRoute().query.filter as string | undefined
    if (q === 'overdue' || q === 'success' || q === 'warning' || q === 'failed') return q
    return 'all'
  })(),
)

const { isMobile } = useMobile()
const showMobileFilters = ref(false)

const cancelLoading = ref<number | null>(null)
const toggleLoading = ref<number | null>(null)
const { success: toastSuccess, error: toastError } = useToast()
function scheduleTypeLabel(t: ScheduleType): string {
  switch (t) {
    case 'backup':
      return 'Backup'
    case 'check':
      return 'Integrity Check'
    case 'verify':
      return 'Verify'
  }
}

const { runNowLoading, runNow } = useScheduleRun(scheduleTypeLabel)

const repoMap = computed(() => {
  const m = new Map<number, Repo>()
  repos.value.forEach((r) => m.set(r.id, r))
  return m
})

interface EnrichedSchedule extends ScheduleRow {
  hostLabels: string[]
  repo: Repo | null
  health: ScheduleHealthEntry | null
  overdueEntries: ScheduleHealthEntry[]
  isRunning: boolean
}

const RUNNING_STATUSES = new Set(['pending', 'started'])

const agentMap = computed(() => {
  const map = new Map<string, AgentRow>()
  agents.value.forEach((agent) => map.set(agent.hostname, agent))
  return map
})

function hostLabel(hostname: string): string {
  const displayName = agentMap.value.get(hostname)?.display_name
  return displayName ? `${displayName} (${hostname})` : hostname
}

const healthBySchedule = computed(() => {
  const m = new Map<number, ScheduleHealthEntry[]>()
  health.value.forEach((h) => {
    const entries = m.get(h.schedule_id) ?? []
    entries.push(h)
    m.set(h.schedule_id, entries)
  })
  return m
})

const enrichedSchedules = computed<EnrichedSchedule[]>(() =>
  schedules.value.map((s) => {
    const hostLabels = s.target_hostnames.map(hostLabel)
    const repo: Repo | null = s.repo_id != null ? (repoMap.value.get(s.repo_id) ?? null) : null
    const entries = healthBySchedule.value.get(s.id) ?? []
    const healthEntry: ScheduleHealthEntry | null =
      entries.find((h) => h.is_overdue) ??
      entries.find(
        (h) => h.last_status !== null && normalizeBackupStatus(h.last_status) === 'failed',
      ) ??
      entries[0] ??
      null
    const overdueEntries = entries.filter((h) => h.is_overdue)
    const isRunning = entries.some(
      (h) => h.last_status != null && RUNNING_STATUSES.has(h.last_status),
    )
    return { ...s, hostLabels, repo, health: healthEntry, overdueEntries, isRunning }
  }),
)

const filteredSchedules = computed(() => {
  let list = [...enrichedSchedules.value]

  if (filterStatus.value === 'enabled') {
    list = list.filter((s) => s.enabled)
  } else if (filterStatus.value === 'disabled') {
    list = list.filter((s) => !s.enabled)
  }

  if (filterType.value !== 'all') {
    list = list.filter((s) => s.schedule_type === filterType.value)
  }

  if (filterHealth.value === 'overdue') {
    list = list.filter((s) => s.health?.is_overdue)
  } else if (filterHealth.value === 'success') {
    list = list.filter((s) => scheduleRunStatus(s.health) === 'success')
  } else if (filterHealth.value === 'warning') {
    list = list.filter((s) => scheduleRunStatus(s.health) === 'warning')
  } else if (filterHealth.value === 'failed') {
    list = list.filter((s) => scheduleRunStatus(s.health) === 'failed')
  }

  if (filterText.value.trim()) {
    const q = filterText.value.toLowerCase()
    list = list.filter(
      (s) =>
        (s.name?.toLowerCase().includes(q) ?? false) ||
        s.hostLabels.some((label) => label.toLowerCase().includes(q)) ||
        (s.repo?.name.toLowerCase().includes(q) ?? false),
    )
  }

  list.sort((a, b) => {
    let cmp = 0
    switch (sortField.value) {
      case 'agent':
        cmp = (a.hostLabels[0] ?? '').localeCompare(b.hostLabels[0] ?? '')
        break
      case 'next_run':
        cmp = (a.next_run_at ?? '').localeCompare(b.next_run_at ?? '')
        break
      case 'last_run':
        cmp = (a.last_run_at ?? '').localeCompare(b.last_run_at ?? '')
        break
      case 'type':
        cmp = a.schedule_type.localeCompare(b.schedule_type)
        break
    }
    return cmp * sortSign()
  })

  return list
})

// Enriches the shared chip set with SchedulesView-specific tooltips: this
// page aggregates across every host a schedule targets, so a bare "Overdue"
// or "Failed" chip benefits from a hover detail of which host and why.
// AgentDetailView's schedule cards are already scoped to a single host and
// use scheduleIssuesFromEntries directly without this enrichment.
function scheduleIssues(s: EnrichedSchedule): EntityIssue[] {
  const entries = healthBySchedule.value.get(s.id) ?? []
  const issues = withErrorTitles(scheduleIssuesFromEntries(entries, s.id, router), entries)
  return issues.map((issue) =>
    issue.key === 'overdue' ? { ...issue, title: overdueMessage(s.overdueEntries) } : issue,
  )
}

function connectivityNote(hostname: string): string {
  const agent = agentMap.value.get(hostname)
  if (!agent || !isAgentOffline(agent)) return ''
  return ` — agent offline (${lastSeenText(agent)})`
}

function overdueMessage(entries: ScheduleHealthEntry[]): string {
  return entries
    .map((h) => {
      const last = h.last_backup_at ? formatDateShort(h.last_backup_at) : 'never'
      return `${hostLabel(h.hostname)} — last backup: ${last}${connectivityNote(h.hostname)}`
    })
    .join('\n')
}

async function fetchAll(): Promise<void> {
  await run(async () => {
    const [schRes, repoRes, agentsRes, healthRes] = await Promise.all([
      apiClient.get<ScheduleRow[]>('/schedules'),
      apiClient.get<Repo[]>('/repos'),
      apiClient.get<AgentRow[]>('/agents'),
      apiClient.get<ScheduleHealthEntry[]>('/stats/health'),
    ])
    schedules.value = schRes.data
    repos.value = repoRes.data
    agents.value = agentsRes.data
    health.value = healthRes.data
  })
}

function navigateToSchedule(s: ScheduleRow): void {
  router.push(`/schedules/${s.id}`)
}

async function toggleScheduleEnabled(s: ScheduleRow): Promise<void> {
  const nextEnabled = !s.enabled
  toggleLoading.value = s.id
  try {
    const res = await apiClient.put<ScheduleRow>(`/schedules/${s.id}`, {
      name: s.name,
      cron_expression: s.cron_expression,
      enabled: nextEnabled,
      canary_enabled: s.canary_enabled,
      exclude_patterns_raw: s.exclude_patterns_raw,
      file_change_patterns_raw: s.file_change_patterns_raw,
      ignore_global_excludes: s.ignore_global_excludes,
      keep_hourly: s.keep_hourly,
      keep_daily: s.keep_daily,
      keep_weekly: s.keep_weekly,
      keep_monthly: s.keep_monthly,
      keep_yearly: s.keep_yearly,
      compact_enabled: s.compact_enabled,
      rate_limit_kbps: s.rate_limit_kbps,
      pre_backup_commands: s.pre_backup_commands,
      post_backup_commands: s.post_backup_commands,
      on_failure: s.on_failure,
    })
    const index = schedules.value.findIndex((row) => row.id === s.id)
    if (index !== -1) {
      schedules.value[index] = res.data
    }
    toastSuccess(nextEnabled ? 'Schedule enabled.' : 'Schedule disabled.')
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    toggleLoading.value = null
  }
}

async function cancelBackup(s: ScheduleRow): Promise<void> {
  cancelLoading.value = s.id
  try {
    await apiClient.post(`/schedules/${s.id}/cancel`)
    toastSuccess('Cancel request sent.')
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    cancelLoading.value = null
  }
}

onMounted(fetchAll)

const { onMessage } = useWebSocket()
onMessage('DataChanged', () => fetchAll().catch(logger.error))
</script>

<template>
  <div class="schedules-view">
    <div class="page-header">
      <h1 class="page-title">Schedules</h1>
      <div class="header-actions">
        <RouterLink
          to="/schedules/new"
          class="btn btn-primary"
        >
          <Plus :size="14" />
          New
        </RouterLink>
      </div>
    </div>

    <div
      v-if="error"
      class="error-banner"
    >
      {{ error }}
    </div>

    <div class="toolbar">
      <input
        v-model="filterText"
        class="input search-input"
        placeholder="Filter by name, agent, or repo..."
      />
      <button
        v-if="isMobile"
        class="filter-toggle"
        :class="{
          active: filterStatus !== 'all' || filterType !== 'all' || filterHealth !== 'all',
        }"
        @click="showMobileFilters = !showMobileFilters"
      >
        <SlidersHorizontal :size="14" />
        <span
          v-if="filterStatus !== 'all' || filterType !== 'all' || filterHealth !== 'all'"
          class="filter-badge"
        ></span>
      </button>
      <template v-if="!isMobile || showMobileFilters">
        <select
          v-model="filterStatus"
          class="input select-input select-input--sm"
        >
          <option value="all">All</option>
          <option value="enabled">Enabled</option>
          <option value="disabled">Disabled</option>
        </select>
        <select
          v-model="filterType"
          class="input select-input select-input--sm"
        >
          <option value="all">All types</option>
          <option value="backup">Backup</option>
          <option value="check">Check</option>
          <option value="verify">Verify</option>
        </select>
        <select
          v-model="filterHealth"
          class="input select-input select-input--sm"
        >
          <option value="all">All health</option>
          <option value="success">Passed only</option>
          <option value="warning">Warned only</option>
          <option value="failed">Failed only</option>
          <option value="overdue">Overdue only</option>
        </select>
        <SortControls
          :field="sortField"
          :direction="sortDir"
          :options="SORT_OPTIONS"
          @toggle="toggleSort"
        />
      </template>
    </div>

    <BaseSpinner
      v-if="loading && schedules.length === 0"
      size="lg"
    />

    <EmptyState
      v-else-if="enrichedSchedules.length === 0 && !loading"
      :icon="Clock"
      title="No schedules configured"
      description="Create a schedule to automate your backups."
      action="Create Schedule"
      @action="router.push('/schedules/new')"
    />

    <div
      v-else
      class="card-grid"
    >
      <ScheduleCard
        v-for="s in filteredSchedules"
        :key="s.id"
        :schedule="s"
        :issues="scheduleIssues(s)"
        :format-run="formatDateShort"
        :running="s.isRunning"
        spread-actions
        :data-schedule-id="s.id"
        @select="navigateToSchedule(s)"
      >
        <template #title>{{
          s.name || s.repo?.name || (s.repo_id != null ? `repo #${s.repo_id}` : 'no repository')
        }}</template>
        <template #meta>
          <span class="meta-pill">
            {{ s.target_hostnames.length }} agent{{ s.target_hostnames.length === 1 ? '' : 's' }}
          </span>
        </template>
        <template #actions>
          <div class="schedule-toggle">
            <ToggleSwitch
              :model-value="s.enabled"
              :disabled="toggleLoading === s.id"
              :label="s.enabled ? 'Disable schedule' : 'Enable schedule'"
              @update:model-value="toggleScheduleEnabled(s)"
            />
            <span class="schedule-toggle-label">{{ s.enabled ? 'Enabled' : 'Disabled' }}</span>
          </div>
          <button
            v-if="s.isRunning"
            class="btn btn-sm btn-danger"
            :disabled="cancelLoading === s.id"
            title="Cancel the running backup"
            @click="cancelBackup(s)"
          >
            {{ cancelLoading === s.id ? '...' : 'Cancel' }}
          </button>
          <button
            v-else
            class="btn btn-sm btn-ghost"
            :disabled="runNowLoading === s.id"
            :title="`Run ${scheduleTypeLabel(s.schedule_type ?? 'backup').toLowerCase()} now`"
            @click="runNow(s)"
          >
            {{ runNowLoading === s.id ? '...' : 'Run' }}
          </button>
        </template>
      </ScheduleCard>
    </div>
  </div>
</template>

<style scoped>
.schedules-view {
  max-width: 1100px;
  overflow-x: hidden;
  min-width: 0;
}

.schedule-toggle {
  display: flex;
  align-items: center;
  gap: 0.4rem;
}

.schedule-toggle-label {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-secondary);
}
</style>
