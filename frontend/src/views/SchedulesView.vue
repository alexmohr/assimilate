<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { apiClient } from '../api/client'
import { listSchedules, updateSchedule, cancelSchedule, getScheduleHealth } from '../api/schedules'
import { listRepos } from '../api/repos'
import { listAgents } from '../api/agents'
import { formatDateShort } from '../utils/format'
import { cronToHuman } from '../utils/cron'
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
import { parseFilterQuery, matchesFilterQuery, type FilterSubject } from '../utils/filterQuery'
import { Plus, Clock, SlidersHorizontal } from '@lucide/vue'
import BaseSpinner from '../components/BaseSpinner.vue'
import EmptyState from '../components/EmptyState.vue'
import SortControls from '../components/SortControls.vue'
import EntityStatusBadges, { type EntityIssue } from '../components/EntityStatusBadges.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import RunHistoryStrip, { type RunHistoryEntry } from '../components/RunHistoryStrip.vue'
import ScheduleTimelineRail, { type TimelineEntry } from '../components/ScheduleTimelineRail.vue'
import FilterSyntaxHelp from '../components/FilterSyntaxHelp.vue'
import type { AgentRow } from '../types/agent'
import { scheduleDisabledLabel } from '../utils/scheduleStatus'
import type { ScheduleRow, ScheduleType } from '../types/schedule'
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
}

const ACTIVITY_WINDOW_DAYS = 30
// Matches RunHistoryStrip's default `maxBars` - each card only ever renders
// this many of its most recent runs.
const RUN_HISTORY_BARS = 10

const schedules = ref<ScheduleRow[]>([])
const repos = ref<Repo[]>([])
const agents = ref<AgentRow[]>([])
const health = ref<ScheduleHealthEntry[]>([])
const activity = ref<ScheduleActivityEntry[]>([])
const { loading, error, run } = useAsyncAction('Failed to load schedules.')
const router = useRouter()
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
      return 'Integrity check'
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

  return list
})

const runsBySchedule = computed(() => {
  const map = new Map<number, RunHistoryEntry[]>()
  for (const entry of activity.value) {
    if (entry.schedule_id === null) continue
    const list = map.get(entry.schedule_id) ?? []
    list.push({
      id: entry.id,
      startedAt: entry.started_at,
      durationSecs: entry.duration_secs,
      status: entry.status,
    })
    map.set(entry.schedule_id, list)
  }
  return map
})

type TimeBucketKey = 'now' | 'next6' | 'next24' | 'week' | 'later' | 'unscheduled' | 'paused'

const TIME_BUCKETS: { key: TimeBucketKey; title: string }[] = [
  { key: 'now', title: 'Due now' },
  { key: 'next6', title: 'Next 6 hours' },
  { key: 'next24', title: 'Next 24 hours' },
  { key: 'week', title: 'This week' },
  { key: 'later', title: 'Later' },
  { key: 'unscheduled', title: 'Unscheduled' },
  { key: 'paused', title: 'Paused' },
]

const HOUR_MS = 3_600_000
const DAY_MS = 24 * HOUR_MS

function timeBucketOf(s: EnrichedSchedule): TimeBucketKey {
  if (!s.enabled) return 'paused'
  if (!s.next_run_at) return 'unscheduled'
  const ms = new Date(s.next_run_at).getTime() - Date.now()
  if (ms <= 0) return 'now'
  if (ms <= 6 * HOUR_MS) return 'next6'
  if (ms <= DAY_MS) return 'next24'
  if (ms <= 7 * DAY_MS) return 'week'
  return 'later'
}

const groupedSchedules = computed(() => {
  const buckets = new Map<TimeBucketKey, EnrichedSchedule[]>()
  for (const s of filteredSchedules.value) {
    const key = timeBucketOf(s)
    const list = buckets.get(key) ?? []
    list.push(s)
    buckets.set(key, list)
  }
  return TIME_BUCKETS.map(({ key, title }) => ({
    key,
    title,
    schedules: buckets.get(key) ?? [],
  })).filter((group) => group.schedules.length > 0)
})

const railEntries = computed<TimelineEntry[]>(() =>
  filteredSchedules.value
    .filter((s) => s.enabled && s.next_run_at)
    .map((s) => ({
      id: s.id,
      label: s.name || s.repo?.name || `Schedule #${s.id}`,
      atIso: s.next_run_at as string,
      host: s.repo?.ssh_host ?? null,
      repoId: s.repo_id,
      repoName: s.repo?.name ?? null,
    })),
)

