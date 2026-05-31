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
import { logger } from '../utils/logger'
import {
  Plus,
  Clock,
  AlertTriangle,
  CheckCircle,
  AlertCircle,
  SlidersHorizontal,
} from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'

type ScheduleType = 'backup' | 'check' | 'verify'

interface ScheduleRow {
  id: number
  repo_id: number
  schedule_type: ScheduleType
  cron_expression: string
  enabled: boolean
  canary_enabled: boolean
  last_run_at: string | null
  next_run_at: string | null
  exclude_patterns: string[]
  ignore_global_excludes: boolean
  keep_daily: number
  keep_weekly: number
  keep_monthly: number
  keep_yearly: number
  compact_enabled: boolean
  pre_backup_commands: string
  post_backup_commands: string
  execution_mode: string
  on_failure: string
}

interface ClientRow {
  id: number
  hostname: string
  display_name: string | null
}

interface RepoRow {
  id: number
  name: string
  repo_path: string
  enabled: boolean
}

interface HealthEntry {
  hostname: string
  target_name: string
  last_status: string | null
  last_backup_at: string | null
  is_overdue: boolean
  last_error_message: string | null
  cron_expression: string | null
  schedule_enabled: boolean | null
}

const schedules = ref<ScheduleRow[]>([])
const repos = ref<RepoRow[]>([])
const clients = ref<ClientRow[]>([])
const health = ref<HealthEntry[]>([])
const loading = ref(false)
const error = ref<string | null>(null)
const router = useRouter()
const expandedError = ref<number | null>(null)

type SortField = 'client' | 'next_run' | 'last_run' | 'type'
type SortDir = 'asc' | 'desc'
type FilterStatus = 'all' | 'enabled' | 'disabled'
type FilterType = 'all' | 'backup' | 'check' | 'verify'
type FilterHealth = 'all' | 'overdue' | 'success' | 'failed'

const sortField = ref<SortField>('client')
const sortDir = ref<SortDir>('asc')
const filterStatus = ref<FilterStatus>('all')
const filterType = ref<FilterType>('all')
const filterText = ref('')
const filterHealth = ref<FilterHealth>(
  (() => {
    const q = useRoute().query.filter as string | undefined
    if (q === 'overdue' || q === 'success' || q === 'failed') return q
    return 'all'
  })(),
)

const { isMobile } = useMobile()
const showMobileFilters = ref(false)

const runNowLoading = ref<number | null>(null)

function scheduleTypeLabel(t: ScheduleType): string {
  switch (t) {
    case 'backup':
      return 'Backup'
    case 'check':
      return 'Integrity Check'
    case 'verify':
      return 'Verify (extract dry-run)'
  }
}

const repoMap = computed(() => {
  const m = new Map<number, RepoRow>()
  repos.value.forEach((r) => m.set(r.id, r))
  return m
})

interface EnrichedSchedule extends ScheduleRow {
  machine: ClientRow | null
  repo: RepoRow | null
  health: HealthEntry | null
}

const healthByRepo = computed(() => {
  const m = new Map<string, HealthEntry[]>()
  health.value.forEach((h) => {
    const entries = m.get(h.target_name) ?? []
    entries.push(h)
    m.set(h.target_name, entries)
  })
  return m
})

