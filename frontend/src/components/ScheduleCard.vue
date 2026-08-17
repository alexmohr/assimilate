<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { cronToHuman } from '../utils/cron'
import EntityStatusBadges, { type EntityIssue } from './EntityStatusBadges.vue'
import type { ScheduleRow, ScheduleType } from '../types/schedule'

withDefaults(
  defineProps<{
    schedule: ScheduleRow
    issues: EntityIssue[]
    /** Formats next/last run. The two detail views use different precision. */
    formatRun: (value: string | null) => string
    /** Draws attention to a schedule the caller has flagged, e.g. overdue. */
    highlighted?: boolean
  }>(),
  { highlighted: false },
)

defineEmits<{ select: [] }>()

function scheduleTypeLabel(t: ScheduleType): string {
  switch (t) {
    case 'backup':
      return 'Backup'
    case 'check':
      return 'Integrity Check'
    case 'verify':
      return 'Verify (extract dry-run)'
  }
}
</script>

<template>
  <div
    class="schedule-card"
    :class="{
      'schedule-card-notable': !schedule.enabled,
      'schedule-card-highlighted': highlighted,
    }"
    @click="$emit('select')"
  >
    <span class="card-hostname">
      <slot name="title">{{ schedule.name || `Schedule #${schedule.id}` }}</slot>
    </span>
    <EntityStatusBadges
      :notable="!schedule.enabled"
      notable-label="Disabled"
      :issues="issues"
    />
    <div class="card-meta">
      <slot name="meta" />
      <span
        class="badge badge--neutral"
        :class="`type-${schedule.schedule_type ?? 'backup'}`"
      >
        {{ scheduleTypeLabel(schedule.schedule_type ?? 'backup') }}
      </span>
    </div>
    <div class="card-stats">
      <div class="stat">
        <span class="stat-value">
          {{ cronToHuman(schedule.cron_expression) ?? schedule.cron_expression }}
        </span>
        <span class="stat-label">Schedule</span>
      </div>
      <div class="stat">
        <span class="stat-value">{{ formatRun(schedule.next_run_at) }}</span>
        <span class="stat-label">Next run</span>
      </div>
      <div class="stat">
        <span class="stat-value">{{ formatRun(schedule.last_run_at) }}</span>
        <span class="stat-label">Last run</span>
      </div>
    </div>
    <div
      v-if="$slots.actions"
      class="card-actions"
      @click.stop
    >
      <slot name="actions" />
    </div>
  </div>
</template>

<style scoped>
.schedule-card {
  background: var(--bg-card);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 1.25rem;
  cursor: pointer;
  transition:
    box-shadow var(--duration-base),
    border-color var(--duration-base);
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.schedule-card:hover {
  border-color: var(--accent);
  box-shadow: var(--shadow);
}

.schedule-card-notable {
  background: var(--bg-hover);
}

.schedule-card-highlighted {
  border-color: var(--warning);
}

.card-hostname {
  font-weight: 600;
  font-family: var(--mono);
  font-size: var(--fs-md);
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.card-meta {
  display: flex;
  gap: 0.4rem;
  flex-wrap: wrap;
}

.card-stats {
  display: flex;
  gap: 1.25rem;
}

.stat {
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
}

.card-actions {
  display: flex;
  justify-content: flex-end;
  gap: 0.25rem;
  margin-top: auto;
}

.type-backup {
  background: var(--success-subtle);
  color: var(--success);
}

.type-check {
  background: var(--accent-subtle);
  color: var(--accent);
}

.type-verify {
  background: var(--warning-subtle);
  color: var(--warning);
}
</style>
