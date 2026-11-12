<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { runLogLink } from '../utils/activityLink'
import { ref, computed, onMounted } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { apiClient } from '../api/client'
import { listSchedules, getScheduleHealth } from '../api/schedules'
import { listRepos } from '../api/repos'
import { listAgents } from '../api/agents'
import { formatDateShort } from '../utils/format'
import { cronToHuman } from '../utils/cron'
import { useWebSocket } from '../composables/useWebSocket'
import { useMobile } from '../composables/useMobile'
import { useListSort } from '../composables/useListSort'
import { usePersistedRef, useQueryOverride } from '../composables/usePersistedRef'
import { useAsyncAction } from '../composables/useAsyncAction'
import { getConfiguredTimezone } from '../composables/useTimezone'
import { logger } from '../utils/logger'
import { normalizeBackupStatus } from '../utils/backupStatus'
import { isAgentOffline, lastSeenText } from '../utils/agent'
import type { BadgeTone } from '../utils/badge'
import { scheduleRunStatus, type ScheduleHealthEntry } from '../utils/scheduleHealth'
import { scheduleRowHealth, type ScheduleRowHealth } from '../utils/scheduleRowHealth'
import {
  DEFAULT_RUN_ESTIMATE_SECS,
  findScheduleOverlaps,
  overlappingAgents,
  type ScheduleOverlap,
} from '../utils/scheduleOverlap'
import { parseFilterQuery, matchesFilterQuery, type FilterSubject } from '../utils/filterQuery'
import {
  AlertTriangle,
  ArrowRight,
  ChevronDown,
  ChevronRight,
  Clock,
  Plus,
  SlidersHorizontal,
} from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import SortControls from '../components/SortControls.vue'
import BaseSegmented, { type SegmentedOption } from '../components/BaseSegmented.vue'
import FilterSyntaxHelp from '../components/FilterSyntaxHelp.vue'
import type { AgentRow } from '../types/agent'
import { scheduleDisabledLabel } from '../utils/scheduleStatus'
import type { ScheduleRow } from '../types/schedule'
import { scheduleTypeShortLabel } from '../utils/scheduleType'
import type { Repo } from '../types/repo'

/**
 * Local shape for `/stats/activity` responses, matching the wire fields of
 * `db::ActivityRow` (not ts-rs generated - see the same pattern in
 * DashboardView/ActivityLogView). Only the fields this view consumes.
 */
interface ScheduleActivityEntry {
  id: number
  started_at: string
  duration_secs: number
  status: string
  schedule_id: number | null
  run_id: string | null
  hostname: string
  target_name: string
  /** The repository the run wrote into; a multi-repository schedule has several. */
  repo_id?: number | null
}

const ACTIVITY_WINDOW_DAYS = 30
// How many of each schedule's latest runs are fetched: enough to see every
// repository it writes into and to estimate how long its next run takes.
// The backend counts runs (every agent's report for one firing), not report
// rows, so a multi-agent schedule still gets its last ten firings.
const RECENT_RUNS_PER_SCHEDULE = 10

const schedules = ref<ScheduleRow[]>([])
const repos = ref<Repo[]>([])
const agents = ref<AgentRow[]>([])
const health = ref<ScheduleHealthEntry[]>([])
const activity = ref<ScheduleActivityEntry[]>([])
const { loading, error, run } = useAsyncAction('Failed to load schedules.')
const router = useRouter()
const route = useRoute()
type SortField = 'agent' | 'next_run' | 'last_run' | 'type'

const SORT_OPTIONS: readonly { field: SortField; label: string }[] = [
  { field: 'agent', label: 'Agent' },
  { field: 'next_run', label: 'Next run' },
  { field: 'last_run', label: 'Last run' },
  { field: 'type', label: 'Type' },
]
type FilterStatus = 'all' | 'enabled' | 'disabled'
type FilterType = 'all' | 'backup' | 'check' | 'verify'
type FilterHealth = 'all' | 'overdue' | 'success' | 'warning' | 'failed'

function isFilterStatus(value: string): value is FilterStatus {
  return value === 'all' || value === 'enabled' || value === 'disabled'
}
function isFilterType(value: string): value is FilterType {
  return value === 'all' || value === 'backup' || value === 'check' || value === 'verify'
}
function isFilterHealth(value: string): value is FilterHealth {
  return (
    value === 'all' ||
    value === 'overdue' ||
    value === 'success' ||
    value === 'warning' ||
    value === 'failed'
  )
}

