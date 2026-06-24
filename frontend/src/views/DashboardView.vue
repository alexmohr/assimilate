<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { useRouter } from 'vue-router'
import { apiClient } from '../api/client'
import { useWebSocket } from '../composables/useWebSocket'
import { formatBytes, relativeTime } from '../utils/format'
import { logger } from '../utils/logger'
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
import type { DashboardOverview } from '../types/dashboard'

interface StorageRepoEntry {
  name: string
  compressed_size: number
  deduplicated_size: number
  percentage: number
}

interface DashboardSummary {
  online_agents: number
  total_agents: number
  total_repos: number
  last_backup_at: string | null
  next_backup_at: string | null
  last_backup_schedule_id: number | null
  last_backup_repo_id: number | null
  last_backup_archive_name: string | null
  next_backup_schedule_id: number | null
  active_schedules: number
  total_schedules: number
  total_storage_bytes: number
  success_30d: number
  failed_30d: number
  total_30d: number
  storage_by_repo: StorageRepoEntry[]
  last_failure_at: string | null
  last_warning_at: string | null
  last_failure_schedule_id: number | null
  last_warning_schedule_id: number | null
  last_failure_message: string | null
  last_warning_message: string | null
  last_failure_repo_id: number | null
  last_warning_repo_id: number | null
  last_failure_repo_name: string | null
  last_warning_repo_name: string | null
  last_failure_schedule_name: string | null
  last_warning_schedule_name: string | null
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

interface ActivityEntry {
  id: number
  hostname: string
  target_name: string
  started_at: string
  finished_at: string
  status: string
  duration_secs: number
}

interface BackupPayload {
  hostname: string
  target_name: string
}

interface AgentPayload {
  hostname: string
}

interface RepoOption {
  id: number
  name: string
}

const summary = ref<DashboardSummary | null>(null)
const overview = ref<DashboardOverview | null>(null)
const health = ref<HealthEntry[]>([])
const repoOptions = ref<RepoOption[]>([])
const loading = ref(true)

const router = useRouter()

interface ActiveBackup {
  hostname: string
  target_name: string
  started_at: number
}

const activeBackups = ref<ActiveBackup[]>([])

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

async function fetchSuccessActivity(): Promise<void> {
  const params = new URLSearchParams({ days: String(successDaysFilter.value) })
  if (successRepoFilter.value !== undefined) {
    params.set('repo_id', String(successRepoFilter.value))
  }
  const response = await apiClient.get<ActivityEntry[]>(`/stats/activity?${params.toString()}`)
  successActivity.value = response.data
}

async function fetchAll(): Promise<void> {
  try {
    const [s, h, o, r] = await Promise.all([
      apiClient.get<DashboardSummary>('/stats/summary'),
      apiClient.get<HealthEntry[]>('/stats/health'),
      apiClient.get<DashboardOverview>('/stats/dashboard-overview'),
      apiClient.get<RepoOption[]>('/repos'),
    ])
    summary.value = s.data
    health.value = h.data
    overview.value = o.data
    repoOptions.value = r.data.map((repo) => ({ id: repo.id, name: repo.name }))
    storageBreakdown.value = s.data.storage_by_repo
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

onMessage<BackupPayload>('BackupCompleted', (payload) => {
  activeBackups.value = activeBackups.value.filter(
    (b) => !(b.hostname === payload.hostname && b.target_name === payload.target_name),
  )
  fetchAll().catch(logger.error)
})
onMessage<BackupPayload>('BackupStarted', (payload) => {
  const exists = activeBackups.value.some(
    (b) => b.hostname === payload.hostname && b.target_name === payload.target_name,
  )
  if (!exists) {
    activeBackups.value.push({
      hostname: payload.hostname,
      target_name: payload.target_name,
      started_at: Date.now(),
    })
  }
  fetchAll().catch(logger.error)
})
onMessage<AgentPayload>('AgentConnected', () => {
  fetchAll().catch(logger.error)
})
onMessage<AgentPayload>('AgentDisconnected', () => {
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

const overdueCount = computed((): number => health.value.filter((h) => h.is_overdue).length)

const successTotal = computed((): number => successActivity.value.length)
const successCount = computed(
  (): number => successActivity.value.filter((a) => a.status === 'success').length,
)
const warnedCount = computed(
  (): number => successActivity.value.filter((a) => a.status === 'warning').length,
)
const failedCount = computed(
  (): number => successActivity.value.filter((a) => a.status === 'failed').length,
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
  const o = await apiClient.get<DashboardOverview>('/stats/dashboard-overview')
  overview.value = o.data
}
</script>

<template>
  <div class="dashboard">
    <template v-if="loading">
      <div style="display: flex; flex-direction: column; gap: 1.5rem">
        <div
          style="
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
            gap: 1rem;
          "
        >
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
          class="stat-card stat-card-link"
          @click="router.push({ name: 'agents', query: { status: 'offline' } })"
        >
          <span class="stat-label">Online Agents</span>
          <span class="stat-value">
            <span
              class="stat-dot"
              :style="{ background: agentIndicatorColor }"
            />
            {{ summary?.online_agents ?? 0 }}/{{ summary?.total_agents ?? 0 }}
          </span>
        </div>
        <div
          class="stat-card stat-card-link"
          @click="router.push({ name: 'repos' })"
        >
          <span class="stat-label">Repositories</span>
          <span class="stat-value">{{ summary?.total_repos ?? 0 }}</span>
        </div>
        <div
          class="stat-card stat-card-link"
          @click="router.push({ name: 'schedules', query: { filter: 'overdue' } })"
        >
          <span class="stat-label">Overdue</span>
          <span
            class="stat-value"
            :class="{ 'stat-danger': overdueCount > 0 }"
          >
            {{ overdueCount }}
          </span>
        </div>
        <div
          class="stat-card"
          :class="{ 'stat-card-link': summary?.last_backup_repo_id }"
          @click="navigateToLastBackup"
        >
          <span class="stat-label">Last Backup</span>
          <span class="stat-value stat-value-sm">
            {{ summary?.last_backup_at ? relativeTime(summary.last_backup_at) : '\u2014' }}
          </span>
        </div>
        <div
          class="stat-card"
          :class="{ 'stat-card-link': summary?.next_backup_schedule_id }"
          @click="
            summary?.next_backup_schedule_id && navigateToSchedule(summary.next_backup_schedule_id)
          "
        >
          <span class="stat-label">Next Backup</span>
          <span class="stat-value stat-value-sm">
            <template v-if="activeBackups.length > 0">Active</template>
            <template v-else>
              {{ summary?.next_backup_at ? relativeTime(summary.next_backup_at) : '—' }}
            </template>
          </span>
        </div>
        <div class="stat-card">
          <span class="stat-label">Total Storage</span>
          <span class="stat-value stat-value-sm">
            {{ formatBytes(summary?.total_storage_bytes ?? 0) }}
          </span>
        </div>
        <div
          class="stat-card"
          :class="{ 'stat-card-link': summary?.last_failure_at }"
          @click="navigateToLastFailure"
        >
          <span class="stat-label">Last Failure</span>
          <span
            class="stat-value stat-value-sm"
            :class="{ 'stat-danger': summary?.last_failure_at }"
          >
            {{ summary?.last_failure_at ? relativeTime(summary.last_failure_at) : '\u2014' }}
          </span>
        </div>
        <div
          class="stat-card"
          :class="{ 'stat-card-link': summary?.last_warning_at }"
          @click="navigateToLastWarning"
        >
          <span class="stat-label">Last Warning</span>
          <span
            class="stat-value stat-value-sm"
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
        <h2 class="panel-title">Backups In Progress</h2>
        <div class="active-backups-list">
          <div
            v-for="backup in activeBackups"
            :key="`${backup.hostname}-${backup.target_name}`"
            class="active-backup-item"
          >
            <span class="active-backup-pulse" />
            <span class="active-backup-host">{{ backup.hostname }}</span>
            <span class="active-backup-sep">&rarr;</span>
            <span class="active-backup-target">{{ backup.target_name }}</span>
          </div>
        </div>
      </section>

      <div class="stats-coverage-row">
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
        <div class="calendar-cell">
          <BackupCalendar :repos="repoOptions" />
        </div>
      </div>

      <div class="attention-row">
        <NeedsAttention
          :findings="overview?.findings ?? []"
          @dismissed="fetchOverview().catch(logger.error)"
        />
        <div class="attention-sidebar">
          <UpcomingWork
            :operations="overview?.running_operations ?? []"
            :schedules="overview?.upcoming_schedules ?? []"
          />
          <RecentActivityWidget />
        </div>
      </div>

      <!-- Main Grid: rings side by side, capacity below -->
      <div class="main-grid">
        <RepositoryCapacity :repositories="overview?.repository_capacity ?? []" />
        <div class="rings-row">
          <!-- Section 2: 30-Day Success Ring -->
          <section class="panel">
            <div class="panel-header">
              <h2 class="panel-title">Success Rate</h2>
              <div class="trends-controls">
                <select
                  v-model="successRepoFilter"
                  class="trends-select"
                >
                  <option :value="undefined">All Repos</option>
                  <option
                    v-for="repo in repoOptions"
                    :key="repo.id"
                    :value="repo.id"
                  >
                    {{ repo.name }}
                  </option>
                </select>
                <div class="view-toggle">
                  <button
                    class="toggle-btn"
                    :class="{ active: successDaysFilter === 7 }"
                    @click="successDaysFilter = 7"
                  >
                    7d
                  </button>
                  <button
                    class="toggle-btn"
                    :class="{ active: successDaysFilter === 14 }"
                    @click="successDaysFilter = 14"
                  >
                    14d
                  </button>
                  <button
                    class="toggle-btn"
                    :class="{ active: successDaysFilter === 30 }"
                    @click="successDaysFilter = 30"
                  >
                    30d
                  </button>
                  <button
                    class="toggle-btn"
                    :class="{ active: successDaysFilter === 90 }"
                    @click="successDaysFilter = 90"
                  >
                    90d
                  </button>
                </div>
              </div>
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
            <div class="ring-legend">
              <span
                class="legend-item legend-pass legend-link"
                @click="router.push({ name: 'schedules', query: { filter: 'success' } })"
              >
                <span class="legend-dot" />
                Passed: {{ successCount }}
              </span>
              <span
                class="legend-item legend-warn legend-link"
                @click="router.push({ name: 'schedules', query: { filter: 'warning' } })"
              >
                <span class="legend-dot" />
                Warned: {{ warnedCount }}
              </span>
              <span
                class="legend-item legend-fail legend-link"
                @click="router.push({ name: 'schedules', query: { filter: 'failed' } })"
              >
                <span class="legend-dot" />
                Failed: {{ failedCount }}
              </span>
            </div>
          </section>

          <!-- Section 3: Storage Donut -->
          <section class="panel">
            <div class="panel-header">
              <h2 class="panel-title">Storage Breakdown</h2>
            </div>
            <p class="chart-desc">
              Current on-disk usage per repository — deduplicated (unique chunks across all
              archives).
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
            <div class="storage-legend">
              <div
                v-for="seg in storageLegendItems"
                :key="seg.name"
                class="storage-legend-item"
                :class="{ 'storage-legend-item-hidden': seg.hidden }"
                @click="toggleSegment(seg.name)"
              >
                <span
                  class="legend-color"
                  :style="{ background: seg.hidden ? 'var(--border)' : seg.color }"
                />
                <span class="legend-name">{{ seg.name }}</span>
                <span class="legend-detail"
                  >{{ formatBytes(seg.compressedSize) }} compressed &middot;
                  {{ formatBytes(seg.size) }} dedup</span
                >
              </div>
            </div>
          </section>
        </div>
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
  gap: 1.5rem;
}

.stats-coverage-row {
  display: grid;
  grid-template-columns: 1fr 1fr 1fr;
  gap: 1.5rem;
  align-items: stretch;
  min-width: 0;
}

.calendar-cell {
  min-width: 0;
  overflow: hidden;
}

@media (max-width: 1100px) {
  .stats-coverage-row {
    grid-template-columns: 1fr 1fr;
  }

  .calendar-cell {
    grid-column: 1 / -1;
  }
}

@media (max-width: 700px) {
  .stats-coverage-row {
    grid-template-columns: 1fr;
  }

  .calendar-cell {
    grid-column: auto;
  }
}

.attention-row {
  display: grid;
  grid-template-columns: 3fr 2fr;
  gap: 1.5rem;
  align-items: stretch;
}

.attention-left {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

.attention-sidebar {
  display: grid;
  grid-template-rows: 1fr 1fr;
  gap: 1.5rem;
}

@media (max-width: 900px) {
  .attention-row {
    grid-template-columns: 1fr;
  }
}

/* Section 1: Status Banner */
.status-banner {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 0.75rem;
}

@media (max-width: 500px) {
  .status-banner {
    grid-template-columns: repeat(2, 1fr);
  }
}

.stat-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1rem 1.25rem;
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
}

.stat-card-link {
  cursor: pointer;
  transition:
    border-color 0.15s,
    background 0.15s;
}

.stat-card-link:hover {
  border-color: var(--accent);
  background: var(--bg-hover);
}

.stat-label {
  font-size: 0.7rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-muted);
}

.stat-value {
  font-size: 1.5rem;
  font-weight: 700;
  color: var(--text-primary);
  display: flex;
  align-items: center;
  gap: 0.4rem;
}

.stat-value-sm {
  font-size: 1.1rem;
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

/* Main grid: rings row + full-width capacity */
.main-grid {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

.rings-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 1.5rem;
}

@media (max-width: 900px) {
  .rings-row {
    grid-template-columns: 1fr;
  }
}

/* Panel */
.panel {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem;
}

.panel-full {
  flex: 1;
}

.panel-title {
  font-size: 0.875rem;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0 0 1rem;
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 0.5rem;
  margin-bottom: 1rem;
}

.panel-header .panel-title {
  margin: 0;
  white-space: nowrap;
}

.trends-controls {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.trends-select {
  padding: 0.25rem 0.5rem;
  font-size: 0.75rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-base);
  color: var(--text-primary);
}

.view-toggle {
  display: flex;
  flex-shrink: 0;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  overflow: hidden;
}

.toggle-btn {
  padding: 0.25rem 0.5rem;
  font-size: 0.65rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.03em;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  transition:
    background 0.15s,
    color 0.15s;
}

.toggle-btn:not(:last-child) {
  border-right: 1px solid var(--border);
}

.toggle-btn:hover {
  background: var(--bg-hover);
}

.toggle-btn.active {
  background: var(--accent);
  color: var(--text-on-accent, #fff);
}

.state-msg {
  color: var(--text-muted);
  font-size: 0.875rem;
  padding: 1rem 0;
}

.chart-desc {
  color: var(--text-muted);
  font-size: 0.7rem;
  margin: 0 0 0.75rem;
  line-height: 1.4;
}

/* Ring / Donut shared */
.ring-container {
  position: relative;
  width: 160px;
  height: 160px;
  margin: 0 auto 1rem;
}

.ring-svg {
  width: 100%;
  height: 100%;
}

.ring-progress {
  transition: stroke-dasharray 0.6s ease;
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
  font-size: 1.75rem;
  font-weight: 700;
  color: var(--text-primary);
}

.ring-pct-sm {
  font-size: 1rem;
}

.ring-sub {
  font-size: 0.7rem;
  color: var(--text-muted);
}

/* Success ring legend */
.ring-legend {
  display: flex;
  justify-content: center;
  gap: 1.25rem;
}

.legend-item {
  display: flex;
  align-items: center;
  gap: 0.35rem;
  font-size: 0.75rem;
  color: var(--text-secondary);
}

.legend-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
}

.legend-pass .legend-dot {
  background: var(--success);
}

.legend-warn .legend-dot {
  background: var(--warning);
}

.legend-fail .legend-dot {
  background: var(--danger);
}

.legend-link {
  cursor: pointer;
  border-radius: 4px;
  padding: 2px 6px;
  transition: background 0.15s;
}

.legend-link:hover {
  background: var(--hover);
  text-decoration: underline;
}

/* Storage legend */
.storage-legend {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.storage-legend-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.75rem;
  cursor: pointer;
  user-select: none;
  transition: opacity 0.15s;
}

.storage-legend-item:hover {
  opacity: 0.7;
}

.storage-legend-item-hidden {
  opacity: 0.4;
}

.storage-legend-item-hidden .legend-name {
  text-decoration: line-through;
}

.legend-color {
  width: 10px;
  height: 10px;
  border-radius: 2px;
  flex-shrink: 0;
}

.legend-name {
  font-weight: 600;
  color: var(--text-primary);
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.legend-detail {
  color: var(--text-muted);
  white-space: nowrap;
}

/* Health Cards */
.health-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: 0.75rem;
}

.health-card {
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.75rem;
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
}

.health-card-link {
  cursor: pointer;
  transition:
    border-color 0.15s,
    background 0.15s;
}

.health-card-link:hover {
  border-color: var(--accent);
  background: var(--bg-hover);
}

.hc-header {
  display: flex;
  align-items: center;
  gap: 0.4rem;
}

.hc-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}

.hc-host {
  font-weight: 600;
  font-size: 0.85rem;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
}

.overdue-badge {
  background: var(--danger-subtle);
  color: var(--danger);
  padding: 0.1rem 0.4rem;
  border-radius: 0.25rem;
  font-weight: 700;
  font-size: 0.6rem;
  flex-shrink: 0;
}

.hc-target {
  font-size: 0.75rem;
  color: var(--text-muted);
}

.hc-time {
  font-size: 0.7rem;
  color: var(--text-muted);
}

/* Trends Row */
.trends-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 1.5rem;
  align-items: start;
}

@media (max-width: 1100px) {
  .trends-row {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 500px) {
  .panel-header {
    flex-direction: column;
    align-items: flex-start;
  }

  .trends-controls {
    flex-wrap: wrap;
    width: 100%;
  }

  .trends-select {
    flex: 1;
    min-width: 0;
  }

  .panel-timeline {
    min-width: 0;
    overflow: hidden;
  }

  .health-grid {
    grid-template-columns: 1fr;
  }
}

/* Active Backups */
.active-backups-panel {
  background: var(--bg-card);
  border: 1px solid var(--accent);
  border-radius: var(--radius);
  padding: 1.25rem;
}

.active-backups-list {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.active-backup-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.85rem;
}

.active-backup-pulse {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--accent);
  animation: pulse 1.5s ease-in-out infinite;
  flex-shrink: 0;
}

@keyframes pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.3;
  }
}

.active-backup-host {
  font-weight: 600;
  color: var(--text-primary);
}

.active-backup-sep {
  color: var(--text-muted);
}

.active-backup-target {
  color: var(--text-secondary);
  font-family: var(--mono);
  font-size: 0.8rem;
}
</style>
