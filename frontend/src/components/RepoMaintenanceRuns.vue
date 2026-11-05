<!--
SPDX-License-Identifier: Apache-2.0
SPDX-FileCopyrightText: 2026 Alexander Mohr
-->

<script setup lang="ts">
import { formatBytes, formatDate, formatDuration } from '../utils/format'
import { badgeClass } from '../utils/badge'
import { maintenanceOutcomeBadge, maintenanceRunSeconds } from '../utils/appendOnly'
import type { MaintenanceRunResponse, PendingDeletionResponse } from '../types/generated'

/**
 * The two tables under the maintenance schedule: archives waiting to be
 * removed by hand-requested deletion, and what past runs did. Retention-driven
 * deletions are not listed in advance - they are decided when a run starts.
 */
defineProps<{
  pending: PendingDeletionResponse[]
  runs: MaintenanceRunResponse[]
  busy: boolean
}>()

const emit = defineEmits<{ keep: [archiveName: string] }>()
</script>

<template>
  <div class="panel">
    <h3 class="panel-title">Pending deletions</h3>
    <p
      v-if="pending.length === 0"
      class="muted runs-empty"
    >
      Nothing is waiting. Archives you delete here are queued until the next maintenance run.
    </p>
    <div
      v-else
      class="table-wrap"
    >
      <table class="data-table">
        <thead>
          <tr>
            <th>Archive</th>
            <th>Requested by</th>
            <th>Requested at</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="item in pending"
            :key="item.archive_name"
          >
            <td class="cell-mono">{{ item.archive_name }}</td>
            <td>{{ item.requested_by ?? 'Unknown' }}</td>
            <td class="cell-date">{{ formatDate(item.requested_at) }}</td>
            <td>
              <button
                type="button"
                class="btn btn-xs btn-ghost"
                :disabled="busy"
                :aria-label="`Keep ${item.archive_name}`"
                @click="emit('keep', item.archive_name)"
              >
                Keep
              </button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>

  <div class="panel">
    <h3 class="panel-title">Recent runs</h3>
    <p
      v-if="runs.length === 0"
      class="muted runs-empty"
    >
      No maintenance run yet.
    </p>
    <div
      v-else
      class="table-wrap"
    >
      <table class="data-table">
        <thead>
          <tr>
            <th>Started</th>
            <th>Result</th>
            <th>Archives removed</th>
            <th>Space freed</th>
            <th>Took</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="run in runs"
            :key="run.id"
          >
            <td class="cell-date">{{ formatDate(run.started_at) }}</td>
            <td>
              <span
                class="badge"
                :class="badgeClass(maintenanceOutcomeBadge(run).tone)"
              >
                {{ maintenanceOutcomeBadge(run).label }}
              </span>
              <span
                v-if="run.detail"
                class="muted run-detail"
              >
                {{ run.detail }}
              </span>
            </td>
            <td>{{ run.archives_removed }}</td>
            <td class="cell-size">{{ formatBytes(run.bytes_freed) }}</td>
            <td>
              {{
                maintenanceRunSeconds(run) === null
                  ? 'Running'
                  : formatDuration(maintenanceRunSeconds(run) ?? 0)
              }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>

<style scoped>
.runs-empty {
  margin: var(--space-4) 0 0;
}

.run-detail {
  margin-left: var(--space-3);
}
</style>
