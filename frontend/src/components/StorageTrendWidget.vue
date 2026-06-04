<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted, watch, onBeforeUnmount } from 'vue'
import { Line } from 'vue-chartjs'
import {
  Chart as ChartJS,
  CategoryScale,
  LinearScale,
  PointElement,
  LineElement,
  Title,
  Tooltip,
  Legend,
  Filler,
} from 'chart.js'
import type { TooltipItem } from 'chart.js'
import { apiClient } from '../api/client'
import { formatBytes } from '../utils/format'
import { logger } from '../utils/logger'

ChartJS.register(
  CategoryScale,
  LinearScale,
  PointElement,
  LineElement,
  Title,
  Tooltip,
  Legend,
  Filler,
)

interface TrendEntry {
  date: string
  original_size: number
  compressed_size: number
  deduplicated_size: number
}

interface ByRepoEntry {
  date: string
  repo_id: number
  repo_name: string
  original_size: number
  compressed_size: number
  deduplicated_size: number
}

interface RepoOption {
  id: number
  name: string
}

const props = defineProps<{ repos: RepoOption[] }>()

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim()
}

const themeGeneration = ref(0)
let themeObserver: MutationObserver | null = null

onMounted(() => {
  themeObserver = new MutationObserver(() => {
    themeGeneration.value++
  })
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['class'],
  })
})

onBeforeUnmount(() => {
  themeObserver?.disconnect()
})

const selectedDays = ref<number>(30)
const selectedRepoId = ref<number | undefined>(undefined)
const viewMode = ref<'total' | 'stacked'>('total')
const entries = ref<TrendEntry[]>([])
const byRepoEntries = ref<ByRepoEntry[]>([])
const loading = ref(true)

async function fetchTrends(): Promise<void> {
  loading.value = true
  try {
    if (viewMode.value === 'stacked' && selectedRepoId.value === undefined) {
      const response = await apiClient.get<ByRepoEntry[]>(
        `/stats/storage-trends/by-repo?days=${selectedDays.value}`,
      )
      byRepoEntries.value = response.data
      entries.value = []
    } else {
      const params = new URLSearchParams({ days: String(selectedDays.value) })
      if (selectedRepoId.value !== undefined) {
        params.set('repo_id', String(selectedRepoId.value))
      }
      const response = await apiClient.get<TrendEntry[]>(
        `/stats/storage-trends?${params.toString()}`,
      )
      entries.value = response.data
      byRepoEntries.value = []
    }
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  fetchTrends().catch(logger.error)
})

watch([selectedDays, selectedRepoId, viewMode], () => {
  fetchTrends().catch(logger.error)
})

const COLORS = [
  'oklch(0.62 0.19 255)',
  'oklch(0.72 0.17 162)',
  'oklch(0.75 0.16 75)',
  'oklch(0.65 0.20 330)',
  'oklch(0.70 0.15 200)',
  'oklch(0.68 0.18 30)',
  'oklch(0.60 0.14 280)',
  'oklch(0.73 0.12 120)',
]

const stackedRepoNames = computed((): string[] => {
  const names = new Set<string>()
  byRepoEntries.value.forEach((e) => names.add(e.repo_name))
  return [...names]
})

const stackedDates = computed((): string[] => {
  const dates = new Set<string>()
  byRepoEntries.value.forEach((e) => dates.add(e.date))
  return [...dates].sort()
})

