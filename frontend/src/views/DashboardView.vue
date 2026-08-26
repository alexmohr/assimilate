<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { useRouter, RouterLink } from 'vue-router'
import {
  getActivityByRange,
  getActivityDurationSamples,
  getDashboardOverview,
  getDashboardSummary,
  type ActivityEntry,
  type DashboardSummary,
} from '../api/stats'
import { listRepos } from '../api/repos'
import { getScheduleHealth } from '../api/schedules'
import { useWebSocket } from '../composables/useWebSocket'
import { useElapsedClock } from '../composables/useElapsedTimer'
import { formatBytes, formatDuration, relativeTime } from '../utils/format'
import { logger } from '../utils/logger'
import { normalizeBackupStatus } from '../utils/backupStatus'
import BaseSkeleton from '../components/BaseSkeleton.vue'
import TrendsChart from '../components/TrendsChart.vue'
import BackupCalendar from '../components/BackupCalendar.vue'
import RecentActivityWidget from '../components/RecentActivityWidget.vue'
import BackupStatsWidget from '../components/BackupStatsWidget.vue'
import StorageTrendWidget from '../components/StorageTrendWidget.vue'
import NeedsAttention from '../components/NeedsAttention.vue'
import ProtectionCoverage from '../components/ProtectionCoverage.vue'
import UpcomingWork from '../components/UpcomingWork.vue'
import RepositoryCapacity from '../components/RepositoryCapacity.vue'
import type { DashboardOperation, DashboardOverview } from '../types/dashboard'
import type { Repo } from '../types/repo'
import { type SegmentedOption } from '../components/BaseSegmented.vue'
import ChartRangeControls from '../components/ChartRangeControls.vue'
import { ChevronRight } from '@lucide/vue'

const rangeOptions: SegmentedOption<number>[] = [
  { value: 7, label: '7d' },
  { value: 14, label: '14d' },
  { value: 30, label: '30d' },
  { value: 90, label: '90d' },
]

interface StorageRepoEntry {
  name: string
  compressed_size: number
  deduplicated_size: number
  percentage: number
}

interface HealthEntry {
  repo_id: number
  hostname: string
  target_name: string
  last_status: string | null
  last_backup_at: string | null
  is_overdue: boolean
  cron_expression: string | null
  schedule_enabled: boolean | null
}

const summary = ref<DashboardSummary | null>(null)
const overview = ref<DashboardOverview | null>(null)
const health = ref<HealthEntry[]>([])
const repoOptions = ref<Repo[]>([])
const loading = ref(true)

const router = useRouter()

interface ActiveBackup {
  hostname: string
  target_name: string
  started_at: number
  repo_id: number | null
  schedule_id: number | null
  schedule_name: string | null
}

const activeBackups = ref<ActiveBackup[]>([])
const hasActiveBackups = computed(() => activeBackups.value.length > 0)
const { now } = useElapsedClock(hasActiveBackups)

function elapsedSecsFor(backup: ActiveBackup): number {
  return Math.max(0, Math.floor((now.value - backup.started_at) / 1000))
}

// Fetched in one batch of the most recent runs of any status (not just
// success/warning) so an interleaved failure can't starve the average of
// data points: the last 5 *matching* runs are taken from this wider window,
// per docs/dashboard.md's "last five successful or warned runs".
const AVG_DURATION_SAMPLE_WINDOW = 20
const AVG_DURATION_SAMPLE_COUNT = 5

const avgDurationSecs = ref<Map<string, number>>(new Map())
// Tracks in-flight requests only, so a page reload or reconnect can retry a
// pair whose fetch previously failed or came back with too little history -
// entries are removed as soon as the request settles, successful or not.
const avgDurationInFlight = new Set<string>()

function avgDurationKey(scheduleId: number, repoId: number): string {
  return `${scheduleId}:${repoId}`
}

