<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { Search, SlidersHorizontal, Activity } from '@lucide/vue'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import { apiClient } from '../api/client'
import { useWebSocket } from '../composables/useWebSocket'
import { useMobile } from '../composables/useMobile'
import { formatDuration, formatBytes, formatDateShort } from '../utils/format'
import { logger } from '../utils/logger'
import type { ReportRow } from '../types/report'

interface ActivityRow {
  id: number
  hostname: string
  target_name: string
  started_at: string
  finished_at: string
  status: string
  duration_secs: number
  schedule_id: number | null
  schedule_name: string | null
  run_id: string | null
}

interface ScheduleOption {
  id: number
  name: string
}

interface SystemEvent {
  id: number
  created_at: string
  event_type: string
  hostname: string | null
  message: string
}


interface Agent {
  id: number
  hostname: string
}

interface LogEntry {
  timestamp: string
  level: string
  target: string
  message: string
}

type CategoryFilter = 'all' | 'backup' | 'system' | 'logs'
type StatusFilter = 'all' | 'success' | 'warning' | 'failed' | 'started' | 'pending'
type LogLevel = '' | 'error' | 'warn' | 'info' | 'debug' | 'trace'

const rows = ref<ActivityRow[]>([])
const systemEvents = ref<SystemEvent[]>([])
const agents = ref<Agent[]>([])
const schedules = ref<ScheduleOption[]>([])
const loading = ref(false)
const loadingMore = ref(false)
const expandedId = ref<number | null>(null)
const expandedDetail = ref<ReportRow | null>(null)
const expandedLoading = ref(false)
const expandedSystemId = ref<number | null>(null)
const offset = ref(0)
const hasMore = ref(true)
const PAGE_SIZE = 50

const activeCategory = ref<CategoryFilter>('all')
const filterMachine = ref('')
const filterTarget = ref('all')
const filterStatus = ref<StatusFilter>('all')
const filterFrom = ref('')
const filterTo = ref('')
const filterScheduleId = ref<number | null>(null)
const filterRunId = ref<string | null>(null)

const logEntries = ref<LogEntry[]>([])
const logLevel = ref<LogLevel>('')
const logSearch = ref('')
const loadingLogs = ref(false)
let logSearchTimer: ReturnType<typeof setTimeout> | null = null

const { isMobile } = useMobile()
const showMobileFilters = ref(false)
const route = useRoute()
const router = useRouter()

const availableTargets = computed(() => {
  const targets = new Set(rows.value.map((r) => r.target_name))
  return [...targets].sort()
})

const hasActiveFilters = computed((): boolean => {
  if (activeCategory.value === 'logs') {
    return logLevel.value !== '' || logSearch.value !== ''
  }
  return (
    filterMachine.value !== '' ||
    filterTarget.value !== 'all' ||
    filterStatus.value !== 'all' ||
    filterFrom.value !== '' ||
    filterTo.value !== '' ||
    filterScheduleId.value !== null ||
    filterRunId.value !== null
  )
})

onMounted(async () => {
  const catParam = route.query.category as string | undefined
  if (catParam === 'all' || catParam === 'backup' || catParam === 'system' || catParam === 'logs') {
    activeCategory.value = catParam
  }
  const targetParam = route.query.target as string | undefined
  if (targetParam) {
    filterTarget.value = targetParam
  }
  const hostnameParam = route.query.hostname as string | undefined
  if (hostnameParam) {
    filterMachine.value = hostnameParam
    activeCategory.value = 'backup'
  }
  const statusParam = route.query.status as string | undefined
  if (statusParam === 'success' || statusParam === 'warning' || statusParam === 'failed') {
    filterStatus.value = statusParam
    activeCategory.value = 'backup'
  }
  const daysParam = route.query.days as string | undefined
  if (daysParam) {
    const days = Number(daysParam)
    if (days > 0) {
      const from = new Date()
      from.setDate(from.getDate() - days)
      filterFrom.value = from.toISOString().slice(0, 10)
    }
  }
  const scheduleIdParam = route.query.schedule_id as string | undefined
  if (scheduleIdParam) {
    const id = Number(scheduleIdParam)
    if (id > 0) {
      filterScheduleId.value = id
      activeCategory.value = 'backup'
    }
  }
  const runIdParam = route.query.run_id as string | undefined
  if (runIdParam) {
    filterRunId.value = runIdParam
    activeCategory.value = 'backup'
  }
  await Promise.all([fetchMachines(), fetchSchedules(), fetchData(true)])
})

interface LiveBackupSession {
  hostname: string
  target_name: string
  lines: string[]
}

const MAX_ACTIVITY_LOG_LINES = 200
const liveBackupSessions = ref<Map<string, LiveBackupSession>>(new Map())

