<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { relativeTime } from '../utils/format'
import { normalizeBackupStatus, type NormalizedBackupStatus } from '../utils/backupStatus'
import type { ReportRow } from '../types/report'

/**
 * The outcome of an agent's last N runs, drawn one cell per run.
 *
 * Deliberately a run count rather than a span of days. A percentage over a
 * fixed window means something different at every cadence - one missed run is
 * 25% of a weekly schedule's month and 0.1% of an hourly one's - and it
 * cannot show whether failures were a single incident or a standing pattern,
 * which is the distinction this tile exists to make. Position carries that;
 * a percentage cannot.
 *
 * The count matches the dashboard's own run-count sampling convention (see
 * `AVG_DURATION_SAMPLE_WINDOW` in DashboardView.vue). It is kept separate
 * from that constant on purpose: averaging durations and drawing outcomes are
 * unrelated decisions that should be free to diverge.
 */
const RUN_STRIP_WINDOW = 20

const props = withDefaults(
  defineProps<{
    /** Newest first, as every reports endpoint returns them. */
    reports: readonly ReportRow[]
    limit?: number
  }>(),
  { limit: RUN_STRIP_WINDOW },
)

interface Cell {
  id: number
  status: NormalizedBackupStatus
  title: string
}

/**
 * Only settled runs count. A pending or started run has no outcome yet, and
 * drawing it as anything would make the strip's newest cell flicker between
 * states while a backup is in flight.
 */
const settled = computed(() =>
  props.reports.filter((r) => {
    const status = normalizeBackupStatus(r.status)
    return status !== 'pending' && status !== 'started'
  }),
)

/** Oldest on the left, so the strip reads left-to-right like a timeline. */
const cells = computed<Cell[]>(() =>
  settled.value
    .slice(0, props.limit)
    .map((r) => {
      const status = normalizeBackupStatus(r.status)
      return {
        id: r.id,
        status,
        title: `${r.repo_name ?? 'backup'} - ${status} - ${relativeTime(r.finished_at)}`,
      }
    })
    .reverse(),
)

const failedCount = computed(() => cells.value.filter((c) => c.status === 'failed').length)
const warningCount = computed(() => cells.value.filter((c) => c.status === 'warning').length)

/**
 * The real time span the cells cover. A run count is cadence-independent by
 * construction, which is the point - but it means the window is 20 hours on
 * an hourly schedule and five months on a weekly one, so the tile has to say
 * which it is rather than let the reader assume.
 */
const spanLabel = computed<string | null>(() => {
  const oldest = cells.value[0]
  if (!oldest) return null
  const report = settled.value.find((r) => r.id === oldest.id)
  if (!report) return null
  return relativeTime(report.finished_at)
})

const headline = computed(() => {
  if (cells.value.length === 0) return 'No runs yet'
  if (failedCount.value > 0) return `${failedCount.value} failed`
  if (warningCount.value > 0) return `${warningCount.value} with warnings`
  return `All ${cells.value.length} clean`
})

/**
 * Contiguous failures at the newest end are one incident that is still
 * going; contiguous failures anywhere else are one that has since recovered.
 * Either way it is a different situation from the same count scattered
 * across the window, so it earns a different chip.
 */
const isSingleIncident = computed(() => {
  if (failedCount.value < 2) return false
  const first = cells.value.findIndex((c) => c.status === 'failed')
  let last = first
  for (let i = cells.value.length - 1; i > first; i--) {
    if (cells.value[i]?.status === 'failed') {
      last = i
      break
    }
  }
  return last - first + 1 === failedCount.value
})
</script>

<template>
  <div class="run-strip">
    <div class="run-strip-head">
      <span
        class="stat-value stat-value--lg"
        :class="{ 'run-strip-value--bad': failedCount > 0 }"
        >{{ headline }}</span
      >
      <span
        v-if="isSingleIncident"
        class="badge badge--danger"
        >Incident</span
      >
    </div>
    <div
      v-if="cells.length > 0"
      class="run-strip-cells"
      role="img"
      :aria-label="`Last ${cells.length} runs, oldest first: ${headline}`"
    >
      <i
        v-for="cell in cells"
        :key="cell.id"
        class="run-cell"
        :class="`run-cell--${cell.status}`"
        :title="cell.title"
      />
    </div>
    <span
      v-if="spanLabel"
      class="run-strip-span"
      >{{ cells.length }} runs back to {{ spanLabel }}</span
    >
  </div>
</template>

<style scoped>
.run-strip {
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
}

.run-strip-head {
  display: flex;
  align-items: center;
  gap: 0.4rem;
}

.run-strip-value--bad {
  color: var(--danger);
}

.run-strip-cells {
  display: flex;
  gap: 2px;
  margin-top: 0.15rem;
}

.run-cell {
  flex: 1;
  min-width: 2px;
  height: 14px;
  border-radius: var(--radius-sm);
  background: var(--success);
}

.run-cell--warning {
  background: var(--warning);
}

.run-cell--failed {
  background: var(--danger);
}

.run-cell--cancelled {
  background: var(--text-muted);
}

.run-strip-span {
  font-size: var(--fs-2xs);
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}
</style>