async function fetchAvgDuration(scheduleId: number, repoId: number): Promise<void> {
  const key = avgDurationKey(scheduleId, repoId)
  if (avgDurationSecs.value.has(key) || avgDurationInFlight.has(key)) return
  avgDurationInFlight.add(key)
  try {
    const data = await getActivityDurationSamples({
      schedule_id: scheduleId,
      repo_id: repoId,
      limit: AVG_DURATION_SAMPLE_WINDOW,
    })
    const completed = data
      .filter((entry) => {
        const status = normalizeBackupStatus(entry.status)
        return status === 'success' || status === 'warning'
      })
      .slice(0, AVG_DURATION_SAMPLE_COUNT)
    if (completed.length === 0) return
    const avg = completed.reduce((sum, entry) => sum + entry.duration_secs, 0) / completed.length
    avgDurationSecs.value = new Map(avgDurationSecs.value).set(key, avg)
  } catch (e: unknown) {
    logger.error('fetchAvgDuration failed', e)
  } finally {
    avgDurationInFlight.delete(key)
  }
}

function estimatedRemainingFor(backup: ActiveBackup): number | null {
  if (backup.schedule_id === null || backup.repo_id === null) return null
  const avg = avgDurationSecs.value.get(avgDurationKey(backup.schedule_id, backup.repo_id))
  if (avg === undefined) return null
  const remaining = Math.round(avg - elapsedSecsFor(backup))
  return Math.max(0, remaining)
}

const successDaysFilter = ref<number>(30)
const successRepoFilter = ref<number | undefined>(undefined)

const storageBreakdown = ref<StorageRepoEntry[]>([])
const hiddenSegments = ref<Set<string>>(new Set())
const DONUT_COLORS = [
  'oklch(0.62 0.19 255)',
  'oklch(0.72 0.17 162)',
  'oklch(0.75 0.16 75)',
  'oklch(0.63 0.22 25)',
  'oklch(0.59 0.19 293)',
  'oklch(0.72 0.13 200)',
]

const successActivity = ref<ActivityEntry[]>([])

function mergeActiveBackups(operations: DashboardOperation[]): void {
  const existingByKey = new Map(
    activeBackups.value.map((backup) => [`${backup.hostname}::${backup.target_name}`, backup]),
  )
  activeBackups.value = operations.map((operation) => {
    const key = `${operation.hostname}::${operation.repo_name}`
    const existing = existingByKey.get(key)
    void fetchAvgDuration(operation.schedule_id, operation.repo_id)
    return {
      hostname: operation.hostname,
      target_name: operation.repo_name,
      started_at: existing?.started_at ?? Date.parse(operation.started_at),
      repo_id: operation.repo_id,
      schedule_id: operation.schedule_id,
      schedule_name: operation.schedule_name,
    }
  })
}

async function fetchSuccessActivity(): Promise<void> {
  successActivity.value = await getActivityByRange({
    days: successDaysFilter.value,
    repo_id: successRepoFilter.value,
  })
}

async function fetchAll(): Promise<void> {
  try {
    const [s, h, o, r] = await Promise.all([
      getDashboardSummary(),
      getScheduleHealth(),
      getDashboardOverview(),
      listRepos(),
    ])
    summary.value = s
    health.value = h
    overview.value = o
    mergeActiveBackups(o.running_operations)
    repoOptions.value = r
    storageBreakdown.value = s.storage_by_repo
    await fetchSuccessActivity()
  } finally {
    loading.value = false
  }
}

function toggleSegment(name: string): void {
  const next = new Set(hiddenSegments.value)
  if (next.has(name)) {
    next.delete(name)
  } else {
    next.add(name)
  }
  hiddenSegments.value = next
}

const { onMessage, status: wsStatus } = useWebSocket()