function liveSessionKey(hostname: string, target: string): string {
  return `${hostname}::${target}`
}

const { onMessage } = useWebSocket()
onMessage('DataChanged', () => fetchData(true).catch(logger.error))
onMessage('AgentConnected', () => fetchData(true).catch(logger.error))
onMessage('AgentDisconnected', () => fetchData(true).catch(logger.error))

onMessage<{ hostname: string; target_name: string }>('BackupStarted', (payload) => {
  const key = liveSessionKey(payload.hostname, payload.target_name)
  const next = new Map(liveBackupSessions.value)
  next.set(key, { hostname: payload.hostname, target_name: payload.target_name, lines: [] })
  liveBackupSessions.value = next
})

onMessage<{ hostname: string; target_name: string }>('BackupCompleted', (payload) => {
  const key = liveSessionKey(payload.hostname, payload.target_name)
  const next = new Map(liveBackupSessions.value)
  next.delete(key)
  liveBackupSessions.value = next
})

onMessage<{ hostname: string; repo_id: number; schedule_id: number | null; line: string }>(
  'BackupLog',
  (payload) => {
    try {
      const obj = JSON.parse(payload.line) as Record<string, unknown>
      if (obj['type'] === 'archive_progress') return
    } catch {
      // non-JSON line — show it
    }
    const sessions = new Map(liveBackupSessions.value)
    for (const [key, session] of sessions) {
      if (session.hostname === payload.hostname) {
        sessions.set(key, {
          ...session,
          lines: [...session.lines.slice(-(MAX_ACTIVITY_LOG_LINES - 1)), payload.line],
        })
        break
      }
    }
    liveBackupSessions.value = sessions
  },
)

const activeLiveSessions = computed<LiveBackupSession[]>(() =>
  [...liveBackupSessions.value.values()].filter((s) => s.lines.length > 0),
)

watch(activeCategory, (cat) => {
  router.replace({ query: { ...route.query, category: cat } }).catch(() => {})
  if (cat === 'logs') {
    fetchLogs().catch(logger.error)
  } else {
    fetchData(true).catch(logger.error)
  }
})

watch(logLevel, () => {
  if (activeCategory.value === 'logs') fetchLogs().catch(logger.error)
})

watch(logSearch, () => {
  if (logSearchTimer) clearTimeout(logSearchTimer)
  logSearchTimer = setTimeout(() => {
    if (activeCategory.value === 'logs') fetchLogs().catch(logger.error)
  }, 300)
})

watch(filterScheduleId, () => {
  if (activeCategory.value !== 'logs') fetchData(true).catch(logger.error)
})

watch(filterRunId, () => {
  if (activeCategory.value !== 'logs') fetchData(true).catch(logger.error)
})

async function fetchMachines(): Promise<void> {
  const res = await apiClient.get<Agent[]>('/agents')
  agents.value = res.data
}

async function fetchSchedules(): Promise<void> {
  const res = await apiClient.get<ScheduleOption[]>('/schedules')
  schedules.value = res.data
}

async function fetchLogs(): Promise<void> {
  loadingLogs.value = true
  try {
    const params: Record<string, string | number> = { limit: 500 }
    if (logLevel.value) params.level = logLevel.value
    if (logSearch.value) params.search = logSearch.value
    const res = await apiClient.get<LogEntry[]>('/logs', { params })
    logEntries.value = res.data
  } catch (e: unknown) {
    logger.error('fetchLogs failed', e)
    logEntries.value = []
  } finally {
    loadingLogs.value = false
  }
}

async function fetchData(reset: boolean): Promise<void> {
  if (activeCategory.value === 'logs') return

  if (reset) {
    loading.value = true
    offset.value = 0
    rows.value = []
    systemEvents.value = []
    expandedId.value = null
    expandedDetail.value = null
    expandedSystemId.value = null
  } else {
    loadingMore.value = true
  }

  try {
    const limit = PAGE_SIZE + offset.value
    const cat = activeCategory.value

    if (cat === 'backup' || cat === 'all') {
      const activityParams: Record<string, string | number> = { limit }
      if (filterScheduleId.value !== null) activityParams.schedule_id = filterScheduleId.value
      if (filterRunId.value !== null) activityParams.run_id = filterRunId.value
      const res = await apiClient.get<ActivityRow[]>('/stats/activity', {
        params: activityParams,
      })
      rows.value = res.data
    }

    if (cat === 'system' || cat === 'all') {
      const res = await apiClient.get<SystemEvent[]>('/stats/system-events', {
        params: { limit },
      })
      systemEvents.value = res.data
    }

    const totalFetched =
      cat === 'all'
        ? Math.max(rows.value.length, systemEvents.value.length)
        : cat === 'backup'
          ? rows.value.length
          : systemEvents.value.length
    hasMore.value = totalFetched >= limit
    offset.value += PAGE_SIZE
  } finally {
    loading.value = false
    loadingMore.value = false
  }
}

