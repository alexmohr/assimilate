<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, nextTick, onMounted, watch, type Ref } from 'vue'
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
  ScrollText,
} from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import LogList from '../components/LogList.vue'
import LogRow from '../components/LogRow.vue'
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
import {
  formatDuration,
  formatBytes,
  formatDate,
  formatDateShort,
  formatEventType,
} from '../utils/format'
import { logger } from '../utils/logger'
import { formatClientLogs, type ClientLogEntry, type ClientLogLevel } from '../utils/clientLog'
import { extractError } from '../utils/error'
import { normalizeBackupStatus } from '../utils/backupStatus'
import { useReportRetry } from '../composables/useReportRetry'
import { RotateCcw } from '@lucide/vue'
import { scheduleLabel } from '../utils/machine'
import type { ReportRow } from '../types/report'
import { backupStatusTone, logLevelTone, systemEventTone } from '../utils/badge'
import type { SegmentedOption } from '../components/BaseSegmented.vue'
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
  // A link to one run (`?report=`) opens it, so its output is the first thing read.
  const reportId = Number(route.query.report)
  if (Number.isInteger(reportId) && reportId > 0) {
    const row = await findLinkedRun(reportId)
    if (row) await toggleRow(row)
  }
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

onMessage('BackupCompleted', (payload) => endLiveSession(payload.hostname, payload.target_name))
// A backup that was cancelled, or lost with an agent that restarted, never
// completes.
onMessage('BackupStopped', (payload) => endLiveSession(payload.hostname, payload.target_name))

function endLiveSession(hostname: string, targetName: string): void {
  const key = liveSessionKey(hostname, targetName)
  const next = new Map(liveBackupSessions.value)
  next.delete(key)
  liveBackupSessions.value = next
}

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

/** Set while a linked run narrows the filters itself and fetches once. */
let narrowingToLinkedRun = false

watch(filterRunId, () => {
  if (!isLogTab.value && !narrowingToLinkedRun) fetchData(true).catch(logger.error)
})

watch(filterAcknowledged, () => {
  if (!isLogTab.value && !narrowingToLinkedRun) fetchData(true).catch(logger.error)
})

/**
 * A linked run that the first page does not hold - older than it, or
 * acknowledged and so hidden - is looked up by id and the log narrowed to its
 * run, with acknowledged runs shown, so the link still opens it. The run
 * filter shows as a chip the reader can clear.
 */
async function findLinkedRun(reportId: number): Promise<ActivityEntry | undefined> {
  const loaded = rows.value.find((r) => r.id === reportId)
  if (loaded) return loaded
  const entry = (await getActivity({ report_id: reportId })).find((r) => r.id === reportId)
  if (!entry) return undefined
  narrowingToLinkedRun = true
  try {
    if (entry.run_id) filterRunId.value = entry.run_id
    if (entry.acknowledged) filterAcknowledged.value = 'all'
    await nextTick()
  } finally {
    narrowingToLinkedRun = false
  }
  await fetchData(true)
  const found = rows.value.find((r) => r.id === reportId)
  if (found) return found
  // A run without a run id cannot be narrowed to; show it at the end instead.
  rows.value = [...rows.value, entry]
  return entry
}

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

    await loadLaterRunsOfSchedule()

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

/**
 * Bumped by every open and close, so a detail fetch that finishes after
 * another row was opened (or this one closed and reopened) is dropped
 * instead of showing its run under the row now open.
 */
let detailRequest = 0

