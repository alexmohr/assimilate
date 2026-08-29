<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { getRunEvents } from '../api/runs'
import { formatBytes, formatDuration, relativeTime } from '../utils/format'
import { normalizeBackupStatus } from '../utils/backupStatus'
import { backupStatusBadgeClass } from '../utils/badge'
import { logger } from '../utils/logger'
import BaseSpinner from './BaseSpinner.vue'
import RunEventTimeline from './RunEventTimeline.vue'
import type { ReportRow } from '../types/report'
import type { RunEventResponse } from '../types/generated'

/**
 * One backup run, as a single line, following the same grammar as
 * AgentScheduleRow. Previously each run rendered as a four-line card, so a
 * month of history did not fit on a screen; one line per run does.
 *
 * A run that produced an archive links to it; a warned or failed run also
 * expands its output in place, which is what you came to the page to read.
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

const runEvents = ref<RunEventResponse[]>([])
const loadingEvents = ref(false)
// Most runs have wake/start disabled and record no power-management events at
// all, so an empty result is the common case, not a failure - distinguished
// from a fetch that errored (surfaced below) so expanding a run always
// shows *something* rather than a toggle that silently does nothing.
const eventsFetched = ref(false)
const eventsError = ref(false)

watch(
  () => props.expanded,
  (expanded) => {
    const runId = props.report.run_id
    if (!expanded || !runId || eventsFetched.value || loadingEvents.value) return
    loadingEvents.value = true
    eventsError.value = false
    getRunEvents(runId, props.report.agent_id, props.report.repo_id)
      .then((events) => {
        runEvents.value = events
        eventsFetched.value = true
      })
      .catch((e: unknown) => {
        logger.error('failed to load run events', e)
        eventsError.value = true
      })
      .finally(() => {
        loadingEvents.value = false
      })
  },
  // A report can arrive already expanded (a deep link pins a specific run),
  // and that first render deserves its timeline fetched too, not just a
  // later toggle.
  { immediate: true },
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
      <strong class="group-label group-label--warning detail-label">Warnings</strong>
      <pre class="detail-output">{{ warnings.join('\n') }}</pre>
    </div>
    <div
      v-if="report.error_message && status !== 'warning'"
      class="detail-block"
    >
      <strong class="group-label group-label--danger detail-label">Error</strong>
      <pre class="detail-output detail-output--danger">{{ report.error_message }}</pre>
    </div>
    <div
      v-if="report.run_id && (loadingEvents || eventsFetched || eventsError)"
      class="detail-block"
    >
      <strong class="group-label detail-label">Power management</strong>
      <div
        v-if="loadingEvents"
        class="loading-row"
      >
        <BaseSpinner size="sm" />
      </div>
      <p
        v-else-if="eventsError"
        class="field-hint field-hint-error"
      >
        Couldn't load power-management activity for this run.
      </p>
      <p
        v-else-if="runEvents.length === 0"
        class="field-hint"
      >
        No power-management activity for this run.
      </p>
      <RunEventTimeline
        v-else
        :events="runEvents"
        :source-label="report.hostname ?? 'source'"
        :repository-label="report.repo_name ?? 'repository'"
      />
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
  gap: var(--space-4);
}

.agent-row-detail:hover {
  background: none;
}

.detail-block {
  min-width: 0;
}

/* The shared label plus the space this block wants under it. */
.detail-label {
  margin-bottom: var(--space-2);
}

.detail-output {
  font-size: var(--fs-2xs);
  background: var(--bg-code);
  border-radius: var(--radius-sm);
  padding: var(--space-4);
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
