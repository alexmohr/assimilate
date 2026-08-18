<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed } from 'vue'
import '../chartSetup'
import type { TooltipItem } from 'chart.js'
import type { Repo } from '../types/repo'
import ChartRangeControls from './ChartRangeControls.vue'
import MetricLineChart from './MetricLineChart.vue'
import { useBytesLineChartOptions } from '../composables/useBytesLineChartOptions'
import { useChartTheme } from '../composables/useChartTheme'
import { useRangeFilteredFetch } from '../composables/useRangeFilteredFetch'
import { STORAGE_TREND_RANGE_OPTIONS } from '../utils/chartRangeOptions'

interface TrendEntry {
  date: string
  original_size: number
  compressed_size: number
  deduplicated_size: number
  dedup_ratio: number
  file_count: number
  duration_seconds: number
}

const props = defineProps<{
  repos: Repo[]
}>()

const selectedRepoId = ref<number | undefined>(undefined)
const selectedDays = ref<number>(30)
const { entries: trends, loading } = useRangeFilteredFetch<TrendEntry>(
  '/stats/trends',
  selectedDays,
  selectedRepoId,
)

const combinedSizeData = computed(() => ({
  labels: trends.value.map((t) => t.date.slice(5)),
  datasets: [
    {
      label: 'Original',
      data: trends.value.map((t) => t.original_size),
      borderColor: 'oklch(0.75 0.16 75)',
      backgroundColor: 'oklch(0.75 0.16 75 / 0.0)',
      fill: false,
      tension: 0.3,
    },
    {
      label: 'Compressed',
      data: trends.value.map((t) => t.compressed_size),
      borderColor: 'oklch(0.62 0.19 255)',
      backgroundColor: 'oklch(0.62 0.19 255 / 0.0)',
      fill: false,
      tension: 0.3,
    },
  ],
}))

const deduplicatedData = computed(() => ({
  labels: trends.value.map((t) => t.date.slice(5)),
  datasets: [
    {
      label: 'Deduplicated',
      data: trends.value.map((t) => t.deduplicated_size),
      borderColor: 'oklch(0.72 0.17 162)',
      backgroundColor: 'oklch(0.72 0.17 162 / 0.1)',
      fill: true,
      tension: 0.3,
    },
  ],
}))

const { bytesLineOptions } = useBytesLineChartOptions()
const combinedOptions = computed(() => bytesLineOptions(true))
const chartOptions = computed(() => bytesLineOptions(false))

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

const { textMuted, border } = useChartTheme()

const dedupOptions = computed(() => {
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
        ticks: { color: textMuted.value, font: { size: 10 } },
      },
      y: {
        grid: { color: border.value },
        ticks: {
          color: textMuted.value,
          font: { size: 10 },
          callback: (value: string | number): string => `${Number(value).toFixed(0)}%`,
        },
        min: Math.max(0, Math.floor(dataMin - padding)),
        max: Math.ceil(dataMax + padding),
      },
    },
  }
})
</script>

<template>
  <section class="panel">
    <div class="panel-header">
      <h2 class="panel-title panel-title--truncate">Backup Size Trends (Deduplicated)</h2>
      <ChartRangeControls
        v-model:repo-id="selectedRepoId"
        v-model:days="selectedDays"
        :repos="props.repos"
        :options="STORAGE_TREND_RANGE_OPTIONS"
        label="Trend range"
      />
    </div>
    <p class="chart-desc">
      Size of each backup run over the selected period. <strong>Deduplicated</strong> = new unique
      chunks this backup added to the repository (data not already stored).
    </p>
    <div
      v-if="loading"
      class="state-msg state-msg--inline"
    >
      Loading trends...
    </div>
    <div
      v-else-if="trends.length === 0"
      class="state-msg state-msg--inline"
    >
      No backup data available for the selected period.
    </div>
    <div
      v-else
      class="charts-row"
    >
      <MetricLineChart
        label="Original & Compressed"
        :data="combinedSizeData"
        :options="combinedOptions"
      />
      <MetricLineChart
        label="Deduplicated"
        :data="deduplicatedData"
        :options="chartOptions"
      />
      <MetricLineChart
        label="Dedup Ratio"
        :data="dedupRatioData"
        :options="dedupOptions"
      />
    </div>
  </section>
</template>

<style scoped>
.charts-row {
  display: flex;
  flex-direction: column;
  gap: 1.25rem;
}
</style>
