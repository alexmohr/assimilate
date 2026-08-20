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
    /** Marks a run currently in flight. */
    running?: boolean
    /**
     * Pushes the first action to the opposite end of the footer, for a card
     * whose actions row leads with a control rather than a button.
     */
    spreadActions?: boolean
  }>(),
  { highlighted: false, running: false, spreadActions: false },
)

defineEmits<{ select: [] }>()

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
</script>

<template>
  <div
    class="entity-card"
    :class="{
      'entity-card--notable': !schedule.enabled,
      'entity-card--highlighted': highlighted,
    }"
    @click="$emit('select')"
  >
    <span class="card-name">
      <slot name="title">{{ schedule.name || `Schedule #${schedule.id}` }}</slot>
    </span>
    <EntityStatusBadges
      :notable="!schedule.enabled"
      notable-label="Disabled"
      :running="running"
      running-label="Running"
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
      :class="{ 'card-actions--spread': spreadActions }"
      @click.stop
    >
      <slot name="actions" />
    </div>
  </div>
</template>
