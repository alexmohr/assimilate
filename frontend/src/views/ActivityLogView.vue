<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted, watch, type Ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import {
  Search,
  SlidersHorizontal,
  Activity,
  X,
  ArrowRight,
  CheckCheck,
  Copy,
  Trash2,
} from '@lucide/vue'
import DataTable from 'primevue/datatable'
import Column from 'primevue/column'
import BaseSpinner from '../components/BaseSpinner.vue'
import BaseDisclosure from '../components/BaseDisclosure.vue'
import EmptyState from '../components/EmptyState.vue'
import RestoreRunsPanel from '../components/RestoreRunsPanel.vue'
import { apiClient } from '../api/client'
import {
  getActivity,
  getSystemEvents,
  acknowledgeActivityEntry,
  unacknowledgeActivityEntry,
  acknowledgeSystemEvent,
  unacknowledgeSystemEvent,
  acknowledgeAllActivity,
  getOutstandingAcknowledgements,
  type ActivityEntry,
  type SystemEventEntry,
} from '../api/stats'
import { listAgents, listAgentReports } from '../api/agents'
import { listSchedules } from '../api/schedules'
import { useWebSocket } from '../composables/useWebSocket'
import { useMobile } from '../composables/useMobile'
import { useTimeout } from '../composables/useTimeout'
import { useToast } from '../composables/useToast'
import { useClientLogs } from '../composables/useClientLogs'
import { useClipboard } from '../composables/useClipboard'
import { formatDuration, formatBytes, formatDateShort, formatEventType } from '../utils/format'
import { logger } from '../utils/logger'
import { formatClientLogs, type ClientLogEntry, type ClientLogLevel } from '../utils/clientLog'
import { extractError } from '../utils/error'
import { normalizeBackupStatus } from '../utils/backupStatus'
import type { ReportRow } from '../types/report'
import { backupStatusBadgeClass, badgeClass, logLevelTone, systemEventTone } from '../utils/badge'
import BaseSegmented, { type SegmentedOption } from '../components/BaseSegmented.vue'
import { useAuthStore } from '../stores/auth'
import type { AcknowledgedFilter } from '../types/generated'

interface ScheduleOption {
  id: number
  name: string
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
  /** Browser logs only: the redacted stack of a logged error. */
  stack?: string | null
}

type CategoryFilter = 'all' | 'backup' | 'system' | 'restores' | 'logs' | 'browser'
type StatusFilter = 'all' | 'success' | 'warning' | 'failed' | 'started' | 'pending'
type LogLevel = '' | 'error' | 'warn' | 'info' | 'debug' | 'trace'

interface LevelOption {
  value: LogLevel
  label: string
}

const SERVER_LEVEL_OPTIONS: readonly LevelOption[] = [
  { value: 'error', label: 'Error' },
  { value: 'warn', label: 'Warn' },
  { value: 'info', label: 'Info' },
  { value: 'debug', label: 'Debug' },
  { value: 'trace', label: 'Trace' },
]

const BROWSER_LEVEL_OPTIONS: readonly LevelOption[] = [
  { value: 'error', label: 'Error' },
  { value: 'warn', label: 'Warn' },
  { value: 'debug', label: 'Debug' },
]

/**
 * Severity rank of a browser log level, lowest is most severe. Like the
 * server's level filter, choosing a level keeps it and everything above it.
 */
const BROWSER_LEVEL_RANK: Record<ClientLogLevel, number> = { error: 0, warn: 1, debug: 2 }

function isClientLogLevel(level: LogLevel): level is ClientLogLevel {
  return level === 'error' || level === 'warn' || level === 'debug'
}

// Target names are open-ended (arbitrary hostnames from data), so "all" can't
// be expressed as a closed literal union alongside them. Naming the sentinel
// keeps the comparison out of raw-literal territory.
const ALL_TARGETS_FILTER = 'all'

function isCategoryFilter(value: string): value is CategoryFilter {
  return (
    value === 'all' ||
    value === 'backup' ||
    value === 'system' ||
    value === 'restores' ||
    value === 'logs' ||
    value === 'browser'
  )
}

function isQueryBackupStatus(value: string): value is 'success' | 'warning' | 'failed' {
  return value === 'success' || value === 'warning' || value === 'failed'
}

const rows = ref<ActivityEntry[]>([])
const systemEvents = ref<SystemEventEntry[]>([])
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

