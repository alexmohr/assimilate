<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref } from 'vue'
import { formatBytes, formatDuration } from '../utils/format'
import { backupStatusBadgeClass } from '../utils/badge'
import { runOutcome, type RunOutcome } from '../utils/scheduleRuns'
import { useRunEvents } from '../composables/useRunEvents'
import RunReportDetail from './RunReportDetail.vue'
import type { ReportRow } from '../types/report'

/**
 * One host's part in a schedule run, as a line in the Overview's run detail:
 * what happened, a short note on why, how much it wrote and how long it took.
 * The full output - warnings, the error or skip reason, the power-management
 * timeline - stays collapsed behind "Show detail", as it does on the host's
 * own rows.
 */
const props = defineProps<{
  report: ReportRow
  hostLabel: string
  /** The repository it wrote into, named only on a multi-repository schedule. */
  repoLabel: string | null
}>()

const NO_VALUE = '–'

const expanded = ref(false)
const status = computed(() => runOutcome(props.report.status))
const warnings = computed(() => props.report.warnings ?? [])

const STRIPES: Record<RunOutcome, 'success' | 'warning' | 'danger' | 'muted'> = {
  success: 'success',
  warning: 'warning',
  skipped: 'warning',
  failed: 'danger',
  cancelled: 'muted',
}

const stripe = computed(() => STRIPES[status.value])

/**
 * The line beside the name. A skipped or failed run gets the first line of its
 * reason, which is usually enough to know whether it is worth opening; the
 * whole message is behind the toggle.
 */
const note = computed<string | null>(() => {
  if (status.value === 'warning') {
    const count = warnings.value.length
    if (count > 0) return count === 1 ? '1 warning' : `${count} warnings`
  }
  if ((status.value === 'failed' || status.value === 'skipped') && props.report.error_message) {
    return props.report.error_message.split('\n')[0] ?? null
  }
  return null
})

/** A skipped or failed run wrote no archive, so it has no size to show. */
const sizeText = computed(() =>
  status.value === 'success' || status.value === 'warning'
    ? formatBytes(props.report.original_size)
    : NO_VALUE,
)

const durationText = computed(() =>
  props.report.duration_secs > 0 ? formatDuration(props.report.duration_secs) : NO_VALUE,
)

/**
 * Something to expand into: output to read, or a power-management timeline,
 * which only a run id can say whether there is.
 */
const hasDetail = computed(
  () =>
    warnings.value.length > 0 ||
    (props.report.error_message !== null && status.value !== 'success') ||
    Boolean(props.report.run_id),
)

const { runEvents, loadingEvents, eventsFetched, eventsError } = useRunEvents(
  () => props.report,
  () => expanded.value,
)
</script>

<template>
  <div class="agent-row">
    <i
      class="agent-row-stripe"
      :class="`agent-row-stripe--${stripe}`"
      aria-hidden="true"
    />
    <div class="agent-row-line">
      <span class="agent-row-name mono">{{ hostLabel }}</span>
      <span
        v-if="repoLabel"
        class="meta-pill"
        >{{ repoLabel }}</span
      >
      <span
        v-if="status !== 'success'"
        class="badge"
        :class="backupStatusBadgeClass(report.status)"
        >{{ status }}</span
      >
      <span
        v-if="note"
        class="agent-row-sub run-report-note"
        :title="note"
        >{{ note }}</span
      >
    </div>
    <div class="agent-row-line">
      <span class="agent-row-stats">
        <span>{{ sizeText }}</span>
        <span>{{ durationText }}</span>
      </span>
      <div
        v-if="hasDetail"
        class="agent-row-actions"
      >
        <button
          class="btn btn-sm btn-ghost"
          type="button"
          :aria-expanded="expanded"
          @click="expanded = !expanded"
        >
          {{ expanded ? 'Hide detail' : 'Show detail' }}
        </button>
      </div>
    </div>
  </div>
  <RunReportDetail
    v-if="expanded"
    :report="report"
    :run-events="runEvents"
    :loading-events="loadingEvents"
    :events-fetched="eventsFetched"
    :events-error="eventsError"
    flush
  />
</template>

<style scoped>
/* One line, however long the reason: the whole message is a click away. */
.run-report-note {
  min-width: 0;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
