<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'
import { Play } from '@lucide/vue'
import { cronToHuman } from '../utils/cron'
import { formatDateShort, relativeTime } from '../utils/format'
import {
  latestCompletedBackupAt,
  navigateToScheduleIssue,
  scheduleRunStatus,
  type ScheduleHealthEntry,
} from '../utils/scheduleHealth'
import type { ScheduleRow } from '../types/schedule'

/**
 * Schedules as one table: a status dot, the name over its
 * cadence and repository, the last run and the next one. A failing schedule
 * shows its last error on the row itself and links to the Activity log
 * filtered to it. Tapping the row opens the schedule; the play button is the
 * only other action.
 */
const props = defineProps<{
  schedules: readonly ScheduleRow[]
  health: readonly ScheduleHealthEntry[]
  /** The line under a schedule's name after its cadence: its repository, or its agents. */
  subtitleFor: (schedule: ScheduleRow) => string
  /** Rings an overdue schedule, for a page opened from an Overdue link. */
  highlightOverdue?: boolean
  /** The schedule whose Run now is in flight. */
  runningId: number | null
}>()

const emit = defineEmits<{ open: [schedule: ScheduleRow]; run: [schedule: ScheduleRow] }>()

const router = useRouter()

type Tone = 'danger' | 'warning' | 'success' | 'idle'
type Filter = 'all' | 'failing' | 'healthy'

interface Row {
  schedule: ScheduleRow
  tone: Tone
  subtitle: string
  cadence: string
  lastAt: string | null
  lastLabel: string
  error: { kind: 'failed' | 'warning'; message: string | null } | null
  overdue: boolean
}

function healthFor(schedule: ScheduleRow): ScheduleHealthEntry[] {
  return props.health.filter((h) => h.schedule_id === schedule.id)
}

const rows = computed<Row[]>(() =>
  props.schedules.map((schedule) => {
    const entries = healthFor(schedule)
    const failed = entries.find((h) => scheduleRunStatus(h) === 'failed')
    const warned = entries.find((h) => scheduleRunStatus(h) === 'warning')
    const overdue = entries.some((h) => h.is_overdue)
    const lastAt = latestCompletedBackupAt(entries)
    let tone: Tone = 'success'
    if (failed) tone = 'danger'
    else if (overdue || warned) tone = 'warning'
    else if (!lastAt || !schedule.enabled) tone = 'idle'
    let lastLabel = 'Never run'
    if (failed) lastLabel = 'Failed'
    else if (warned) lastLabel = 'Warning'
    else if (lastAt) lastLabel = 'Succeeded'
    return {
      schedule,
      tone,
      subtitle: props.subtitleFor(schedule),
      cadence: cronToHuman(schedule.cron_expression) || schedule.cron_expression,
      lastAt,
      lastLabel,
      error: failed
        ? { kind: 'failed', message: failed.last_error_message }
        : warned
          ? { kind: 'warning', message: warned.last_error_message }
          : null,
      overdue,
    }
  }),
)

const filter = ref<Filter>('all')
const counts = computed(() => ({
  all: rows.value.length,
  failing: rows.value.filter((r) => r.tone === 'danger' || r.tone === 'warning').length,
  healthy: rows.value.filter((r) => r.tone === 'success').length,
}))
const visible = computed(() =>
  rows.value.filter((r) => {
    if (filter.value === 'failing') return r.tone === 'danger' || r.tone === 'warning'
    if (filter.value === 'healthy') return r.tone === 'success'
    return true
  }),
)
const FILTERS: readonly { id: Filter; label: string; dot?: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'failing', label: 'Failing', dot: 'status-dot--danger' },
  { id: 'healthy', label: 'Healthy', dot: 'status-dot--success' },
]

function nextRun(schedule: ScheduleRow): string {
  return schedule.enabled && schedule.next_run_at
    ? formatDateShort(schedule.next_run_at)
    : schedule.enabled
      ? '\u2014'
      : 'Disabled'
}

function openActivity(row: Row): void {
  if (row.error) navigateToScheduleIssue(router, row.schedule.id, row.error.kind)
}
</script>