const BASE_CATEGORY_OPTIONS: readonly SegmentedOption<CategoryFilter>[] = [
  { value: 'all', label: 'All' },
  { value: 'backup', label: 'Backup' },
  { value: 'system', label: 'System' },
  { value: 'logs', label: 'Server Logs' },
]

// Restores onto agents are an admin operation, and the browser log viewer
// is a debugging aid, so like the APIs behind them both are offered to
// admins only.
const categoryOptions = computed((): SegmentedOption<CategoryFilter>[] =>
  auth.isAdmin
    ? [
        ...BASE_CATEGORY_OPTIONS.slice(0, 3),
        { value: 'restores', label: 'Restores' },
        ...BASE_CATEGORY_OPTIONS.slice(3),
        { value: 'browser', label: 'Browser logs' },
      ]
    : [...BASE_CATEGORY_OPTIONS],
)

const activeCategory = ref<CategoryFilter>('all')
const filterMachine = ref('')
const filterTarget = ref('all')
const filterStatus = ref<StatusFilter>('all')
const filterFrom = ref('')
const filterTo = ref('')
const filterScheduleId = ref<number | null>(null)
const filterRunId = ref<string | null>(null)
// Reviewed problems drop out of the feed by default - the point of
// acknowledging one is to stop seeing it. The filter brings them back.
const DEFAULT_ACKNOWLEDGED_FILTER: AcknowledgedFilter = 'unacknowledged'
const filterAcknowledged = ref<AcknowledgedFilter>(DEFAULT_ACKNOWLEDGED_FILTER)

const logEntries = ref<LogEntry[]>([])
const logLevel = ref<LogLevel>('')
const logSearch = ref('')
const loadingLogs = ref(false)
// Cancelled on unmount, so a search typed just before leaving the page does
// not fetch logs for a view that no longer exists.
const logSearchDebounce = useTimeout()

const { entries: clientLogEntries, clear: clearClientLogs } = useClientLogs()
const { copied: clientLogsCopied, copy: copyToClipboard } = useClipboard()

/** Either log tab: they share the level/search filters and the log table. */
const isLogTab = computed(
  (): boolean => activeCategory.value === 'logs' || activeCategory.value === 'browser',
)
const isBrowserTab = computed((): boolean => activeCategory.value === 'browser')
/** The restores tab lists restore runs in its own panel, with no filters. */
const isRestoresTab = computed((): boolean => activeCategory.value === 'restores')

const levelOptions = computed((): readonly LevelOption[] =>
  isBrowserTab.value ? BROWSER_LEVEL_OPTIONS : SERVER_LEVEL_OPTIONS,
)

/** Browser entries passing the level and search filters, newest first. */
const filteredClientLogs = computed((): readonly ClientLogEntry[] => {
  const level = logLevel.value
  const maxRank = isClientLogLevel(level) ? BROWSER_LEVEL_RANK[level] : Infinity
  const needle = logSearch.value.trim().toLowerCase()
  return clientLogEntries.value.filter(
    (e) =>
      BROWSER_LEVEL_RANK[e.level] <= maxRank &&
      (needle === '' ||
        e.message.toLowerCase().includes(needle) ||
        e.source.toLowerCase().includes(needle)),
  )
})

const displayedLogs = computed((): LogEntry[] =>
  isBrowserTab.value
    ? filteredClientLogs.value.map((e) => ({
        timestamp: e.timestamp,
        level: e.level,
        target: e.source,
        message: e.message,
        stack: e.stack,
      }))
    : logEntries.value,
)

const emptyLogsMessage = computed((): string =>
  isBrowserTab.value && clientLogEntries.value.length === 0
    ? 'Nothing has been logged in this browser tab yet.'
    : 'No log entries match the current filters.',
)

async function copyClientLogs(): Promise<void> {
  try {
    await copyToClipboard(formatClientLogs([...filteredClientLogs.value].reverse()))
  } catch (e: unknown) {
    toastError(extractError(e))
  }
}

const { isMobile } = useMobile()
const showMobileFilters = ref(false)
const route = useRoute()
const router = useRouter()

const availableTargets = computed(() => {
  const targets = new Set(rows.value.map((r) => r.target_name))
  return [...targets].sort()
})

