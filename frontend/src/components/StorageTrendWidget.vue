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
import type { Repo } from '../types/repo'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'

const rangeOptions: SegmentedOption<number>[] = [
  { value: 14, label: '14d' },
  { value: 30, label: '30d' },
  { value: 90, label: '90d' },
  { value: 365, label: '1y' },
]

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
  deduplicated_size: number | null
}

const props = defineProps<{ repos: Repo[] }>()

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
const entries = ref<TrendEntry[]>([])
const loading = ref(true)

async function fetchTrends(): Promise<void> {
  loading.value = true
  try {
    const params = new URLSearchParams({ days: String(selectedDays.value) })
    if (selectedRepoId.value !== undefined) {
      params.set('repo_id', String(selectedRepoId.value))
    }
    const response = await apiClient.get<TrendEntry[]>(`/stats/storage-trends?${params.toString()}`)
    entries.value = response.data
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  fetchTrends().catch(logger.error)
})

watch([selectedDays, selectedRepoId], () => {
  fetchTrends().catch(logger.error)
})

const combinedSizeData = computed(() => ({
  labels: entries.value.map((t) => t.date.slice(5)),
  datasets: [
    {
      label: 'Original',
      data: entries.value.map((t) => t.original_size),
      borderColor: 'oklch(0.75 0.16 75)',
      backgroundColor: 'oklch(0.75 0.16 75 / 0.0)',
      fill: false,
      tension: 0.3,
    },
    {
      label: 'Compressed',
      data: entries.value.map((t) => t.compressed_size),
      borderColor: 'oklch(0.62 0.19 255)',
      backgroundColor: 'oklch(0.62 0.19 255 / 0.0)',
      fill: false,
      tension: 0.3,
    },
  ],
}))

const deduplicatedData = computed(() => ({
  labels: entries.value.map((t) => t.date.slice(5)),
  datasets: [
    {
      label: 'Deduplicated',
      data: entries.value.map((t) => t.deduplicated_size),
      borderColor: 'oklch(0.72 0.17 162)',
      backgroundColor: 'oklch(0.72 0.17 162 / 0.15)',
      fill: true,
      tension: 0.3,
    },
  ],
}))

const combinedOptions = computed(() => {
  void themeGeneration.value
  const textMuted = cssVar('--text-muted')
  const border = cssVar('--border')
  return {
    responsive: true,
    maintainAspectRatio: false,
    interaction: { intersect: false, mode: 'index' as const },
    plugins: {
      legend: {
        display: true,
        labels: { color: textMuted, boxWidth: 12, font: { size: 10 } },
      },
      tooltip: {
        callbacks: {
          label: (context: TooltipItem<'line'>): string =>
            `${context.dataset.label ?? ''}: ${formatBytes(context.parsed.y ?? 0)}`,
        },
      },
    },
    scales: {
      x: { grid: { display: false }, ticks: { color: textMuted, font: { size: 10 } } },
      y: {
        grace: '10%',
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

const singleSeriesOptions = computed(() => {
  void themeGeneration.value
  const textMuted = cssVar('--text-muted')
  const border = cssVar('--border')
  return {
    responsive: true,
    maintainAspectRatio: false,
    interaction: { intersect: false, mode: 'index' as const },
    plugins: {
      legend: { display: false },
      tooltip: {
        callbacks: {
          label: (context: TooltipItem<'line'>): string =>
            `${context.dataset.label ?? ''}: ${formatBytes(context.parsed.y ?? 0)}`,
        },
      },
    },
    scales: {
      x: { grid: { display: false }, ticks: { color: textMuted, font: { size: 10 } } },
      y: {
        grace: '10%',
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

const hasData = computed((): boolean => entries.value.length >= 2)
</script>

<template>
  <section class="panel">
    <div class="panel-header">
      <h2 class="panel-title">Storage Trend</h2>
      <div class="controls">
        <select
          v-model="selectedRepoId"
          class="input stats-select"
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
        <BaseSegmented
          v-model="selectedDays"
          :options="rangeOptions"
          label="Storage trend range"
        />
      </div>
    </div>
    <p class="chart-desc">
      Repository disk usage over time. <strong>Deduplicated</strong> = actual on-disk footprint (all
      unique compressed chunks across every archive in the repo).
    </p>
    <div
      v-if="loading"
      class="state-msg state-msg--inline"
    >
      Loading...
    </div>
    <div
      v-else-if="!hasData"
      class="state-msg state-msg--inline"
    >
      Not enough data.
    </div>
    <div
      v-else
      class="charts-col"
    >
      <div class="chart-cell">
        <span class="metric-label">Original &amp; Compressed</span>
        <div class="chart-container">
          <Line
            :data="combinedSizeData"
            :options="combinedOptions"
          />
        </div>
      </div>
      <div class="chart-cell">
        <span class="metric-label">Deduplicated</span>
        <div class="chart-container">
          <Line
            :data="deduplicatedData"
            :options="singleSeriesOptions"
          />
        </div>
      </div>
    </div>
  </section>
</template>

<style scoped>
/* The range selector shares the header row; keep the heading on one line. */
.panel-title {
  white-space: nowrap;
}

.controls {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.stats-select {
  width: auto;
  padding: 0.25rem 0.5rem;
  font-size: var(--fs-xs);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-base);
  color: var(--text-primary);
}

.chart-desc {
  color: var(--text-muted);
  font-size: var(--fs-2xs);
  margin: 0 0 0.75rem;
  line-height: 1.4;
}

.charts-col {
  display: flex;
  flex-direction: column;
  gap: 1.25rem;
}

.chart-cell {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.metric-label {
  font-size: var(--fs-2xs);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-muted);
}

.chart-container {
  height: 220px;
  position: relative;
}
</style>