async function loadMore(): Promise<void> {
  await fetchData(false)
}

function toggleSystemRow(event: SystemEvent): void {
  expandedSystemId.value = expandedSystemId.value === event.id ? null : event.id
}

async function toggleRow(row: ActivityRow): Promise<void> {
  if (expandedId.value === row.id) {
    expandedId.value = null
    expandedDetail.value = null
    return
  }
  expandedId.value = row.id
  expandedDetail.value = null
  expandedLoading.value = true
  try {
    const res = await apiClient.get<ReportRow[]>(`/agents/${row.hostname}/reports`, {
      params: { limit: 100, target: row.target_name },
    })
    const match = res.data.find(
      (r) => r.started_at === row.started_at || r.duration_secs === row.duration_secs,
    )
    expandedDetail.value = match ?? res.data[0] ?? null
  } finally {
    expandedLoading.value = false
  }
}

const filtered = computed(() => {
  return rows.value.filter((r) => {
    if (filterMachine.value && r.hostname !== filterMachine.value) {
      return false
    }
    if (filterTarget.value !== 'all' && r.target_name !== filterTarget.value) {
      return false
    }
    if (filterStatus.value !== 'all') {
      const s = r.status.toLowerCase()
      if (filterStatus.value === 'success' && s !== 'success') return false
      if (filterStatus.value === 'warning' && s !== 'warning') return false
      if (filterStatus.value === 'failed' && s !== 'failed' && s !== 'error') return false
      if (filterStatus.value === 'started' && s !== 'started') return false
      if (filterStatus.value === 'pending' && s !== 'pending') return false
    }
    if (filterFrom.value) {
      if (new Date(r.started_at) < new Date(filterFrom.value)) return false
    }
    if (filterTo.value) {
      if (new Date(r.started_at) > new Date(filterTo.value + 'T23:59:59')) return false
    }
    return true
  })
})

interface UnifiedRow {
  kind: 'backup' | 'system'
  id: number
  timestamp: string
  hostname: string | null
  backup?: ActivityRow
  event?: SystemEvent
}

