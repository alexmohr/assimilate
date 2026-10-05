<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { formatBytes, formatDuration, relativeTime } from '../utils/format'
import { normalizeBackupStatus, reportMessageLabel } from '../utils/backupStatus'
import { backupStatusBadgeClass } from '../utils/badge'
import { useRunEvents } from '../composables/useRunEvents'
import RunReportDetail from './RunReportDetail.vue'
import type { ReportRow } from '../types/report'

/**
 * One backup run, as a single line, following the same grammar as
 * AgentScheduleRow. Previously each run rendered as a four-line card, so a
 * month of history did not fit on a screen; one line per run does.
 *
 * A run that produced an archive links to it; a warned or failed run also
 * expands its output in place, which is what you came to the page to read.
 * In a preview (`showDetail` off) there is nothing to expand into, so the
 * same run offers a jump to the full row instead.
 */
const props = defineProps<{
  report: ReportRow
  expanded?: boolean
  highlighted?: boolean
  /** Hidden in the Overview preview, which is a summary, not a log reader. */
  showDetail?: boolean
}>()

const emit = defineEmits<{ open: []; toggle: []; detail: [] }>()

const status = computed(() => normalizeBackupStatus(props.report.status))
const isSuccess = computed(() => status.value === 'success')

const stripe = computed(() => {
  if (status.value === 'success') return 'success'
  if (status.value === 'warning') return 'warning'
  if (status.value === 'failed') return 'danger'
  return 'muted'
})

const warnings = computed(() => props.report.warnings ?? [])

/**
 * Warnings are shown for a warned run and errors for a failed one. A warned
 * run can also carry an `error_message` describing the warning it was
 * downgraded from, which would otherwise be rendered twice. A run tied to a
 * power-management timeline is always worth offering to expand, even before
 * its events have loaded - `run_id` alone doesn't say whether any were
 * actually recorded (most runs have wake/start disabled and record none).
 */
const hasDetail = computed(
  () =>
    warnings.value.length > 0 ||
    (props.report.error_message !== null && !isSuccess.value) ||
    props.report.run_id !== null,
)

/**
 * Something to *read*, as opposed to `hasDetail`, which also counts a run
 * whose only detail is a power-management timeline. The preview rows offer a
 * jump straight to it; a row that would expand into "no power-management
 * activity for this run" is not worth a button there. Null when there is
 * nothing to read, so the label is the predicate too.
 */
const messageLabel = computed(() => reportMessageLabel(props.report))

// Fetched on first expand and kept live while open; see `useRunEvents`.
const { runEvents, loadingEvents, eventsFetched, eventsError } = useRunEvents(
  () => props.report,
  () => props.expanded === true,
)
</script>

<template>
  <div
    :id="`report-${report.id}`"
    class="agent-row"
    :class="{ 'agent-row--highlighted': highlighted }"
  >
    <i
      class="agent-row-stripe"
      :class="`agent-row-stripe--${stripe}`"
      aria-hidden="true"
    />
    <div class="agent-row-line">
      <span class="agent-row-when">{{ relativeTime(report.finished_at) }}</span>
      <button
        v-if="report.archive_name"
        class="agent-row-name mono"
        type="button"
        title="Browse this archive"
        @click="emit('open')"
      >
        {{ report.repo_name }}
      </button>
      <span
        v-else
        class="agent-row-name mono"
        >{{ report.repo_name }}</span
      >
      <span
        v-if="!isSuccess"
        class="badge"
        :class="backupStatusBadgeClass(report.status)"
        >{{ status }}</span
      >
    </div>
    <!--
      Named only when it differs from the repository, which is the case a bare
      repo name cannot disambiguate: several schedules can write to one repo,
      and tracing a failure means knowing which one produced it.
    -->
    <div
      v-if="
        (report.schedule_id && report.schedule_name && report.schedule_name !== report.repo_name) ||
        report.archive_name
      "
      class="agent-row-line"
    >
      <RouterLink
        v-if="
          report.schedule_id && report.schedule_name && report.schedule_name !== report.repo_name
        "
        class="agent-row-sub row-schedule-link"
        :to="`/schedules/${report.schedule_id}`"
      >
        {{ report.schedule_name }}
      </RouterLink>
      <span
        v-if="report.archive_name"
        class="agent-row-sub mono"
        >{{ report.archive_name }}</span
      >
    </div>
    <div class="agent-row-line">
      <span class="agent-row-stats">
        <template v-if="isSuccess || status === 'warning'">
          <span>{{ formatBytes(report.original_size) }}</span>
          <span>{{ formatBytes(report.deduplicated_size) }} dedup</span>
          <span>{{ report.files_processed }} files</span>
        </template>
        <span>{{ formatDuration(report.duration_secs) }}</span>
      </span>
      <!--
        A preview row cannot expand in place - it has no detail block - so what
        it offers instead is the trip to the row that does, on the Backups tab.
        Without it a failed run in a preview is a dead end: the badge says it
        broke and nothing on the row says why.
      -->
      <button
        v-if="!showDetail && messageLabel"
        class="btn btn-sm btn-ghost"
        type="button"
        title="Open this run on the Backups tab"
        @click="emit('detail')"
      >
        {{ messageLabel }}
      </button>
      <button
        v-if="showDetail && hasDetail"
        class="btn btn-sm btn-ghost"
        type="button"
        :aria-expanded="expanded"
        @click="emit('toggle')"
      >
        {{ expanded ? 'Hide detail' : 'Show detail' }}
      </button>
    </div>
  </div>
  <RunReportDetail
    v-if="expanded && hasDetail"
    :report="report"
    :run-events="runEvents"
    :loading-events="loadingEvents"
    :events-fetched="eventsFetched"
    :events-error="eventsError"
  />
</template>

<style scoped>
.row-schedule-link {
  color: var(--text-muted);
}

.row-schedule-link:hover {
  color: var(--accent);
  text-decoration: underline;
}
</style>