onMessage('BackupCompleted', (payload) => {
  activeBackups.value = activeBackups.value.filter(
    (b) => !(b.hostname === payload.hostname && b.target_name === payload.target_name),
  )
  fetchAll().catch(logger.error)
})
onMessage('BackupStarted', (payload) => {
  const exists = activeBackups.value.some(
    (b) => b.hostname === payload.hostname && b.target_name === payload.target_name,
  )
  if (!exists) {
    activeBackups.value.push({
      hostname: payload.hostname,
      target_name: payload.target_name,
      started_at: Date.now(),
      repo_id: null,
      schedule_id: null,
      schedule_name: null,
    })
  }
  fetchAll().catch(logger.error)
})
onMessage('AgentConnected', () => {
  fetchAll().catch(logger.error)
})
onMessage('AgentDisconnected', () => {
  fetchAll().catch(logger.error)
})

watch(wsStatus, (newStatus, oldStatus) => {
  if (newStatus === 'connected' && oldStatus !== 'connected') {
    fetchAll().catch(logger.error)
  }
})

onMounted(() => {
  fetchAll().catch(logger.error)
})

watch([successDaysFilter, successRepoFilter], () => {
  fetchSuccessActivity().catch(logger.error)
})

const hasFindings = computed((): boolean => (overview.value?.findings?.length ?? 0) > 0)

const overdueCount = computed((): number => health.value.filter((h) => h.is_overdue).length)

const successTotal = computed((): number => successActivity.value.length)
const successCount = computed(
  (): number =>
    successActivity.value.filter((a) => normalizeBackupStatus(a.status) === 'success').length,
)
const warnedCount = computed(
  (): number =>
    successActivity.value.filter((a) => normalizeBackupStatus(a.status) === 'warning').length,
)
const failedCount = computed(
  (): number =>
    successActivity.value.filter((a) => normalizeBackupStatus(a.status) === 'failed').length,
)

const successRate = computed((): number => {
  if (successTotal.value === 0) return 0
  return Math.round((successCount.value / successTotal.value) * 100)
})

const successRingColor = computed((): string => {
  const rate = successRate.value
  if (rate >= 90) return 'var(--success)'
  if (rate >= 70) return 'var(--warning)'
  return 'var(--danger)'
})

const successRingDasharray = computed((): string => {
  const circumference = 2 * Math.PI * 54
  const filled = (successRate.value / 100) * circumference
  return `${filled} ${circumference - filled}`
})

const agentIndicatorColor = computed((): string => {
  if (!summary.value) return 'var(--text-muted)'
  return summary.value.online_agents === summary.value.total_agents
    ? 'var(--success)'
    : 'var(--warning)'
})

const storageDonuts = computed(
  (): Array<{
    name: string
    percentage: number
    size: number
    compressedSize: number
    color: string
    offset: number
  }> => {
    if (storageBreakdown.value.length === 0) return []
    const visible = storageBreakdown.value.filter((entry) => !hiddenSegments.value.has(entry.name))
    if (visible.length === 0) return []
    const totalPct = visible.reduce((sum, e) => sum + e.percentage, 0)
    const circumference = 2 * Math.PI * 54
    let cumulative = 0
    return visible.map((entry) => {
      const normalizedPct = totalPct > 0 ? (entry.percentage / totalPct) * 100 : 0
      const offset = cumulative
      cumulative += (normalizedPct / 100) * circumference
      const originalIndex = storageBreakdown.value.indexOf(entry)
      return {
        name: entry.name,
        percentage: normalizedPct,
        size: entry.deduplicated_size,
        compressedSize: entry.compressed_size,
        color: DONUT_COLORS[originalIndex % DONUT_COLORS.length],
        offset,
      }
    })
  },
)

const storageLegendItems = computed(
  (): Array<{
    name: string
    size: number
    compressedSize: number
    color: string
    hidden: boolean
  }> => {
    return storageBreakdown.value.map((entry, i) => ({
      name: entry.name,
      size: entry.deduplicated_size,
      compressedSize: entry.compressed_size,
      color: DONUT_COLORS[i % DONUT_COLORS.length],
      hidden: hiddenSegments.value.has(entry.name),
    }))
  },
)