const unifiedRows = computed<UnifiedRow[]>(() => {
  const cat = activeCategory.value

  if (cat === 'backup') {
    return filtered.value.map((r) => ({
      kind: 'backup' as const,
      id: r.id,
      timestamp: r.started_at,
      hostname: r.hostname,
      backup: r,
    }))
  }

  if (cat === 'system') {
    return systemEvents.value.map((e) => ({
      kind: 'system' as const,
      id: e.id + 1_000_000,
      timestamp: e.created_at,
      hostname: e.hostname,
      event: e,
    }))
  }

  const backupRows: UnifiedRow[] = filtered.value.map((r) => ({
    kind: 'backup' as const,
    id: r.id,
    timestamp: r.started_at,
    hostname: r.hostname,
    backup: r,
  }))

  const eventRows: UnifiedRow[] = systemEvents.value.map((e) => ({
    kind: 'system' as const,
    id: e.id + 1_000_000,
    timestamp: e.created_at,
    hostname: e.hostname,
    event: e,
  }))

  return [...backupRows, ...eventRows].sort(
    (a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime(),
  )
})

function statusClass(status: string): string {
  const s = status.toLowerCase()
  if (s === 'success') return 'badge-success'
  if (s === 'warning') return 'badge-warning'
  if (s === 'started') return 'badge-started'
  if (s === 'pending') return 'badge-pending'
  return 'badge-failed'
}

function eventTypeClass(eventType: string): string {
  switch (eventType) {
    case 'repo_sync':
    case 'agent_connected':
    case 'backup_complete':
      return 'badge-success'
    case 'repo_sync_slow':
    case 'backup_warning':
    case 'agent_disconnected':
      return 'badge-warning'
    case 'repo_sync_failed':
    case 'backup_failed':
    case 'auth_failed':
    case 'error':
      return 'badge-failed'
    default:
      return 'badge-started'
  }
}

function logRowClass(entry: LogEntry): string {
  return `log-entry-row log-level-${entry.level.toLowerCase()}`
}

function clearFilters(): void {
  activeCategory.value = 'all'
  filterMachine.value = ''
  filterTarget.value = 'all'
  filterStatus.value = 'all'
  filterFrom.value = ''
  filterTo.value = ''
  filterScheduleId.value = null
  filterRunId.value = null
  logLevel.value = ''
  logSearch.value = ''
}

function filterByRun(runId: string): void {
  filterRunId.value = runId
  filterScheduleId.value = null
  activeCategory.value = 'backup'
  fetchData(true).catch(logger.error)
}
</script>

<template>
  <div class="activity-log">
    <div class="page-header">
      <h1 class="page-title">Activity Log</h1>
      <div class="header-actions">
        <span class="row-count">{{
          activeCategory === 'logs'
            ? `${logEntries.length} entries`
            : `${unifiedRows.length} entries`
        }}</span>
      </div>
    </div>

    <div
      v-if="activeLiveSessions.length > 0 && activeCategory !== 'logs'"
      class="live-sessions"
    >
      <div
        v-for="session in activeLiveSessions"
        :key="liveSessionKey(session.hostname, session.target_name)"
        class="live-session-card"
      >
        <div class="live-session-header">
          <span class="live-session-pulse" />
          <span class="live-session-title">Live backup output</span>
          <span class="live-session-meta">{{ session.hostname }} → {{ session.target_name }}</span>
        </div>
        <div class="live-session-output">
          <div
            v-for="(line, i) in session.lines"
            :key="i"
            class="live-session-line"
          >
            {{ line }}
          </div>
        </div>
      </div>
    </div>

    <section class="filters">
      <div class="filter-row">
        <div class="filter-group">
          <label class="filter-label">Type</label>
          <div class="segment-group">
            <button
              class="segment-btn"
              :class="{ active: activeCategory === 'all' }"
              @click="activeCategory = 'all'"
            >
              All
            </button>
            <button
              class="segment-btn"
              :class="{ active: activeCategory === 'backup' }"
              @click="activeCategory = 'backup'"
            >
              Backup
            </button>
            <button
              class="segment-btn"
              :class="{ active: activeCategory === 'system' }"
              @click="activeCategory = 'system'"
            >
              System
            </button>
            <button
              class="segment-btn"
              :class="{ active: activeCategory === 'logs' }"
              @click="activeCategory = 'logs'"
            >
              Server Logs
            </button>
          </div>
        </div>

        <button
          v-if="isMobile"
          class="btn-filter-toggle"
          :class="{ active: hasActiveFilters }"
          @click="showMobileFilters = !showMobileFilters"
        >
          <SlidersHorizontal :size="14" />
          Filters
          <span
            v-if="hasActiveFilters"
            class="filter-badge"
          ></span>
        </button>

        <template v-if="!isMobile || showMobileFilters">
          <template v-if="activeCategory !== 'logs'">
            <div class="filter-group">
              <label class="filter-label">Machine</label>
              <select
                v-model="filterMachine"
                class="select-input"
              >
                <option value="">All Machines</option>
                <option
                  v-for="m in agents"
                  :key="m.hostname"
                  :value="m.hostname"
                >
                  {{ m.hostname }}
                </option>
              </select>
            </div>

            <div class="filter-group">
              <label class="filter-label">Schedule</label>
              <select
                v-model="filterScheduleId"
                class="select-input"
              >
                <option :value="null">All Schedules</option>
                <option
                  v-for="s in schedules"
                  :key="s.id"
                  :value="s.id"
                >
                  {{ s.name }}
                </option>
              </select>
            </div>

            <div
              v-if="filterRunId !== null"
              class="filter-group"
            >
              <label class="filter-label">Run</label>
              <div class="run-id-filter">
                <span class="run-id-label">{{ filterRunId.slice(0, 8) }}...</span>
                <button
                  class="btn-clear-run"
                  title="Clear run filter"
                  @click="filterRunId = null"
                >
                  ✕
                </button>
              </div>
            </div>

            <div class="filter-group">
              <label class="filter-label">Target</label>
              <select
                v-model="filterTarget"
                class="select-input"
              >
                <option value="all">All</option>
                <option
                  v-for="t in availableTargets"
                  :key="t"
                  :value="t"
                >
                  {{ t }}
                </option>
              </select>
            </div>

            <div class="filter-group">
              <label class="filter-label">Status</label>
              <select
                v-model="filterStatus"
                class="select-input"
              >
                <option value="all">All</option>
                <option value="success">Success</option>
                <option value="warning">Warning</option>
                <option value="failed">Failed</option>
                <option value="started">Started</option>
                <option value="pending">Pending</option>
              </select>
            </div>

            <div class="filter-group">
              <label class="filter-label">From</label>
              <input
                v-model="filterFrom"
                type="date"
                class="date-input"
              />
            </div>

            <div class="filter-group">
              <label class="filter-label">To</label>
              <input
                v-model="filterTo"
                type="date"
                class="date-input"
              />
            </div>
          </template>

          <template v-if="activeCategory === 'logs'">
            <div class="filter-group">
              <label class="filter-label">Level</label>
              <select
                v-model="logLevel"
                class="select-input"
              >
                <option value="">All</option>
                <option value="error">Error</option>
                <option value="warn">Warn</option>
                <option value="info">Info</option>
                <option value="debug">Debug</option>
                <option value="trace">Trace</option>
              </select>
            </div>

            <div class="filter-group filter-group-search">
              <label class="filter-label">Search</label>
              <div class="search-input-wrap">
                <Search
                  :size="14"
                  class="search-icon"
                />
                <input
                  v-model="logSearch"
                  type="text"
                  class="search-input"
                  placeholder="Filter messages..."
                />
              </div>
            </div>
          </template>

          <button
            class="btn-clear"
            @click="clearFilters"
          >
            Clear
          </button>
        </template>
      </div>
    </section>

    <template v-if="activeCategory === 'logs'">
      <div
        v-if="loadingLogs"
        class="loading"
      >
        Loading server logs...
      </div>

      <div
        v-else-if="logEntries.length === 0"
        class="state-msg"
      >
        No log entries match the current filters.
      </div>

      <div
        v-else
        class="log-panel"
      >
        <DataTable
          :value="logEntries"
          :row-class="logRowClass"
          table-class="log-table log-table-mono"
        >
          <Column header="Timestamp">
            <template #body="{ data }">
              <span class="cell-ts cell-mono">{{ formatDateShort(data.timestamp) }}</span>
            </template>
          </Column>
          <Column header="Level">
            <template #body="{ data }">
              <span
                class="badge badge-level"
                :class="`badge-${data.level.toLowerCase()}`"
              >
                {{ data.level }}
              </span>
            </template>
          </Column>
          <Column header="Target">
            <template #body="{ data }">
              <span class="cell-target-log cell-mono">{{ data.target }}</span>
            </template>
          </Column>
          <Column header="Message">
            <template #body="{ data }">
              <span class="cell-msg-log">{{ data.message }}</span>
            </template>
          </Column>
          <template #empty>
            <div class="state-msg">No log entries match the current filters.</div>
          </template>
        </DataTable>
      </div>
    </template>

    <template v-else>
      <BaseSpinner
        v-if="loading"
        size="lg"
      />

      <EmptyState
        v-else-if="unifiedRows.length === 0"
        :icon="Activity"
        title="No activity"
        description="Backup activity will appear here once backups run."
      />

      <div
        v-else
        class="table-wrap"
      >
        <table class="log-table">
          <thead>
            <tr>
              <th>Timestamp</th>
              <th>Machine</th>
              <th>Target / Event</th>
              <th>Status</th>
              <th>Duration</th>
            </tr>
          </thead>
          <tbody>
            <template
              v-for="row in unifiedRows"
              :key="row.id"
            >
              <tr
                v-if="row.kind === 'backup' && row.backup"
                class="log-row"
                :class="{ expanded: expandedId === row.backup.id }"
                @click="toggleRow(row.backup)"
              >
                <td class="cell-ts">
                  {{ formatDateShort(row.backup.started_at) }}
                </td>
                <td class="cell-host">
                  {{ row.backup.hostname }}
                </td>
                <td class="cell-target">
                  <span>{{ row.backup.target_name }}</span>
                  <span
                    v-if="row.backup.schedule_name"
                    class="schedule-label"
                    >{{ row.backup.schedule_name }}</span
                  >
                </td>
                <td>
                  <span
                    class="badge"
                    :class="statusClass(row.backup.status)"
                    >{{ row.backup.status }}</span
                  >
                </td>
                <td class="cell-dur">
                  <span>{{ formatDuration(row.backup.duration_secs) }}</span>
                  <button
                    v-if="row.backup.run_id && filterRunId !== row.backup.run_id"
                    class="btn-run-filter"
                    title="View all events for this run"
                    @click.stop="filterByRun(row.backup.run_id)"
                  >
                    View run
                  </button>
                </td>
              </tr>
              <tr
                v-if="row.kind === 'backup' && row.backup && expandedId === row.backup.id"
                class="detail-row"
              >
                <td colspan="5">
                  <div class="detail-panel">
                    <div
                      v-if="expandedLoading"
                      class="detail-loading"
                    >
                      Loading details...
                    </div>
                    <div
                      v-else-if="expandedDetail"
                      class="detail-grid"
                    >
                      <div class="detail-section">
                        <h3 class="detail-heading">Timing</h3>
                        <dl class="detail-dl">
                          <dt>Started</dt>
                          <dd>{{ formatDateShort(expandedDetail.started_at) }}</dd>
                          <dt>Finished</dt>
                          <dd>{{ formatDateShort(expandedDetail.finished_at) }}</dd>
                          <dt>Duration</dt>
                          <dd>{{ formatDuration(expandedDetail.duration_secs) }}</dd>
                        </dl>
                      </div>
                      <div class="detail-section">
                        <h3 class="detail-heading">Sizes</h3>
                        <dl class="detail-dl">
                          <dt>Original</dt>
                          <dd>{{ formatBytes(expandedDetail.original_size) }}</dd>
                          <dt>Compressed</dt>
                          <dd>{{ formatBytes(expandedDetail.compressed_size) }}</dd>
                          <dt>Deduplicated</dt>
                          <dd>{{ formatBytes(expandedDetail.deduplicated_size) }}</dd>
                        </dl>
                      </div>
                      <div class="detail-section">
                        <h3 class="detail-heading">Stats</h3>
                        <dl class="detail-dl">
                          <dt>Files processed</dt>
                          <dd>{{ expandedDetail.files_processed.toLocaleString() }}</dd>
                          <dt>Borg version</dt>
                          <dd>{{ expandedDetail.borg_version ?? '—' }}</dd>
                        </dl>
                      </div>
                      <div
                        v-if="expandedDetail.borg_command"
                        class="detail-section detail-command-section"
                      >
                        <h3 class="detail-heading">Command</h3>
                        <pre class="command-pre">{{ expandedDetail.borg_command }}</pre>
                      </div>
                      <div
                        v-if="expandedDetail.warnings && expandedDetail.warnings.length > 0"
                        class="detail-section detail-warning-section"
                      >
                        <h3 class="detail-heading warning-heading">Warnings</h3>
                        <pre class="warning-pre">{{ expandedDetail.warnings.join('\n') }}</pre>
                      </div>
                      <div
                        v-if="expandedDetail.error_message"
                        class="detail-section detail-error-section"
                      >
                        <h3 class="detail-heading error-heading">Error</h3>
                        <pre class="error-pre">{{ expandedDetail.error_message }}</pre>
                      </div>
                    </div>
                    <div
                      v-else
                      class="detail-loading"
                    >
                      No detail available.
                    </div>
                  </div>
                </td>
              </tr>

              <tr
                v-if="row.kind === 'system' && row.event"
                class="log-row row-system"
                :class="{ expanded: expandedSystemId === row.event.id }"
                @click="toggleSystemRow(row.event)"
              >
                <td class="cell-ts">
                  {{ formatDateShort(row.event.created_at) }}
                </td>
                <td class="cell-host">
                  {{ row.event.hostname ?? '—' }}
                </td>
                <td class="cell-target cell-message">
                  {{ row.event.message }}
                </td>
                <td>
                  <span
                    class="badge"
                    :class="eventTypeClass(row.event.event_type)"
                    >{{ row.event.event_type }}</span
                  >
                </td>
                <td class="cell-dur">—</td>
              </tr>
              <tr
                v-if="row.kind === 'system' && row.event && expandedSystemId === row.event.id"
                class="detail-row"
              >
                <td colspan="5">
                  <div class="detail-panel">
                    <pre class="error-pre">{{ row.event.message }}</pre>
                  </div>
                </td>
              </tr>
            </template>
          </tbody>
        </table>
      </div>

      <div
        v-if="!loading && hasMore && unifiedRows.length > 0"
        class="load-more"
      >
        <button
          class="btn-load-more"
          :disabled="loadingMore"
          @click="loadMore"
        >
          {{ loadingMore ? 'Loading...' : 'Load More' }}
        </button>
      </div>
    </template>
  </div>
</template>

<style scoped>
.activity-log {
  display: flex;
  flex-direction: column;
  gap: 1.25rem;
  color: var(--text-primary);
}

.row-count {
  font-size: 0.85rem;
  color: var(--text-muted);
}

.filters {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1rem 1.25rem;
  display: flex;
  flex-direction: column;
  gap: 0.875rem;
}

.filter-row {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  gap: 1rem;
}

.filter-group {
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
}

.filter-label {
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-muted);
}

