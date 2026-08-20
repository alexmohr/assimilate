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
  if (elapsedSecs.value === null) return 'No backups yet'
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
</script>

<template>
  <div class="coverage-meter">
    <div
      class="coverage-track"
      :class="{ 'coverage-track-unknown': health === 'no-cadence' || health === 'no-data' }"
    >
      <div
        class="coverage-fill"
        :class="`coverage-fill-${health}`"
        :style="{ width: `${fillPercent}%` }"
      ></div>
    </div>
    <div class="coverage-row">
      <span class="coverage-usage">{{ usageLabel }}</span>
      <span
        class="coverage-status"
        :class="`coverage-status-${health}`"
      >
        {{ statusLabel }}
      </span>
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
