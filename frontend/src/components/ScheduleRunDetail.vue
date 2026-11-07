<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { formatDateShort } from '../utils/format'
import { backupStatusBadgeClass } from '../utils/badge'
import { RUN_OUTCOME_LABELS, reportToOpen, type ScheduleRun } from '../utils/scheduleRuns'
import ScheduleRunReportRow from './ScheduleRunReportRow.vue'
import type { ReportRow } from '../types/report'

/**
 * The run picked on the Recent runs strip: when it ran, how it ended, and one
 * row per host (and per repository, on a schedule that writes to several)
 * saying what that part of it did.
 */
const props = defineProps<{
  run: ScheduleRun
  hostLabel: (report: ReportRow) => string
  /** The repository a report wrote into; null on a single-repository schedule. */
  repoLabel: (report: ReportRow) => string | null
}>()

const emit = defineEmits<{ openReportDetail: [report: ReportRow] }>()

const statusLabel = computed(() => RUN_OUTCOME_LABELS[props.run.status])

/** The report the Logs tab opens on: the first that did not succeed. */
const target = computed(() => reportToOpen(props.run))
</script>

<template>
  <div
    class="run-detail"
    aria-live="polite"
  >
    <div class="run-detail-head">
      <span class="run-detail-title">{{ formatDateShort(run.finishedAt) }}</span>
      <span
        class="badge"
        :class="backupStatusBadgeClass(run.status)"
      >
        <span class="badge-dot" />
        {{ statusLabel }}
      </span>
      <button
        v-if="target"
        class="btn btn-sm btn-ghost run-detail-open"
        type="button"
        title="Open this run in the Activity log"
        @click="emit('openReportDetail', target)"
      >
        Open in Activity log
      </button>
    </div>
    <div class="rows">
      <ScheduleRunReportRow
        v-for="report in run.reports"
        :key="report.id"
        :report="report"
        :host-label="hostLabel(report)"
        :repo-label="repoLabel(report)"
      />
    </div>
  </div>
</template>

<style scoped>
/* Already inside the overview's card: rows split by hairlines, no second frame. */
.run-detail > .rows {
  border: none;
  border-radius: 0;
  background: none;
}

.run-detail {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}

.run-detail-head {
  padding-inline: var(--space-6);
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-4);
}

.run-detail-title {
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

.run-detail-open {
  margin-left: auto;
}
</style>
