<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, watch, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { CalendarClock } from '@lucide/vue'
import { apiClient } from '../api/client'
import { formatDate } from '../utils/format'
import { useScheduleRun } from '../composables/useScheduleRun'
import {
  scheduleIssuesFromEntries,
  withErrorTitles,
  type ScheduleHealthEntry,
} from '../utils/scheduleHealth'
import BaseSpinner from './BaseSpinner.vue'
import EmptyState from './EmptyState.vue'
import type { EntityIssue } from './EntityStatusBadges.vue'
import ScheduleCard from './ScheduleCard.vue'
import type { ScheduleRow, ScheduleType } from '../types/schedule'

const props = defineProps<{ repoId: number }>()

const router = useRouter()
function scheduleTypeLabel(t: ScheduleType): string {
  switch (t) {
    case 'backup':
      return 'Backup'
    case 'check':
      return 'Integrity check'
    case 'verify':
      return 'Verify (extract dry-run)'
  }
}

const { runNowLoading, runNow } = useScheduleRun(scheduleTypeLabel)

const schedules = ref<ScheduleRow[]>([])
const loading = ref(false)
const error = ref<string | null>(null)
const health = ref<ScheduleHealthEntry[]>([])

function scheduleIssues(s: ScheduleRow): EntityIssue[] {
  const entries = health.value.filter((h) => h.schedule_id === s.id)
  return withErrorTitles(scheduleIssuesFromEntries(entries, s.id, router), entries)
}

async function load(): Promise<void> {
  loading.value = true
  error.value = null
  try {
    const [schRes, healthRes] = await Promise.all([
      apiClient.get<ScheduleRow[]>(`/repos/${props.repoId}/schedules`),
      apiClient.get<ScheduleHealthEntry[]>('/stats/health'),
    ])
    schedules.value = schRes.data
    health.value = healthRes.data
  } catch {
    error.value = 'Failed to load schedules.'
  } finally {
    loading.value = false
  }
}

watch(() => props.repoId, load)
onMounted(load)

defineExpose({ reload: load })
</script>

<template>
  <BaseSpinner
    v-if="loading"
    size="lg"
  />
  <div
    v-else-if="error"
    class="error-banner"
  >
    {{ error }}
  </div>
  <EmptyState
    v-else-if="schedules.length === 0"
    :icon="CalendarClock"
    title="No schedules yet"
    description="Nothing backs up to this repository. Create a schedule to start."
  />
  <div
    v-else
    class="card-grid"
  >
    <ScheduleCard
      v-for="s in schedules"
      :key="s.id"
      :schedule="s"
      :issues="scheduleIssues(s)"
      :format-run="formatDate"
      @select="router.push(`/schedules/${s.id}`)"
    >
      <template #title>{{ s.name || `Schedule #${s.id}` }}</template>
      <template #meta>
        <span class="meta-pill">
          {{ s.target_hostnames.length }}
          agent{{ s.target_hostnames.length === 1 ? '' : 's' }}
        </span>
      </template>
      <template #actions>
        <button
          class="btn btn-sm btn-ghost"
          :disabled="runNowLoading === s.id"
          :title="`Run ${scheduleTypeLabel(s.schedule_type ?? 'backup').toLowerCase()} now`"
          @click="runNow(s)"
        >
          {{ runNowLoading === s.id ? '...' : 'Run' }}
        </button>
      </template>
    </ScheduleCard>
  </div>
</template>

<style scoped></style>
