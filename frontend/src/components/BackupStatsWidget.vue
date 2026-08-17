<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed } from 'vue'
import { useRouter } from 'vue-router'
import { formatDuration } from '../utils/format'
import { normalizeBackupStatus } from '../utils/backupStatus'
import type { Repo } from '../types/repo'
import { type SegmentedOption } from './BaseSegmented.vue'
import ChartRangeControls from './ChartRangeControls.vue'
import { useRangeFilteredFetch } from '../composables/useRangeFilteredFetch'

const rangeOptions: SegmentedOption<number>[] = [
  { value: 7, label: '7d' },
  { value: 14, label: '14d' },
  { value: 30, label: '30d' },
  { value: 90, label: '90d' },
]

interface ActivityEntry {
  id: number
  hostname: string
  target_name: string
  started_at: string
  finished_at: string
  status: string
  duration_secs: number
}

const props = defineProps<{ repos: Repo[] }>()
const router = useRouter()

const selectedDays = ref<number>(30)
const selectedRepoId = ref<number | undefined>(undefined)
const { entries, loading } = useRangeFilteredFetch<ActivityEntry>(
  '/stats/activity',
  selectedDays,
  selectedRepoId,
)

const totalCount = computed((): number => entries.value.length)
const successCount = computed(
  (): number => entries.value.filter((e) => normalizeBackupStatus(e.status) === 'success').length,
)
const failedCount = computed(
  (): number => entries.value.filter((e) => normalizeBackupStatus(e.status) !== 'success').length,
)
const successRate = computed((): number => {
  if (totalCount.value === 0) return 0
  return Math.round((successCount.value / totalCount.value) * 100)
})
const avgDurationSecs = computed((): number => {
  if (entries.value.length === 0) return 0
  const total = entries.value.reduce((sum, e) => sum + e.duration_secs, 0)
  return Math.round(total / entries.value.length)
})

function navigateToActivity(status?: string): void {
  const query: Record<string, string> = { days: String(selectedDays.value) }
  if (status) {
    query.status = status
  }
  router.push({ name: 'activity', query })
}
</script>

<template>
  <section class="panel">
    <div class="panel-header">
      <h2 class="panel-title">Backup Stats</h2>
      <ChartRangeControls
        v-model:repo-id="selectedRepoId"
        v-model:days="selectedDays"
        :repos="props.repos"
        :options="rangeOptions"
        label="Backup statistics range"
      />
    </div>
    <div
      v-if="loading"
      class="state-msg state-msg--inline"
    >
      Loading...
    </div>
    <div
      v-else
      class="stats-grid"
    >
      <div
        class="mini-stat mini-stat-link"
        @click="navigateToActivity()"
      >
        <span class="stat-value stat-value--lg">{{ totalCount }}</span>
        <span class="stat-label">Total</span>
      </div>
      <div
        class="mini-stat mini-stat-link"
        @click="navigateToActivity('success')"
      >
        <span
          class="stat-value stat-value--lg"
          :class="{
            'color-success': successRate >= 90,
            'color-warning': successRate >= 70 && successRate < 90,
            'color-danger': successRate < 70,
          }"
        >
          {{ successRate }}%
        </span>
        <span class="stat-label">Success</span>
      </div>
      <div
        class="mini-stat mini-stat-link"
        @click="navigateToActivity('failed')"
      >
        <span
          class="stat-value stat-value--lg"
          :class="{ 'color-danger': failedCount > 0 }"
        >
          {{ failedCount }}
        </span>
        <span class="stat-label">Failed</span>
      </div>
      <div class="mini-stat">
        <span class="stat-value stat-value--lg">{{ formatDuration(avgDurationSecs) }}</span>
        <span class="stat-label">Avg Duration</span>
      </div>
    </div>
  </section>
</template>

<style scoped>
.stats-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.75rem;
}

.mini-stat {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.15rem;
  padding: 0.5rem;
  background: var(--bg-base);
  border-radius: var(--radius-sm);
}

.mini-stat-link {
  cursor: pointer;
  transition:
    background var(--duration-base),
    border-color var(--duration-base);
}

.mini-stat-link:hover {
  background: var(--bg-hover);
}

.color-success {
  color: var(--success);
}

.color-warning {
  color: var(--warning);
}

.color-danger {
  color: var(--danger);
}
</style>
