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

interface StorageRepoEntry {
  name: string
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
  next_backup_schedule_id: number | null
  active_schedules: number
  total_schedules: number
  total_storage_bytes: number
  success_30d: number
  failed_30d: number
  total_30d: number
  storage_by_repo: StorageRepoEntry[]
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

type StorageViewMode = 'repo' | 'host' | 'server'
const storageViewMode = ref<StorageViewMode>('repo')
const storageBreakdown = ref<StorageRepoEntry[]>([])

const DONUT_COLORS = [
  'oklch(0.62 0.19 255)',
  'oklch(0.72 0.17 162)',
  'oklch(0.75 0.16 75)',
  'oklch(0.63 0.22 25)',
  'oklch(0.59 0.19 293)',
  'oklch(0.72 0.13 200)',
]

async function fetchStorageBreakdown(): Promise<void> {
  const response = await apiClient.get<StorageRepoEntry[]>(
    `/stats/storage-breakdown?group_by=${storageViewMode.value}`,
  )
  storageBreakdown.value = response.data
}

async function fetchAll(): Promise<void> {
  try {
    const [s, h, a, r] = await Promise.all([
      apiClient.get<DashboardSummary>('/stats/summary'),
      apiClient.get<HealthEntry[]>('/stats/health'),
      apiClient.get<ActivityEntry[]>('/stats/activity?days=14'),
      apiClient.get<RepoOption[]>('/repos'),
    ])
    summary.value = s.data
    health.value = h.data
    activity.value = a.data
    repoOptions.value = r.data.map((repo) => ({ id: repo.id, name: repo.name }))
    storageBreakdown.value = s.data.storage_by_repo
  } finally {
    loading.value = false
  }
}

function setStorageViewMode(mode: StorageViewMode): void {
  storageViewMode.value = mode
  fetchStorageBreakdown().catch(logger.error)
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

const overdueCount = computed((): number => health.value.filter((h) => h.is_overdue).length)

const successRate = computed((): number => {
  if (!summary.value || summary.value.total_30d === 0) return 0
  return Math.round((summary.value.success_30d / summary.value.total_30d) * 100)
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
  (): Array<{ name: string; percentage: number; size: number; color: string; offset: number }> => {
    if (storageBreakdown.value.length === 0) return []
    const circumference = 2 * Math.PI * 54
    let cumulative = 0
    return storageBreakdown.value.map((entry, i) => {
      const offset = cumulative
      cumulative += (entry.percentage / 100) * circumference
      return {
        name: entry.name,
        percentage: entry.percentage,
        size: entry.deduplicated_size,
        color: DONUT_COLORS[i % DONUT_COLORS.length],
        offset,
      }
    })
  },
)

const activityDays = computed((): string[] => {
  const days: string[] = []
  for (let i = 13; i >= 0; i--) {
    const d = new Date()
    d.setDate(d.getDate() - i)
    days.push(d.toISOString().slice(0, 10))
  }
  return days
})

const activityDots = computed((): Array<{ x: number; y: number; color: string; key: number }> => {
  const days = activityDays.value
  if (days.length === 0) return []
  const width = 900
  const height = 200
  const padX = 60
  const padY = 30
  const plotW = width - padX * 2
  const plotH = height - padY * 2

  return activity.value.map((entry) => {
    const date = entry.started_at.slice(0, 10)
    const dayIndex = days.indexOf(date)
    const x = dayIndex >= 0 ? padX + (dayIndex / 13) * plotW : padX
    const hour =
      new Date(entry.started_at).getHours() + new Date(entry.started_at).getMinutes() / 60
    const y = padY + (hour / 24) * plotH

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
          @click="router.push({ name: 'clients', query: { status: 'online' } })"
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
          :class="{ 'stat-card-link': summary?.last_backup_schedule_id }"
          @click="
            summary?.last_backup_schedule_id &&
            router.push(`/schedules/${summary.last_backup_schedule_id}`)
          "
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
            summary?.next_backup_schedule_id &&
            router.push(`/schedules/${summary.next_backup_schedule_id}`)
          "
        >
          <span class="stat-label">Next Backup</span>
          <span class="stat-value stat-value-sm">
            {{ summary?.next_backup_at ? relativeTime(summary.next_backup_at) : '\u2014' }}
          </span>
        </div>
        <div class="stat-card">
          <span class="stat-label">Total Storage</span>
          <span class="stat-value stat-value-sm">
            {{ formatBytes(summary?.total_storage_bytes ?? 0) }}
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
            <h2 class="panel-title">30-Day Success Rate</h2>
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
                <span class="ring-sub">
                  {{ summary?.success_30d ?? 0 }}/{{ summary?.total_30d ?? 0 }} OK
                </span>
              </div>
            </div>
            <div class="ring-legend">
              <span class="legend-item legend-pass">
                <span class="legend-dot" />
                Passed: {{ summary?.success_30d ?? 0 }}
              </span>
              <span class="legend-item legend-fail">
                <span class="legend-dot" />
                Failed: {{ summary?.failed_30d ?? 0 }}
              </span>
            </div>
          </section>

          <!-- Section 3: Storage Donut -->
          <section class="panel">
            <div class="panel-header">
              <h2 class="panel-title">Storage Breakdown</h2>
              <div class="view-toggle">
                <button
                  class="toggle-btn"
                  :class="{ active: storageViewMode === 'repo' }"
                  @click="setStorageViewMode('repo')"
                >
                  Repo
                </button>
                <button
                  class="toggle-btn"
                  :class="{ active: storageViewMode === 'host' }"
                  @click="setStorageViewMode('host')"
                >
                  Client
                </button>
                <button
                  class="toggle-btn"
                  :class="{ active: storageViewMode === 'server' }"
                  @click="setStorageViewMode('server')"
                >
                  Server
                </button>
              </div>
            </div>
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
                <span class="ring-sub"
                  >{{ storageDonuts.length }}
                  {{
                    storageViewMode === 'repo'
                      ? 'repos'
                      : storageViewMode === 'host'
                        ? 'clients'
                        : 'servers'
                  }}</span
                >
              </div>
            </div>
            <div class="storage-legend">
              <div
                v-for="seg in storageDonuts"
                :key="seg.name"
                class="storage-legend-item"
              >
                <span
                  class="legend-color"
                  :style="{ background: seg.color }"
                />
                <span class="legend-name">{{ seg.name }}</span>
                <span class="legend-detail"
                  >{{ seg.percentage }}% &middot; {{ formatBytes(seg.size) }}</span
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

      <!-- Section 5: Activity Timeline -->
      <section class="panel panel-timeline">
        <h2 class="panel-title">Activity (14 Days)</h2>
        <div
          v-if="activity.length === 0 && !loading"
          class="state-msg"
        >
          No activity in the last 14 days.
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
              x="50"
              y="34"
              class="axis-label"
            >
              0h
            </text>
            <text
              x="50"
              y="69"
              class="axis-label"
            >
              6h
            </text>
            <text
              x="50"
              y="104"
              class="axis-label"
            >
              12h
            </text>
            <text
              x="50"
              y="139"
              class="axis-label"
            >
              18h
            </text>
            <text
              x="50"
              y="174"
              class="axis-label"
            >
              24h
            </text>
            <!-- Grid lines -->
            <line
              x1="60"
              y1="30"
              x2="840"
              y2="30"
              class="grid-line"
            />
            <line
              x1="60"
              y1="65"
              x2="840"
              y2="65"
              class="grid-line"
            />
            <line
              x1="60"
              y1="100"
              x2="840"
              y2="100"
              class="grid-line"
            />
            <line
              x1="60"
              y1="135"
              x2="840"
              y2="135"
              class="grid-line"
            />
            <line
              x1="60"
              y1="170"
              x2="840"
              y2="170"
              class="grid-line"
            />
            <!-- X-axis labels -->
            <text
              v-for="(day, idx) in activityDays"
              :key="day"
              :x="60 + (idx / 13) * 780"
              y="195"
              class="axis-label axis-label-x"
            >
              {{ day.slice(5) }}
            </text>
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

      <!-- Section 6: Trends Chart -->
      <TrendsChart :repos="repoOptions" />

      <!-- Section 7: Backup Calendar -->
      <BackupCalendar :repos="repoOptions" />
    </template>
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
  min-width: 600px;
  height: auto;
}

.axis-label {
  font-size: 10px;
  fill: var(--text-muted);
  text-anchor: end;
}

.axis-label-x {
  text-anchor: middle;
  font-size: 9px;
}

.grid-line {
  stroke: var(--border);
  stroke-width: 0.5;
  stroke-dasharray: 3 3;
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