function navigateToLastBackup(): void {
  if (!summary.value?.last_backup_repo_id) return
  const query: Record<string, string> = { tab: 'archives' }
  if (summary.value.last_backup_archive_name) {
    query.archive = summary.value.last_backup_archive_name
  }
  router.push({ path: `/repos/${summary.value.last_backup_repo_id}`, query })
}

function navigateToLastFailure(): void {
  if (!summary.value?.last_failure_at) return
  const query: Record<string, string> = { status: 'failed', category: 'backup' }
  if (summary.value.last_failure_schedule_id) {
    query.schedule_id = String(summary.value.last_failure_schedule_id)
  }
  router.push({ path: '/activity', query })
}

function navigateToLastWarning(): void {
  if (!summary.value?.last_warning_at) return
  const query: Record<string, string> = { status: 'warning', category: 'backup' }
  if (summary.value.last_warning_schedule_id) {
    query.schedule_id = String(summary.value.last_warning_schedule_id)
  }
  router.push({ path: '/activity', query })
}

function navigateToSchedule(scheduleId: number | null): void {
  if (scheduleId) {
    router.push(`/schedules/${scheduleId}`)
  }
}

async function fetchOverview(): Promise<void> {
  overview.value = await getDashboardOverview()
}
</script>

<template>
  <div class="dashboard">
    <template v-if="loading">
      <div class="dashboard-skeleton">
        <div class="dashboard-skeleton-tiles">
          <BaseSkeleton
            v-for="i in 6"
            :key="i"
            variant="card"
            height="5rem"
          />
        </div>
        <BaseSkeleton
          variant="card"
          height="16rem"
        />
        <BaseSkeleton
          variant="card"
          height="12rem"
        />
      </div>
    </template>

    <template v-else>
      <section class="status-banner">
        <div
          class="tile tile--lg tile--link"
          @click="router.push({ name: 'agents', query: { status: 'offline' } })"
        >
          <span class="stat-label">Online agents</span>
          <span class="stat-value stat-value--xl">
            <span
              class="stat-dot"
              :style="{ background: agentIndicatorColor }"
            />
            {{ summary?.online_agents ?? 0 }}/{{ summary?.total_agents ?? 0 }}
          </span>
        </div>
        <div
          class="tile tile--lg tile--link"
          @click="router.push({ name: 'repos' })"
        >
          <span class="stat-label">Repositories</span>
          <span class="stat-value stat-value--xl">{{ summary?.total_repos ?? 0 }}</span>
        </div>
        <div
          class="tile tile--lg tile--link"
          @click="router.push({ name: 'schedules', query: { filter: 'overdue' } })"
        >
          <span class="stat-label">Overdue</span>
          <span
            class="stat-value stat-value--xl"
            :class="{ 'stat-danger': overdueCount > 0 }"
          >
            {{ overdueCount }}
          </span>
        </div>
        <div
          class="tile tile--lg"
          :class="{ 'tile--link': summary?.last_backup_repo_id }"
          @click="navigateToLastBackup"
        >
          <span class="stat-label">Last backup</span>
          <span class="stat-value stat-value--lg">
            {{ summary?.last_backup_at ? relativeTime(summary.last_backup_at) : '\u2014' }}
          </span>
        </div>
        <div
          class="tile tile--lg"
          :class="{ 'tile--link': summary?.next_backup_schedule_id }"
          @click="
            summary?.next_backup_schedule_id && navigateToSchedule(summary.next_backup_schedule_id)
          "
        >
          <span class="stat-label">Next backup</span>
          <span class="stat-value stat-value--lg">
            <template v-if="activeBackups.length > 0">Active</template>
            <template v-else>
              {{ summary?.next_backup_at ? relativeTime(summary.next_backup_at) : '\u2014' }}
            </template>
          </span>
        </div>
        <div class="tile tile--lg">
          <span class="stat-label">Total storage</span>
          <span class="stat-value stat-value--lg">
            {{ formatBytes(summary?.total_storage_bytes ?? 0) }}
          </span>
        </div>
        <div
          class="tile tile--lg"
          :class="{ 'tile--link': summary?.last_failure_at }"
          @click="navigateToLastFailure"
        >
          <span class="stat-label">Last failure</span>
          <span
            class="stat-value stat-value--lg"
            :class="{ 'stat-danger': summary?.last_failure_at }"
          >
            {{ summary?.last_failure_at ? relativeTime(summary.last_failure_at) : '\u2014' }}
          </span>
        </div>
        <div
          class="tile tile--lg"
          :class="{ 'tile--link': summary?.last_warning_at }"
          @click="navigateToLastWarning"
        >
          <span class="stat-label">Last warning</span>
          <span
            class="stat-value stat-value--lg"
            :class="{ 'stat-warning': summary?.last_warning_at }"
          >
            {{ summary?.last_warning_at ? relativeTime(summary.last_warning_at) : '\u2014' }}
          </span>
        </div>
      </section>

      <!-- In-Progress Backups -->
      <section
        v-if="activeBackups.length > 0"
        class="panel active-backups-panel"
      >
        <h2 class="panel-title">Backups in progress</h2>
        <div class="active-backups-list">
          <div
            v-for="backup in activeBackups"
            :key="`${backup.hostname}-${backup.target_name}`"
            class="active-backup-item"
          >
            <span class="pulse-dot pulse-dot--accent" />
            <span
              v-if="backup.schedule_name"
              class="active-backup-schedule"
            >
              {{ backup.schedule_name }}
            </span>
            <RouterLink
              :to="{ name: 'agent-detail', params: { hostname: backup.hostname } }"
              class="active-backup-link"
            >
              {{ backup.hostname }}
            </RouterLink>
            <ChevronRight
              class="active-backup-sep"
              :size="14"
            />
            <RouterLink
              v-if="backup.repo_id !== null"
              :to="{ name: 'repo-detail', params: { id: String(backup.repo_id) } }"
              class="active-backup-link"
            >
              {{ backup.target_name }}
            </RouterLink>
            <span
              v-else
              class="active-backup-target"
              >{{ backup.target_name }}</span
            >
            <span class="active-backup-time">
              Running for {{ formatDuration(elapsedSecsFor(backup)) }}
            </span>
            <span
              v-if="estimatedRemainingFor(backup) !== null"
              class="active-backup-time"
            >
              &middot; ~{{ formatDuration(estimatedRemainingFor(backup)!) }} left
            </span>
          </div>
        </div>
      </section>

      <div class="summary-row">
        <BackupStatsWidget :repos="repoOptions" />
        <ProtectionCoverage
          :protection="
            overview?.protection ?? {
              protected_hosts: 0,
              eligible_hosts: 0,
              protected_agent_links: [],
              unassigned_agents: [],
              never_succeeded_targets: 0,
              never_succeeded_agents: [],
              disabled_only_agents: [],
            }
          "
        />
      </div>

      <div class="dashboard-columns">
        <div class="dashboard-column">
          <NeedsAttention
            v-if="hasFindings"
            :findings="overview?.findings ?? []"
            @dismissed="fetchOverview().catch(logger.error)"
          />
          <BackupCalendar :repos="repoOptions" />
          <RepositoryCapacity :repositories="overview?.repository_capacity ?? []" />
        </div>
        <div class="dashboard-column">
          <UpcomingWork
            :operations="overview?.running_operations ?? []"
            :schedules="overview?.upcoming_schedules ?? []"
          />
          <RecentActivityWidget />
        </div>
      </div>

      <div class="rings-row">
        <!-- Success rate ring -->
        <section class="panel">
          <div class="panel-header">
            <h2 class="panel-title">Success rate</h2>
            <ChartRangeControls
              v-model:repo-id="successRepoFilter"
              v-model:days="successDaysFilter"
              :repos="repoOptions"
              :options="rangeOptions"
              label="Success rate range"
            />
          </div>
          <p class="chart-desc">
            Proportion of scheduled backup runs that completed without errors over the selected
            window.
          </p>
          <div class="ring-container">
            <svg
              viewBox="0 0 128 128"
              class="ring-svg"
            >
              <circle
                cx="64"
                cy="64"
                r="54"
                fill="none"
                stroke="var(--border)"
                stroke-width="10"
              />
              <circle
                cx="64"
                cy="64"
                r="54"
                fill="none"
                :stroke="successRingColor"
                stroke-width="10"
                stroke-linecap="round"
                :stroke-dasharray="successRingDasharray"
                stroke-dashoffset="0"
                transform="rotate(-90 64 64)"
                class="ring-progress"
              />
            </svg>
            <div class="ring-center">
              <span class="ring-pct">{{ successRate }}%</span>
              <span class="ring-sub"> {{ successCount }}/{{ successTotal }} OK </span>
            </div>
          </div>
          <div class="chart-legend">
            <span
              class="chart-legend-item chart-legend-item--link legend-pass"
              @click="router.push({ name: 'schedules', query: { filter: 'success' } })"
            >
              <span class="chart-legend-swatch chart-legend-swatch--dot" />
              Passed: {{ successCount }}
            </span>
            <span
              class="chart-legend-item chart-legend-item--link legend-warn"
              @click="router.push({ name: 'schedules', query: { filter: 'warning' } })"
            >
              <span class="chart-legend-swatch chart-legend-swatch--dot" />
              Warned: {{ warnedCount }}
            </span>
            <span
              class="chart-legend-item chart-legend-item--link legend-fail"
              @click="router.push({ name: 'schedules', query: { filter: 'failed' } })"
            >
              <span class="chart-legend-swatch chart-legend-swatch--dot" />
              Failed: {{ failedCount }}
            </span>
          </div>
        </section>

        <!-- Section 3: Storage Donut -->
        <section class="panel">
          <div class="panel-header">
            <h2 class="panel-title">Storage breakdown</h2>
          </div>
          <p class="chart-desc">
            Current on-disk usage per repository — deduplicated (unique chunks across all archives).
          </p>
          <div class="ring-container">
            <svg
              viewBox="0 0 128 128"
              class="ring-svg"
            >
              <circle
                cx="64"
                cy="64"
                r="54"
                fill="none"
                stroke="var(--border)"
                stroke-width="10"
              />
              <circle
                v-for="seg in storageDonuts"
                :key="seg.name"
                cx="64"
                cy="64"
                r="54"
                fill="none"
                :stroke="seg.color"
                stroke-width="10"
                :stroke-dasharray="`${(seg.percentage / 100) * 2 * Math.PI * 54} ${2 * Math.PI * 54}`"
                :stroke-dashoffset="`${-seg.offset}`"
                transform="rotate(-90 64 64)"
              />
            </svg>
            <div class="ring-center">
              <span class="ring-pct ring-pct-sm">
                {{ formatBytes(summary?.total_storage_bytes ?? 0) }}
              </span>
              <span class="ring-sub">{{ storageDonuts.length }} repos</span>
            </div>
          </div>
          <div class="chart-legend chart-legend--stack">
            <div
              v-for="seg in storageLegendItems"
              :key="seg.name"
              class="chart-legend-item chart-legend-item--toggle"
              :class="{ 'chart-legend-item--off': seg.hidden }"
              @click="toggleSegment(seg.name)"
            >
              <span
                class="chart-legend-swatch"
                :style="{ background: seg.hidden ? 'var(--border)' : seg.color }"
              />
              <span class="chart-legend-name">{{ seg.name }}</span>
              <span class="chart-legend-detail"
                >{{ formatBytes(seg.compressedSize) }} compressed &middot;
                {{ formatBytes(seg.size) }} dedup</span
              >
            </div>
          </div>
        </section>
      </div>

      <div class="trends-row">
        <StorageTrendWidget :repos="repoOptions" />
        <TrendsChart :repos="repoOptions" />
      </div>
    </template>
  </div>