/**
 * How the rows are bucketed into groups. Time answers "what runs next",
 * agent "what does this machine back up", repo "what writes into this
 * repository" - the last two cut across the time buckets, so they are a mode
 * rather than another sort field.
 */
type GroupMode = 'time' | 'agent' | 'repo'
function isGroupMode(value: string): value is GroupMode {
  return value === 'time' || value === 'agent' || value === 'repo'
}

const GROUP_OPTIONS: readonly SegmentedOption<GroupMode>[] = [
  { value: 'time', label: 'Time' },
  { value: 'agent', label: 'Agent' },
  { value: 'repo', label: 'Repo' },
]
// Every toolbar choice below is persisted under its own `assimilate-schedules-*`
// key and restored on the next visit - how this list is grouped, sorted or
// filtered is a setting a person makes once, not a one-off query that should
// reset the moment they navigate away. `filterHealth` alone also takes a
// query override, since a link such as `?filter=overdue` is an explicit
// choice made just now and should win over whatever was last persisted.
const groupMode = usePersistedRef<GroupMode>('assimilate-schedules-group', 'time', isGroupMode)

const {
  field: sortField,
  direction: sortDir,
  toggle: toggleSort,
  sign: sortSign,
} = useListSort<SortField>('agent', 'asc', {
  key: 'assimilate-schedules-sort',
  fields: SORT_OPTIONS.map((option) => option.field),
})
const filterStatus = usePersistedRef<FilterStatus>(
  'assimilate-schedules-filter-status',
  'all',
  isFilterStatus,
)
const filterType = usePersistedRef<FilterType>(
  'assimilate-schedules-filter-type',
  'all',
  isFilterType,
)
const filterText = ref('')
const filterHealth = usePersistedRef<FilterHealth>(
  'assimilate-schedules-filter-health',
  'all',
  isFilterHealth,
  route.query.filter as string | undefined,
)

const { isMobile } = useMobile()
const showMobileFilters = ref(false)

const repoMap = computed(() => {
  const m = new Map<number, Repo>()
  repos.value.forEach((r) => m.set(r.id, r))
  return m
})

const agentMap = computed(() => {
  const map = new Map<string, AgentRow>()
  agents.value.forEach((agent) => map.set(agent.hostname, agent))
  return map
})

/**
 * How many agents report each hostname. Only `(hostname, domain)` is unique
 * (`agents_hostname_domain_idx`), so a hostname can name more than one machine
 * - and a schedule's `target_hostnames` carries the hostname alone, with no
 * way to tell which of them it targets.
 */
const agentsPerHostname = computed(() => {
  const counts = new Map<string, number>()
  agents.value.forEach((agent) => counts.set(agent.hostname, (counts.get(agent.hostname) ?? 0) + 1))
  return counts
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

const activityBySchedule = computed(() => {
  const map = new Map<number, ScheduleActivityEntry[]>()
  for (const entry of activity.value) {
    if (entry.schedule_id === null) continue
    const list = map.get(entry.schedule_id) ?? []
    list.push(entry)
    map.set(entry.schedule_id, list)
  }
  return map
})

/**
 * Every repository a schedule writes into. The row itself names only its
 * primary one, so the others are read off the runs it recently made - a
 * multi-repository schedule leaves one report per repository behind.
 */
function repoIdsOf(s: ScheduleRow): number[] {
  const ids = new Set<number>()
  if (s.repo_id !== null) ids.add(s.repo_id)
  for (const entry of activityBySchedule.value.get(s.id) ?? []) {
    if (entry.repo_id != null) ids.add(entry.repo_id)
  }
  return [...ids]
}

/** Durations of a schedule's recent finished runs, for estimating the next one. */
function finishedDurations(scheduleId: number): number[] {
  return (activityBySchedule.value.get(scheduleId) ?? [])
    .filter((entry) => {
      const status = normalizeBackupStatus(entry.status)
      return status === 'success' || status === 'warning' || status === 'failed'
    })
    .map((entry) => entry.duration_secs)
}

/**
 * Schedules on different agents whose next runs write into one repository at
 * the same time - the one collision worth a warning, since the repository
 * lock makes one of them wait or fail. Checked across every enabled schedule,
 * not just the filtered ones: a filter does not make the clash go away.
 */
const overlaps = computed<ScheduleOverlap[]>(() =>
  findScheduleOverlaps(
    schedules.value
      .filter((s) => s.enabled)
      .map((s) => ({
        id: s.id,
        agents: s.target_hostnames,
        repoIds: repoIdsOf(s),
        nextStartMs: s.next_run_at ? new Date(s.next_run_at).getTime() : null,
        recentDurationsSecs: finishedDurations(s.id),
      })),
  ),
)

const overlapAgentsBySchedule = computed(() => overlappingAgents(overlaps.value))

function repoName(repoId: number): string {
  return repoMap.value.get(repoId)?.name ?? `repo #${repoId}`
}

/** "02:00" in the configured timezone. */
function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
    timeZone: getConfiguredTimezone(),
  })
}

