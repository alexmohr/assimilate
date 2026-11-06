<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { ref } from 'vue'
import { CalendarClock, Plus } from '@lucide/vue'
import { useScheduleRun } from '../composables/useScheduleRun'
import EmptyState from './EmptyState.vue'
import ScheduleTable from './ScheduleTable.vue'
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

const runError = ref<string | null>(null)

/**
 * The run is restricted to this agent: a schedule can target several hosts,
 * and "Run now" pressed on one host's page must not kick off a backup on the
 * others. The failure goes inline rather than to a toast, next to the rows it
 * applies to.
 */
const { runNowLoading: runningId, runNow } = useScheduleRun(null, {
  body: () => ({ agent_ids: [props.agent.id] }),
  onError: (message) => {
    runError.value = message
  },
})
</script>

<template>
  <div class="schedules-tab">
    <p
      v-if="runError"
      class="form-error"
    >
      {{ runError }}
    </p>

    <!--
      An imported host is a host reconstructed from archives found in a repo,
      so there is no agent on it to run anything.
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
    <ScheduleTable
      v-else
      :schedules="schedules"
      :health="health"
      :subtitle-for="repoNameFor"
      :highlight-overdue="highlightOverdue"
      :running-id="runningId"
      @open="emit('open', $event)"
      @run="runNow"
    >
      <template #actions>
        <RouterLink
          v-if="!agent.is_imported"
          :to="{ name: 'schedule-create', query: { agent_id: agent.id } }"
          class="btn btn-primary btn-sm"
        >
          <Plus :size="14" />
          Add schedule
        </RouterLink>
      </template>
    </ScheduleTable>
  </div>
</template>

<style scoped>
.schedules-tab {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}
</style>
