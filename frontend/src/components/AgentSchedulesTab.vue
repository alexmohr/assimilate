<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { CalendarClock } from '@lucide/vue'
import { apiClient } from '../api/client'
import { extractError } from '../utils/error'
import EmptyState from './EmptyState.vue'
import AgentScheduleRow from './AgentScheduleRow.vue'
import type { ScheduleHealthEntry } from '../utils/scheduleHealth'
import type { ScheduleRow } from '../types/schedule'
import type { AgentRow } from '../types/agent'

const props = defineProps<{
  agent: AgentRow
  schedules: readonly ScheduleRow[]
  health: readonly ScheduleHealthEntry[]
  /** Set from `?health=overdue`, to ring the schedule that was linked to. */
  highlightOverdue: boolean
  repoNameFor: (schedule: ScheduleRow) => string
}>()

const emit = defineEmits<{ open: [schedule: ScheduleRow] }>()

const runningId = ref<number | null>(null)
const runError = ref<string | null>(null)

function healthFor(schedule: ScheduleRow): ScheduleHealthEntry[] {
  return props.health.filter((h) => h.schedule_id === schedule.id)
}

/**
 * Restricted to this agent. A schedule can target several hosts, and "Run
 * now" pressed on one host's page must not kick off a backup on the others.
 */
async function runNow(schedule: ScheduleRow): Promise<void> {
  runningId.value = schedule.id
  runError.value = null
  try {
    await apiClient.post(`/schedules/${schedule.id}/run`, { agent_ids: [props.agent.id] })
  } catch (e: unknown) {
    runError.value = extractError(e)
  } finally {
    runningId.value = null
  }
}
</script>

<template>
  <div class="schedules-tab">
    <div class="tab-header">
      <h2 class="panel-title">
        {{ schedules.length }} schedule{{ schedules.length === 1 ? '' : 's' }} target this agent
      </h2>
      <RouterLink
        v-if="!agent.is_imported"
        :to="{ name: 'schedule-create', query: { agent_id: agent.id } }"
        class="btn btn-primary btn-sm"
      >
        Add schedule
      </RouterLink>
    </div>

    <p
      v-if="runError"
      class="form-error"
    >
      {{ runError }}
    </p>

    <!--
      An imported host is a host reconstructed from archives found in a repo,
      so there is no agent on it to run anything. The tab is kept rather than
      hidden: a tab bar whose contents shift per host means the position of
      Backups moves, and this is exactly where the emptiness ends once the
      host is adopted.
    -->
    <EmptyState
      v-if="schedules.length === 0 && agent.is_imported"
      :icon="CalendarClock"
      title="No schedules yet"
      :description="`${agent.hostname} was reconstructed from archives found in a repository, so there is no agent here to run a backup. Adopt the host to install one, or merge it into an agent that already exists.`"
    />
    <EmptyState
      v-else-if="schedules.length === 0"
      :icon="CalendarClock"
      title="No schedules yet"
      description="This agent has no backup schedules. Create one to start backing it up."
    />
    <div
      v-else
      class="rows"
    >
      <AgentScheduleRow
        v-for="s in schedules"
        :key="s.id"
        :schedule="s"
        :repo-name="repoNameFor(s)"
        :health="healthFor(s)"
        :highlighted="highlightOverdue && healthFor(s).some((h) => h.is_overdue)"
        :running="runningId === s.id"
        show-actions
        @open="emit('open', s)"
        @run="runNow(s)"
      />
    </div>
  </div>
</template>

<style scoped>
.schedules-tab {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.tab-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  flex-wrap: wrap;
}
</style>
