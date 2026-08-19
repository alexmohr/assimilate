<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { normalizeBackupStatus } from '../utils/backupStatus'
import { formatDateShort, formatDuration } from '../utils/format'

export interface RunHistoryEntry {
  id: number | string
  startedAt: string
  durationSecs: number
  status: string
}

type RunTone = 'success' | 'warning' | 'danger' | 'accent' | 'neutral'

const props = withDefaults(
  defineProps<{
    runs: RunHistoryEntry[]
    /** How many of the most recent runs to draw. */
    maxBars?: number
  }>(),
  { maxBars: 10 },
)

const MIN_BAR_HEIGHT_PERCENT = 25

const visible = computed(() =>
  [...props.runs].sort((a, b) => a.startedAt.localeCompare(b.startedAt)).slice(-props.maxBars),
)

function tone(run: RunHistoryEntry): RunTone {
  const status = normalizeBackupStatus(run.status)
  if (status === 'success') return 'success'
  if (status === 'warning') return 'warning'
  if (status === 'started' || status === 'pending') return 'accent'
  if (status === 'cancelled') return 'neutral'
  return 'danger'
}

// Height encodes duration for a completed run, scaled against the longest
// completed run in the strip. A failed run is drawn at full height instead
// of its own (usually short) elapsed time - a duration-proportional bar
// would draw the most important event in the row as the shortest bar.
const maxCompletedDuration = computed(() => {
  const durations = visible.value
    .filter((r) => tone(r) === 'success' || tone(r) === 'warning')
    .map((r) => r.durationSecs)
  return Math.max(1, ...durations)
})

function heightPercent(run: RunHistoryEntry): number {
  if (tone(run) === 'danger') return 100
  const raw = (run.durationSecs / maxCompletedDuration.value) * 100
  return Math.min(100, Math.max(MIN_BAR_HEIGHT_PERCENT, raw))
}

const TONE_LABELS: Record<RunTone, string> = {
  success: 'Succeeded',
  warning: 'Warning',
  accent: 'Running',
  danger: 'Failed',
  neutral: 'Cancelled',
}

function barTitle(run: RunHistoryEntry): string {
  return `${formatDateShort(run.startedAt)} · ${TONE_LABELS[tone(run)]} · ${formatDuration(run.durationSecs)}`
}

const caption = computed(() => {
  const count = visible.value.length
  if (count === 0) return 'No runs yet'
  const plural = count === 1 ? '' : 's'

  const failedCount = visible.value.filter((r) => tone(r) === 'danger').length
  if (failedCount > 0) {
    return `${count} run${plural} · ${failedCount} failed`
  }

  // Only completed runs have a meaningful duration - an in-progress run
  // (accent tone) still carries durationSecs: 0, which would otherwise pull
  // the low end of the range down to 0s while it's still running.
  const durations = visible.value
    .filter((r) => tone(r) === 'success' || tone(r) === 'warning')
    .map((r) => r.durationSecs)
  if (durations.length === 0) return `${count} run${plural}`
  const min = Math.min(...durations)
  const max = Math.max(...durations)
  const range = min === max ? formatDuration(min) : `${formatDuration(min)}-${formatDuration(max)}`
  return `${count} run${plural} · ${range}`
})
</script>

<template>
  <div
    class="run-history"
    role="img"
    :aria-label="caption"
  >
    <div class="run-history-bars">
      <template v-if="visible.length === 0">
        <span
          v-for="i in maxBars"
          :key="i"
          class="run-bar run-bar-empty"
        ></span>
      </template>
      <span
        v-for="run in visible"
        :key="run.id"
        class="run-bar"
        :class="`run-bar-${tone(run)}`"
        :style="{ height: `${heightPercent(run)}%` }"
        :title="barTitle(run)"
        :data-run-id="run.id"
      ></span>
    </div>
    <span class="run-history-caption">{{ caption }}</span>
  </div>
</template>

<style scoped>
.run-history {
  display: flex;
  align-items: flex-end;
  gap: 0.5rem;
  height: 26px;
}

.run-history-bars {
  display: flex;
  align-items: flex-end;
  gap: 3px;
  height: 100%;
  flex-shrink: 0;
}

.run-bar {
  width: 7px;
  border-radius: var(--radius-sm);
  flex-shrink: 0;
}

.run-bar-success {
  background: var(--success);
  opacity: 0.55;
}

.run-bar-success:last-child {
  opacity: 1;
}

.run-bar-warning {
  background: var(--warning);
}

.run-bar-danger {
  background: var(--danger);
}

.run-bar-accent {
  background: var(--accent);
}

.run-bar-neutral {
  background: var(--text-muted);
}

.run-bar-empty {
  height: 6px;
  background: var(--border);
}

.run-history-caption {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  white-space: nowrap;
  margin-left: auto;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
