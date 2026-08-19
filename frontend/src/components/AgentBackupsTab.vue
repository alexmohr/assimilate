<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import BaseSegmented, { type SegmentedOption } from './BaseSegmented.vue'
import AgentBackupRow from './AgentBackupRow.vue'
import { normalizeBackupStatus } from '../utils/backupStatus'
import type { ReportRow } from '../types/report'

export type BackupFilter = 'all' | 'success' | 'warning' | 'failed'

const props = defineProps<{
  reports: readonly ReportRow[]
  filter: BackupFilter
  sortAscending: boolean
  expandedReportId: number | null
  highlightedArchiveName: string | undefined
  pinnedReportId: number | null
}>()

const emit = defineEmits<{
  'update:filter': [value: BackupFilter]
  'update:sortAscending': [value: boolean]
  toggle: [report: ReportRow]
  open: [report: ReportRow]
}>()

function countOf(status: BackupFilter): number {
  if (status === 'all') return props.reports.length
  return props.reports.filter((r) => normalizeBackupStatus(r.status) === status).length
}

/**
 * Counts live in the labels so the filter doubles as a summary - the shape of
 * a month of history without having to click through all four segments.
 */
const filterOptions = computed<SegmentedOption<BackupFilter>[]>(() => [
  { value: 'all', label: `All ${countOf('all')}` },
  { value: 'success', label: `Success ${countOf('success')}` },
  { value: 'warning', label: `Warning ${countOf('warning')}` },
  { value: 'failed', label: `Failed ${countOf('failed')}` },
])

const visible = computed(() => {
  const filtered =
    props.filter === 'all'
      ? [...props.reports]
      : props.reports.filter((r) => normalizeBackupStatus(r.status) === props.filter)
  return filtered.sort((a, b) => {
    const diff = new Date(b.finished_at).getTime() - new Date(a.finished_at).getTime()
    return props.sortAscending ? -diff : diff
  })
})
</script>

<template>
  <div class="backups-tab">
    <div class="toolbar backups-toolbar">
      <BaseSegmented
        :model-value="filter"
        :options="filterOptions"
        label="Filter backups by status"
        @update:model-value="emit('update:filter', $event)"
      />
      <button
        class="btn btn-sm btn-ghost backups-sort"
        type="button"
        @click="emit('update:sortAscending', !sortAscending)"
      >
        {{ sortAscending ? 'Oldest first' : 'Newest first' }}
      </button>
    </div>

    <div
      v-if="visible.length === 0"
      class="state-msg"
    >
      {{
        reports.length === 0
          ? 'No backup reports available.'
          : 'No backups match the current filter.'
      }}
    </div>
    <div
      v-else
      class="rows"
    >
      <AgentBackupRow
        v-for="r in visible"
        :key="r.id"
        :report="r"
        :expanded="expandedReportId === r.id"
        :highlighted="r.archive_name === highlightedArchiveName || r.id === pinnedReportId"
        show-detail
        @toggle="emit('toggle', r)"
        @open="emit('open', r)"
      />
    </div>
  </div>
</template>

<style scoped>
.backups-tab {
  display: flex;
  flex-direction: column;
  gap: var(--space-5);
}

.backups-toolbar {
  justify-content: flex-start;
}

.backups-sort {
  margin-left: auto;
}
</style>