const chartData = computed(() => {
  if (viewMode.value === 'stacked' && selectedRepoId.value === undefined) {
    const dates = stackedDates.value
    const repos = stackedRepoNames.value
    const dataByRepo = new Map<string, Map<string, number>>()
    byRepoEntries.value.forEach((e) => {
      if (!dataByRepo.has(e.repo_name)) {
        dataByRepo.set(e.repo_name, new Map())
      }
      dataByRepo.get(e.repo_name)!.set(e.date, e.deduplicated_size)
    })
    return {
      labels: dates.map((d) => d.slice(5)),
      datasets: repos.map((repo, i) => ({
        label: repo,
        data: dates.map((d) => dataByRepo.get(repo)?.get(d) ?? 0),
        borderColor: COLORS[i % COLORS.length],
        backgroundColor: COLORS[i % COLORS.length].replace(')', ' / 0.3)'),
        fill: true,
        tension: 0.3,
      })),
    }
  }
  return {
    labels: entries.value.map((t) => t.date.slice(5)),
    datasets: [
      {
        label: 'Original',
        data: entries.value.map((t) => t.original_size),
        borderColor: 'oklch(0.75 0.16 75)',
        backgroundColor: 'oklch(0.75 0.16 75 / 0.1)',
        fill: true,
        tension: 0.3,
      },
      {
        label: 'Compressed',
        data: entries.value.map((t) => t.compressed_size),
        borderColor: 'oklch(0.62 0.19 255)',
        backgroundColor: 'oklch(0.62 0.19 255 / 0.1)',
        fill: true,
        tension: 0.3,
      },
      {
        label: 'Deduplicated',
        data: entries.value.map((t) => t.deduplicated_size),
        borderColor: 'oklch(0.72 0.17 162)',
        backgroundColor: 'oklch(0.72 0.17 162 / 0.1)',
        fill: true,
        tension: 0.3,
      },
    ],
  }
})

const chartOptions = computed(() => {
  void themeGeneration.value
  const textMuted = cssVar('--text-muted')
  const border = cssVar('--border')
  const isStacked = viewMode.value === 'stacked' && selectedRepoId.value === undefined
  const showLegend = isStacked || viewMode.value === 'total'
  return {
    responsive: true,
    maintainAspectRatio: false,
    interaction: {
      intersect: false,
      mode: 'index' as const,
    },
    plugins: {
      legend: {
        display: showLegend,
        position: 'bottom' as const,
        labels: {
          color: textMuted,
          usePointStyle: true,
          pointStyle: 'circle' as const,
        },
      },
      tooltip: {
        callbacks: {
          label: (context: TooltipItem<'line'>): string => {
            return `${context.dataset.label ?? ''}: ${formatBytes(context.parsed.y ?? 0)}`
          },
        },
      },
    },
    scales: {
      x: {
        grid: { display: false },
        ticks: { color: textMuted, font: { size: 10 } },
      },
      y: {
        stacked: isStacked,
        grid: { color: border },
        ticks: {
          color: textMuted,
          font: { size: 10 },
          callback: (value: string | number): string => formatBytes(Number(value)),
        },
      },
    },
  }
})

const hasData = computed(
  (): boolean => entries.value.length >= 2 || byRepoEntries.value.length >= 2,
)

const currentOriginal = computed((): number => {
  if (entries.value.length === 0) return 0
  return entries.value[entries.value.length - 1].original_size
})

const currentCompressed = computed((): number => {
  if (entries.value.length === 0) return 0
  return entries.value[entries.value.length - 1].compressed_size
})

const currentSize = computed((): number => {
  if (entries.value.length === 0) return 0
  return entries.value[entries.value.length - 1].deduplicated_size
})

const delta = computed((): number => {
  if (entries.value.length < 2) return 0
  return (
    entries.value[entries.value.length - 1].deduplicated_size - entries.value[0].deduplicated_size
  )
})

const deltaPositive = computed((): boolean => delta.value >= 0)
</script>