<template>
  <div class="schedule-table">
    <div class="sched-toolbar">
      <div
        class="filter-pills"
        role="group"
        aria-label="Filter schedules"
      >
        <button
          v-for="f in FILTERS"
          :key="f.id"
          type="button"
          class="filter-pill"
          :class="{ active: filter === f.id }"
          :aria-pressed="filter === f.id"
          @click="filter = f.id"
        >
          <span
            v-if="f.dot"
            class="status-dot"
            :class="f.dot"
          ></span>
          {{ f.label }} · {{ counts[f.id] }}
        </button>
      </div>
      <div class="sched-add">
        <slot name="actions" />
      </div>
    </div>

    <div class="rows">
      <div
        class="sched-row sched-cols"
        aria-hidden="true"
      >
        <span />
        <span>Schedule</span>
        <span class="sched-col-cadence">Cadence</span>
        <span class="sched-col-last">Last run</span>
        <span>Next run</span>
        <span />
      </div>
      <div
        v-for="r in visible"
        :key="r.schedule.id"
        class="sched-row"
        :class="{ 'sched-row--highlighted': highlightOverdue && r.overdue }"
        :data-schedule-id="r.schedule.id"
        role="link"
        tabindex="0"
        @click="emit('open', r.schedule)"
        @keydown.enter="emit('open', r.schedule)"
      >
        <i
          class="sched-dot"
          :class="`sched-dot--${r.tone}`"
          aria-hidden="true"
        />
        <div class="sched-name">
          <span class="sched-title">{{ r.schedule.name || r.subtitle }}</span>
          <!-- Always two lines, so every row is the same height: a failing
               schedule's error takes the place of its cadence line. -->
          <button
            v-if="r.error"
            type="button"
            class="sched-sub sched-error"
            :class="`sched-error--${r.error.kind}`"
            :title="r.error.message ?? undefined"
            @click.stop="openActivity(r)"
          >
            <span class="sched-error-label"
              >{{ r.error.kind === 'failed' ? 'Failed' : 'Warning'
              }}{{ r.lastAt ? ` ${relativeTime(r.lastAt)}` : '' }}</span
            >
            <span class="sched-error-msg">{{ r.error.message ?? 'Open in Activity log' }}</span>
          </button>
          <span
            v-else
            class="sched-sub"
          >
            <span class="sched-sub-cadence">{{ r.cadence }} · </span>{{ r.subtitle }}
          </span>
        </div>
        <span
          class="sched-col-cadence"
          :title="r.schedule.cron_expression"
          >{{ r.cadence }}</span
        >
        <div class="sched-col-last sched-two">
          <span
            :class="{
              'text-danger': r.error?.kind === 'failed',
              'text-warning': r.error?.kind === 'warning',
              muted: !r.lastAt,
            }"
            >{{ r.lastLabel }}</span
          >
          <span class="sched-sub">{{ r.lastAt ? relativeTime(r.lastAt) : '\u00a0' }}</span>
        </div>
        <span class="sched-next">{{ nextRun(r.schedule) }}</span>
        <button
          type="button"
          class="sched-run"
          :aria-label="`Run ${r.schedule.name || r.subtitle} now`"
          title="Run now"
          :disabled="runningId === r.schedule.id || !r.schedule.enabled"
          @click.stop="emit('run', r.schedule)"
          @keydown.enter.stop
        >
          <Play :size="16" />
        </button>
      </div>
      <p
        v-if="visible.length === 0"
        class="muted sched-none"
      >
        No schedules match this filter.
      </p>
    </div>
  </div>
</template>

<style scoped>
.schedule-table {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}

.sched-toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  flex-wrap: wrap;
}

.sched-add {
  margin-left: auto;
}

.sched-add :slotted(.btn) {
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
}

.sched-row {
  display: grid;
  grid-template-columns: 10px minmax(0, 1fr) auto 32px;
  column-gap: var(--space-4);
  row-gap: var(--space-2);
  align-items: center;
  /* The Overview tab's repository rows set the rhythm for every row list. */
  padding: var(--space-5) var(--space-4) var(--space-5) var(--space-6);
  border-top: 1px solid var(--border-subtle);
  font-size: var(--fs-sm);
  cursor: pointer;
}

.sched-row:hover {
  background: var(--bg-hover);
}

.sched-row--highlighted {
  box-shadow: inset 0 0 0 2px var(--warning);
}

.sched-cols {
  display: none;
  cursor: default;
  border-top: none;
  background: var(--bg-card);
  font-size: var(--fs-2xs);
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--text-muted);
  font-weight: 600;
}

.sched-cols:hover {
  background: var(--bg-card);
}

.sched-cols + .sched-row {
  border-top: none;
}

.sched-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
}

.sched-dot--success {
  background: var(--success);
}

.sched-dot--danger {
  background: var(--danger);
}

.sched-dot--warning {
  background: var(--warning);
}

.sched-dot--idle {
  border: 1.5px solid var(--text-muted);
}

.sched-name,
.sched-two {
  display: flex;
  flex-direction: column;
  min-width: 0;
  gap: var(--space-3);
}

.sched-title {
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.sched-sub {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.sched-row > .sched-col-cadence,
.sched-row > .sched-col-last {
  display: none;
}

.sched-next {
  white-space: nowrap;
  text-align: right;
  font-size: var(--fs-xs);
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}

.sched-run {
  width: 32px;
  height: 32px;
  display: grid;
  place-items: center;
  border: none;
  border-radius: var(--radius-sm);
  background: none;
  color: var(--text-muted);
  cursor: pointer;
}

.sched-run:hover:not(:disabled) {
  background: var(--bg-elevated);
  color: var(--text-primary);
}

.sched-run:disabled {
  opacity: 0.4;
  cursor: default;
}

.sched-error {
  display: flex;
  align-items: baseline;
  gap: var(--space-3);
  padding: 0;
  border: none;
  background: none;
  font: inherit;
  font-size: var(--fs-xs);
  text-align: left;
  cursor: pointer;
}

.sched-error--failed .sched-error-label {
  color: var(--danger);
}

.sched-error--warning .sched-error-label {
  color: var(--warning);
}

.sched-error-label {
  flex: none;
  font-weight: 600;
}

.sched-error-msg {
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}

.sched-none {
  padding: var(--space-5);
  margin: 0;
}

.text-danger {
  color: var(--danger);
  font-weight: 600;
}

.text-warning {
  color: var(--warning);
  font-weight: 600;
}

@media (min-width: 769px) {
  .sched-row {
    grid-template-columns:
      10px minmax(0, 2.2fr) minmax(0, 1.6fr) minmax(0, 1.3fr) minmax(0, 1fr)
      40px;
    column-gap: var(--space-6);
    padding: var(--space-5) var(--space-4) var(--space-5) var(--space-6);
  }

  .sched-cols {
    display: grid;
    padding-block: var(--space-3);
  }

  .sched-row > .sched-col-last {
    display: flex;
  }

  .sched-row > .sched-col-cadence {
    display: block;
  }

  .sched-sub-cadence {
    display: none;
  }

  .sched-next {
    text-align: left;
    font-size: var(--fs-sm);
  }
}
</style>
