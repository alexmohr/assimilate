<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { apiClient } from '../api/client'
import { formatBytes } from '../utils/format'
import { logger } from '../utils/logger'

interface TrendEntry {
  date: string
  total_size: number
}

interface RepoOption {
  id: number
  name: string
}

const props = defineProps<{ repos: RepoOption[] }>()

const entries = ref<TrendEntry[]>([])
const loading = ref(true)
const selectedDays = ref<number>(30)
const selectedRepoId = ref<number | undefined>(undefined)

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

const padLeft = 50
const padRight = 10
const padTop = 5
const padBottom = 20
const svgW = 240
const svgH = 80
const plotW = svgW - padLeft - padRight
const plotH = svgH - padTop - padBottom

const values = computed((): number[] => entries.value.map((e) => e.total_size))

const yMin = computed((): number => {
  if (values.value.length === 0) return 0
  return Math.min(...values.value)
})

const yMax = computed((): number => {
  if (values.value.length === 0) return 1
  return Math.max(...values.value)
})

const yRange = computed((): number => yMax.value - yMin.value || 1)

const sparklinePath = computed((): string => {
  if (values.value.length < 2) return ''
  const step = plotW / (values.value.length - 1)
  return values.value
    .map((v, i) => {
      const x = padLeft + i * step
      const y = padTop + plotH - ((v - yMin.value) / yRange.value) * plotH
      return `${i === 0 ? 'M' : 'L'}${x.toFixed(1)},${y.toFixed(1)}`
    })
    .join(' ')
})

const yTicks = computed((): Array<{ label: string; y: number }> => {
  const ticks: Array<{ label: string; y: number }> = []
  const count = 3
  for (let i = 0; i <= count; i++) {
    const val = yMin.value + (yRange.value * i) / count
    const y = padTop + plotH - (i / count) * plotH
    ticks.push({ label: formatBytes(val), y })
  }
  return ticks
})

const xLabels = computed((): Array<{ label: string; x: number }> => {
  if (entries.value.length < 2) return []
  const labels: Array<{ label: string; x: number }> = []
  const step = plotW / (entries.value.length - 1)
  const interval = Math.max(1, Math.floor(entries.value.length / 4))
  for (let i = 0; i < entries.value.length; i += interval) {
    labels.push({
      label: entries.value[i].date.slice(5),
      x: padLeft + i * step,
    })
  }
  return labels
})

const currentSize = computed((): number => {
  if (entries.value.length === 0) return 0
  return entries.value[entries.value.length - 1].total_size
})

const delta = computed((): number => {
  if (entries.value.length < 2) return 0
  return entries.value[entries.value.length - 1].total_size - entries.value[0].total_size
})

const deltaPositive = computed((): boolean => delta.value >= 0)
</script>

<template>
  <section class="panel">
    <div class="panel-header">
      <h2 class="panel-title">Storage Trend</h2>
      <div class="controls">
        <select
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
    <div
      v-if="loading"
      class="state-msg"
    >
      Loading…
    </div>
    <template v-else-if="entries.length >= 2">
      <svg
        :viewBox="`0 0 ${svgW} ${svgH}`"
        class="sparkline"
        preserveAspectRatio="xMidYMid meet"
      >
        <!-- Y-axis labels -->
        <text
          v-for="(tick, i) in yTicks"
          :key="`y-${i}`"
          :x="padLeft - 4"
          :y="tick.y + 3"
          class="axis-text axis-text-y"
        >
          {{ tick.label }}
        </text>
        <!-- X-axis labels -->
        <text
          v-for="(lbl, i) in xLabels"
          :key="`x-${i}`"
          :x="lbl.x"
          :y="svgH - 2"
          class="axis-text axis-text-x"
        >
          {{ lbl.label }}
        </text>
        <!-- Line -->
        <path
          :d="sparklinePath"
          fill="none"
          stroke="var(--accent)"
          stroke-width="1.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
      <div class="trend-summary">
        <div class="trend-stat">
          <span class="trend-current">{{ formatBytes(currentSize) }}</span>
          <span class="trend-label">Current Size</span>
        </div>
        <div class="trend-stat">
          <span
            class="trend-delta"
            :class="{ 'delta-up': deltaPositive, 'delta-down': !deltaPositive }"
          >
            {{ deltaPositive ? '+' : '' }}{{ formatBytes(Math.abs(delta)) }}
          </span>
          <span class="trend-label">Change ({{ selectedDays }}d)</span>
        </div>
      </div>
    </template>
    <div
      v-else
      class="state-msg"
    >
      Not enough data.
    </div>
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
  margin-bottom: 0.75rem;
}

.panel-title {
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
  margin: 0;
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
  padding: 0.5rem 0;
}

.sparkline {
  width: 100%;
  height: 10rem;
  display: block;
  margin-bottom: 0.5rem;
}

@media (max-width: 768px) {
  .sparkline {
    height: 12rem;
  }
}

@media (max-width: 768px) {
  .sparkline {
    height: 8rem;
  }
}

.axis-text {
  font-size: 5px;
  fill: var(--text-muted);
}

.axis-text-y {
  text-anchor: end;
}

.axis-text-x {
  text-anchor: middle;
}

.trend-summary {
  display: flex;
  align-items: flex-start;
  gap: 1.5rem;
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