interface OverlapNotice {
  key: string
  repoId: number
  title: string
  detail: string
}

const overlapNotices = computed<OverlapNotice[]>(() =>
  overlaps.value.map((o) => {
    const estimate = o.first.estimate.fromHistory
      ? 'estimated from recent runs'
      : `assumed ${DEFAULT_RUN_ESTIMATE_SECS / 60} minutes, no recent runs`
    return {
      key: `${o.repoId}:${o.first.scheduleId}:${o.second.scheduleId}`,
      repoId: o.repoId,
      title: `Two agents write to repo ${repoName(o.repoId)} at the same time`,
      detail:
        `${o.first.agent} runs ${clock(o.first.startMs)} – ~${clock(o.first.endMs)} ` +
        `(${estimate}), ${o.second.agent} starts ${clock(o.second.startMs)}. ` +
        'The repo lock makes one of them wait or fail.',
    }
  }),
)

interface EnrichedSchedule extends ScheduleRow {
  hostLabels: string[]
  repo: Repo | null
  repoNames: string[]
  health: ScheduleHealthEntry | null
  isOverdue: boolean
  rowHealth: ScheduleRowHealth
}

const RUNNING_STATUSES = new Set(['pending', 'started'])

function rowName(s: ScheduleRow): string {
  if (s.name) return s.name
  if (s.repo_id === null) return 'no repository'
  return repoName(s.repo_id)
}

const enrichedSchedules = computed<EnrichedSchedule[]>(() =>
  schedules.value.map((s) => {
    const hostLabels = s.target_hostnames.map(hostLabel)
    const repo: Repo | null = s.repo_id != null ? (repoMap.value.get(s.repo_id) ?? null) : null
    const entries = healthBySchedule.value.get(s.id) ?? []
    const healthEntry: ScheduleHealthEntry | null =
      entries.find((h) => h.is_overdue) ??
      entries.find((h) => scheduleRunStatus(h) === 'failed') ??
      entries[0] ??
      null
    const overdueEntries = entries.filter((h) => h.is_overdue)
    const offlineAgents = s.target_hostnames.flatMap((hostname) => {
      const agent = agentMap.value.get(hostname)
      return agent && isAgentOffline(agent)
        ? [`${hostLabel(hostname)} - ${lastSeenText(agent)}`]
        : []
    })
    const rowHealth = scheduleRowHealth({
      enabled: s.enabled,
      disabledLabel: scheduleDisabledLabel(s),
      autoDisabled: !s.enabled && (s.auto_disabled_agent_unreachable || s.consecutive_failures > 0),
      running: entries.some((h) => h.last_status != null && RUNNING_STATUSES.has(h.last_status)),
      entries,
      overdueTitle: overdueMessage(overdueEntries),
      offlineAgents,
      catchUpPendingCount: s.catch_up_pending_count ?? 0,
      overlapAgents: overlapAgentsBySchedule.value.get(s.id) ?? [],
    })
    return {
      ...s,
      hostLabels,
      repo,
      repoNames: repoIdsOf(s).map(repoName),
      health: healthEntry,
      isOverdue: s.enabled && overdueEntries.length > 0,
      rowHealth,
    }
  }),
)

