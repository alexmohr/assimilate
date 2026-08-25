<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { useRouter } from 'vue-router'
import { formatDateShort, relativeTime } from '../utils/format'
import {
  scheduleIssuesFromEntries,
  scheduleRunStatus,
  withErrorTitles,
  type ScheduleHealthEntry,
} from '../utils/scheduleHealth'
import { scheduleDisabledLabel } from '../utils/scheduleStatus'
import EntityStatusBadges from './EntityStatusBadges.vue'
import type { ScheduleRow } from '../types/schedule'

/**
 * One schedule, as a single line. Shared by the agent's Schedules tab and the
 * Overview preview so both read with the same grammar: a status stripe, a
 * name, when it runs, stats pushed right, actions last. The backup row beside
 * it follows the same shape.
 */
const props = defineProps<{
  schedule: ScheduleRow
  repoName: string
  health: readonly ScheduleHealthEntry[]
  highlighted?: boolean
  running?: boolean
  /** Hidden in the Overview preview, where the row is a link, not a console. */
  showActions?: boolean
}>()

const emit = defineEmits<{ open: []; run: [] }>()

const router = useRouter()

/**
 * The same chips the schedule cards carried, from the same helper: an Overdue
 * or Failed chip is a link into the filtered activity log, not decoration.
 * Rendering a plain badge here dropped that, which is what
 * hosts.spec.ts's "Failed chip navigates to the filtered activity log" is for.
 */
const issues = computed(() =>
  withErrorTitles(scheduleIssuesFromEntries([...props.health], props.schedule.id, router), [
    ...props.health,
  ]),
)

const isOverdue = computed(() => props.health.some((h) => h.is_overdue))
const hasFailed = computed(() => props.health.some((h) => scheduleRunStatus(h) === 'failed'))
const hasWarning = computed(() => props.health.some((h) => scheduleRunStatus(h) === 'warning'))

const stripe = computed(() => {
  if (hasFailed.value) return 'danger'
  if (isOverdue.value || hasWarning.value) return 'warning'
  if (!props.schedule.enabled) return 'muted'
  return 'success'
})

const lastRun = computed(() => {
  const at = props.schedule.last_run_at
  return at ? relativeTime(at) : 'never run'
})
</script>

<template>
  <div
    class="agent-row"
    :class="{ 'agent-row--highlighted': highlighted }"
  >
    <i
      class="agent-row-stripe"
      :class="`agent-row-stripe--${stripe}`"
      aria-hidden="true"
    />
    <button
      class="agent-row-name mono"
      type="button"
      @click="emit('open')"
    >
      {{ schedule.name || repoName }}
    </button>
    <span class="agent-row-when">{{ schedule.cron_expression }}</span>
    <span class="agent-row-sub">{{ repoName }}</span>
    <EntityStatusBadges
      :notable="!schedule.enabled"
      :notable-label="scheduleDisabledLabel(schedule)"
      :issues="issues"
    />
    <span class="agent-row-stats">
      <span>last {{ lastRun }}</span>
      <span v-if="schedule.next_run_at && schedule.enabled">
        next {{ formatDateShort(schedule.next_run_at) }}
      </span>
    </span>
    <div
      v-if="showActions"
      class="agent-row-actions"
    >
      <button
        class="btn btn-sm"
        type="button"
        :disabled="running || !schedule.enabled"
        @click="emit('run')"
      >
        {{ running ? 'Starting...' : 'Run now' }}
      </button>
      <button
        class="btn btn-sm btn-ghost"
        type="button"
        @click="emit('open')"
      >
        Open
      </button>
    </div>
  </div>
</template>
