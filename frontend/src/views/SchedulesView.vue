<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { apiClient } from '../api/client'
import { formatDateShort } from '../utils/format'
import { cronToHuman } from '../utils/cron'
import { extractError } from '../utils/error'
import { useWebSocket } from '../composables/useWebSocket'
import { useMobile } from '../composables/useMobile'
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
import EntityStatusBadges, { type EntityIssue } from '../components/EntityStatusBadges.vue'
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
type SortDir = 'asc' | 'desc'
type FilterStatus = 'all' | 'enabled' | 'disabled'
type FilterType = 'all' | 'backup' | 'check' | 'verify'
type FilterHealth = 'all' | 'overdue' | 'success' | 'warning' | 'failed'

const sortField = ref<SortField>('agent')
const sortDir = ref<SortDir>('asc')
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
    return sortDir.value === 'desc' ? -cmp : cmp
  })

  return list
})

function toggleSort(field: SortField): void {
  if (sortField.value === field) {
    sortDir.value = sortDir.value === 'asc' ? 'desc' : 'asc'
  } else {
    sortField.value = field
    sortDir.value = 'asc'
  }
}

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
        class="btn-filter-toggle"
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
          class="input select-input"
        >
          <option value="all">All</option>
          <option value="enabled">Enabled</option>
          <option value="disabled">Disabled</option>
        </select>
        <select
          v-model="filterType"
          class="input select-input"
        >
          <option value="all">All types</option>
          <option value="backup">Backup</option>
          <option value="check">Check</option>
          <option value="verify">Verify</option>
        </select>
        <select
          v-model="filterHealth"
          class="input select-input"
        >
          <option value="all">All health</option>
          <option value="success">Passed only</option>
          <option value="warning">Warned only</option>
          <option value="failed">Failed only</option>
          <option value="overdue">Overdue only</option>
        </select>
        <div class="sort-controls">
          <span class="sort-label">Sort:</span>
          <button
            class="btn btn-sm btn-ghost"
            :class="{ active: sortField === 'agent' }"
            @click="toggleSort('agent')"
          >
            Agent {{ sortField === 'agent' ? (sortDir === 'asc' ? '\u2191' : '\u2193') : '' }}
          </button>
          <button
            class="btn btn-sm btn-ghost"
            :class="{ active: sortField === 'next_run' }"
            @click="toggleSort('next_run')"
          >
            Next Run
            {{ sortField === 'next_run' ? (sortDir === 'asc' ? '\u2191' : '\u2193') : '' }}
          </button>
          <button
            class="btn btn-sm btn-ghost"
            :class="{ active: sortField === 'last_run' }"
            @click="toggleSort('last_run')"
          >
            Last Run
            {{ sortField === 'last_run' ? (sortDir === 'asc' ? '\u2191' : '\u2193') : '' }}
          </button>
          <button
            class="btn btn-sm btn-ghost"
            :class="{ active: sortField === 'type' }"
            @click="toggleSort('type')"
          >
            Type {{ sortField === 'type' ? (sortDir === 'asc' ? '\u2191' : '\u2193') : '' }}
          </button>
        </div>
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
      class="schedule-grid"
    >
      <div
        v-for="s in filteredSchedules"
        :key="s.id"
        class="schedule-card"
        :class="{ 'schedule-card-notable': !s.enabled }"
        :data-schedule-id="s.id"
        @click="navigateToSchedule(s)"
      >
        <span class="card-hostname">{{
          s.name || s.repo?.name || (s.repo_id != null ? `repo #${s.repo_id}` : 'no repository')
        }}</span>
        <EntityStatusBadges
          :notable="!s.enabled"
          notable-label="Disabled"
          :running="s.isRunning"
          running-label="Running"
          :issues="scheduleIssues(s)"
        />
        <div class="card-meta">
          <span class="host-count">
            {{ s.target_hostnames.length }} agent{{ s.target_hostnames.length === 1 ? '' : 's' }}
          </span>
          <span
            class="badge badge--neutral"
            :class="`type-${s.schedule_type ?? 'backup'}`"
          >
            {{ scheduleTypeLabel(s.schedule_type ?? 'backup') }}
          </span>
        </div>
        <div class="card-stats">
          <div class="stat">
            <span class="stat-value">{{
              cronToHuman(s.cron_expression) ?? s.cron_expression
            }}</span>
            <span class="stat-label">Schedule</span>
          </div>
          <div class="stat">
            <span class="stat-value">{{ formatDateShort(s.next_run_at) }}</span>
            <span class="stat-label">Next run</span>
          </div>
          <div class="stat">
            <span class="stat-value">{{ formatDateShort(s.last_run_at) }}</span>
            <span class="stat-label">Last run</span>
          </div>
        </div>
        <div
          class="card-actions"
          @click.stop
        >
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
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.schedules-view {
  max-width: 1100px;
  overflow-x: hidden;
  min-width: 0;
}

.error-banner {
  background: var(--danger-subtle);
  border: 1px solid var(--danger);
  color: var(--danger);
  padding: 0.75rem 1rem;
  border-radius: var(--radius-sm);
  margin-bottom: 1rem;
  font-size: var(--fs-base);
}

.toolbar {
  flex-wrap: wrap;
}

.search-input {
  width: 220px;
}

.select-input {
  width: auto;
  min-width: 100px;
}

.btn-filter-toggle {
  display: flex;
  align-items: center;
  gap: 0.35rem;
  padding: 0.4rem 0.6rem;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg-input);
  color: var(--text-secondary);
  font-size: var(--fs-base);
  cursor: pointer;
  position: relative;
  transition:
    color var(--duration-base),
    border-color var(--duration-base);
}

.btn-filter-toggle:hover {
  color: var(--text-primary);
  border-color: var(--text-muted);
}

.btn-filter-toggle.active {
  border-color: var(--accent);
  color: var(--accent);
}

.filter-badge {
  position: absolute;
  top: -3px;
  right: -3px;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--accent);
}

.sort-controls {
  display: flex;
  align-items: center;
  gap: 0.35rem;
  margin-left: auto;
  overflow-x: auto;
  flex-shrink: 0;
}

.sort-label {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  margin-right: 0.25rem;
}

.sort-controls .btn.active {
  background: var(--bg-hover);
  color: var(--text-primary);
  font-weight: 600;
}

.schedule-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(320px, 100%), 1fr));
  gap: 1rem;
}

.card-hostname {
  font-weight: 600;
  font-family: var(--mono);
  font-size: var(--fs-md);
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.card-meta {
  display: flex;
  gap: 0.4rem;
  flex-wrap: wrap;
}

.host-count {
  display: inline-block;
  padding: 0.1rem 0.45rem;
  border-radius: var(--radius-pill);
  font-size: var(--fs-2xs);
  font-weight: 600;
  letter-spacing: 0.02em;
  background: var(--bg-card);
  color: var(--text-secondary);
}

.type-backup {
  background: var(--success-subtle);
  color: var(--success);
}

.type-check {
  background: var(--accent-subtle);
  color: var(--accent);
}

.type-verify {
  background: var(--warning-subtle);
  color: var(--warning);
}

.card-stats {
  display: flex;
  gap: 1.25rem;
}

.stat {
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
}

.card-actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  margin-top: auto;
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