/**
 * What a schedule offers each filter field. `host` is the storage host the
 * repository lives on - the machine being backed up is the `agent`, and the two
 * were previously indistinguishable because the search box only ever looked at
 * the agent labels.
 */
function filterSubject(s: EnrichedSchedule): FilterSubject {
  return {
    name: [s.name],
    // hostLabels carry "Display name (hostname)", so one entry covers both.
    agent: s.hostLabels,
    host: [s.repo?.ssh_host ?? null],
    repo: [s.repo?.name ?? null],
  }
}

function isFailing(s: EnrichedSchedule): boolean {
  return scheduleRunStatus(s.health) === 'failed'
}

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
    list = list.filter(isFailing)
  }

  const query = parseFilterQuery(filterText.value)
  if (query.length > 0) {
    list = list.filter((s) => matchesFilterQuery(query, filterSubject(s)))
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
  // Problems first, each half still in the chosen order: the sort is stable.
  list.sort((a, b) => Number(b.rowHealth.problem) - Number(a.rowHealth.problem))

  return list
})

/**
 * The counted pills over the list. Each one is a shortcut for a combination
 * of the selects in the toolbar, so the two always agree on what is shown.
 */
type QuickFilter = 'all' | 'overdue' | 'failing' | 'disabled'

interface QuickPill {
  value: QuickFilter
  label: string
  count: number
  dot: BadgeTone | null
}

const quickFilter = computed<QuickFilter | null>(() => {
  if (filterType.value !== 'all') return null
  if (filterStatus.value === 'all' && filterHealth.value === 'all') return 'all'
  if (filterStatus.value === 'all' && filterHealth.value === 'overdue') return 'overdue'
  if (filterStatus.value === 'all' && filterHealth.value === 'failed') return 'failing'
  if (filterStatus.value === 'disabled' && filterHealth.value === 'all') return 'disabled'
  return null
})

const quickPills = computed<QuickPill[]>(() => {
  const all = enrichedSchedules.value
  return [
    { value: 'all', label: 'All', count: all.length, dot: null },
    {
      value: 'overdue',
      label: 'Overdue',
      count: all.filter((s) => s.health?.is_overdue).length,
      dot: 'warning',
    },
    { value: 'failing', label: 'Failing', count: all.filter(isFailing).length, dot: 'danger' },
    {
      value: 'disabled',
      label: 'Disabled',
      count: all.filter((s) => !s.enabled).length,
      dot: 'neutral',
    },
  ]
})

function applyQuickFilter(value: QuickFilter): void {
  filterType.value = 'all'
  filterStatus.value = value === 'disabled' ? 'disabled' : 'all'
  if (value === 'overdue') filterHealth.value = 'overdue'
  else if (value === 'failing') filterHealth.value = 'failed'
  else filterHealth.value = 'all'
}

function openRunIssue(scheduleId: number, kind: 'failed' | 'warning'): void {
  router.push(runLogLink({ scheduleId, status: kind }))
}

/** `.status-dot` modifier for a tone; neutral is the dot's own grey. */
function dotClass(tone: BadgeTone | null): string | null {
  if (tone === null || tone === 'neutral') return null
  return `status-dot--${tone}`
}

type TimeBucketKey = 'now' | 'next6' | 'next24' | 'week' | 'later' | 'unscheduled' | 'disabled'

const TIME_BUCKETS: { key: TimeBucketKey; title: string }[] = [
  { key: 'now', title: 'Due now' },
  { key: 'next6', title: 'Next 6 hours' },
  { key: 'next24', title: 'Next 24 hours' },
  { key: 'week', title: 'This week' },
  { key: 'later', title: 'Later' },
  { key: 'unscheduled', title: 'Unscheduled' },
  { key: 'disabled', title: 'Disabled' },
]

const HOUR_MS = 3_600_000
const DAY_MS = 24 * HOUR_MS

function timeBucketOf(s: EnrichedSchedule): TimeBucketKey {
  if (!s.enabled) return 'disabled'
  if (!s.next_run_at) return 'unscheduled'
  const ms = new Date(s.next_run_at).getTime() - Date.now()
  if (ms <= 0) return 'now'
  if (ms <= 6 * HOUR_MS) return 'next6'
  if (ms <= DAY_MS) return 'next24'
  if (ms <= 7 * DAY_MS) return 'week'
  return 'later'
}

