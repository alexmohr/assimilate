<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { formatDuration } from '../utils/format'

const props = defineProps<{
  /** Most recent completed backup for this agent, across its schedules. */
  lastBackupAt: string | null
  /** Shortest cadence among this agent's enabled backup schedules, in seconds. */
  cadenceSecs: number | null
}>()

// Split rather than a single 'unknown' bucket: an agent can have a real
// cadence configured but no completed backup yet, which is a materially
// different situation from having no enabled schedule at all - collapsing
// them produced a "No cadence" badge on agents that do have one.
type CoverageHealth = 'no-cadence' | 'no-data' | 'ok' | 'warning' | 'critical'

const elapsedSecs = computed(() => {
  if (!props.lastBackupAt) return null
  return Math.max(0, (Date.now() - new Date(props.lastBackupAt).getTime()) / 1000)
})

const health = computed<CoverageHealth>(() => {
  if (props.cadenceSecs === null) return 'no-cadence'
  if (elapsedSecs.value === null) return 'no-data'
  const ratio = elapsedSecs.value / props.cadenceSecs
  if (ratio >= 2) return 'critical'
  if (ratio >= 1) return 'warning'
  return 'ok'
})

const fillPercent = computed(() => {
  if (props.cadenceSecs === null || elapsedSecs.value === null) return 0
  return Math.min(100, (elapsedSecs.value / props.cadenceSecs) * 100)
})

const usageLabel = computed(() => {
  // With a cadence but nothing run yet, repeating the status verbatim wasted
  // the line - name the cadence the bar will fill over instead.
  if (elapsedSecs.value === null) {
    if (props.cadenceSecs === null) return 'No backups yet'
    return `Awaiting first backup, ${formatDuration(props.cadenceSecs)} cadence`
  }
  if (props.cadenceSecs === null) {
    return `${formatDuration(Math.round(elapsedSecs.value))} since last backup`
  }
  return `${formatDuration(Math.round(elapsedSecs.value))} of ${formatDuration(props.cadenceSecs)} cadence`
})

const STATUS_LABELS: Record<CoverageHealth, string> = {
  'no-cadence': 'No cadence',
  'no-data': 'No backups yet',
  ok: 'On time',
  warning: 'Due soon',
  critical: 'Overdue',
}

const statusLabel = computed(() => STATUS_LABELS[health.value])

// The bar on its own is ambiguous - a full green bar reads like "complete"
// when it actually means "a backup is about to fall due". The caption names
// what the fill measures, and this sentence (tooltip plus the progress bar's
// accessible value) spells out the scale it is measured against.
const meterExplanation = computed(() => {
  const elapsed = elapsedSecs.value
  if (props.cadenceSecs === null) {
    const since =
      elapsed === null
        ? 'No completed backup yet.'
        : `${formatDuration(Math.round(elapsed))} since the last backup.`
    return `${since} This agent has no enabled backup schedule, so there is no cadence to measure against.`
  }
  const cadence = formatDuration(props.cadenceSecs)
  if (elapsed === null) {
    return `No completed backup yet. The bar fills over ${cadence}, the shortest cadence among this agent's enabled schedules.`
  }
  return `${formatDuration(Math.round(elapsed))} since the last backup, ${Math.round(fillPercent.value)}% of the ${cadence} cadence. The bar fills as the next backup falls due, turning amber at the cadence and red at twice it.`
})
</script>

<template>
  <div class="coverage-meter">
    <div class="coverage-row">
      <span class="group-label">Time since last backup</span>
      <span
        class="coverage-status"
        :class="`coverage-status-${health}`"
      >
        {{ statusLabel }}
      </span>
    </div>
    <div
      class="coverage-track"
      :class="{ 'coverage-track-unknown': health === 'no-cadence' || health === 'no-data' }"
      role="progressbar"
      aria-label="Time since last backup, against the backup cadence"
      aria-valuemin="0"
      aria-valuemax="100"
      :aria-valuenow="Math.round(fillPercent)"
      :aria-valuetext="meterExplanation"
      :title="meterExplanation"
    >
      <div
        class="coverage-fill"
        :class="`coverage-fill-${health}`"
        :style="{ width: `${fillPercent}%` }"
      ></div>
    </div>
    <div class="coverage-row">
      <span class="coverage-usage">{{ usageLabel }}</span>
    </div>
  </div>
</template>

<style scoped>
.coverage-meter {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.coverage-track {
  position: relative;
  height: 6px;
  border-radius: var(--radius-pill);
  background: var(--border);
  overflow: hidden;
}

.coverage-track-unknown {
  opacity: 0.5;
}

.coverage-fill {
  height: 100%;
  border-radius: var(--radius-pill);
  transition: width var(--duration-slow) ease;
}

.coverage-fill-ok {
  background: var(--success);
}

.coverage-fill-warning {
  background: var(--warning);
}

.coverage-fill-critical {
  background: var(--danger);
}

.coverage-fill-no-cadence,
.coverage-fill-no-data {
  background: var(--text-muted);
}

.coverage-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-4);
  font-size: var(--fs-2xs);
}

.coverage-usage {
  font-family: var(--mono);
  color: var(--text-secondary);
}

.coverage-status {
  font-weight: 600;
  white-space: nowrap;
}

.coverage-status-ok {
  color: var(--success);
}

.coverage-status-warning {
  color: var(--warning);
}

.coverage-status-critical {
  color: var(--danger);
}

.coverage-status-no-cadence,
.coverage-status-no-data {
  color: var(--text-muted);
}
</style>
