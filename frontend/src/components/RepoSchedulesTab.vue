<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref, watch, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { CalendarClock } from '@lucide/vue'
import { listRepoSchedules, getScheduleHealth } from '../api/schedules'
import { useScheduleRun } from '../composables/useScheduleRun'
import type { ScheduleHealthEntry } from '../utils/scheduleHealth'
import BaseSpinner from './BaseSpinner.vue'
import EmptyState from './EmptyState.vue'
import ScheduleTable from './ScheduleTable.vue'
import type { ScheduleRow } from '../types/schedule'
import { scheduleTypeLabel } from '../utils/scheduleType'

const props = defineProps<{ repoId: number }>()

const router = useRouter()
const { runNowLoading, runNow } = useScheduleRun(scheduleTypeLabel)

const schedules = ref<ScheduleRow[]>([])
const loading = ref(false)
const error = ref<string | null>(null)
const health = ref<ScheduleHealthEntry[]>([])

function agentsLine(s: ScheduleRow): string {
  const n = s.target_hostnames.length
  return n === 1 ? s.target_hostnames[0] : `${n} agents`
}

async function load(): Promise<void> {
  loading.value = true
  error.value = null
  try {
    const [scheduleRows, healthRows] = await Promise.all([
      listRepoSchedules(props.repoId),
      getScheduleHealth(),
    ])
    schedules.value = scheduleRows
    health.value = healthRows
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
  <ScheduleTable
    v-else
    :schedules="schedules"
    :health="health"
    :subtitle-for="agentsLine"
    :running-id="runNowLoading"
    @open="router.push(`/schedules/${$event.id}`)"
    @run="runNow"
  />
</template>