function openTimelineEntry(entry: TimelineEntry): void {
  router.push(`/schedules/${entry.id}`)
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
    const [scheduleRows, repoRows, agentRows, healthRows, activityRes] = await Promise.all([
      listSchedules(),
      listRepos(),
      listAgents(),
      getScheduleHealth(),
      // The backend caps this per schedule (not the result set overall), so
      // RUN_HISTORY_BARS alone is enough to guarantee every card gets its
      // own last-N runs regardless of how many schedules exist or how often
      // any one of them runs.
      apiClient
        .get<
          ScheduleActivityEntry[]
        >(`/stats/activity?days=${ACTIVITY_WINDOW_DAYS}&limit_per_schedule=${RUN_HISTORY_BARS}`)
        .catch(() => ({ data: [] as ScheduleActivityEntry[] })),
    ])
    schedules.value = scheduleRows
    repos.value = repoRows
    agents.value = agentRows
    health.value = healthRows
    activity.value = activityRes.data
  })
}

function navigateToSchedule(s: ScheduleRow): void {
  router.push(`/schedules/${s.id}`)
}

async function toggleScheduleEnabled(s: ScheduleRow): Promise<void> {
  const nextEnabled = !s.enabled
  toggleLoading.value = s.id
  try {
    const updated = await updateSchedule(s.id, {
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
      rate_limit_kbps: s.rate_limit_kbps ?? 0,
      pre_backup_commands: s.pre_backup_commands,
      post_backup_commands: s.post_backup_commands,
      on_failure: s.on_failure,
    })
    const index = schedules.value.findIndex((row) => row.id === s.id)
    if (index !== -1) {
      schedules.value[index] = updated
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
    await cancelSchedule(s.id)
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
      action="New schedule"
      @action="router.push('/schedules/new')"
    />

    <template v-else>
      <ScheduleTimelineRail
        :entries="railEntries"
        @select="openTimelineEntry"
      />

      <div
        v-for="group in groupedSchedules"
        :key="group.key"
        class="schedule-group"
      >
        <div class="schedule-group-header">
          <h2 class="schedule-group-title">{{ group.title }}</h2>
          <span class="schedule-group-count">{{ group.schedules.length }}</span>
          <span class="schedule-group-rule"></span>
        </div>
        <div class="card-grid">
          <div
            v-for="s in group.schedules"
            :key="s.id"
            class="entity-card"
            :class="{
              'entity-card--notable': !s.enabled,
              'entity-card--highlighted': s.overdueEntries.length > 0,
            }"
            :data-schedule-id="s.id"
            @click="navigateToSchedule(s)"
          >
            <span class="card-name">{{
              s.name || s.repo?.name || (s.repo_id != null ? `repo #${s.repo_id}` : 'no repository')
            }}</span>
            <EntityStatusBadges
              :notable="!s.enabled"
              :notable-label="scheduleDisabledLabel(s)"
              :running="s.isRunning"
              running-label="Running"
              :issues="scheduleIssues(s)"
            />
            <div class="card-meta">
              <span class="meta-pill">
                {{ s.target_hostnames.length }} agent{{
                  s.target_hostnames.length === 1 ? '' : 's'
                }}
              </span>
              <span
                class="badge badge--neutral"
                :class="`type-${s.schedule_type ?? 'backup'}`"
              >
                {{ scheduleTypeLabel(s.schedule_type ?? 'backup') }}
              </span>
            </div>
            <RunHistoryStrip :runs="runsBySchedule.get(s.id) ?? []" />
            <div class="card-stats">
              <div class="stat">
                <span class="stat-value">{{
                  cronToHuman(s.cron_expression) ?? s.cron_expression
                }}</span>
                <span class="stat-label">Every</span>
              </div>
              <div class="stat stat-align-end">
                <span class="stat-value">{{ formatDateShort(s.next_run_at) }}</span>
                <span class="stat-label">Next run</span>
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
                <span class="schedule-toggle-label">{{ scheduleDisabledLabel(s) }}</span>
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
  gap: var(--space-3);
}

.schedule-toggle-label {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-secondary);
}

.schedule-group + .schedule-group {
  margin-top: var(--space-8);
}

.schedule-group-header {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  margin-bottom: var(--space-5);
}

.schedule-group-title {
  margin: 0;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-primary);
}

.schedule-group-count {
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.schedule-group-rule {
  flex-grow: 1;
  height: 1px;
  background: var(--border);
}

/* Local layout additions on top of the shared `.card-stats` rule (which only
   sets `display`/`gap`) - this card shows just two stats, spread to the
   card's full width instead of left-packed. */
.card-stats {
  align-items: flex-end;
  justify-content: space-between;
}

.stat-align-end {
  align-items: flex-end;
}
</style>