const enrichedSchedules = computed<EnrichedSchedule[]>(() =>
  schedules.value.map((s) => {
    const machine: ClientRow | null = null
    const repo: RepoRow | null = repoMap.value.get(s.repo_id) ?? null
    const entries = repo ? (healthByRepo.value.get(repo.name) ?? []) : []
    const healthEntry: HealthEntry | null =
      entries.find((h) => h.is_overdue) ??
      entries.find((h) => h.last_status === 'failed') ??
      entries[0] ??
      null
    return { ...s, machine, repo, health: healthEntry }
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
    list = list.filter((s) => s.health?.last_status === 'success')
  } else if (filterHealth.value === 'failed') {
    list = list.filter((s) => s.health?.last_status === 'failed')
  }

  if (filterText.value.trim()) {
    const q = filterText.value.toLowerCase()
    list = list.filter(
      (s) =>
        (s.machine?.hostname.toLowerCase().includes(q) ?? false) ||
        (s.machine?.display_name?.toLowerCase().includes(q) ?? false) ||
        (s.repo?.name.toLowerCase().includes(q) ?? false),
    )
  }

  list.sort((a, b) => {
    let cmp = 0
    switch (sortField.value) {
      case 'client':
        cmp = (a.machine?.hostname ?? '').localeCompare(b.machine?.hostname ?? '')
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

function statusClass(entry: HealthEntry | null): string {
  if (!entry) return ''
  if (entry.is_overdue) return 'status-overdue'
  switch (entry.last_status) {
    case 'success':
      return 'status-success'
    case 'warning':
      return 'status-warning'
    case 'failed':
      return 'status-failed'
    default:
      return ''
  }
}

function statusLabel(entry: HealthEntry | null): string {
  if (!entry) return ''
  if (entry.is_overdue) return 'Overdue'
  switch (entry.last_status) {
    case 'success':
      return 'Success'
    case 'warning':
      return 'Warning'
    case 'failed':
      return 'Failed'
    default:
      return 'No data'
  }
}

function toggleError(id: number): void {
  expandedError.value = expandedError.value === id ? null : id
}

async function fetchAll(): Promise<void> {
  loading.value = true
  error.value = null
  try {
    const [schRes, repoRes, machRes, healthRes] = await Promise.all([
      apiClient.get<ScheduleRow[]>('/schedules'),
      apiClient.get<RepoRow[]>('/repos'),
      apiClient.get<ClientRow[]>('/clients'),
      apiClient.get<HealthEntry[]>('/stats/health'),
    ])
    schedules.value = schRes.data
    repos.value = repoRes.data
    clients.value = machRes.data
    health.value = healthRes.data
  } catch {
    error.value = 'Failed to load schedules.'
  } finally {
    loading.value = false
  }
}

function navigateToSchedule(s: ScheduleRow): void {
  router.push(`/schedules/${s.id}`)
}

async function runNow(s: ScheduleRow): Promise<void> {
  runNowLoading.value = s.id
  try {
    await apiClient.post(`/schedules/${s.id}/run`)
  } catch (e: unknown) {
    error.value = extractError(e)
  } finally {
    runNowLoading.value = null
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
        placeholder="Filter by client or repo..."
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
          <option value="failed">Failed only</option>
          <option value="overdue">Overdue only</option>
        </select>
        <div class="sort-controls">
          <span class="sort-label">Sort:</span>
          <button
            class="btn btn-sm btn-ghost"
            :class="{ active: sortField === 'client' }"
            @click="toggleSort('client')"
          >
            Client {{ sortField === 'client' ? (sortDir === 'asc' ? '\u2191' : '\u2193') : '' }}
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
        :class="{ disabled: !s.enabled }"
        @click="navigateToSchedule(s)"
      >
        <div class="card-top">
          <div class="card-info">
            <span class="card-hostname">{{ s.repo?.name ?? `repo #${s.repo_id}` }}</span>
            <span class="card-repo">
              {{ s.execution_mode === 'sequential' ? 'Sequential' : 'Parallel' }}
            </span>
          </div>
          <div class="card-badges">
            <span
              v-if="s.health && (s.health.last_status || s.health.is_overdue)"
              class="health-badge"
              :class="statusClass(s.health)"
            >
              <CheckCircle
                v-if="s.health.last_status === 'success' && !s.health.is_overdue"
                :size="12"
              />
              <AlertTriangle
                v-else-if="s.health.last_status === 'warning' || s.health.is_overdue"
                :size="12"
              />
              <AlertCircle
                v-else-if="s.health.last_status === 'failed'"
                :size="12"
              />
              {{ statusLabel(s.health) }}
            </span>
            <span
              class="status-badge"
              :class="s.enabled ? 'status-online' : 'status-offline'"
            >
              {{ s.enabled ? 'Enabled' : 'Disabled' }}
            </span>
          </div>
        </div>
        <div class="card-meta">
          <span
            class="type-badge"
            :class="`type-${s.schedule_type ?? 'backup'}`"
          >
            {{ scheduleTypeLabel(s.schedule_type ?? 'backup') }}
          </span>
        </div>
        <div
          v-if="s.health?.last_error_message"
          class="card-error"
          @click.stop
        >
          <button
            class="error-toggle"
            @click="toggleError(s.id)"
          >
            <AlertCircle :size="12" />
            Last backup failed
            <span class="toggle-arrow">{{ expandedError === s.id ? '\u25B4' : '\u25BE' }}</span>
          </button>
          <pre
            v-if="expandedError === s.id"
            class="error-pre"
            >{{ s.health.last_error_message }}</pre
          >
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
          <button
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
  font-size: 0.875rem;
}

.toolbar {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  margin-bottom: 1.5rem;
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
  font-size: 0.875rem;
  cursor: pointer;
  position: relative;
  transition:
    color 0.15s,
    border-color 0.15s;
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
  font-size: 0.75rem;
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

.schedule-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem;
  cursor: pointer;
  transition:
    box-shadow 0.15s,
    border-color 0.15s;
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.schedule-card:hover {
  border-color: var(--accent);
  box-shadow: var(--shadow);
}

.schedule-card.disabled {
  opacity: 0.5;
}

.card-top {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 0.75rem;
}

.card-badges {
  display: flex;
  gap: 0.4rem;
  align-items: center;
  flex-shrink: 0;
}

.health-badge {
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
  padding: 0.15rem 0.5rem;
  border-radius: 999px;
  font-size: 0.65rem;
  font-weight: 600;
  letter-spacing: 0.02em;
}

.health-badge.status-success {
  background: var(--success-subtle);
  color: var(--success);
}

.health-badge.status-warning {
  background: var(--warning-subtle);
  color: var(--warning);
}

.health-badge.status-failed {
  background: var(--danger-subtle);
  color: var(--danger);
}

.health-badge.status-overdue {
  background: var(--warning-subtle);
  color: var(--warning);
}

.card-error {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.error-toggle {
  display: inline-flex;
  align-items: center;
  gap: 0.3rem;
  background: none;
  border: none;
  color: var(--danger);
  font-size: 0.75rem;
  font-weight: 500;
  cursor: pointer;
  padding: 0.2rem 0;
}

.error-toggle:hover {
  text-decoration: underline;
}

.toggle-arrow {
  font-size: 0.6rem;
  margin-left: 0.1rem;
}

.error-pre {
  background: var(--bg-input);
  border: 1px solid var(--danger-subtle);
  border-radius: var(--radius-sm);
  padding: 0.6rem 0.75rem;
  font-size: 0.72rem;
  font-family: var(--mono);
  color: var(--danger);
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 150px;
  overflow-y: auto;
  margin: 0;
}

.card-info {
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
  min-width: 0;
}

.card-hostname {
  font-weight: 600;
  font-family: var(--mono);
  font-size: 0.9rem;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.card-repo {
  font-size: 0.78rem;
  color: var(--text-muted);
  font-family: var(--mono);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.card-meta {
  display: flex;
  gap: 0.4rem;
}

.type-badge {
  display: inline-block;
  padding: 0.1rem 0.45rem;
  border-radius: 999px;
  font-size: 0.65rem;
  font-weight: 600;
  letter-spacing: 0.02em;
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

.stat-value {
  font-size: 0.85rem;
  font-weight: 600;
  color: var(--text-primary);
}

.stat-label {
  font-size: 0.7rem;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.card-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.25rem;
  margin-top: auto;
}
</style>