.segment-group {
  display: flex;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  overflow: hidden;
}

.segment-btn {
  padding: 0.4rem 0.75rem;
  border: none;
  border-right: 1px solid var(--border);
  background: var(--bg-input);
  color: var(--text-secondary);
  font-size: 0.875rem;
  cursor: pointer;
  white-space: nowrap;
  transition:
    background 0.15s,
    color 0.15s;
}

.segment-btn:last-child {
  border-right: none;
}

.segment-btn.active {
  background: var(--accent);
  color: #fff;
}

.segment-btn:hover:not(.active) {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.select-input,
.date-input {
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  padding: 0.4rem 0.6rem;
  font-size: 0.875rem;
  outline: none;
  transition: border-color 0.15s;
}

.select-input:focus,
.date-input:focus {
  border-color: var(--accent);
}

.btn-clear {
  padding: 0.4rem 0.9rem;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: transparent;
  color: var(--text-secondary);
  font-size: 0.875rem;
  cursor: pointer;
  transition:
    color 0.15s,
    border-color 0.15s;
  align-self: flex-end;
}

.btn-clear:hover {
  color: var(--text-primary);
  border-color: var(--text-muted);
}

.btn-filter-toggle {
  display: flex;
  align-items: center;
  gap: 0.35rem;
  padding: 0.4rem 0.75rem;
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

.loading,
.state-msg {
  text-align: center;
  padding: 3rem;
  color: var(--text-muted);
  font-size: 0.95rem;
}

.table-wrap {
  overflow-x: auto;
  border-radius: var(--radius);
  border: 1px solid var(--border);
}

.log-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 0.875rem;
}

.log-table thead tr {
  background: var(--bg-card);
}

.log-table th {
  padding: 0.75rem 1rem;
  text-align: left;
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-muted);
  border-bottom: 1px solid var(--border);
}

