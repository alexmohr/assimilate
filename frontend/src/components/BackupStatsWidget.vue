<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { useRouter } from 'vue-router'
import { apiClient } from '../api/client'
import { formatDuration } from '../utils/format'
import { logger } from '../utils/logger'
import { normalizeBackupStatus } from '../utils/backupStatus'
import type { Repo } from '../types/repo'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'

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
const entries = ref<ActivityEntry[]>([])
const loading = ref(true)

async function fetchStats(): Promise<void> {
  loading.value = true
  try {
    const params = new URLSearchParams({ days: String(selectedDays.value) })
    if (selectedRepoId.value !== undefined) {
      params.set('repo_id', String(selectedRepoId.value))
    }
    const response = await apiClient.get<ActivityEntry[]>(`/stats/activity?${params.toString()}`)
    entries.value = response.data
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  fetchStats().catch(logger.error)
})

watch([selectedDays, selectedRepoId], () => {
  fetchStats().catch(logger.error)
})

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
          label="Backup statistics range"
        />
      </div>
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
        <span class="mini-stat-value">{{ totalCount }}</span>
        <span class="mini-stat-label">Total</span>
      </div>
      <div
        class="mini-stat mini-stat-link"
        @click="navigateToActivity('success')"
      >
        <span
          class="mini-stat-value"
          :class="{
            'color-success': successRate >= 90,
            'color-warning': successRate >= 70 && successRate < 90,
            'color-danger': successRate < 70,
          }"
        >
          {{ successRate }}%
        </span>
        <span class="mini-stat-label">Success</span>
      </div>
      <div
        class="mini-stat mini-stat-link"
        @click="navigateToActivity('failed')"
      >
        <span
          class="mini-stat-value"
          :class="{ 'color-danger': failedCount > 0 }"
        >
          {{ failedCount }}
        </span>
        <span class="mini-stat-label">Failed</span>
      </div>
      <div class="mini-stat">
        <span class="mini-stat-value mini-stat-value-sm">{{
          formatDuration(avgDurationSecs)
        }}</span>
        <span class="mini-stat-label">Avg Duration</span>
      </div>
    </div>
  </section>
</template>

<style scoped>
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

.mini-stat-value {
  font-size: var(--fs-lg);
  font-weight: 700;
  color: var(--text-primary);
}

.mini-stat-value-sm {
  font-size: var(--fs-lg);
}

.mini-stat-label {
  font-size: var(--fs-2xs);
  font-weight: 600;
  text-transform: uppercase;
  color: var(--text-muted);
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
