<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { RouterLink } from 'vue-router'
import type { DashboardOperation, DashboardUpcomingSchedule } from '../types/dashboard'
import { relativeTime } from '../utils/format'

defineProps<{
  operations: DashboardOperation[]
  schedules: DashboardUpcomingSchedule[]
}>()
</script>

<template>
  <section
    id="upcoming-work"
    class="panel"
  >
    <h2 class="panel-title">Upcoming Work</h2>
    <div
      v-if="operations.length === 0 && schedules.length === 0"
      class="empty-state"
    >
      No active or scheduled work
    </div>
    <div class="work-list">
      <RouterLink
        v-for="operation in operations"
        :key="`operation-${operation.report_id}`"
        :to="{ path: '/activity', query: { report: operation.report_id } }"
        class="work-row"
      >
        <span class="running-dot" />
        <strong>{{ operation.schedule_name }}</strong>
        <span>{{ operation.hostname }} · {{ operation.repo_name }}</span>
        <time>Running {{ relativeTime(operation.started_at) }}</time>
      </RouterLink>
      <RouterLink
        v-for="schedule in schedules"
        :key="`schedule-${schedule.schedule_id}`"
        :to="`/schedules/${schedule.schedule_id}`"
        class="work-row"
      >
        <span class="scheduled-dot" />
        <strong>{{ schedule.schedule_name }}</strong>
        <span>
          {{ schedule.target_count }} targets
          <template v-if="schedule.offline_target_count > 0">
            · {{ schedule.offline_target_count }} offline
          </template>
        </span>
        <time>{{ relativeTime(schedule.next_run_at) }}</time>
      </RouterLink>
    </div>
  </section>
</template>

<style scoped>
.work-list {
  display: flex;
  flex-direction: column;
}

.work-row {
  display: grid;
  grid-template-columns: 10px minmax(140px, 0.8fr) 1fr auto;
  gap: 0.75rem;
  align-items: center;
  padding: 0.75rem 0;
  border-top: 1px solid var(--border);
  color: inherit;
  text-decoration: none;
  font-size: 0.75rem;
}

.work-row span,
.work-row time,
.empty-state {
  color: var(--text-muted);
}

.running-dot,
.scheduled-dot {
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: var(--success);
}

.running-dot {
  box-shadow: 0 0 0 4px color-mix(in srgb, var(--success) 20%, transparent);
}

.scheduled-dot {
  background: var(--accent);
}

@media (max-width: 700px) {
  .work-row {
    grid-template-columns: 10px 1fr auto;
  }

  .work-row > span:nth-of-type(2) {
    display: none;
  }
}
</style>
