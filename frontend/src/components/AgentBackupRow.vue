<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { formatBytes, formatDuration, relativeTime } from '../utils/format'
import { normalizeBackupStatus } from '../utils/backupStatus'
import { backupStatusBadgeClass } from '../utils/badge'
import type { ReportRow } from '../types/report'

/**
 * One backup run, as a single line, following the same grammar as
 * AgentScheduleRow. Previously each run rendered as a four-line card, so a
 * month of history did not fit on a screen; one line per run does.
 *
 * A successful run links to its archive; a failed or warned one expands its
 * output in place, which is what you came to the page to read.
 */
const props = defineProps<{
  report: ReportRow
  expanded?: boolean
  highlighted?: boolean
  /** Hidden in the Overview preview, which is a summary, not a log reader. */
  showDetail?: boolean
}>()

const emit = defineEmits<{ open: []; toggle: [] }>()

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
 * downgraded from, which would otherwise be rendered twice.
 */
const hasDetail = computed(
  () => warnings.value.length > 0 || (props.report.error_message !== null && !isSuccess.value),
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
    <span class="agent-row-when">{{ relativeTime(report.finished_at) }}</span>
    <button
      v-if="isSuccess"
      class="agent-row-name mono"
      type="button"
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
    <!--
      Named only when it differs from the repository, which is the case a bare
      repo name cannot disambiguate: several schedules can write to one repo,
      and tracing a failure means knowing which one produced it.
    -->
    <RouterLink
      v-if="report.schedule_id && report.schedule_name && report.schedule_name !== report.repo_name"
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
    <span class="agent-row-stats">
      <template v-if="isSuccess || status === 'warning'">
        <span>{{ formatBytes(report.original_size) }}</span>
        <span>{{ formatBytes(report.deduplicated_size) }} dedup</span>
        <span>{{ report.files_processed }} files</span>
      </template>
      <span>{{ formatDuration(report.duration_secs) }}</span>
    </span>
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
  <div
    v-if="expanded && hasDetail"
    class="agent-row agent-row-detail"
  >
    <div
      v-if="warnings.length > 0"
      class="detail-block"
    >
      <strong class="detail-label detail-label--warning">Warnings</strong>
      <pre class="detail-output">{{ warnings.join('\n') }}</pre>
    </div>
    <div
      v-if="report.error_message && status !== 'warning'"
      class="detail-block"
    >
      <strong class="detail-label detail-label--danger">Error</strong>
      <pre class="detail-output detail-output--danger">{{ report.error_message }}</pre>
    </div>
  </div>
</template>

<style scoped>
.row-schedule-link {
  color: var(--text-muted);
}

.row-schedule-link:hover {
  color: var(--accent);
  text-decoration: underline;
}

.agent-row-detail {
  flex-direction: column;
  align-items: stretch;
  gap: 0.5rem;
}

.agent-row-detail:hover {
  background: none;
}

.detail-block {
  min-width: 0;
}

.detail-label {
  font-size: var(--fs-2xs);
  font-weight: 600;
  display: block;
  margin-bottom: 0.25rem;
}

.detail-label--warning {
  color: var(--warning);
}

.detail-label--danger {
  color: var(--danger);
}

.detail-output {
  font-size: var(--fs-2xs);
  background: var(--bg-code);
  border-radius: var(--radius-sm);
  padding: 0.5rem;
  margin: 0;
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 12rem;
}

.detail-output--danger {
  color: var(--danger);
}
</style>