async function toggleRow(row: ActivityEntry): Promise<void> {
  const request = ++detailRequest
  if (expandedId.value === row.id) {
    expandedId.value = null
    expandedDetail.value = null
    expandedLoading.value = false
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
    if (request !== detailRequest) return
    // The feed entry and the host's report share an id; the time and
    // duration match is only a fallback for an entry from an older server.
    const match =
      reports.find((r) => r.id === row.id) ??
      reports.find((r) => r.started_at === row.started_at || r.duration_secs === row.duration_secs)
    expandedDetail.value = match ?? reports[0] ?? null
  } finally {
    if (request === detailRequest) expandedLoading.value = false
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

/**
 * Narrowed to one run, the feed holds none of its schedule's later runs, so
 * it cannot tell whether a failure there has since been fixed or retried.
 * Those later runs are fetched alongside it for Retry to compare against;
 * null while that lookup failed, when Retry is withheld rather than guessed.
 */
const laterRunsOfSchedule = ref<ActivityEntry[] | null>([])

async function loadLaterRunsOfSchedule(): Promise<void> {
  const scheduleId = filterRunId.value === null ? null : (rows.value[0]?.schedule_id ?? null)
  if (scheduleId === null) {
    laterRunsOfSchedule.value = []
    return
  }
  try {
    laterRunsOfSchedule.value = await getActivity({
      limit: PAGE_SIZE,
      schedule_id: scheduleId,
      acknowledged: 'all',
    })
  } catch (e: unknown) {
    logger.error('Could not load the later runs of the schedule', e)
    laterRunsOfSchedule.value = null
  }
}

/**
 * Retry on a failed run: re-runs its schedule for that one host and that one
 * repository, and only while it is still the newest run of that copy, so a
 * failure that has since succeeded, or already been retried, offers nothing.
 */
const retryableIds = computed<ReadonlySet<number>>(() => {
  const later = laterRunsOfSchedule.value
  if (later === null) return new Set()
  const newest = new Map<string, ActivityEntry>()
  ;[...rows.value, ...later].forEach((r) => {
    if (r.schedule_id === null) return
    const key = `${r.schedule_id}:${r.hostname}:${r.repo_id}`
    const current = newest.get(key)
    if (!current || r.id > current.id) newest.set(key, r)
  })
  return new Set(
    [...newest.values()]
      .filter((r) => normalizeBackupStatus(r.status) === 'failed')
      .map((r) => r.id),
  )
})
const { retryingReportIds, retry } = useReportRetry(() => fetchData(true))

/** The opened run's report, once loaded: what a retry needs (its agent). */
function retryTarget(entry: ActivityEntry): ReportRow | null {
  return expandedDetail.value?.id === entry.id ? expandedDetail.value : null
}
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

/** The schedule a run belongs to, for the row's second line; none for a manual run. */
function scheduleText(entry: ActivityEntry): string | null {
  if (!entry.schedule_id) return null
  return scheduleLabel({
    schedule_id: entry.schedule_id,
    schedule_name: entry.schedule_name ?? '',
  })
}

/** A system event's message block, in the event's own tone. */
function systemEventPreClass(event: SystemEventEntry): string {
  switch (systemEventTone(event.severity)) {
    case 'danger':
      return 'error-pre'
    case 'warning':
      return 'warning-pre'
    default:
      return 'detail-pre'
  }
}

/** The open log entry, by its position in the shown list. */
const expandedLogIndex = ref<number | null>(null)

function toggleLogRow(index: number): void {
  expandedLogIndex.value = expandedLogIndex.value === index ? null : index
}

// A new fetch or a new filter reorders the list, so a remembered position
// would open a different entry.
watch(displayedLogs, () => {
  expandedLogIndex.value = null
})

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
          <!-- Pills, which wrap on a phone, rather than a segmented control
               that cut its last options off. -->
          <div
            class="filter-pills"
            role="group"
            aria-label="Activity type"
          >
            <button
              v-for="option in categoryOptions"
              :key="option.value"
              type="button"
              class="filter-pill"
              :class="{ active: activeCategory === option.value }"
              :aria-pressed="activeCategory === option.value"
              @click="activeCategory = option.value"
            >
              {{ option.label }}
            </button>
          </div>
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
      <BaseSpinner
        v-if="!isBrowserTab && loadingLogs"
        size="lg"
      />

      <EmptyState
        v-else-if="displayedLogs.length === 0"
        :icon="ScrollText"
        title="No log entries"
        :description="emptyLogsMessage"
      />

      <LogList
        v-else
        data-testid="log-entries"
      >
        <LogRow
          v-for="(entry, i) in displayedLogs"
          :key="`${entry.timestamp}-${i}`"
          :tone="logLevelTone(entry.level)"
          :title="entry.message"
          :facts="[entry.level.toUpperCase(), entry.target]"
          :time="formatDateShort(entry.timestamp)"
          :time-title="formatDate(entry.timestamp)"
          :expanded="expandedLogIndex === i"
          @toggle="toggleLogRow(i)"
        >
          <template #detail>
            <dl class="info-grid">
              <dt>Time</dt>
              <dd>{{ formatDate(entry.timestamp) }}</dd>
              <dt>Level</dt>
              <dd>{{ entry.level }}</dd>
              <dt>{{ isBrowserTab ? 'Source' : 'Target' }}</dt>
              <dd class="log-detail-value">{{ entry.target }}</dd>
            </dl>
            <pre class="detail-pre">{{ entry.message }}</pre>
            <template v-if="entry.stack">
              <h3 class="group-label">Stack trace</h3>
              <pre class="detail-pre">{{ entry.stack }}</pre>
            </template>
          </template>
        </LogRow>
      </LogList>
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

      <LogList
        v-else
        data-testid="activity-entries"
      >
        <template
          v-for="row in unifiedRows"
          :key="row.id"
        >
          <!-- Two lines like every other log: who and where, then which
               schedule and how long. The actions wait in the opened entry. -->
          <LogRow
            v-if="row.kind === 'backup' && row.backup"
            :tone="backupStatusTone(row.backup.status)"
            :title="row.backup.hostname"
            :subtitle="row.backup.target_name"
            :facts="[scheduleText(row.backup), formatDuration(row.backup.duration_secs)]"
            :time="formatDateShort(row.backup.started_at)"
            :time-title="formatDate(row.backup.started_at)"
            :expanded="expandedId === row.backup.id"
            :dimmed="row.backup.acknowledged"
            @toggle="toggleRow(row.backup)"
          >
            <template
              v-if="backupStatusTone(row.backup.status) !== 'success' || row.backup.acknowledged"
              #badges
            >
              <span
                v-if="row.backup.acknowledged"
                class="badge badge--neutral"
                >Acknowledged</span
              >
            </template>
            <template
              v-if="
                (row.backup.run_id && filterRunId !== row.backup.run_id) ||
                isAckable(row.backup) ||
                retryableIds.has(row.backup.id)
              "
              #actions
            >
              <button
                v-if="row.backup.run_id && filterRunId !== row.backup.run_id"
                class="btn btn-sm btn-ghost"
                title="View all events for this run"
                @click="filterByRun(row.backup.run_id)"
              >
                View run
              </button>
              <button
                v-if="retryableIds.has(row.backup.id) && retryTarget(row.backup)"
                class="btn btn-sm btn-primary"
                :disabled="retryingReportIds.has(row.backup.id)"
                title="Run this schedule again for this host and repository"
                @click="retry(retryTarget(row.backup)!)"
              >
                <RotateCcw :size="14" />
                {{ retryingReportIds.has(row.backup.id) ? 'Retrying...' : 'Retry' }}
              </button>
              <button
                v-if="isAckable(row.backup)"
                class="btn btn-sm"
                :disabled="ackingId === row.backup.id"
                @click="toggleAcknowledge(row.backup)"
              >
                {{ row.backup.acknowledged ? 'Unacknowledge' : 'Acknowledge' }}
              </button>
            </template>
            <template #detail>
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
                  <pre class="detail-pre command-pre">{{ expandedDetail.borg_command }}</pre>
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
            </template>
          </LogRow>

          <LogRow
            v-if="row.kind === 'system' && row.event"
            class="run-card-system"
            :tone="systemEventTone(row.event.severity)"
            :title="row.event.hostname ?? 'Server'"
            :subtitle="formatEventType(row.event.event_type)"
            :facts="[row.event.message]"
            :time="formatDateShort(row.event.created_at)"
            :time-title="formatDate(row.event.created_at)"
            :expanded="expandedSystemId === row.event.id"
            :dimmed="row.event.acknowledged"
            @toggle="toggleSystemRow(row.event)"
          >
            <template
              v-if="row.event.acknowledged"
              #badges
            >
              <span class="badge badge--neutral">Acknowledged</span>
            </template>
            <template
              v-if="isSystemEventAckable(row.event)"
              #actions
            >
              <button
                class="btn btn-sm"
                :disabled="ackingSystemId === row.event.id"
                @click="toggleSystemAcknowledge(row.event)"
              >
                {{ row.event.acknowledged ? 'Unacknowledge' : 'Acknowledge' }}
              </button>
            </template>
            <template #detail>
              <pre :class="systemEventPreClass(row.event)">{{ row.event.message }}</pre>
            </template>
          </LogRow>
        </template>
      </LogList>

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
  word-break: break-all;
}

.load-more {
  display: flex;
  justify-content: flex-start;
  padding: var(--space-4) 0;
}

.filter-group-search {
  flex: 1;
  min-width: 180px;
}

/* A browser source location is one long unbroken token. */
.log-detail-value {
  word-break: break-all;
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
  background: var(--bg-code);
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