interface ScheduleGroup {
  /** Unique across the group modes, so switching mode always re-keys the list. */
  key: string
  title: string
  /** Names a state of the section itself, beside its title. */
  badge: { label: string; title: string } | null
  schedules: EnrichedSchedule[]
}

const timeGroups = computed<ScheduleGroup[]>(() => {
  const buckets = new Map<TimeBucketKey, EnrichedSchedule[]>()
  for (const s of filteredSchedules.value) {
    const key = timeBucketOf(s)
    const list = buckets.get(key) ?? []
    list.push(s)
    buckets.set(key, list)
  }
  return TIME_BUCKETS.map(({ key, title }) => ({
    key: `time:${key}`,
    title,
    badge: null,
    schedules: buckets.get(key) ?? [],
  })).filter((group) => group.schedules.length > 0)
})

/**
 * How one group mode buckets the schedules: which keys a schedule belongs
 * under, what each key is called in its header, and what the trailing section
 * of schedules with no key at all is called.
 */
interface GroupSpec<K extends string | number> {
  /** Prefixes every key this mode emits, keeping them unique across modes. */
  prefix: string
  /** Empty puts the schedule in the fallback section; several fan it out. */
  keysOf: (s: EnrichedSchedule) => K[]
  titleOf: (key: K) => string
  /** A state of the section worth naming beside its title, if any. */
  badgeOf?: (key: K) => ScheduleGroup['badge']
  fallbackTitle: string
}

/**
 * The shape both the agent and the repo mode need: bucket by key, order the
 * sections by the label they show, and append the keyless ones last. Only the
 * key and the two labels differ between them, so they are a spec, not a second
 * copy of this.
 */
function buildGroups<K extends string | number>(
  schedules: EnrichedSchedule[],
  { prefix, keysOf, titleOf, badgeOf, fallbackTitle }: GroupSpec<K>,
): ScheduleGroup[] {
  const byKey = new Map<K, EnrichedSchedule[]>()
  const keyless: EnrichedSchedule[] = []
  for (const s of schedules) {
    const keys = keysOf(s)
    if (keys.length === 0) {
      keyless.push(s)
      continue
    }
    for (const key of keys) {
      const list = byKey.get(key) ?? []
      list.push(s)
      byKey.set(key, list)
    }
  }
  const groups: ScheduleGroup[] = [...byKey.entries()]
    .map(([key, grouped]) => ({
      key: `${prefix}:${key}`,
      title: titleOf(key),
      badge: badgeOf?.(key) ?? null,
      schedules: grouped,
    }))
    .sort((a, b) => a.title.localeCompare(b.title))
  if (keyless.length > 0) {
    // A repository can be named "No repository", which would put two sections
    // under one title even though their keys differ. The keyless one gives way,
    // since the other is titled with a name its owner chose.
    const collides = groups.some((group) => group.title === fallbackTitle)
    groups.push({
      key: `${prefix}:none`,
      title: collides ? `${fallbackTitle} (none assigned)` : fallbackTitle,
      badge: null,
      schedules: keyless,
    })
  }
  return groups
}

/**
 * One section per targeted agent, ordered by the label shown in the header.
 * A schedule targeting several agents is listed under each of them - the whole
 * point of the view is to answer "what backs up this machine", which a card
 * shown only under its first target would not.
 */
const agentGroups = computed<ScheduleGroup[]>(() =>
  buildGroups(filteredSchedules.value, {
    prefix: 'agent',
    // Deduplicated because a hostname is unique only per domain
    // (`agents_hostname_domain_idx`): a schedule targeting two agents that
    // report the same hostname from different domains lists it twice, which
    // would render its card twice in the one section they share.
    keysOf: (s) => [...new Set(s.target_hostnames)],
    // A display name would name one of the agents sharing the hostname and
    // silently mislabel the others' schedules, so an ambiguous section keeps
    // the bare hostname and carries a badge saying how many agents it covers.
    titleOf: (hostname) =>
      (agentsPerHostname.value.get(hostname) ?? 0) > 1 ? hostname : hostLabel(hostname),
    badgeOf: (hostname) => {
      const count = agentsPerHostname.value.get(hostname) ?? 0
      if (count <= 1) return null
      return {
        label: `${count} agents`,
        title:
          `${count} agents report the hostname ${hostname} from different domains. ` +
          'A schedule names only the hostname it targets, so their schedules share this section.',
      }
    },
    fallbackTitle: 'No agents',
  }),
)

