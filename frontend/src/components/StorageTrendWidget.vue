<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed } from 'vue'
import '../chartSetup'
import type { Repo } from '../types/repo'
import ChartRangeControls from './ChartRangeControls.vue'
import MetricLineChart from './MetricLineChart.vue'
import { useBytesLineChartOptions } from '../composables/useBytesLineChartOptions'
import { useRangeFilteredFetch } from '../composables/useRangeFilteredFetch'
import { STORAGE_TREND_RANGE_OPTIONS } from '../utils/chartRangeOptions'

interface TrendEntry {
  date: string
  original_size: number
  compressed_size: number
  deduplicated_size: number | null
}

const props = defineProps<{ repos: Repo[] }>()

const selectedDays = ref<number>(30)
const selectedRepoId = ref<number | undefined>(undefined)
const { entries, loading } = useRangeFilteredFetch<TrendEntry>(
  '/stats/storage-trends',
  selectedDays,
  selectedRepoId,
)

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

const { bytesLineOptions } = useBytesLineChartOptions()
const combinedOptions = computed(() => bytesLineOptions(true))
const singleSeriesOptions = computed(() => bytesLineOptions(false))

const hasData = computed((): boolean => entries.value.length >= 2)
</script>

<template>
  <section class="panel">
    <div class="panel-header">
      <h2 class="panel-title">Storage Trend</h2>
      <ChartRangeControls
        v-model:repo-id="selectedRepoId"
        v-model:days="selectedDays"
        :repos="props.repos"
        :options="STORAGE_TREND_RANGE_OPTIONS"
        label="Storage trend range"
      />
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
      <MetricLineChart
        label="Original & Compressed"
        :data="combinedSizeData"
        :options="combinedOptions"
      />
      <MetricLineChart
        label="Deduplicated"
        :data="deduplicatedData"
        :options="singleSeriesOptions"
      />
    </div>
  </section>
</template>

<style scoped>
/* The range selector shares the header row; keep the heading on one line. */
.panel-title {
  white-space: nowrap;
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
</style>