</template>

<style scoped>
.dashboard {
  display: flex;
  flex-direction: column;
  gap: var(--space-8);
  max-width: 1100px;
  margin: 0 auto;
}

/* The loading placeholder mirrors the loaded layout: one column of cards
   over a row of tiles. */
.dashboard-skeleton {
  display: flex;
  flex-direction: column;
  gap: var(--space-8);
}

.dashboard-skeleton-tiles {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
  gap: var(--space-6);
}

/* Two counter panels of comparable height. The calendar is deliberately not
   one of them: at a third of the page its seven day columns cannot fit, and
   the overflow was clipping Saturday off the month entirely. */
.summary-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-8);
  align-items: stretch;
  min-width: 0;
}

@media (max-width: 768px) {
  .summary-row {
    grid-template-columns: 1fr;
  }
}

/* Each half is an independent stack rather than a grid cell, because these
   panels size to their content: a short one is followed by the next panel in
   its own column instead of leaving a hole beside a tall neighbour. */
.dashboard-columns {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-8);
  align-items: start;
}

.dashboard-column {
  display: flex;
  flex-direction: column;
  gap: var(--space-8);
  min-width: 0;
}

/* Section 1: Status Banner */
.status-banner {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: var(--space-5);
}

@media (max-width: 640px) {
  .status-banner {
    grid-template-columns: repeat(2, 1fr);
  }
  .panel-header {
    flex-direction: column;
    align-items: flex-start;
  }
}