/** One section per repository the schedules write into, unassigned ones last. */
const repoGroups = computed<ScheduleGroup[]>(() =>
  buildGroups(filteredSchedules.value, {
    prefix: 'repo',
    keysOf: (s) => (s.repo_id === null ? [] : [s.repo_id]),
    titleOf: (repoId) => repoMap.value.get(repoId)?.name ?? `repo #${repoId}`,
    fallbackTitle: 'No repository',
  }),
)

/**
 * The Disabled group starts folded to its header: nothing in it runs, so it
 * is there to be found rather than read. Opening it lasts until the page is
 * left. A search or the Disabled filter asks for exactly these rows, so
 * either one shows them without the extra click.
 */
const COLLAPSED_BY_DEFAULT = new Set(['time:disabled'])
const expandedGroups = ref(new Set<string>())

function isCollapsible(group: ScheduleGroup): boolean {
  return (
    COLLAPSED_BY_DEFAULT.has(group.key) &&
    filterText.value.trim() === '' &&
    filterStatus.value !== 'disabled'
  )
}

function isCollapsed(group: ScheduleGroup): boolean {
  return isCollapsible(group) && !expandedGroups.value.has(group.key)
}

function toggleGroup(group: ScheduleGroup): void {
  const next = new Set(expandedGroups.value)
  if (next.has(group.key)) next.delete(group.key)
  else next.add(group.key)
  expandedGroups.value = next
}

function overdueCount(group: ScheduleGroup): number {
  return group.schedules.filter((s) => s.isOverdue).length
}

/** The Agent column: the first target, and how many more there are. */
function agentsLabel(s: EnrichedSchedule): string {
  const [first, ...rest] = s.target_hostnames
  if (first === undefined) return 'no agents'
  return rest.length > 0 ? `${first} +${rest.length}` : first
}

function reposLabel(s: EnrichedSchedule): string {
  return s.repoNames.length > 0 ? s.repoNames.join(', ') : 'no repository'
}

function cadence(s: EnrichedSchedule): string {
  return cronToHuman(s.cron_expression) ?? s.cron_expression
}

/** Next run, or a dash for a schedule that will not run. */
function nextRunText(s: EnrichedSchedule): string {
  return s.enabled ? formatDateShort(s.next_run_at) : '\u2014'
}

// Time last rather than in a `switch`: `vue/return-in-computed-property` does
// not follow a union's exhaustiveness, and only the selected mode's groups are
// computed either way.
const groupedSchedules = computed<ScheduleGroup[]>(() => {
  if (groupMode.value === 'agent') return agentGroups.value
  if (groupMode.value === 'repo') return repoGroups.value
  return timeGroups.value
})

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
    const [scheduleRows, repoRows, agentRows, healthRows, activityRes] = await Promise.all([
      listSchedules(),
      listRepos(),
      listAgents(),
      getScheduleHealth(),
      // The backend caps this per schedule (not the result set overall), so
      // RECENT_RUNS_PER_SCHEDULE alone is enough to guarantee every card gets its
      // own last-N runs regardless of how many schedules exist or how often
      // any one of them runs.
      apiClient
        .get<
          ScheduleActivityEntry[]
        >(`/stats/activity?days=${ACTIVITY_WINDOW_DAYS}&limit_per_schedule=${RECENT_RUNS_PER_SCHEDULE}`)
        .catch(() => ({ data: [] as ScheduleActivityEntry[] })),
    ])
    schedules.value = scheduleRows
    repos.value = repoRows
    agents.value = agentRows
    health.value = healthRows
    activity.value = activityRes.data
  })
}

onMounted(fetchAll)

const { onMessage } = useWebSocket()
onMessage('DataChanged', () => fetchAll().catch(logger.error))
// A run reaching its host clears the schedule's missed-backup streak, and a
// scheduled start sends no DataChanged of its own - without this, the
// "N missed" chip would stay until the whole run had finished. Health alone,
// since that is all the chip reads: many schedules often share a start time,
// and a full refetch per start would be a burst of requests for nothing.
async function refreshHealth(): Promise<void> {
  health.value = await getScheduleHealth()
}
onMessage('BackupStarted', () => refreshHealth().catch(logger.error))