const hasActiveFilters = computed((): boolean => {
  if (isLogTab.value) {
    return logLevel.value !== '' || logSearch.value !== ''
  }
  return (
    filterMachine.value !== '' ||
    filterTarget.value !== ALL_TARGETS_FILTER ||
    filterStatus.value !== 'all' ||
    filterFrom.value !== '' ||
    filterTo.value !== '' ||
    filterScheduleId.value !== null ||
    filterRunId.value !== null ||
    filterAcknowledged.value !== DEFAULT_ACKNOWLEDGED_FILTER
  )
})

onMounted(async () => {
  const catParam = route.query.category as string | undefined
  if (catParam !== undefined && isCategoryFilter(catParam)) {
    const adminOnly = catParam === 'browser' || catParam === 'restores'
    activeCategory.value = adminOnly && !auth.isAdmin ? 'all' : catParam
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
  if (statusParam !== undefined && isQueryBackupStatus(statusParam)) {
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
onMessage('DataChanged', () => fetchData(true, true).catch(logger.error))
onMessage('AgentConnected', () => fetchData(true, true).catch(logger.error))
onMessage('AgentDisconnected', () => fetchData(true, true).catch(logger.error))

onMessage('BackupStarted', (payload) => {
  const key = liveSessionKey(payload.hostname, payload.target_name)
  const next = new Map(liveBackupSessions.value)
  next.set(key, { hostname: payload.hostname, target_name: payload.target_name, lines: [] })
  liveBackupSessions.value = next
})

onMessage('BackupCompleted', (payload) => {
  const key = liveSessionKey(payload.hostname, payload.target_name)
  const next = new Map(liveBackupSessions.value)
  next.delete(key)
  liveBackupSessions.value = next
})

onMessage('BackupLog', (payload) => {
  try {
    const obj = JSON.parse(payload.line) as Record<string, unknown>
    if (obj['type'] === 'archive_progress') return
  } catch {
    // non-JSON line - show it
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
})

const activeLiveSessions = computed<LiveBackupSession[]>(() =>
  [...liveBackupSessions.value.values()].filter((s) => s.lines.length > 0),
)

watch(activeCategory, (cat) => {
  router.replace({ query: { ...route.query, category: cat } }).catch(() => {})
  if (cat === 'logs') {
    fetchLogs().catch(logger.error)
  } else if (cat === 'browser') {
    // The server offers Info and Trace; the browser logger has neither.
    if (logLevel.value !== '' && !isClientLogLevel(logLevel.value)) logLevel.value = ''
  } else if (cat !== 'restores') {
    fetchData(true).catch(logger.error)
  }
})

watch(logLevel, () => {
  if (activeCategory.value === 'logs') fetchLogs().catch(logger.error)
})

watch(logSearch, () => {
  logSearchDebounce.start(() => {
    if (activeCategory.value === 'logs') fetchLogs().catch(logger.error)
  }, 300)
})

watch(filterScheduleId, () => {
  if (!isLogTab.value) fetchData(true).catch(logger.error)
})

watch(filterRunId, () => {
  if (!isLogTab.value) fetchData(true).catch(logger.error)
})

watch(filterAcknowledged, () => {
  if (!isLogTab.value) fetchData(true).catch(logger.error)
})

async function fetchMachines(): Promise<void> {
  agents.value = await listAgents()
}

async function fetchSchedules(): Promise<void> {
  schedules.value = await listSchedules()
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

async function fetchData(reset: boolean, preserveExpanded = false): Promise<void> {
  if (isLogTab.value || isRestoresTab.value) return

  if (reset) {
    loading.value = true
    offset.value = 0
    rows.value = []
    systemEvents.value = []
    if (!preserveExpanded) {
      expandedId.value = null
      expandedDetail.value = null
      expandedSystemId.value = null
    }
  } else {
    loadingMore.value = true
  }

  try {
    const limit = PAGE_SIZE + offset.value
    const cat = activeCategory.value

    if (cat === 'backup' || cat === 'all') {
      rows.value = await getActivity({
        limit,
        schedule_id: filterScheduleId.value ?? undefined,
        run_id: filterRunId.value ?? undefined,
        acknowledged: filterAcknowledged.value,
      })
    }

    if (cat === 'system' || cat === 'all') {
      systemEvents.value = await getSystemEvents(limit, filterAcknowledged.value)
    }

    const totalFetched =
      cat === 'all'
        ? Math.max(rows.value.length, systemEvents.value.length)
        : cat === 'backup'
          ? rows.value.length
          : systemEvents.value.length
    hasMore.value = totalFetched >= limit
    offset.value += PAGE_SIZE
    // Only on a reset. Paging further into history cannot change what is
    // still outstanding, so "Load more" has nothing to re-probe.
    if (reset) await refreshOutstanding()
  } finally {
    loading.value = false
    loadingMore.value = false
  }
}

async function loadMore(): Promise<void> {
  await fetchData(false)
}

function toggleSystemRow(event: SystemEventEntry): void {
  expandedSystemId.value = expandedSystemId.value === event.id ? null : event.id
}

async function toggleRow(row: ActivityEntry): Promise<void> {
  if (expandedId.value === row.id) {
    expandedId.value = null
    expandedDetail.value = null
    return
  }
  expandedId.value = row.id
  expandedDetail.value = null
  expandedLoading.value = true
  try {
    const { reports } = await listAgentReports(row.hostname, {
      limit: 100,
      target: row.target_name,
    })
    const match = reports.find(
      (r) => r.started_at === row.started_at || r.duration_secs === row.duration_secs,
    )
    expandedDetail.value = match ?? reports[0] ?? null
  } finally {
    expandedLoading.value = false
  }
}

const filtered = computed(() => {
  return rows.value.filter((r) => {
    if (filterMachine.value && r.hostname !== filterMachine.value) {
      return false
    }
    if (filterTarget.value !== ALL_TARGETS_FILTER && r.target_name !== filterTarget.value) {
      return false
    }
    if (filterStatus.value !== 'all' && normalizeBackupStatus(r.status) !== filterStatus.value) {
      return false
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
  backup?: ActivityEntry
  event?: SystemEventEntry
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
  return backupStatusBadgeClass(status)
}

/** Only a warning or a failure needs acknowledging - a clean run has nothing to mute. */
function isAckable(entry: ActivityEntry): boolean {
  const status = normalizeBackupStatus(entry.status)
  return status === 'warning' || status === 'failed'
}

const { error: toastError, success: toastSuccess } = useToast()
const auth = useAuthStore()
/** The two calls that flip one entry's acknowledgement, either way round. */
interface AcknowledgeEndpoints {
  acknowledge: (id: number) => Promise<void>
  unacknowledge: (id: number) => Promise<void>
}

const ackingId = ref<number | null>(null)
const ackingSystemId = ref<number | null>(null)
const ackingAll = ref(false)

/** Whether a row with this acknowledgment state still belongs in the feed. */
function matchesAcknowledgedFilter(acknowledged: boolean): boolean {
  switch (filterAcknowledged.value) {
    case 'all':
      return true
    case 'unacknowledged':
      return !acknowledged
    case 'acknowledged':
      return acknowledged
  }
}

/**
 * Both acknowledge toggles are the same small state machine: mark the row
 * busy, flip it server-side, drop it from the feed when the active filter no
 * longer wants it, re-probe what is outstanding, and surface any failure.
 * Shared so the backup and system-event paths cannot drift apart.
 */
async function toggleAcknowledged<T extends { id: number; acknowledged: boolean }>(
  entry: T,
  busy: Ref<number | null>,
  list: Ref<T[]>,
  endpoints: AcknowledgeEndpoints,
): Promise<void> {
  busy.value = entry.id
  try {
    const next = !entry.acknowledged
    await (next ? endpoints.acknowledge(entry.id) : endpoints.unacknowledge(entry.id))
    entry.acknowledged = next
    if (!matchesAcknowledgedFilter(next)) {
      list.value = list.value.filter((item) => item.id !== entry.id)
    }
    await refreshOutstanding()
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    busy.value = null
  }
}

function toggleAcknowledge(entry: ActivityEntry): Promise<void> {
  return toggleAcknowledged(entry, ackingId, rows, {
    acknowledge: acknowledgeActivityEntry,
    unacknowledge: unacknowledgeActivityEntry,
  })
}

/**
 * System events are global rather than repository-scoped, so the server only
 * lets an admin acknowledge one - hide the button for everyone else instead of
 * offering an action that would come back 403.
 */
function isSystemEventAckable(event: SystemEventEntry): boolean {
  return event.acknowledgeable && auth.isAdmin
}

function toggleSystemAcknowledge(event: SystemEventEntry): Promise<void> {
  return toggleAcknowledged(event, ackingSystemId, systemEvents, {
    acknowledge: acknowledgeSystemEvent,
    unacknowledge: unacknowledgeSystemEvent,
  })
}

/**
 * How much is still outstanding server-side, independent of the category tab
 * and every filter. Deriving this from the rows on screen would hide the
 * button exactly when it is most useful: the feed only loads the active
 * category, is capped at a page, and is narrowed further by the machine /
 * target / status / date filters, so a view showing nothing acknowledgeable
 * says nothing about what is left elsewhere.
 */
const outstanding = ref({ backup_reports: 0, system_events: 0 })

async function refreshOutstanding(): Promise<void> {
  try {
    outstanding.value = await getOutstandingAcknowledgements()
  } catch (e: unknown) {
    logger.error('failed to load outstanding acknowledgements', e)
  }
}

/** Whether a bulk acknowledge would actually do anything for this user. */
const hasUnacknowledged = computed(
  (): boolean => outstanding.value.backup_reports + outstanding.value.system_events > 0,
)

async function acknowledgeAll(): Promise<void> {
  ackingAll.value = true
  try {
    const result = await acknowledgeAllActivity()
    const total = result.backup_reports + result.system_events
    toastSuccess(total === 1 ? 'Acknowledged 1 entry' : `Acknowledged ${total} entries`)
    await fetchData(true, true)
  } catch (e: unknown) {
    toastError(extractError(e))
  } finally {
    ackingAll.value = false
  }
}

function eventBadgeClass(event: SystemEventEntry): string {
  return badgeClass(systemEventTone(event.severity))
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
  filterAcknowledged.value = DEFAULT_ACKNOWLEDGED_FILTER
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
        <span
          v-if="!isRestoresTab"
          class="row-count"
          >{{
            isLogTab ? `${displayedLogs.length} entries` : `${unifiedRows.length} entries`
          }}</span
        >
        <template v-if="isBrowserTab">
          <button
            class="btn btn-sm btn-ghost"
            :disabled="displayedLogs.length === 0"
            @click="copyClientLogs"
          >
            <Copy :size="14" />
            {{ clientLogsCopied ? 'Copied' : 'Copy' }}
          </button>
          <button
            class="btn btn-sm btn-ghost"
            :disabled="clientLogEntries.length === 0"
            @click="clearClientLogs"
          >
            <Trash2 :size="14" />
            Clear logs
          </button>
        </template>
        <button
          v-if="!isLogTab && !isRestoresTab && hasUnacknowledged"
          class="btn btn-sm btn-ghost"
          :disabled="ackingAll"
          @click="acknowledgeAll"
        >
          <CheckCheck :size="14" />
          {{ ackingAll ? 'Acknowledging...' : 'Acknowledge all' }}
        </button>
      </div>
    </div>

    <div
      v-if="activeLiveSessions.length > 0 && !isLogTab && !isRestoresTab"
      class="live-sessions"
    >
      <div
        v-for="session in activeLiveSessions"
        :key="liveSessionKey(session.hostname, session.target_name)"
        class="live-session-card"
      >
        <div class="live-session-header">
          <span class="pulse-dot pulse-dot--success" />
          <span class="live-session-title">Live backup output</span>
          <span class="live-session-meta">
            {{ session.hostname }}
            <ArrowRight :size="12" />
            {{ session.target_name }}
          </span>
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
          <BaseSegmented
            v-model="activeCategory"
            :options="categoryOptions"
            label="Activity type"
          />
        </div>

        <button
          v-if="isMobile"
          class="filter-toggle"
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
          <template v-if="!isLogTab && !isRestoresTab">
            <div class="filter-group">
              <label class="filter-label">Machine</label>
              <select
                v-model="filterMachine"
                class="input input-sm select-input"
              >
                <option value="">All machines</option>
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
                class="input input-sm select-input"
              >
                <option :value="null">All schedules</option>
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
                  class="btn btn-xs btn-ghost"
                  title="Clear run filter"
                  aria-label="Clear run filter"
                  @click="filterRunId = null"
                >
                  <X :size="12" />
                </button>
              </div>
            </div>

            <div class="filter-group">
              <label class="filter-label">Target</label>
              <select
                v-model="filterTarget"
                class="input input-sm select-input"
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
                class="input input-sm select-input"
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
              <label class="filter-label">Acknowledged</label>
              <select
                v-model="filterAcknowledged"
                class="input input-sm select-input"
              >
                <option value="unacknowledged">Hidden</option>
                <option value="all">Shown</option>
                <option value="acknowledged">Only acknowledged</option>
              </select>
            </div>

            <div class="filter-group">
              <label class="filter-label">From</label>
              <input
                v-model="filterFrom"
                type="date"
                class="input input-sm date-input"
              />
            </div>

            <div class="filter-group">
              <label class="filter-label">To</label>
              <input
                v-model="filterTo"
                type="date"
                class="input input-sm date-input"
              />
            </div>
          </template>

          <template v-if="isLogTab">
            <div class="filter-group">
              <label class="filter-label">Level</label>
              <select
                v-model="logLevel"
                class="input input-sm select-input"
              >
                <option value="">All</option>
                <option
                  v-for="opt in levelOptions"
                  :key="opt.value"
                  :value="opt.value"
                >
                  {{ opt.label }}
                </option>
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
                  class="input search-input search-input--icon"
                  placeholder="Filter messages..."
                />
              </div>
            </div>
          </template>

          <button
            class="btn btn-sm btn-ghost"
            @click="clearFilters"
          >
            Clear
          </button>
        </template>
      </div>
    </section>

    <template v-if="isLogTab">
      <div
        v-if="!isBrowserTab && loadingLogs"
        class="loading"
      >
        Loading server logs...
      </div>

      <div
        v-else-if="displayedLogs.length === 0"
        class="state-msg"
      >
        {{ emptyLogsMessage }}
      </div>

      <div
        v-else
        class="log-panel"
      >
        <DataTable
          :value="displayedLogs"
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
                class="badge"
                :class="badgeClass(logLevelTone(data.level))"
              >
                {{ data.level }}
              </span>
            </template>
          </Column>
          <Column :header="isBrowserTab ? 'Source' : 'Target'">
            <template #body="{ data }">
              <span
                class="cell-target-log cell-mono"
                :title="data.target"
                >{{ data.target }}</span
              >
            </template>
          </Column>
          <Column header="Message">
            <template #body="{ data }">
              <span class="cell-msg-log">{{ data.message }}</span>
              <BaseDisclosure
                v-if="data.stack"
                title="Stack trace"
              >
                <pre class="detail-pre">{{ data.stack }}</pre>
              </BaseDisclosure>
            </template>
          </Column>
          <template #empty>
            <div class="state-msg">No log entries match the current filters.</div>
          </template>
        </DataTable>
      </div>
    </template>

    <RestoreRunsPanel v-else-if="isRestoresTab" />

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
        class="run-list"
      >
        <template
          v-for="row in unifiedRows"
          :key="row.id"
        >
          <article
            v-if="row.kind === 'backup' && row.backup"
            class="panel panel--sectioned run-card"
            :class="{
              expanded: expandedId === row.backup.id,
              'run-card--acknowledged': row.backup.acknowledged,
            }"
          >
            <div
              class="run-card-summary"
              @click="toggleRow(row.backup)"
            >
              <div class="run-card-top">
                <div class="run-card-host">
                  <span class="run-card-hostname">{{ row.backup.hostname }}</span>
                  <span class="run-card-time">{{ formatDateShort(row.backup.started_at) }}</span>
                </div>
                <div class="run-card-badges">
                  <span
                    class="badge"
                    :class="statusClass(row.backup.status)"
                    >{{ row.backup.status }}</span
                  >
                  <span
                    v-if="row.backup.acknowledged"
                    class="badge badge--neutral"
                    >Acknowledged</span
                  >
                </div>
              </div>
              <div class="run-card-meta">
                <span>{{ row.backup.target_name }}</span>
                <span
                  v-if="row.backup.schedule_name"
                  class="schedule-label"
                  >{{ row.backup.schedule_name }}</span
                >
              </div>
              <div class="run-card-foot">
                <span class="run-card-duration">{{
                  formatDuration(row.backup.duration_secs)
                }}</span>
                <div class="run-card-actions">
                  <button
                    v-if="row.backup.run_id && filterRunId !== row.backup.run_id"
                    class="btn btn-xs btn-ghost"
                    title="View all events for this run"
                    @click.stop="filterByRun(row.backup.run_id)"
                  >
                    View run
                  </button>
                  <button
                    v-if="isAckable(row.backup)"
                    class="btn btn-xs btn-ghost"
                    :disabled="ackingId === row.backup.id"
                    @click.stop="toggleAcknowledge(row.backup)"
                  >
                    {{ row.backup.acknowledged ? 'Unacknowledge' : 'Acknowledge' }}
                  </button>
                </div>
              </div>
            </div>

            <div
              v-if="expandedId === row.backup.id"
              class="detail-panel"
              @click.stop
            >
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
                  <dl class="info-grid">
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
                  <dl class="info-grid">
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
                  <dl class="info-grid">
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
                  v-if="expandedDetail.warnings.length > 0"
                  class="detail-section detail-warning-section"
                >
                  <h3 class="detail-heading status-heading warning-heading">Warnings</h3>
                  <pre class="warning-pre">{{ expandedDetail.warnings.join('\n') }}</pre>
                </div>
                <div
                  v-if="
                    expandedDetail.error_message &&
                    normalizeBackupStatus(expandedDetail.status) !== 'warning'
                  "
                  class="detail-section detail-error-section"
                >
                  <h3 class="detail-heading status-heading error-heading">Error</h3>
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
          </article>

          <article
            v-if="row.kind === 'system' && row.event"
            class="panel panel--sectioned run-card run-card-system"
            :class="{
              expanded: expandedSystemId === row.event.id,
              'run-card--acknowledged': row.event.acknowledged,
            }"
          >
            <div
              class="run-card-summary"
              @click="toggleSystemRow(row.event)"
            >
              <div class="run-card-top">
                <div class="run-card-host">
                  <span class="run-card-hostname">{{ row.event.hostname ?? '—' }}</span>
                  <span class="run-card-time">{{ formatDateShort(row.event.created_at) }}</span>
                </div>
                <div class="run-card-badges">
                  <span
                    class="badge"
                    :class="eventBadgeClass(row.event)"
                    >{{ formatEventType(row.event.event_type) }}</span
                  >
                  <span
                    v-if="row.event.acknowledged"
                    class="badge badge--neutral"
                    >Acknowledged</span
                  >
                </div>
              </div>
              <p class="run-card-message">{{ row.event.message }}</p>
              <div
                v-if="isSystemEventAckable(row.event)"
                class="run-card-foot"
              >
                <div class="run-card-actions">
                  <button
                    class="btn btn-xs btn-ghost"
                    :disabled="ackingSystemId === row.event.id"
                    @click.stop="toggleSystemAcknowledge(row.event)"
                  >
                    {{ row.event.acknowledged ? 'Unacknowledge' : 'Acknowledge' }}
                  </button>
                </div>
              </div>
            </div>

            <div
              v-if="expandedSystemId === row.event.id"
              class="detail-panel"
              @click.stop
            >
              <pre class="error-pre">{{ row.event.message }}</pre>
            </div>
          </article>
        </template>
      </div>

      <div
        v-if="!loading && hasMore && unifiedRows.length > 0"
        class="load-more"
      >
        <button
          class="btn btn-sm btn-ghost"
          :disabled="loadingMore"
          @click="loadMore"
        >
          {{ loadingMore ? 'Loading...' : 'Load more' }}
        </button>
      </div>
    </template>
  </div>
</template>

<style scoped>
.activity-log {
  display: flex;
  flex-direction: column;
  gap: var(--space-7);
  color: var(--text-primary);
}

.loading,
.log-table {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--fs-base);
}

.log-table thead tr {
  background: var(--bg-card);
}

.log-table th {
  padding: var(--space-5) var(--space-6);
  text-align: left;
  font-size: var(--fs-xs);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-muted);
  border-bottom: 1px solid var(--border);
}

.run-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.run-card.expanded {
  border-color: var(--text-muted);
}

/* Only reachable once the Acknowledged filter is showing reviewed entries -
   under the default they leave the feed instead. Dimmed so they read as
   already dealt with next to the entries that still need attention. */
.run-card--acknowledged {
  opacity: 0.6;
}

.run-card-badges {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-shrink: 0;
}

.run-card-summary {
  cursor: pointer;
  padding: var(--space-5) var(--space-6);
  transition: background var(--duration-fast);
}

.run-card-summary:hover {
  background: var(--bg-hover);
}

.run-card-top {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-5);
}

.run-card-host {
  display: flex;
  align-items: baseline;
  gap: var(--space-4);
  min-width: 0;
  flex-wrap: wrap;
}

.run-card-hostname {
  font-weight: 600;
  color: var(--text-primary);
}

.run-card-time {
  font-size: var(--fs-sm);
  color: var(--text-muted);
  white-space: nowrap;
}

.run-card-meta {
  margin-top: var(--space-3);
  font-size: var(--fs-base);
  color: var(--text-secondary);
}

.run-card-message {
  margin: var(--space-3) 0 0;
  font-size: var(--fs-base);
  color: var(--text-secondary);
  word-break: break-word;
}

.run-card-foot {
  margin-top: var(--space-4);
  padding-top: var(--space-4);
  border-top: 1px solid var(--border-subtle);
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-5);
}

/* Stacked in one right-hand column rather than spread across the card: with
   the buttons as direct children of a space-between row, a second action
   drifts into the middle of a wide card, nowhere near the run it acts on. */
.run-card-actions {
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: var(--space-3);
  margin-left: auto;
}

.run-card-duration {
  font-size: var(--fs-sm);
  color: var(--text-muted);
  font-family: var(--mono);
}

.detail-panel {
  padding: var(--space-6) var(--space-6) var(--space-7);
  border-top: 1px solid var(--border);
  background: var(--bg-base);
}

.detail-loading {
  color: var(--text-muted);
  font-size: var(--fs-base);
}

.detail-grid {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-8);
}

.detail-section {
  min-width: 180px;
}

.detail-error-section {
  flex: 1 1 100%;
}

.detail-heading {
  margin: 0 0 var(--space-4);
  font-size: var(--fs-xs);
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
}

.status-heading.error-heading {
  color: var(--danger);
}

.status-heading.warning-heading {
  color: var(--warning);
}

.detail-warning-section {
  flex: 1 1 100%;
}

.detail-command-section {
  flex: 1 1 100%;
}

.command-pre {
  margin: 0;
  padding: var(--space-5) var(--space-6);
  background: var(--bg-input);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  font-size: var(--fs-sm);
  white-space: pre-wrap;
  word-break: break-all;
  font-family: monospace;
}

.load-more {
  display: flex;
  justify-content: center;
  padding: var(--space-4) 0;
}

.filter-group-search {
  flex: 1;
  min-width: 180px;
}

.log-panel {
  overflow-x: auto;
}

.log-table-mono {
  font-family: 'SF Mono', 'Cascadia Code', 'Fira Code', monospace;
  font-size: var(--fs-sm);
}

.log-entry-row {
  border-bottom: 1px solid var(--border-subtle);
}

.log-entry-row td {
  padding: var(--space-3) var(--space-5);
  vertical-align: top;
}

.log-level-error {
  background: color-mix(in srgb, var(--danger) 6%, transparent);
}

.log-level-warn {
  background: color-mix(in srgb, var(--warning) 6%, transparent);
}

.cell-target-log {
  /* Inline, the max-width below would not apply and a long browser source
     location would push the message column off screen. */
  display: inline-block;
  color: var(--text-muted);
  white-space: nowrap;
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
}

.cell-msg-log {
  /* Keeps the message readable on a phone: the table scrolls inside its
     panel instead of squeezing this column to a few characters. */
  display: inline-block;
  min-width: 240px;
  color: var(--text-primary);
  word-break: break-word;
}

.schedule-label {
  display: block;
  font-size: var(--fs-xs);
  color: var(--text-muted);
  margin-top: var(--space-1);
}

.run-id-filter {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-2) var(--space-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-input);
  font-size: var(--fs-sm);
  color: var(--text-secondary);
}

.run-id-label {
  font-family: monospace;
}

.live-sessions {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
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
  gap: var(--space-4);
  padding: var(--space-4) var(--space-6);
  border-bottom: 1px solid var(--border);
  background: var(--bg-base);
}

.live-session-title {
  font-size: var(--fs-xs);
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
}

.live-session-meta {
  margin-left: auto;
  font-size: var(--fs-xs);
  color: var(--accent);
  font-family: var(--mono);
}

.live-session-output {
  max-height: 200px;
  overflow-y: auto;
  padding: var(--space-4) var(--space-6);
  background: var(--bg-base);
  font-family: var(--mono);
  font-size: var(--fs-xs);
  color: var(--text-secondary);
}

.live-session-line {
  white-space: pre-wrap;
  word-break: break-all;
  line-height: 1.5;
  padding: var(--space-1) 0;
}
</style>
