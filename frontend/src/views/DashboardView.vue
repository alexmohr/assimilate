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
import { cronToHuman } from '../utils/cron'
import { logger } from '../utils/logger'
import BaseSkeleton from '../components/BaseSkeleton.vue'
import TrendsChart from '../components/TrendsChart.vue'
import BackupCalendar from '../components/BackupCalendar.vue'
import RecentActivityWidget from '../components/RecentActivityWidget.vue'
import NextScheduledWidget from '../components/NextScheduledWidget.vue'
import BackupStatsWidget from '../components/BackupStatsWidget.vue'
import RepoHealthWidget from '../components/RepoHealthWidget.vue'
import StorageTrendWidget from '../components/StorageTrendWidget.vue'

interface StorageRepoEntry {
  name: string
  compressed_size: number
  deduplicated_size: number
  percentage: number
}

interface DashboardSummary {
  online_clients: number
  total_clients: number
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
const health = ref<HealthEntry[]>([])
const activity = ref<ActivityEntry[]>([])
const repoOptions = ref<RepoOption[]>([])
const loading = ref(true)

const router = useRouter()

interface ActiveBackup {
  hostname: string
  target_name: string
  started_at: number
}

const activeBackups = ref<ActiveBackup[]>([])

const activityDaysFilter = ref<number>(14)
const activityRepoFilter = ref<number | undefined>(undefined)
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

async function fetchActivity(): Promise<void> {
  const params = new URLSearchParams({ days: String(activityDaysFilter.value) })
  if (activityRepoFilter.value !== undefined) {
    params.set('repo_id', String(activityRepoFilter.value))
  }
  const response = await apiClient.get<ActivityEntry[]>(`/stats/activity?${params.toString()}`)
  activity.value = response.data
}

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
    const [s, h, r] = await Promise.all([
      apiClient.get<DashboardSummary>('/stats/summary'),
      apiClient.get<HealthEntry[]>('/stats/health'),
      apiClient.get<RepoOption[]>('/repos'),
    ])
    summary.value = s.data
    health.value = h.data
    repoOptions.value = r.data.map((repo) => ({ id: repo.id, name: repo.name }))
    storageBreakdown.value = s.data.storage_by_repo
    await Promise.all([fetchActivity(), fetchSuccessActivity()])
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

watch([activityDaysFilter, activityRepoFilter], () => {
  fetchActivity().catch(logger.error)
})

watch([successDaysFilter, successRepoFilter], () => {
  fetchSuccessActivity().catch(logger.error)
})

const overdueCount = computed((): number => health.value.filter((h) => h.is_overdue).length)

const successTotal = computed((): number => successActivity.value.length)
const successCount = computed(
  (): number => successActivity.value.filter((a) => a.status === 'success').length,
)
const failedCount = computed(
  (): number => successActivity.value.filter((a) => a.status !== 'success').length,
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

const clientIndicatorColor = computed((): string => {
  if (!summary.value) return 'var(--text-muted)'
  return summary.value.online_clients === summary.value.total_clients
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

const activityDays = computed((): string[] => {
  const days: string[] = []
  const count = activityDaysFilter.value
  for (let i = count - 1; i >= 0; i--) {
    const d = new Date()
    d.setDate(d.getDate() - i)
    days.push(d.toISOString().slice(0, 10))
  }
  return days
})

const activityHourRange = computed((): { min: number; max: number } => {
  if (activity.value.length === 0) return { min: 0, max: 24 }
  let min = 24
  let max = 0
  for (const entry of activity.value) {
    const d = new Date(entry.started_at)
    const hour = d.getHours() + d.getMinutes() / 60
    if (hour < min) min = hour
    if (hour > max) max = hour
  }
  // Add 1h padding on each side, clamped to 0–24
  min = Math.max(0, Math.floor(min) - 1)
  max = Math.min(24, Math.ceil(max) + 1)
  // Ensure at least 2h range
  if (max - min < 2) {
    min = Math.max(0, min - 1)
    max = Math.min(24, max + 1)
  }
  return { min, max }
})

const activityYTicks = computed((): Array<{ hour: number; y: number }> => {
  const { min, max } = activityHourRange.value
  const range = max - min
  const padY = 30
  const plotH = 200 - padY * 2
  // Choose step: 1h, 2h, 3h, 4h, 6h depending on range
  let step = 1
  if (range > 12) step = 4
  else if (range > 8) step = 3
  else if (range > 4) step = 2

  const ticks: Array<{ hour: number; y: number }> = []
  const startHour = Math.ceil(min / step) * step
  for (let h = startHour; h <= max; h += step) {
    const y = padY + ((h - min) / range) * plotH
    ticks.push({ hour: h, y })
  }
  return ticks
})

const activityDots = computed((): Array<{ x: number; y: number; color: string; key: number }> => {
  const days = activityDays.value
  if (days.length === 0) return []
  const padX = 60
  const padY = 30
  const plotW = 900 - padX * 2
  const plotH = 200 - padY * 2
  const { min, max } = activityHourRange.value
  const range = max - min

  return activity.value.map((entry) => {
    const date = entry.started_at.slice(0, 10)
    const dayIndex = days.indexOf(date)
    const x = dayIndex >= 0 ? padX + (dayIndex / Math.max(days.length - 1, 1)) * plotW : padX
    const hour =
      new Date(entry.started_at).getHours() + new Date(entry.started_at).getMinutes() / 60
    const y = padY + ((hour - min) / range) * plotH

    let color = 'var(--success)'
    if (entry.status === 'warning') color = 'var(--warning)'
    else if (entry.status !== 'success') color = 'var(--danger)'

    return { x, y, color, key: entry.id }
  })
})

function healthStatusColor(entry: HealthEntry): string {
  if (entry.is_overdue) return 'var(--danger)'
  if (entry.last_status === 'success') return 'var(--success)'
  if (entry.last_status === 'warning') return 'var(--warning)'
  return 'var(--danger)'
}

function navigateToLastBackup(): void {
  if (!summary.value?.last_backup_repo_id) return
  const query: Record<string, string> = { tab: 'archives' }
  if (summary.value.last_backup_archive_name) {
    query.archive = summary.value.last_backup_archive_name
  }
  router.push({ path: `/repos/${summary.value.last_backup_repo_id}`, query })
}

interface StatusPopup {
  type: 'failure' | 'warning'
  message: string
  repo_name: string | null
  repo_id: number | null
  schedule_name: string | null
  schedule_id: number | null
  at: string | null
}

const statusPopup = ref<StatusPopup | null>(null)

function showFailurePopup(): void {
  if (!summary.value?.last_failure_at) return
  statusPopup.value = {
    type: 'failure',
    message: summary.value.last_failure_message ?? 'No error details available.',
    repo_name: summary.value.last_failure_repo_name,
    repo_id: summary.value.last_failure_repo_id,
    schedule_name: summary.value.last_failure_schedule_name,
    schedule_id: summary.value.last_failure_schedule_id,
    at: summary.value.last_failure_at,
  }
}

function showWarningPopup(): void {
  if (!summary.value?.last_warning_at) return
  statusPopup.value = {
    type: 'warning',
    message: summary.value.last_warning_message ?? 'No warning details available.',
    repo_name: summary.value.last_warning_repo_name,
    repo_id: summary.value.last_warning_repo_id,
    schedule_name: summary.value.last_warning_schedule_name,
    schedule_id: summary.value.last_warning_schedule_id,
    at: summary.value.last_warning_at,
  }
}

function closeStatusPopup(): void {
  statusPopup.value = null
}

function navigateToRepo(repoId: number | null): void {
  if (repoId) {
    router.push(`/repos/${repoId}`)
  }
  closeStatusPopup()
}

function navigateToSchedule(scheduleId: number | null): void {
  if (scheduleId) {
    router.push(`/schedules/${scheduleId}`)
  }
  closeStatusPopup()
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
      <!-- Section 1: System Status Banner -->
      <section class="status-banner">
        <div
          class="stat-card stat-card-link"
          @click="router.push({ name: 'clients', query: { status: 'offline' } })"
        >
          <span class="stat-label">Online Clients</span>
          <span class="stat-value">
            <span
              class="stat-dot"
              :style="{ background: clientIndicatorColor }"
            />
            {{ summary?.online_clients ?? 0 }}/{{ summary?.total_clients ?? 0 }}
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
          @click="showFailurePopup"
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
          @click="showWarningPopup"
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

      <!-- Main Grid: 2 columns -->
      <div class="main-grid">
        <!-- Left Column: Rings -->
        <div class="left-col">
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
            <p class="chart-desc">Proportion of scheduled backup runs that completed without errors over the selected window.</p>
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
            <p class="chart-desc">Current on-disk usage per repository — deduplicated (unique chunks across all archives).</p>
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

        <!-- Right Column: Health Cards -->
        <div class="right-col">
          <!-- Section 4: Repository Health -->
          <section class="panel panel-full">
            <h2 class="panel-title">Repository Health</h2>
            <div
              v-if="health.length === 0 && !loading"
              class="state-msg"
            >
              No repositories configured yet.
            </div>
            <div class="health-grid">
              <div
                v-for="entry in health"
                :key="`${entry.hostname}-${entry.target_name}`"
                class="health-card health-card-link"
                @click="router.push(`/repos/${entry.repo_id}`)"
              >
                <div class="hc-header">
                  <span
                    class="hc-dot"
                    :style="{ background: healthStatusColor(entry) }"
                  />
                  <span class="hc-host">{{ entry.hostname }}</span>
                  <span
                    v-if="entry.is_overdue"
                    class="overdue-badge"
                  >
                    OVERDUE
                  </span>
                </div>
                <span class="hc-target">{{ entry.target_name }}</span>
                <span class="hc-time">
                  {{ entry.last_backup_at ? relativeTime(entry.last_backup_at) : 'Never' }}
                </span>
              </div>
            </div>
          </section>
        </div>
      </div>

      <!-- Section 5: Activity Timeline + Repo Health -->
      <div class="activity-row">
        <section class="panel panel-timeline">
          <div class="panel-header">
            <h2 class="panel-title">Activity</h2>
            <div class="trends-controls">
              <select
                v-model="activityRepoFilter"
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
                  :class="{ active: activityDaysFilter === 7 }"
                  @click="activityDaysFilter = 7"
                >
                  7d
                </button>
                <button
                  class="toggle-btn"
                  :class="{ active: activityDaysFilter === 14 }"
                  @click="activityDaysFilter = 14"
                >
                  14d
                </button>
                <button
                  class="toggle-btn"
                  :class="{ active: activityDaysFilter === 30 }"
                  @click="activityDaysFilter = 30"
                >
                  30d
                </button>
                <button
                  class="toggle-btn"
                  :class="{ active: activityDaysFilter === 90 }"
                  @click="activityDaysFilter = 90"
                >
                  90d
                </button>
              </div>
            </div>
          </div>
          <div
            v-if="activity.length === 0 && !loading"
            class="state-msg"
          >
            No activity in the selected period.
          </div>
          <div
            v-else
            class="timeline-wrap"
          >
            <svg
              viewBox="0 0 900 200"
              class="timeline-svg"
              preserveAspectRatio="xMidYMid meet"
            >
              <!-- Y-axis labels -->
              <text
                v-for="tick in activityYTicks"
                :key="`y-${tick.hour}`"
                x="50"
                :y="tick.y + 4"
                class="axis-label"
              >
                {{ tick.hour }}h
              </text>
              <!-- Grid lines -->
              <line
                v-for="tick in activityYTicks"
                :key="`g-${tick.hour}`"
                x1="60"
                :y1="tick.y"
                x2="840"
                :y2="tick.y"
                class="grid-line"
              />
              <!-- X-axis labels -->
              <template
                v-for="(day, idx) in activityDays"
                :key="day"
              >
                <text
                  v-if="idx % Math.max(1, Math.floor(activityDays.length / 14)) === 0"
                  :x="60 + (idx / Math.max(activityDays.length - 1, 1)) * 780"
                  y="195"
                  class="axis-label axis-label-x"
                >
                  {{ day.slice(5) }}
                </text>
              </template>
              <!-- Dots -->
              <circle
                v-for="dot in activityDots"
                :key="dot.key"
                :cx="dot.x"
                :cy="dot.y"
                r="4"
                :fill="dot.color"
                opacity="0.85"
              />
            </svg>
          </div>
        </section>
        <RepoHealthWidget :health="health" />
      </div>

      <!-- Section 5b: Widget Row -->
      <div class="widgets-row">
        <BackupStatsWidget :repos="repoOptions" />
        <StorageTrendWidget :repos="repoOptions" />
      </div>

      <!-- Section 6: Trends Chart -->
      <TrendsChart :repos="repoOptions" />

      <!-- Section 7: Calendar + Sidebar -->
      <div class="calendar-row">
        <BackupCalendar :repos="repoOptions" />
        <div class="calendar-sidebar">
          <RecentActivityWidget />
          <NextScheduledWidget />
        </div>
      </div>
    </template>

    <!-- Status Popup (Last Failure / Last Warning) -->
    <div
      v-if="statusPopup"
      class="status-popup-overlay"
      @click="closeStatusPopup"
    >
      <div
        class="status-popup"
        @click.stop
      >
        <div class="status-popup-header">
          <span
            class="status-popup-title"
            :class="{
              'status-popup-title-danger': statusPopup.type === 'failure',
              'status-popup-title-warning': statusPopup.type === 'warning',
            }"
          >
            {{ statusPopup.type === 'failure' ? 'Backup Failed' : 'Backup Warning' }}
          </span>
          <button
            class="status-popup-close"
            @click="closeStatusPopup"
          >
            &times;
          </button>
        </div>
        <div class="status-popup-meta">
          <span v-if="statusPopup.at">{{ relativeTime(statusPopup.at) }}</span>
          <template v-if="statusPopup.repo_name">
            &middot;
            <a
              v-if="statusPopup.repo_id"
              class="status-popup-link"
              @click="navigateToRepo(statusPopup!.repo_id)"
            >
              {{ statusPopup.repo_name }}
            </a>
            <span v-else>{{ statusPopup.repo_name }}</span>
          </template>
          <template v-if="statusPopup.schedule_name">
            &middot;
            <a
              v-if="statusPopup.schedule_id"
              class="status-popup-link"
              @click="navigateToSchedule(statusPopup!.schedule_id)"
            >
              {{ cronToHuman(statusPopup.schedule_name) || statusPopup.schedule_name }}
            </a>
            <span v-else>{{
              cronToHuman(statusPopup.schedule_name) || statusPopup.schedule_name
            }}</span>
          </template>
        </div>
        <pre class="status-popup-msg">{{ statusPopup.message }}</pre>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dashboard {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

/* Section 1: Status Banner */
.status-banner {
  display: grid;
  grid-template-columns: repeat(6, 1fr);
  gap: 0.75rem;
}

@media (max-width: 900px) {
  .status-banner {
    grid-template-columns: repeat(3, 1fr);
  }
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

/* Main 2-col grid */
.main-grid {
  display: grid;
  grid-template-columns: 1fr 1.5fr;
  gap: 1.5rem;
}

@media (max-width: 900px) {
  .main-grid {
    grid-template-columns: 1fr;
  }
}

.left-col {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

.right-col {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
  min-width: 0;
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
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
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

/* Timeline */
.panel-timeline {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem;
}

.timeline-wrap {
  overflow-x: auto;
}

.timeline-svg {
  width: 100%;
  height: auto;
}

@media (max-width: 600px) {
  .timeline-wrap {
    overflow-x: auto;
  }

  .timeline-svg {
    min-width: 500px;
  }
}

.axis-label {
  font-size: 8px;
  fill: var(--text-muted);
  text-anchor: end;
}

.axis-label-x {
  text-anchor: middle;
  font-size: 8px;
}

@media (max-width: 600px) {
  .axis-label {
    font-size: 18px;
  }

  .axis-label-x {
    font-size: 18px;
  }

  .timeline-svg {
    min-width: 500px;
    min-height: 180px;
  }
}

.grid-line {
  stroke: var(--border);
  stroke-width: 0.5;
  stroke-dasharray: 3 3;
}

/* Calendar Row */
.calendar-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 1.5rem;
}

@media (max-width: 900px) {
  .calendar-row {
    grid-template-columns: 1fr;
  }
}

.calendar-sidebar {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

/* Widgets Row */
.widgets-row {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 1.5rem;
}

@media (max-width: 900px) {
  .widgets-row {
    grid-template-columns: 1fr;
  }
}

/* Activity Row */
.activity-row {
  display: grid;
  grid-template-columns: 2fr 1fr;
  gap: 1.5rem;
}

@media (max-width: 900px) {
  .activity-row {
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

/* Status Popup */
.status-popup-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}

.status-popup {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem;
  max-width: 32rem;
  width: 90%;
  max-height: 60vh;
  overflow: auto;
}

.status-popup-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 0.5rem;
}

.status-popup-title {
  font-weight: 600;
  font-size: 0.85rem;
}

.status-popup-title-danger {
  color: var(--danger);
}

.status-popup-title-warning {
  color: var(--warning);
}

.status-popup-close {
  background: transparent;
  border: none;
  font-size: 1.25rem;
  cursor: pointer;
  color: var(--text-muted);
  line-height: 1;
}

.status-popup-meta {
  font-size: 0.75rem;
  color: var(--text-muted);
  margin-bottom: 0.75rem;
}

.status-popup-link {
  color: var(--accent);
  cursor: pointer;
  text-decoration: none;
}

.status-popup-link:hover {
  text-decoration: underline;
}

.status-popup-msg {
  font-family: var(--mono);
  font-size: 0.75rem;
  color: var(--text-primary);
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 0.75rem;
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
}
</style>
