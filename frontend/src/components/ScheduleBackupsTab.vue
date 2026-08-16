<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { computed } from 'vue'
import BaseSpinner from './BaseSpinner.vue'
import ArchiveBrowserLayout from './ArchiveBrowserLayout.vue'
import ArchiveFileBrowser from './ArchiveFileBrowser.vue'
import { formatDateShort, formatBytes } from '../utils/format'
import { normalizeBackupStatus } from '../utils/backupStatus'
import type { ArchiveEntry } from '../composables/useArchiveBrowser'
import type { ReportRow } from '../types/report'
import type { AgentRow } from '../types/agent'

/**
 * The archive browser for one schedule: its successful runs on the left, the
 * selected run's file tree on the right. See
 * docs/contributing/ui-design-audit.md (F-24).
 */
const props = defineProps<{
  /** Every report for this schedule; only the archived ones are listed. */
  reports: ReportRow[]
  loading: boolean
  error: string | null
  agents: Map<number, AgentRow>
  repoId: number | null
}>()

/** The view owns the selection so it can clear it when the route changes. */
const selected = defineModel<ReportRow | null>('selected', { required: true })

const archives = computed<ReportRow[]>(() =>
  props.reports
    .filter((r) => {
      if (r.archive_name == null) return false
      const status = normalizeBackupStatus(r.status)
      return status === 'success' || status === 'warning'
    })
    .sort((a, b) => new Date(b.started_at).getTime() - new Date(a.started_at).getTime()),
)

/** The file browser wants an archive, not the report that produced it. */
const selectedArchive = computed<ArchiveEntry | null>(() => {
  const r = selected.value
  if (!r || r.archive_name == null) return null
  const hostname = props.agents.get(r.agent_id ?? 0)?.hostname ?? r.hostname ?? ''
  return {
    name: r.archive_name,
    start: r.started_at,
    hostname,
    comment: '',
    original_size: r.original_size,
    deduplicated_size: r.deduplicated_size,
    matched: true,
    agent_hostname: hostname,
  }
})

function hostLabel(agentId: number | null): string {
  const agent = props.agents.get(agentId ?? 0)
  return agent?.display_name ?? agent?.hostname ?? `#${agentId ?? 0}`
}

function select(report: ReportRow): void {
  selected.value = report
}
</script>

<template>
  <div
    v-if="loading"
    class="reports-loading"
  >
    <BaseSpinner size="sm" />
  </div>
  <div
    v-else-if="error"
    class="error-banner"
  >
    {{ error }}
  </div>
  <div
    v-else-if="archives.length === 0"
    class="empty-state"
  >
    No backup archives found for this schedule.
  </div>
  <ArchiveBrowserLayout
    v-else
    narrow-list
  >
    <template #list>
      <div class="panel panel--sectioned backups-list-panel">
        <div class="panel-header">
          <span class="panel-title">Archives</span>
        </div>
        <table class="data-table data-table--compact">
          <thead>
            <tr>
              <th>Archive</th>
              <th>Host</th>
              <th>Date</th>
              <th>Size</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="r in archives"
              :key="r.id"
              class="archive-row"
              :class="{ selected: selected?.id === r.id }"
              @click="select(r)"
            >
              <td class="cell-archive-name">{{ r.archive_name }}</td>
              <td class="cell-host">{{ hostLabel(r.agent_id) }}</td>
              <td class="cell-date">{{ formatDateShort(r.started_at) }}</td>
              <td class="cell-size">{{ formatBytes(r.original_size) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </template>
    <template #browser>
      <div class="panel panel--sectioned backups-browser-panel">
        <ArchiveFileBrowser
          :repo-id="repoId"
          :archive="selectedArchive"
        />
      </div>
    </template>
  </ArchiveBrowserLayout>
</template>

<style scoped>
.reports-loading {
  padding: 2rem 0;
  display: flex;
  justify-content: center;
}

.error-banner {
  background: var(--danger-subtle);
  border: 1px solid var(--danger);
  color: var(--danger);
  padding: 0.75rem 1rem;
  border-radius: var(--radius-sm);
  margin-bottom: 1rem;
  font-size: var(--fs-base);
}

.empty-state {
  color: var(--text-muted);
  font-size: var(--fs-base);
  padding: 1rem 0;
}

.backups-list-panel .panel-header {
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--border);
}

.backups-list-panel .panel-title {
  font-size: var(--fs-xs);
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
}

.backups-browser-panel {
  min-height: 300px;
}

.archive-row td {
  border-bottom: 1px solid var(--border-subtle);
}

.archive-row:last-child td {
  border-bottom: none;
}

/* Hover used to come from a blanket `.data-table tr` rule that also made the
   non-clickable log rows look interactive. Scoped to the rows that really are
   clickable, and given the selected state it never had. */
.archive-row {
  cursor: pointer;
  transition: background var(--duration-fast);
}

.archive-row:hover {
  background: var(--bg-hover);
}

.archive-row.selected {
  background: var(--accent-subtle);
}

.cell-archive-name {
  font-family: var(--mono);
  font-size: var(--fs-xs);
  max-width: 140px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-primary);
}

.cell-host {
  font-weight: 500;
  color: var(--text-primary);
}

.cell-date {
  white-space: nowrap;
  font-size: var(--fs-xs);
  color: var(--text-muted);
}

.cell-size {
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
  color: var(--text-secondary);
}
</style>