useQueryOverride(() => route.query.filter, isFilterHealth, filterHealth)
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
        placeholder="Filter by name, agent, host, or repo..."
      />
      <FilterSyntaxHelp />
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
      <div
        class="filter-pills"
        role="group"
        aria-label="Show"
      >
        <button
          v-for="pill in quickPills"
          :key="pill.value"
          type="button"
          class="filter-pill"
          :class="{ active: quickFilter === pill.value }"
          :aria-pressed="quickFilter === pill.value"
          @click="applyQuickFilter(pill.value)"
        >
          <span
            v-if="pill.dot"
            class="status-dot"
            :class="dotClass(pill.dot)"
          ></span>
          {{ pill.label }} · {{ pill.count }}
        </button>
      </div>
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
        <div class="group-controls">
          <span class="sort-label">Group:</span>
          <BaseSegmented
            v-model="groupMode"
            :options="GROUP_OPTIONS"
            label="Group schedules by"
          />
        </div>
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
      action="New schedule"
      @action="router.push('/schedules/new')"
    />

    <div
      v-else
      class="list-stack schedules-list"
    >
      <!--
        Only a real clash earns a call-out: two agents whose next runs write
        into one repository at the same time. The link opens the repository's
        schedules, which is where the two can be told apart and one moved.
      -->
      <RouterLink
        v-for="notice in overlapNotices"
        :key="notice.key"
        :to="`/repos/${notice.repoId}?tab=schedules`"
        class="notice-bar"
      >
        <AlertTriangle :size="16" />
        <span class="notice-bar-text">
          <span class="notice-bar-title">{{ notice.title }}</span>
          <span>{{ notice.detail }}</span>
        </span>
        <ChevronRight :size="16" />
      </RouterLink>

      <section
        v-for="group in groupedSchedules"
        :key="group.key"
        class="list-group"
        :data-group="group.key"
      >
        <div class="list-group-header">
          <h2 class="list-group-title">
            <button
              v-if="isCollapsible(group)"
              type="button"
              class="schedules-group-toggle"
              :aria-expanded="!isCollapsed(group)"
              @click="toggleGroup(group)"
            >
              <ChevronRight
                v-if="isCollapsed(group)"
                :size="16"
              />
              <ChevronDown
                v-else
                :size="16"
              />
              {{ group.title }}
            </button>
            <template v-else>{{ group.title }}</template>
          </h2>
          <span
            v-if="group.badge"
            class="badge badge--warning"
            :title="group.badge.title"
            >{{ group.badge.label }}</span
          >
          <span class="list-group-count">{{ group.schedules.length }}</span>
          <span
            v-if="overdueCount(group) > 0"
            class="list-group-aside schedules-aside-warning"
            >{{ overdueCount(group) }} overdue</span
          >
        </div>
        <template v-if="!isCollapsed(group)">
          <div
            class="list-cols"
            aria-hidden="true"
          >
            <span>Schedule</span>
            <span>Agent</span>
            <span class="schedules-health-head">Health</span>
            <span>Next run</span>
            <span></span>
          </div>
          <RouterLink
            v-for="s in group.schedules"
            :key="s.id"
            :to="`/schedules/${s.id}`"
            class="list-row"
            :data-schedule-id="s.id"
          >
            <span class="list-row-main">
              <span
                class="status-dot"
                :class="dotClass(s.rowHealth.tone)"
              ></span>
              <span class="list-row-text">
                <span class="list-row-name">{{ rowName(s) }}</span>
                <span class="list-row-sub"
                  >{{ cadence(s)
                  }}<span class="list-wide-only">
                    · {{ scheduleTypeShortLabel(s.schedule_type) }}
                    <ArrowRight
                      :size="12"
                      class="schedules-sub-arrow"
                    />
                    {{ reposLabel(s) }}</span
                  ><span class="list-narrow-only"> · {{ agentsLabel(s) }}</span></span
                >
              </span>
              <span
                class="list-narrow-only list-row-value schedules-next-narrow"
                :class="{ 'schedules-next--overdue': s.isOverdue }"
                >{{ nextRunText(s) }}</span
              >
            </span>
            <span
              class="list-row-value mono list-wide-only schedules-agent"
              :title="s.hostLabels.join(', ')"
              >{{ agentsLabel(s) }}</span
            >
            <span class="list-row-chips">
              <!-- The dot already says healthy; anything else is named, in the
                   dot's colour, so it reads without a second, matching pill. -->
              <!-- A failed or warned run links to its output in the Activity
                   log; the row itself still opens the schedule. -->
              <span
                v-if="s.rowHealth.runIssue"
                class="schedules-health schedules-health--link"
                :class="`schedules-health--${s.rowHealth.tone}`"
                :title="s.rowHealth.title || undefined"
                role="link"
                tabindex="0"
                @click.prevent.stop="openRunIssue(s.id, s.rowHealth.runIssue)"
                @keydown.enter.prevent.stop="openRunIssue(s.id, s.rowHealth.runIssue)"
                >{{ s.rowHealth.label }}</span
              >
              <span
                v-else-if="s.rowHealth.tone !== 'success'"
                class="schedules-health"
                :class="`schedules-health--${s.rowHealth.tone}`"
                :title="s.rowHealth.title || undefined"
                >{{ s.rowHealth.label }}</span
              >
            </span>
            <span
              class="list-row-value list-wide-only"
              :class="{ 'schedules-next--overdue': s.isOverdue }"
              >{{ nextRunText(s) }}</span
            >
            <ChevronRight
              :size="16"
              class="list-row-chevron"
            />
          </RouterLink>
        </template>
      </section>
    </div>
  </div>
