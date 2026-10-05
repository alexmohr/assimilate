<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import { normalizeBackupStatus } from '../utils/backupStatus'
import BaseSpinner from './BaseSpinner.vue'
import RunEventTimeline from './RunEventTimeline.vue'
import type { ReportRow } from '../types/report'
import type { RunEventResponse } from '../types/generated'

/**
 * The expanded body under one run's row: its warnings, the error that ended
 * it (or, for a skipped run, why it never started), and its power-management
 * timeline. Shared by the host's backup rows and the schedule Overview's run
 * detail, which render the same report and should read the same.
 */
const props = defineProps<{
  report: ReportRow
  runEvents: RunEventResponse[]
  loadingEvents: boolean
  eventsFetched: boolean
  eventsError: boolean
  /**
   * Indents the body by the row's own padding only, for a row that already
   * sits inside a run's detail and has no leading column to line up with.
   */
  flush?: boolean
}>()

const status = computed(() => normalizeBackupStatus(props.report.status))
const warnings = computed(() => props.report.warnings ?? [])

/**
 * A skipped run never ran, so its message is the reason it was held back - a
 * dependency that did not answer - rather than an error. It takes the warning
 * tone the skip badge already has.
 */
const skipped = computed(() => status.value === 'skipped')
</script>

<template>
  <div
    class="agent-row agent-row-detail"
    :class="{ 'run-report-detail--flush': flush }"
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
      <strong
        v-if="skipped"
        class="group-label group-label--warning detail-label"
        >Skipped</strong
      >
      <strong
        v-else
        class="group-label group-label--danger detail-label"
        >Error</strong
      >
      <pre
        class="detail-output"
        :class="{ 'detail-output--danger': !skipped }"
        >{{ report.error_message }}</pre
      >
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
.agent-row-detail {
  flex-direction: column;
  align-items: stretch;
  gap: var(--space-4);
}

.agent-row-detail:hover {
  background: none;
}

/* Only the row's own padding: the schedule's run detail lists hosts with no
   leading column to line the body up under. */
.run-report-detail--flush {
  padding-left: var(--space-5);
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