.log-row {
  cursor: pointer;
  transition: background 0.1s;
  border-bottom: 1px solid var(--border-subtle);
}

.log-row:hover {
  background: var(--bg-hover);
}

.log-row.expanded {
  background: var(--bg-hover);
}

.log-row td {
  padding: 0.7rem 1rem;
  vertical-align: middle;
}

.cell-ts {
  color: var(--text-muted);
  white-space: nowrap;
}

.cell-host {
  font-weight: 600;
  color: var(--text-primary);
}

.cell-target {
  color: var(--text-secondary);
}

.cell-dur {
  color: var(--text-muted);
  white-space: nowrap;
}

.badge {
  display: inline-block;
  padding: 0.2rem 0.6rem;
  border-radius: 999px;
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: capitalize;
}

.badge-success {
  background: var(--success-subtle);
  color: var(--success);
}

.badge-warning {
  background: var(--warning-subtle);
  color: var(--warning);
}

.badge-failed {
  background: var(--danger-subtle);
  color: var(--danger);
}

.badge-started {
  background: var(--info-subtle);
  color: var(--info);
}

.badge-pending {
  background: color-mix(in srgb, var(--text-muted) 15%, transparent);
  color: var(--text-muted);
}

.detail-row td {
  padding: 0;
  background: var(--bg-base);
}