<template>
  <section class="panel">
    <div class="panel-header">
      <h2 class="panel-title">Storage Trend</h2>
      <div class="controls">
        <select
          v-if="viewMode === 'total'"
          v-model="selectedRepoId"
          class="stats-select"
        >
          <option :value="undefined">All Repos</option>
          <option
            v-for="repo in props.repos"
            :key="repo.id"
            :value="repo.id"
          >
            {{ repo.name }}
          </option>
        </select>
        <div class="view-toggle">
          <button
            class="toggle-btn"
            :class="{ active: viewMode === 'total' }"
            @click="viewMode = 'total'"
          >
            Total
          </button>
          <button
            class="toggle-btn"
            :class="{ active: viewMode === 'stacked' }"
            @click="viewMode = 'stacked'"
          >
            Stacked
          </button>
        </div>
        <div class="view-toggle">
          <button
            class="toggle-btn"
            :class="{ active: selectedDays === 14 }"
            @click="selectedDays = 14"
          >
            14d
          </button>
          <button
            class="toggle-btn"
            :class="{ active: selectedDays === 30 }"
            @click="selectedDays = 30"
          >
            30d
          </button>
          <button
            class="toggle-btn"
            :class="{ active: selectedDays === 90 }"
            @click="selectedDays = 90"
          >
            90d
          </button>
          <button
            class="toggle-btn"
            :class="{ active: selectedDays === 365 }"
            @click="selectedDays = 365"
          >
            1y
          </button>
        </div>
      </div>
    </div>
    <p
      v-if="viewMode === 'stacked' && selectedRepoId === undefined"
      class="chart-desc"
    >
      Actual on-disk size (deduplicated) per repository over time, stacked.
    </p>
    <p
      v-else
      class="chart-desc"
    >
      Repository disk usage over time. <strong>Deduplicated</strong> = actual on-disk footprint
      (all unique compressed chunks across every archive in the repo).
    </p>
    <div
      v-if="loading"
      class="state-msg"
    >
      Loading…
    </div>
    <div
      v-else-if="!hasData"
      class="state-msg"
    >
      Not enough data.
    </div>
    <template v-else>
      <div class="chart-container">
        <Line
          :data="chartData"
          :options="chartOptions"
        />
      </div>
      <div
        v-if="viewMode === 'total' && entries.length >= 2"
        class="trend-summary"
      >
        <div class="trend-stat">
          <span class="trend-current">{{ formatBytes(currentOriginal) }}</span>
          <span class="trend-label">Original Size</span>
        </div>
        <div class="trend-stat">
          <span class="trend-current">{{ formatBytes(currentCompressed) }}</span>
          <span class="trend-label">Compressed</span>
        </div>
        <div class="trend-stat">
          <span class="trend-current">{{ formatBytes(currentSize) }}</span>
          <span class="trend-label">Deduplicated</span>
        </div>
        <div class="trend-stat">
          <span
            class="trend-delta"
            :class="{ 'delta-up': deltaPositive, 'delta-down': !deltaPositive }"
          >
            {{ deltaPositive ? '+' : '' }}{{ formatBytes(Math.abs(delta)) }}
          </span>
          <span class="trend-label">Dedup Change ({{ selectedDays }}d)</span>
        </div>
      </div>
    </template>
  </section>
</template>

<style scoped>
.panel {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem;
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 0.5rem;
  margin-bottom: 1rem;
}

.panel-title {
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
  margin: 0;
  white-space: nowrap;
}

.controls {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.stats-select {
  padding: 0.25rem 0.5rem;
  font-size: 0.75rem;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-base);
  color: var(--text-primary);
}

.view-toggle {
  display: flex;
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

.chart-container {
  height: 220px;
  position: relative;
}

.trend-summary {
  display: flex;
  align-items: flex-start;
  gap: 1.5rem;
  margin-top: 0.75rem;
}

.trend-stat {
  display: flex;
  flex-direction: column;
  gap: 0.15rem;
}

.trend-label {
  font-size: 0.65rem;
  font-weight: 600;
  text-transform: uppercase;
  color: var(--text-muted);
}

.trend-current {
  font-size: 1.1rem;
  font-weight: 700;
  color: var(--text-primary);
}

.trend-delta {
  font-size: 0.75rem;
  font-weight: 600;
}

.delta-up {
  color: var(--warning);
}

.delta-down {
  color: var(--success);
}
</style>
