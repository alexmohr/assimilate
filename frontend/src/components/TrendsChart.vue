<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, onMounted, watch, computed, onBeforeUnmount } from 'vue'
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
  dedup_ratio: number
  file_count: number
  duration_seconds: number
}

interface RepoOption {
  id: number
  name: string
}

const props = defineProps<{
  repos: RepoOption[]
}>()

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

const selectedRepoId = ref<number | undefined>(undefined)
const selectedDays = ref<number>(30)
const trends = ref<TrendEntry[]>([])
const loading = ref(true)

async function fetchTrends(): Promise<void> {
  loading.value = true
  try {
    const params = new URLSearchParams({ days: String(selectedDays.value) })
    if (selectedRepoId.value !== undefined) {
      params.set('repo_id', String(selectedRepoId.value))
    }
    const response = await apiClient.get<TrendEntry[]>(`/stats/trends?${params.toString()}`)
    trends.value = response.data
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  fetchTrends().catch(logger.error)
})

watch([selectedRepoId, selectedDays], () => {
  fetchTrends().catch(logger.error)
})

const chartData = computed(
  (): {
    labels: string[]
    datasets: {
      label: string
      data: number[]
      borderColor: string
      backgroundColor: string
      fill: boolean
      tension: number
    }[]
  } => {
    return {
      labels: trends.value.map((t) => t.date.slice(5)),
      datasets: [
        {
          label: 'Compressed',
          data: trends.value.map((t) => t.compressed_size),
          borderColor: 'oklch(0.62 0.19 255)',
          backgroundColor: 'oklch(0.62 0.19 255 / 0.1)',
          fill: true,
          tension: 0.3,
        },
        {
          label: 'Deduplicated',
          data: trends.value.map((t) => t.deduplicated_size),
          borderColor: 'oklch(0.72 0.17 162)',
          backgroundColor: 'oklch(0.72 0.17 162 / 0.1)',
          fill: true,
          tension: 0.3,
        },
      ],
    }
  },
)

const chartOptions = computed(() => {
  void themeGeneration.value
  const textSecondary = cssVar('--text-secondary')
  const textMuted = cssVar('--text-muted')
  const border = cssVar('--border')
  return {
    responsive: true,
    maintainAspectRatio: false,
    interaction: {
      intersect: false,
      mode: 'index' as const,
    },
    plugins: {
      legend: {
        position: 'bottom' as const,
        labels: {
          color: textSecondary,
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

const dedupRatioData = computed(
  (): {
    labels: string[]
    datasets: {
      label: string
      data: number[]
      borderColor: string
      backgroundColor: string
      fill: boolean
      tension: number
    }[]
  } => {
    return {
      labels: trends.value.map((t) => t.date.slice(5)),
      datasets: [
        {
          label: 'Dedup Ratio %',
          data: trends.value.map((t) => t.dedup_ratio),
          borderColor: 'oklch(0.75 0.16 75)',
          backgroundColor: 'oklch(0.75 0.16 75 / 0.1)',
          fill: true,
          tension: 0.3,
        },
      ],
    }
  },
)

const dedupOptions = computed(() => {
  void themeGeneration.value
  const textMuted = cssVar('--text-muted')
  const border = cssVar('--border')
  const values = trends.value.map((t) => t.dedup_ratio)
  const dataMin = values.length > 0 ? Math.min(...values) : 0
  const dataMax = values.length > 0 ? Math.max(...values) : 100
  const padding = Math.max((dataMax - dataMin) * 0.1, 1)
  return {
    responsive: true,
    maintainAspectRatio: false,
    plugins: {
      legend: { display: false },
      tooltip: {
        callbacks: {
          label: (context: TooltipItem<'line'>): string => `${(context.parsed.y ?? 0).toFixed(1)}%`,
        },
      },
    },
    scales: {
      x: {
        grid: { display: false },
        ticks: { color: textMuted, font: { size: 10 } },
      },
      y: {
        grid: { color: border },
        ticks: {
          color: textMuted,
          font: { size: 10 },
          callback: (value: string | number): string => `${Number(value).toFixed(0)}%`,
        },
        min: Math.max(0, Math.floor(dataMin - padding)),
        max: Math.min(100, Math.ceil(dataMax + padding)),
      },
    },
  }
})
</script>

<template>
  <section class="panel">
    <div class="panel-header">
      <h2 class="panel-title">Backup Size Trends (Deduplicated)</h2>
      <div class="trends-controls">
        <select
          v-model="selectedRepoId"
          class="trends-select"
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
    <p class="chart-desc">
      Size of each backup run over the selected period. <strong>Deduplicated</strong> = new unique
      chunks this backup added to the repository (data not already stored).
    </p>
    <div
      v-if="loading"
      class="state-msg"
    >
      Loading trends…
    </div>
    <div
      v-else-if="trends.length === 0"
      class="state-msg"
    >
      No backup data available for the selected period.
    </div>
    <template v-else>
      <div class="chart-container">
        <Line
          :data="chartData"
          :options="chartOptions"
        />
      </div>
      <div class="chart-container chart-container-sm">
        <Line
          :data="dedupRatioData"
          :options="dedupOptions"
        />
      </div>
      <span class="dedup-label"
        >Dedup Ratio — deduplicated ÷ original size; lower means more data was already stored in the
        repo</span
      >
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

.chart-container {
  height: 220px;
  position: relative;
}

.chart-container-sm {
  height: 100px;
  margin-top: 1rem;
}

.dedup-label {
  display: block;
  text-align: center;
  font-size: 0.65rem;
  color: var(--text-muted);
  margin-top: 0.25rem;
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
</style>