.detail-panel {
  padding: 1.25rem 1.5rem;
  border-top: 1px solid var(--border);
  border-bottom: 1px solid var(--border);
}

.detail-loading {
  color: var(--text-muted);
  font-size: 0.875rem;
}

.detail-grid {
  display: flex;
  flex-wrap: wrap;
  gap: 1.5rem;
}

.detail-section {
  min-width: 180px;
}

.detail-error-section,
.detail-warning-section {
  flex: 1 1 100%;
}

.detail-heading {
  margin: 0 0 0.5rem;
  font-size: 0.75rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
}

.error-heading {
  color: var(--danger);
}

.warning-heading {
  color: var(--warning);
}

.detail-dl {
  margin: 0;
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 0.2rem 0.75rem;
}

.detail-dl dt {
  color: var(--text-muted);
  font-size: 0.8rem;
  white-space: nowrap;
}

.detail-dl dd {
  margin: 0;
  color: var(--text-primary);
  font-size: 0.8rem;
}

.error-pre {
  margin: 0;
  padding: 0.75rem 1rem;
  background: var(--danger-subtle);
  border: 1px solid var(--danger);
  border-radius: var(--radius-sm);
  color: var(--danger);
  font-size: 0.8rem;
  white-space: pre-wrap;
  word-break: break-word;
}

.warning-pre {
  margin: 0;
  padding: 0.75rem 1rem;
  background: var(--warning-subtle);
  border: 1px solid var(--warning);
  border-radius: var(--radius-sm);
  color: var(--warning);
  font-size: 0.8rem;
  white-space: pre-wrap;
  word-break: break-word;
}

.detail-command-section {
  flex: 1 1 100%;
}

.command-pre {
  margin: 0;
  padding: 0.75rem 1rem;
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: 0.8rem;
  white-space: pre-wrap;
  word-break: break-all;
  font-family: monospace;
}

.load-more {
  display: flex;
  justify-content: center;
  padding: 0.5rem 0;
}