</template>

<style scoped>
.schedules-view {
  max-width: 1100px;
  overflow-x: hidden;
  min-width: 0;
}

/* Four filter controls plus the pills, a group control and the sort strip no
   longer fit the 1100px content column on one line, and `.schedules-view`
   hides its overflow - so without this the sort strip is simply cut off.
   Wrapping keeps `.sort-controls`' trailing `margin-left: auto`, so it lands
   right-aligned on the second row. */
.toolbar {
  flex-wrap: wrap;
}

/* The "Group: [Time|Agent|Repo]" pair in the toolbar. `.sort-controls` beside
   it already claims the row's trailing space, so this only has to keep its
   caption glued to the control and out of the toolbar's shrink. */
.group-controls {
  display: flex;
  align-items: center;
  flex-shrink: 0;
}

.schedules-list .list-group {
  /* Every column a share of the row, never its content's width: each row is
     a grid of its own, so a width taken from content set them apart. */
  --list-cols: minmax(0, 2.4fr) minmax(0, 1fr) minmax(0, 1.4fr) 9rem 1.25rem;
}

/* The Disabled group's title is a button that folds the group open. */
.schedules-group-toggle {
  display: inline-flex;
  align-items: center;
  gap: var(--space-3);
  padding: 0;
  border: 0;
  background: none;
  font: inherit;
  color: inherit;
  text-align: left;
  cursor: pointer;
}

.schedules-aside-warning {
  color: var(--warning);
  font-size: var(--fs-sm);
}

.schedules-sub-arrow {
  /* An inline arrow; Tailwind's preflight makes every svg a block, which put
     it on a line of its own. */
  display: inline;
  vertical-align: middle;
}

.schedules-agent {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.schedules-health-head {
  text-align: right;
}

.schedules-health {
  flex: 1 1 auto;
  min-width: 0;
  text-align: right;
  font-size: var(--fs-sm);
  color: var(--text-secondary);
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.schedules-health--link {
  cursor: pointer;
}

.schedules-health--link:hover,
.schedules-health--link:focus-visible {
  text-decoration: underline;
}

.schedules-health--warning {
  color: var(--warning);
}

.schedules-health--danger {
  color: var(--danger);
}

.schedules-health--accent,
.schedules-health--info {
  color: var(--accent);
}

.schedules-next--overdue {
  color: var(--warning);
}

/* On a phone the next run sits at the end of the name's line, clear of the
   chevron that is pinned to the row's top-right corner. */
.schedules-next-narrow {
  flex-shrink: 0;
  margin-left: auto;
  margin-right: var(--space-7);
  font-size: var(--fs-xs);
  white-space: nowrap;
}
</style>