.stat-danger {
  color: var(--danger);
}

.stat-warning {
  color: var(--warning);
}

.stat-dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  flex-shrink: 0;
}

.rings-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-8);
}

/* Panel */

.panel-header .panel-title {
  margin: 0;
  white-space: nowrap;
}

/* Ring / Donut shared */
.ring-container {
  position: relative;
  width: 160px;
  height: 160px;
  margin: 0 auto var(--space-6);
}

.ring-svg {
  width: 100%;
  height: 100%;
}

.ring-progress {
  transition: stroke-dasharray var(--duration-value) ease;
}

.ring-center {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
}

.ring-pct {
  font-size: var(--fs-2xl);
  font-weight: 700;
  color: var(--text-primary);
}

.ring-pct-sm {
  font-size: var(--fs-lg);
}

.ring-sub {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
}

/* Success ring legend */

.legend-pass .chart-legend-swatch {
  background: var(--success);
}

.legend-warn .chart-legend-swatch {
  background: var(--warning);
}

.legend-fail .chart-legend-swatch {
  background: var(--danger);
}

/* Storage legend */

/* Health Cards */

/* Trends Row */
.trends-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-8);
  align-items: start;
}

@media (max-width: 1024px) {
  /* Below this the calendar column is narrower than a seven-day month grid,
     so the two stacks fold into one full-width column. */
  .dashboard-columns {
    grid-template-columns: 1fr;
  }

  .rings-row {
    grid-template-columns: 1fr;
  }

  .trends-row {
    grid-template-columns: 1fr;
  }
}

/* Active Backups */
.active-backups-panel {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: var(--space-7);
}

.active-backups-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.active-backup-item {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-4);
  font-size: var(--fs-base);
}

.active-backup-schedule {
  font-weight: 600;
  color: var(--text-primary);
}

.active-backup-link {
  color: var(--accent);
  text-decoration: none;
}

.active-backup-link:hover {
  text-decoration: underline;
}

.active-backup-time {
  color: var(--text-muted);
  font-size: var(--fs-xs);
  margin-left: auto;
}

.active-backup-sep {
  color: var(--text-muted);
}

.active-backup-target {
  color: var(--text-secondary);
  font-family: var(--mono);
  font-size: var(--fs-sm);
}
</style>