.btn-load-more {
  padding: 0.6rem 2rem;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg-card);
  color: var(--text-primary);
  font-size: 0.875rem;
  cursor: pointer;
  transition:
    background 0.15s,
    border-color 0.15s;
}

.btn-load-more:hover:not(:disabled) {
  background: var(--bg-hover);
  border-color: var(--text-muted);
}

.btn-load-more:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.row-system {
  background: var(--danger-subtle);
}

.row-system:hover {
  background: var(--danger-subtle);
  opacity: 0.9;
}

.cell-message {
  font-size: 0.8rem;
  color: var(--text-secondary);
  max-width: 300px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.filter-group-search {
  flex: 1;
  min-width: 180px;
}

.search-input-wrap {
  position: relative;
  display: flex;
  align-items: center;
}

.search-icon {
  position: absolute;
  left: 0.5rem;
  color: var(--text-muted);
  pointer-events: none;
}

.search-input {
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  padding: 0.4rem 0.6rem 0.4rem 1.75rem;
  font-size: 0.875rem;
  outline: none;
  width: 100%;
  transition: border-color 0.15s;
}

.search-input:focus {
  border-color: var(--accent);
}

.log-panel {
  overflow-x: auto;
}

.log-table-mono {
  font-family: 'SF Mono', 'Cascadia Code', 'Fira Code', monospace;
  font-size: 0.8rem;
}

.log-entry-row {
  border-bottom: 1px solid var(--border-subtle);
}

.log-entry-row td {
  padding: 0.4rem 0.75rem;
  vertical-align: top;
}

.log-level-error {
  background: color-mix(in srgb, var(--danger) 6%, transparent);
}

.log-level-warn {
  background: color-mix(in srgb, var(--warning) 6%, transparent);
}

.cell-mono {
  font-family: 'SF Mono', 'Cascadia Code', 'Fira Code', monospace;
  font-size: 0.8rem;
  white-space: nowrap;
}

.cell-target-log {
  color: var(--text-muted);
  white-space: nowrap;
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
}

.cell-msg-log {
  color: var(--text-primary);
  word-break: break-word;
}

.col-ts {
  width: 140px;
}

.col-lvl {
  width: 70px;
}

.col-target {
  width: 200px;
}

.col-msg {
  width: auto;
}

.badge-level {
  font-size: 0.65rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.03em;
  padding: 0.15rem 0.45rem;
}

.badge-error {
  background: var(--danger-subtle);
  color: var(--danger);
}

.badge-warn {
  background: var(--warning-subtle);
  color: var(--warning);
}

.badge-info {
  background: var(--accent-subtle, color-mix(in srgb, var(--accent) 15%, transparent));
  color: var(--accent);
}

.badge-debug {
  background: color-mix(in srgb, var(--text-muted) 15%, transparent);
  color: var(--text-muted);
}

.badge-trace {
  background: color-mix(in srgb, var(--text-muted) 10%, transparent);
  color: var(--text-muted);
  opacity: 0.7;
}

.schedule-label {
  display: block;
  font-size: 0.75rem;
  color: var(--text-muted);
  margin-top: 0.1rem;
}

.btn-run-filter {
  display: block;
  margin-top: 0.2rem;
  padding: 0.15rem 0.4rem;
  font-size: 0.7rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--accent);
  cursor: pointer;
  white-space: nowrap;
}

.btn-run-filter:hover {
  background: var(--bg-hover);
}

.run-id-filter {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.3rem 0.5rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-input);
  font-size: 0.8rem;
  color: var(--text-secondary);
}

.run-id-label {
  font-family: monospace;
}

.btn-clear-run {
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  padding: 0;
  font-size: 0.8rem;
  line-height: 1;
}

.btn-clear-run:hover {
  color: var(--text-primary);
}

.live-sessions {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.live-session-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
}

.live-session-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.6rem 1rem;
  border-bottom: 1px solid var(--border);
  background: var(--bg-base);
}

.live-session-pulse {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--success);
  animation: session-pulse 1.5s ease-in-out infinite;
  flex-shrink: 0;
}

@keyframes session-pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.3;
  }
}

.live-session-title {
  font-size: 0.75rem;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
}

.live-session-meta {
  margin-left: auto;
  font-size: 0.72rem;
  color: var(--accent);
  font-family: var(--mono);
}

.live-session-output {
  max-height: 200px;
  overflow-y: auto;
  padding: 0.5rem 1rem;
  background: var(--bg-base);
  font-family: var(--mono);
  font-size: 0.72rem;
  color: var(--text-secondary);
}

.live-session-line {
  white-space: pre-wrap;
  word-break: break-all;
  line-height: 1.5;
  padding: 0.05rem 0;
}
</style>
