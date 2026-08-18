<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import BaseSpinner from './BaseSpinner.vue'
import { formatDateShort, formatDuration, formatBytes } from '../utils/format'
import { backupStatusBadgeClass } from '../utils/badge'
import type { ReportRow } from '../types/report'
import type { AgentRow } from '../types/agent'

/**
 * The backup report table for one schedule.
 */
const props = defineProps<{
  reports: ReportRow[]
  loading: boolean
  error: string | null
  /** Agents by id, so a report can name its host rather than show `#3`. */
  agents: Map<number, AgentRow>
}>()

function hostLabel(agentId: number | null): string {
  const agent = props.agents.get(agentId ?? 0)
  return agent?.display_name ?? agent?.hostname ?? `#${agentId ?? 0}`
}
</script>

<template>
  <div
    v-if="loading"
    class="loading-row"
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
    v-else-if="reports.length === 0"
    class="state-msg state-msg--inline"
  >
    No backup reports found for this schedule.
  </div>
  <div
    v-else
    class="table-wrap"
  >
    <table class="data-table data-table--compact">
      <thead>
        <tr>
          <th>Started</th>
          <th>Host</th>
          <th>Status</th>
          <th>Duration</th>
          <th>Size</th>
          <th>Error</th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="r in reports"
          :key="r.id"
          class="report-row"
        >
          <td class="cell-ts">{{ formatDateShort(r.started_at) }}</td>
          <td class="cell-host">{{ hostLabel(r.agent_id) }}</td>
          <td>
            <span
              class="badge"
              :class="backupStatusBadgeClass(r.status)"
              >{{ r.status }}</span
            >
          </td>
          <td class="cell-dur">{{ formatDuration(r.duration_secs) }}</td>
          <td class="cell-size">{{ formatBytes(r.original_size) }}</td>
          <td class="cell-truncate">
            <span
              v-if="r.error_message"
              class="error-snippet"
              :title="r.error_message"
              >{{ r.error_message.slice(0, 80)
              }}{{ r.error_message.length > 80 ? '\u2026' : '' }}</span
            >
            <span
              v-else
              class="no-error"
              >—</span
            >
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>

<style scoped>
.report-row td {
  padding: 0.55rem 0.75rem;
  border-bottom: 1px solid var(--border-subtle);
  vertical-align: middle;
}

.report-row:last-child td {
  border-bottom: none;
}

.cell-dur,
.error-snippet {
  font-family: var(--mono);
  font-size: var(--fs-xs);
  color: var(--danger);
  word-break: break-all;
}

.no-error {
  color: var(--text-muted);
}
</style>
